//! HDR transfer and gamut math shared by the presentation encoders and the
//! FFmpeg metadata builders.
//!
//! Linear 1.0 is BT.2408 HDR Reference White (203 cd/m²), so SDR content keeps
//! its brightness and values above 1.0 become HDR range.
//!
//! This is the CPU reference; `blit.wgsl` mirrors it, checked by code-value tests.

use crate::engine::value::render::HDR_REFERENCE_WHITE_NITS;

// ST 2084 (PQ) constants, named as in the standard.
const M1: f32 = 2610.0 / 16384.0;
const M2: f32 = 2523.0 / 4096.0 * 128.0;
const C1: f32 = 3424.0 / 4096.0;
const C2: f32 = 2413.0 / 4096.0 * 32.0;
const C3: f32 = 2392.0 / 4096.0 * 32.0;

/// Absolute luminance ceiling of the PQ signal, in cd/m².
pub const PQ_MAX_NITS: f32 = 10_000.0;

/// SMPTE ST 2084 inverse EOTF: absolute luminance in cd/m² to a `[0, 1]` PQ code.
#[must_use]
pub fn pq_from_nits(nits: f32) -> f32 {
    let y = (nits / PQ_MAX_NITS).clamp(0.0, 1.0);
    let ym = y.powf(M1);
    ((C1 + C2 * ym) / (1.0 + C3 * ym)).powf(M2)
}

/// SMPTE ST 2084 EOTF: a `[0, 1]` PQ code back to absolute luminance in cd/m².
///
/// Only used by tests and diagnostics.
#[must_use]
pub fn nits_from_pq(code: f32) -> f32 {
    let c = code.clamp(0.0, 1.0).powf(1.0 / M2);
    let num = (c - C1).max(0.0);
    let den = C2 - C3 * c;
    if den <= 0.0 {
        return PQ_MAX_NITS;
    }
    (num / den).powf(1.0 / M1) * PQ_MAX_NITS
}

/// Encode a scene-linear value to a PQ code, anchored at reference white and
/// clamped to the output's configured peak.
///
/// `peak_nits` is the per-output setting. Values beyond it clamp instead of
/// rolling off, so mastering metadata built from the same setting stays accurate.
#[must_use]
pub fn pq_from_linear(linear: f32, peak_nits: f32) -> f32 {
    let nits = (linear.max(0.0) * HDR_REFERENCE_WHITE_NITS).min(peak_nits);
    pq_from_nits(nits)
}

/// Largest scene-linear value an output can carry before clamping.
///
/// The range an HDR output transform targets, in place of SDR's `[0, 1]`.
/// Always greater than 1.0, since [`HDR_PEAK_NITS_MIN`] is above reference white.
///
/// [`HDR_PEAK_NITS_MIN`]: crate::engine::value::render::HDR_PEAK_NITS_MIN
#[must_use]
pub fn linear_headroom(peak_nits: f32) -> f32 {
    peak_nits / HDR_REFERENCE_WHITE_NITS
}

// ── HLG (ARIB STD-B67 / BT.2100) ────────────────────────────────────

// OETF constants from BT.2100.
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_7;

/// Scene-linear HLG value that encodes to 75% signal.
///
/// BT.2408 puts HDR Reference White at 75% HLG signal, so linear 1.0 maps here.
/// Derived by inverting the OETF at `E' = 0.75`; checked by a test.
pub const HLG_REFERENCE_WHITE_SIGNAL: f32 = 0.264_963_1;

/// BT.2100 HLG OETF: scene light in `[0, 1]` to a signal in `[0, 1]`.
#[must_use]
pub fn hlg_from_scene(e: f32) -> f32 {
    let e = e.clamp(0.0, 1.0);
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt()
    } else {
        HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
    }
}

/// Encode a scene-linear value to an HLG signal, anchored at reference white.
///
/// HLG is relative, so there is no peak setting: `1.0` is the display's nominal
/// peak. Values past the top of the range clamp.
#[must_use]
pub fn hlg_from_linear(linear: f32) -> f32 {
    hlg_from_scene(linear.max(0.0) * HLG_REFERENCE_WHITE_SIGNAL)
}

/// Largest scene-linear value HLG can carry before clamping.
#[must_use]
pub fn hlg_linear_headroom() -> f32 {
    1.0 / HLG_REFERENCE_WHITE_SIGNAL
}

/// Linear BT.709 to linear BT.2020 primaries, row-major.
///
/// Apply to linear values, before the transfer encode.
pub const BT2020_FROM_REC709: [[f32; 3]; 3] = [
    [0.627_403_9, 0.329_283_04, 0.043_313_06],
    [0.069_097_29, 0.919_540_1, 0.011_362_63],
    [0.016_391_44, 0.088_013_3, 0.895_595_3],
];

/// Convert a linear Rec.709 triple to linear BT.2020 primaries.
#[must_use]
pub fn bt2020_from_rec709(rgb: [f32; 3]) -> [f32; 3] {
    let m = BT2020_FROM_REC709;
    [
        m[0][0] * rgb[0] + m[0][1] * rgb[1] + m[0][2] * rgb[2],
        m[1][0] * rgb[0] + m[1][1] * rgb[1] + m[1][2] * rgb[2],
        m[2][0] * rgb[0] + m[2][1] * rgb[1] + m[2][2] * rgb[2],
    ]
}

