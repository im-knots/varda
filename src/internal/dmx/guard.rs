//! The white ceiling guard.
//!
//! Runs before the smoother. Where a fixture's color is bright and desaturated, it raises
//! saturation to a value-dependent floor while preserving hue and the peak channel. Because
//! only the lower channels are pulled down, total output actually drops: the guard reduces
//! glare rather than merely recoloring.
//!
//! Off by default and per rig. Varda is an instrument, not a safety system, and an operator who
//! asked for white gets white. It exists because full-saturated white across a rig is a
//! photosensitivity hazard and a palette-identity failure, and an operator who wants protection
//! from it should not have to build it. See /spec/lighting-routing.md § Guards.

use super::role::Role;
use super::values::RoleValues;

/// Below this value no saturation is required: a dim grey is not glare.
const VALUE_KNEE: f32 = 0.45;
/// Saturation required at full value. Ramps from zero at the knee to this at value 1.
const SAT_FLOOR_AT_FULL: f32 = 0.60;
/// Below this saturation a color is treated as achromatic and adopts the key hue, because its
/// own measured hue is numerical noise.
const ACHROMATIC_EPS: f32 = 0.02;
/// Above this saturation the color's own hue is trusted fully. Between the two, hue is blended
/// from the key toward the measured hue so a fixture dithering across the boundary does not
/// strobe between two saturated hues.
const HUE_TRUST_SAT: f32 = 0.06;

/// Configuration for the guard.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WhiteGuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Hue in turns (0..1) adopted by an achromatic fixture.
    #[serde(default)]
    pub key_hue: f32,
}

impl Default for WhiteGuardConfig {
    fn default() -> Self {
        Self {
            // Off by default: an operator who asked for white gets white.
            enabled: false,
            key_hue: 0.0,
        }
    }
}

/// RGB (0..1) to HSV. Hue in turns.
fn rgb_to_hsv(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    if max <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let span = max - min;
    let saturation = span / max;
    if span <= 0.0 {
        return (0.0, 0.0, max); // achromatic: no hue
    }
    let sextant = if (max - red).abs() < f32::EPSILON {
        ((green - blue) / span).rem_euclid(6.0)
    } else if (max - green).abs() < f32::EPSILON {
        (blue - red) / span + 2.0
    } else {
        (red - green) / span + 4.0
    };
    (sextant / 6.0, saturation, max)
}

/// HSV (hue in turns) to RGB (0..1).
fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> (f32, f32, f32) {
    let sextant = (hue.rem_euclid(1.0)) * 6.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = sextant as u32 % 6;
    let frac = sextant - sextant.floor();
    let down = value * (1.0 - saturation);
    let falling = value * (1.0 - saturation * frac);
    let rising = value * (1.0 - saturation * (1.0 - frac));
    match index {
        0 => (value, rising, down),
        1 => (falling, value, down),
        2 => (down, value, rising),
        3 => (down, falling, value),
        4 => (rising, down, value),
        _ => (value, down, falling),
    }
}

/// Shortest-path interpolation between two hues on the turn circle.
fn lerp_hue(from: f32, to: f32, amount: f32) -> f32 {
    let mut delta = to - from;
    if delta > 0.5 {
        delta -= 1.0;
    } else if delta < -0.5 {
        delta += 1.0;
    }
    (from + delta * amount).rem_euclid(1.0)
}

