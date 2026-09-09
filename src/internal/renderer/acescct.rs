//! `ACEScct`, the log encoding a scene-referred look LUT is authored against.
//!
//! A creative grade cannot be applied to scene-linear values directly. Linear
//! light spends most of its code range on highlights, so a 3D LUT indexed by it
//! would resolve shadows into almost no lattice points. ACES specifies `ACEScct`
//! for exactly this: a log curve with a linear toe near black, which is what
//! look transforms (LMTs) are authored in.
//!
//! This is not a Varda invention and must not become one. The constants are from
//! the ACES specification so a `.cube` authored in Resolve or Nuke against
//! `ACEScct` lands where its author intended.
//!
//! See /spec/hdr-color-management.md Decision 5. `lut.wgsl` carries the WGSL
//! mirror, held to these values by tests rather than by inspection.

/// Slope of the linear toe.
const A: f32 = 10.540_237;
/// Intercept of the linear toe.
const B: f32 = 0.072_905_53;
/// Linear value where the toe meets the log segment (2^-7).
const LINEAR_BREAK: f32 = 0.007_812_5;
/// Encoded value at the break, `A * LINEAR_BREAK + B`.
const ENCODED_BREAK: f32 = 0.155_251_14;
/// Log segment offset and scale, from the ACES specification.
const LOG_OFFSET: f32 = 9.72;
const LOG_SCALE: f32 = 17.52;

/// Largest linear value `ACEScct` represents, the half-float maximum.
pub const MAX_LINEAR: f32 = 65504.0;

/// Scene-linear to `ACEScct`.
///
/// Values at or below the break use the linear toe, which is what keeps shadow
/// detail addressable by a LUT lattice. Negatives clamp to the toe rather than
/// producing NaN from `log2`.
#[must_use]
pub fn from_linear(linear: f32) -> f32 {
    if linear <= LINEAR_BREAK {
        A * linear.max(0.0) + B
    } else if linear < MAX_LINEAR {
        (linear.log2() + LOG_OFFSET) / LOG_SCALE
    } else {
        (MAX_LINEAR.log2() + LOG_OFFSET) / LOG_SCALE
    }
}

/// `ACEScct` back to scene-linear.
#[must_use]
pub fn to_linear(encoded: f32) -> f32 {
    if encoded <= ENCODED_BREAK {
        (encoded - B) / A
    } else {
        (encoded * LOG_SCALE - LOG_OFFSET).exp2()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toe_and_the_log_segment_meet_without_a_step() {
        // A discontinuity here shows as a visible band in shadows, exactly where
        // the toe exists to help.
        let below = from_linear(LINEAR_BREAK - 1e-6);
        let above = from_linear(LINEAR_BREAK + 1e-6);
        assert!(
            (below - above).abs() < 1e-4,
            "toe {below} against log {above}"
        );
        assert!((from_linear(LINEAR_BREAK) - ENCODED_BREAK).abs() < 1e-5);
    }

    #[test]
    fn encoding_round_trips_across_the_whole_range() {
        // Both segments, and the values a VJ pipeline actually produces: deep
        // shadows, display white, and the highlights above it that HDR keeps.
        for linear in [
            0.0,
            0.0001,
            0.001,
            0.007_812_5,
            0.05,
            0.18,
            1.0,
            4.0,
            16.0,
            1000.0,
        ] {
            let back = to_linear(from_linear(linear));
            let tolerance = (linear * 1e-3).max(1e-6);
            assert!(
                (back - linear).abs() < tolerance,
                "{linear} came back as {back}"
            );
        }
    }

    #[test]
    fn eighteen_percent_grey_lands_where_aces_says() {
        // The anchor a colorist checks first. ACEScct puts scene-linear 0.18 at
        // roughly 0.4135.
        let mid = from_linear(0.18);
        assert!((mid - 0.413_6).abs() < 1e-3, "mid grey encoded to {mid}");
    }

    #[test]
    fn negatives_clamp_rather_than_producing_nan() {
        // Blend modes can push a channel below zero before the grade sees it.
        let encoded = from_linear(-0.5);
        assert!(encoded.is_finite(), "got {encoded}");
        assert!(
            (encoded - B).abs() < 1e-6,
            "negatives should land on the toe"
        );
    }

    #[test]
    fn the_encoding_is_monotonic() {
        // A non-monotonic shaper would reorder tones, which no grade could undo.
        let mut previous = f32::NEG_INFINITY;
        for step in 0..2000_u16 {
            let linear = f32::from(step) * 0.01;
            let encoded = from_linear(linear);
            assert!(
                encoded >= previous,
                "encoding dipped at linear {linear}: {encoded} after {previous}"
            );
            previous = encoded;
        }
    }

    #[test]
    fn shadow_detail_gets_far_more_lattice_than_linear_indexing_would() {
        // The reason a shaper exists at all. Across the darkest 1% of a [0,1]
        // signal, ACEScct spends a large share of its output range where linear
        // indexing would spend 1%.
        let shadow_span = from_linear(0.01) - from_linear(0.0);
        assert!(
            shadow_span > 0.1,
            "the toe only gave {shadow_span} of encoded range to the bottom 1%"
        );
    }
}