/// `x265` `master-display` string for BT.2020 primaries with a D65 white point.
///
/// Chromaticity is in 0.00002 units and luminance in 0.0001 cd/m², per ST 2086.
/// Luminance comes from the output's configured peak.
#[must_use]
pub fn master_display_string(peak_nits: u16) -> String {
    let max = u32::from(peak_nits) * 10_000;
    format!("G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L({max},1)")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rounded to the 10-bit code an encoder writes.
    fn code10(pq: f32) -> u32 {
        (pq * 1023.0).round() as u32
    }

    #[test]
    fn pq_matches_the_reference_luminance_table() {
        // Reference PQ code values.
        for (nits, expected) in [
            (100.0, 520),
            (203.0, 594),
            (600.0, 712),
            (1000.0, 769),
            (4000.0, 923),
            (10_000.0, 1023),
        ] {
            assert_eq!(code10(pq_from_nits(nits)), expected, "{nits} cd/m²");
        }
    }

    #[test]
    fn linear_one_is_reference_white_at_every_peak() {
        // Display white stays display white at any configured peak.
        for peak in [600.0, 1000.0, 1500.0, 4000.0] {
            assert_eq!(code10(pq_from_linear(1.0, peak)), 594, "peak {peak}");
        }
    }

    #[test]
    fn linear_headroom_maps_exactly_to_the_configured_peak() {
        for peak in [600.0, 1000.0, 4000.0] {
            let top = linear_headroom(peak);
            assert_eq!(
                code10(pq_from_linear(top, peak)),
                code10(pq_from_nits(peak))
            );
        }
    }

    #[test]
    fn values_above_headroom_clamp_to_the_peak() {
        let peak = 1000.0;
        let at_peak = pq_from_linear(linear_headroom(peak), peak);
        assert!((pq_from_linear(50.0, peak) - at_peak).abs() < 1e-6);
    }

    #[test]
    fn negative_linear_encodes_to_black() {
        assert_eq!(code10(pq_from_linear(-0.5, 1000.0)), 0);
    }

    #[test]
    fn pq_round_trips_through_the_eotf() {
        for nits in [0.1_f32, 1.0, 100.0, 203.0, 1000.0, 4000.0] {
            let back = nits_from_pq(pq_from_nits(nits));
            assert!(
                (back - nits).abs() / nits < 1e-3,
                "{nits} came back as {back}"
            );
        }
    }

    #[test]
    fn rec709_white_maps_to_bt2020_white() {
        // Both spaces share a D65 white point, so white is a fixed point. Catches
        // a transposed or mis-scaled matrix.
        let white = bt2020_from_rec709([1.0, 1.0, 1.0]);
        for channel in white {
            assert!((channel - 1.0).abs() < 1e-4, "white drifted to {white:?}");
        }
    }

    #[test]
    fn bt2020_rows_are_normalized() {
        for row in BT2020_FROM_REC709 {
            let sum: f32 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "row {row:?} sums to {sum}");
        }
    }

    #[test]
    fn rec709_primaries_land_inside_the_wider_gamut() {
        // Rec.709 content does not fill BT.2020: its red stays short of BT.2020 red.
        let red = bt2020_from_rec709([1.0, 0.0, 0.0]);
        assert!(red[0] < 1.0, "Rec.709 red should not saturate BT.2020 red");
        assert!(red[1] > 0.0 && red[2] > 0.0, "expected positive crosstalk");
    }

    #[test]
    fn master_display_luminance_tracks_the_configured_peak() {
        assert!(master_display_string(1000).ends_with("L(10000000,1)"));
        assert!(master_display_string(4000).ends_with("L(40000000,1)"));
        assert!(master_display_string(600).starts_with("G(8500,39850)B(6550,2300)"));
    }

    #[test]
    fn hlg_puts_reference_white_at_seventy_five_percent() {
        // ITU-R BT.2408: HDR Reference White is 75% HLG signal, as for PQ.
        assert!(
            (hlg_from_linear(1.0) - 0.75).abs() < 1e-4,
            "{}",
            hlg_from_linear(1.0)
        );
    }

    #[test]
    fn hlg_headroom_reaches_full_signal_and_clamps_past_it() {
        let top = hlg_linear_headroom();
        assert!((hlg_from_linear(top) - 1.0).abs() < 1e-4);
        // Relative transfer, so the top of the range clamps.
        assert!((hlg_from_linear(top * 4.0) - 1.0).abs() < 1e-6);
        assert!((top - 3.774).abs() < 0.01, "headroom drifted to {top}");
    }

    #[test]
    fn hlg_oetf_is_continuous_at_the_segment_join() {
        // The curve switches from sqrt to log at E = 1/12; a mismatch there shows
        // as a visible step in shadows.
        let join = 1.0_f32 / 12.0;
        let below = hlg_from_scene(join - 1e-5);
        let above = hlg_from_scene(join + 1e-5);
        assert!((below - above).abs() < 1e-3, "{below} vs {above}");
        assert!((hlg_from_scene(join) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn hlg_black_is_black_and_negatives_do_not_escape() {
        assert!(hlg_from_linear(0.0).abs() < 1e-6);
        assert!(hlg_from_linear(-2.0).abs() < 1e-6);
    }

    #[test]
    fn hlg_gives_less_headroom_than_pq_at_a_thousand_nits() {
        // Expected: HLG spends part of its range on the display-side OOTF.
        assert!(hlg_linear_headroom() < linear_headroom(1000.0));
    }

    #[test]
    fn every_requestable_peak_leaves_headroom_over_display_white() {
        // A peak at or below reference white would make HDR dimmer than SDR and
        // disagree with the shader, which clamps headroom at 1.0.
        use crate::engine::value::render::{HDR_PEAK_NITS_MAX, HDR_PEAK_NITS_MIN};
        for peak in [HDR_PEAK_NITS_MIN, 600, 1000, 4000, HDR_PEAK_NITS_MAX] {
            let headroom = linear_headroom(f32::from(peak));
            assert!(
                headroom > 1.0,
                "peak {peak} gives headroom {headroom}, at or below display white"
            );
        }
    }
}