/// Apply the guard to one fixture's values in place.
///
/// `flash` bypasses the guard: a short operator-commanded white flash is sanctioned. An
/// automatic per-beat flash must not set this, because it fires continuously through a dense
/// drop and would disable the guard for the whole section.
///
/// Only the additive RGB triple is considered and only those three roles are rewritten. White,
/// amber, UV, and the wheel and correction channels are left alone: re-saturating a color-wheel
/// slot or a CTO value is meaningless and produces a wrong gel.
pub fn apply(cfg: WhiteGuardConfig, values: &mut RoleValues, flash: bool) {
    if !cfg.enabled || flash {
        return;
    }
    let (Some(red), Some(green), Some(blue)) = (
        values.get(Role::Red),
        values.get(Role::Green),
        values.get(Role::Blue),
    ) else {
        return;
    };

    let (hue, saturation, value) = rgb_to_hsv(red, green, blue);
    if value <= VALUE_KNEE {
        return; // not bright enough to glare
    }
    let ramp = ((value - VALUE_KNEE) / (1.0 - VALUE_KNEE)).clamp(0.0, 1.0);
    let sat_floor = ramp * SAT_FLOOR_AT_FULL;
    if saturation >= sat_floor {
        return; // already colorful enough
    }

    let trust = ((saturation - ACHROMATIC_EPS) / (HUE_TRUST_SAT - ACHROMATIC_EPS)).clamp(0.0, 1.0);
    let adopted = lerp_hue(cfg.key_hue, hue, trust);
    let (out_r, out_g, out_b) = hsv_to_rgb(adopted, sat_floor, value);
    values.set(Role::Red, out_r);
    values.set(Role::Green, out_g);
    values.set(Role::Blue, out_b);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WhiteGuardConfig {
        WhiteGuardConfig {
            enabled: true,
            key_hue: 30.0 / 360.0, // warm orange
        }
    }

    fn rgb(r: f32, g: f32, b: f32) -> RoleValues {
        let mut v = RoleValues::new();
        v.set(Role::Red, r);
        v.set(Role::Green, g);
        v.set(Role::Blue, b);
        v
    }

    fn triple(v: &RoleValues) -> (f32, f32, f32) {
        (
            v.get(Role::Red).unwrap(),
            v.get(Role::Green).unwrap(),
            v.get(Role::Blue).unwrap(),
        )
    }

    #[test]
    fn disabled_by_default() {
        assert!(!WhiteGuardConfig::default().enabled);
    }

    #[test]
    fn disabled_guard_changes_nothing() {
        let mut v = rgb(1.0, 1.0, 1.0);
        apply(WhiteGuardConfig::default(), &mut v, false);
        assert_eq!(triple(&v), (1.0, 1.0, 1.0));
    }

    #[test]
    fn pure_white_adopts_the_key_hue() {
        let mut v = rgb(1.0, 1.0, 1.0);
        apply(cfg(), &mut v, false);
        let (r, g, b) = triple(&v);
        assert!(r > b, "warm key hue should leave red above blue: {r} {b}");
        assert!((r - 1.0).abs() < 1e-4, "peak channel preserved, got {r}");
        assert!(g < r && b < g, "must actually be saturated: {r} {g} {b}");
    }

    /// The guard reduces glare rather than merely recoloring: only the lower channels move, so
    /// the total falls.
    #[test]
    fn total_output_drops() {
        let mut v = rgb(1.0, 1.0, 1.0);
        let before: f32 = 3.0;
        apply(cfg(), &mut v, false);
        let (r, g, b) = triple(&v);
        assert!(r + g + b < before, "guard must reduce photon output");
    }

    #[test]
    fn a_bright_pale_tint_keeps_its_own_hue() {
        // Pale cyan: green and blue high, red lower.
        let mut v = rgb(0.86, 1.0, 1.0);
        apply(cfg(), &mut v, false);
        let (r, g, b) = triple(&v);
        assert!(r < g && r < b, "cyan hue must survive: {r} {g} {b}");
        assert!((g.max(b) - 1.0).abs() < 1e-4, "value preserved");
    }

    #[test]
    fn a_dim_grey_is_left_alone() {
        let mut v = rgb(0.3, 0.3, 0.3);
        apply(cfg(), &mut v, false);
        assert_eq!(triple(&v), (0.3, 0.3, 0.3), "a dim grey is not glare");
    }

    #[test]
    fn an_already_saturated_color_is_left_alone() {
        let mut v = rgb(1.0, 0.0, 0.0);
        apply(cfg(), &mut v, false);
        assert_eq!(triple(&v), (1.0, 0.0, 0.0));
    }

    #[test]
    fn an_operator_flash_bypasses_the_guard() {
        let mut v = rgb(1.0, 1.0, 1.0);
        apply(cfg(), &mut v, true);
        assert_eq!(triple(&v), (1.0, 1.0, 1.0));
    }

    /// Re-saturating a wheel slot or a correction channel produces a wrong gel.
    #[test]
    fn non_rgb_roles_are_never_touched() {
        let mut v = rgb(1.0, 1.0, 1.0);
        v.set(Role::White, 1.0);
        v.set(Role::Amber, 0.8);
        v.set(Role::ColorWheel, 0.5);
        v.set(Role::Cto, 0.4);
        v.set(Role::Pan, 0.25);
        apply(cfg(), &mut v, false);
        assert_eq!(v.get(Role::White), Some(1.0));
        assert_eq!(v.get(Role::Amber), Some(0.8));
        assert_eq!(v.get(Role::ColorWheel), Some(0.5));
        assert_eq!(v.get(Role::Cto), Some(0.4));
        assert_eq!(v.get(Role::Pan), Some(0.25));
    }

    #[test]
    fn a_fixture_without_rgb_is_skipped() {
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 1.0);
        v.set(Role::ColorWheel, 0.5);
        apply(cfg(), &mut v, false);
        assert_eq!(v.get(Role::Dimmer), Some(1.0));
    }

    /// A near-white fixture whose saturation dithers across the achromatic boundary must not
    /// strobe between the key hue and its own.
    #[test]
    fn hue_is_continuous_across_the_achromatic_boundary() {
        let sample = |s: f32| {
            // Build a near-white with a small blue-ward tint of saturation `s`.
            let mut v = rgb(1.0 - s, 1.0 - s, 1.0);
            apply(cfg(), &mut v, false);
            triple(&v)
        };
        let below = sample(ACHROMATIC_EPS * 0.9);
        let above = sample(HUE_TRUST_SAT * 1.1);
        let mid = sample(f32::midpoint(ACHROMATIC_EPS, HUE_TRUST_SAT));
        // The midpoint must lie between the two extremes on the red channel rather than
        // jumping past either, which is what a hard switch would produce.
        let (lo, hi) = (below.0.min(above.0), below.0.max(above.0));
        assert!(
            mid.0 >= lo - 1e-3 && mid.0 <= hi + 1e-3,
            "hue jumped at the boundary: below {below:?} mid {mid:?} above {above:?}"
        );
    }

    #[test]
    fn hsv_round_trips() {
        for (r, g, b) in [
            (1.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (0.5, 0.25, 0.75),
            (0.2, 0.2, 0.2),
        ] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r2, g2, b2) = hsv_to_rgb(h, s, v);
            assert!(
                (r - r2).abs() < 1e-4 && (g - g2).abs() < 1e-4 && (b - b2).abs() < 1e-4,
                "{r},{g},{b} -> {r2},{g2},{b2}"
            );
        }
    }

    #[test]
    fn lerp_hue_takes_the_short_way_round() {
        // 0.95 -> 0.05 should pass through 0.0, not back through 0.5.
        let mid = lerp_hue(0.95, 0.05, 0.5);
        assert!(mid > 0.95 || mid < 0.05, "went the long way: {mid}");
    }

    #[test]
    fn black_is_left_alone() {
        let mut v = rgb(0.0, 0.0, 0.0);
        apply(cfg(), &mut v, false);
        assert_eq!(triple(&v), (0.0, 0.0, 0.0));
    }
}
