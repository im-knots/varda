//! Output smoothing in the value domain.
//!
//! Smooths normalized `f32` role values before quantization, never DMX bytes. A 16-bit pan
//! smoothed as two `u8` slots is as steppy as an 8-bit one, which would defeat the whole
//! reason movers are in scope. See /spec/dmx-output.md § C3.
//!
//! The filter form matches /spec/parameter-smoothing.md so the two agree when that spec lands:
//!
//! ```text
//!   alpha = 1 - exp(-dt / tau)
//!   smoothed += alpha * (target - smoothed)
//! ```
//!
//! `tau` is the time constant in seconds, the time to cover about 63% of the remaining
//! distance. The exponential form rather than a fixed per-frame coefficient is what makes the
//! response identical at 30, 60, and 144 fps and under frame drops.

use super::role::{Role, SmoothingClass};
use super::values::RoleValues;

/// Time constants in seconds, per smoothing class.
///
/// Starting values, tuned by eye on a rig. Color carries inertia so a wash reads as a wash;
/// intensity is quicker because it is the channel a performer rides; position is slowest
/// because a head that snaps looks mechanical and abrupt.
const TAU_COLOR: f32 = 0.040;
const TAU_INTENSITY: f32 = 0.030;
const TAU_POSITION: f32 = 0.090;
const TAU_BEAM: f32 = 0.060;

/// Time constants used when a transient is fully engaged.
const TAU_COLOR_FAST: f32 = 0.006;
const TAU_INTENSITY_FAST: f32 = 0.004;

/// Upward jump, in normalized units, at which the transient response is fully engaged.
const TRANSIENT_THRESHOLD: f32 = 0.12;
/// Where the blend from ambient toward fast begins. A hard switch limit-cycles when the delta
/// hovers at the boundary, so the response ramps across `[KNEE, THRESHOLD]` instead.
const TRANSIENT_KNEE: f32 = TRANSIENT_THRESHOLD * 0.5;

/// Convert a time constant to a per-frame coefficient for this frame's `dt`.
#[must_use]
fn alpha(tau: f32, dt: f32) -> f32 {
    if tau <= 0.0 || dt <= 0.0 {
        return 1.0;
    }
    1.0 - (-dt / tau).exp()
}

/// Per-fixture smoothing state.
#[derive(Debug, Clone)]
struct FixtureState {
    smoothed: [f32; Role::COUNT],
    /// Until a role has been seen once, its first value is adopted outright rather than faded
    /// up from zero, so patching a fixture mid-show does not produce a fade-in nobody asked for.
    seen: u32,
}

impl Default for FixtureState {
    fn default() -> Self {
        Self {
            smoothed: [0.0; Role::COUNT],
            seen: 0,
        }
    }
}

/// Smooths role values for a whole rig.
#[derive(Debug, Default)]
pub struct RoleSmoother {
    state: Vec<FixtureState>,
}

impl RoleSmoother {
    #[must_use]
    pub fn new(fixture_count: usize) -> Self {
        Self {
            state: vec![FixtureState::default(); fixture_count],
        }
    }

    /// Resize for a repatched rig, preserving state for fixtures that remain.
    ///
    /// Positional rather than UUID keyed because the caller resizes in patch order; a repatch
    /// that reorders fixtures is a structural change that re-resolves the rig anyway.
    pub fn resize(&mut self, fixture_count: usize) {
        self.state.resize(fixture_count, FixtureState::default());
    }

    /// Smooth one fixture's values in place.
    ///
    /// `values` is the **target on entry and the smoothed result on exit**. The caller supplies
    /// a freshly merged target every frame, which is what the pipeline does: the merge engine
    /// recomputes role values from the looks each tick. Feeding a previous frame's output back
    /// in as the next frame's target stalls the filter, because the target and the state are
    /// then the same number.
    ///
    /// `bypass` syncs internal state to the target without modifying `values`, which is what
    /// blackout needs: dark has to be instant, not a multi-frame fade.
    pub fn tick(&mut self, fixture: usize, values: &mut RoleValues, dt: f32, bypass: bool) {
        let Some(state) = self.state.get_mut(fixture) else {
            return;
        };

        if bypass {
            for (role, v) in values.iter() {
                let i = role.index();
                state.smoothed[i] = v;
                state.seen |= 1 << i;
            }
            return;
        }

        // One transient decision for the whole fixture's color, derived from the largest
        // upward delta across its color roles. Deciding per role skews hue on every attack:
        // for a purple target, red crosses the threshold and snaps while blue advances at
        // nearly the ambient rate, so the fixture reads red on the front of each beat and
        // settles to purple after.
        let mut max_color_rise = 0.0_f32;
        for (role, target) in values.iter() {
            if role.smoothing() == SmoothingClass::Color {
                let i = role.index();
                if state.seen & (1 << i) != 0 {
                    max_color_rise = max_color_rise.max(target - state.smoothed[i]);
                }
            }
        }
        let color_alpha = blended_alpha(TAU_COLOR, TAU_COLOR_FAST, max_color_rise, dt);

        for (role, target) in values.iter().collect::<Vec<_>>() {
            let class = role.smoothing();
            if class == SmoothingClass::Discrete {
                continue;
            }
            let i = role.index();

            // First sight of a role is adopted, not faded into.
            if state.seen & (1 << i) == 0 {
                state.seen |= 1 << i;
                state.smoothed[i] = target;
                continue;
            }

            let a = match class {
                SmoothingClass::Color => color_alpha,
                SmoothingClass::Intensity => {
                    let rise = target - state.smoothed[i];
                    blended_alpha(TAU_INTENSITY, TAU_INTENSITY_FAST, rise, dt)
                }
                // Position and beam never take a transient path. A fast path on a moving head
                // is a snap, which is both ugly and mechanically unkind.
                SmoothingClass::Position => alpha(TAU_POSITION, dt),
                SmoothingClass::Beam => alpha(TAU_BEAM, dt),
                SmoothingClass::Discrete => unreachable!("filtered above"),
            };
            state.smoothed[i] += (target - state.smoothed[i]) * a;
            values.set(role, state.smoothed[i]);
        }
    }
}

/// Blend continuously from the ambient coefficient to the fast one across the transient window.
///
/// Both endpoints are computed from time constants for this frame's `dt`, so the blend is
/// between two frame-rate-independent responses rather than between two raw coefficients.
fn blended_alpha(tau_ambient: f32, tau_fast: f32, rise: f32, dt: f32) -> f32 {
    let ambient = alpha(tau_ambient, dt);
    if rise <= TRANSIENT_KNEE {
        return ambient;
    }
    let fast = alpha(tau_fast, dt);
    let progress =
        ((rise - TRANSIENT_KNEE) / (TRANSIENT_THRESHOLD - TRANSIENT_KNEE)).clamp(0.0, 1.0);
    ambient + (fast - ambient) * progress
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_60: f32 = 1.0 / 60.0;

    /// Hold `target` for `frames` frames, re-supplying it each tick as the real pipeline does.
    fn hold(s: &mut RoleSmoother, role: Role, target: f32, frames: usize) -> f32 {
        let mut v = RoleValues::new();
        let mut last = 0.0;
        for _ in 0..frames {
            v.set(role, target);
            s.tick(0, &mut v, FRAME_60, false);
            last = v.get(role).unwrap();
        }
        last
    }

    #[test]
    fn first_value_is_adopted_not_faded_in() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::Red, 1.0);
        s.tick(0, &mut v, FRAME_60, false);
        assert_eq!(
            v.get(Role::Red),
            Some(1.0),
            "patching a fixture mid-show must not fade it up from black"
        );
    }

    #[test]
    fn a_step_converges_toward_the_target() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::Pan, 0.0);
        s.tick(0, &mut v, FRAME_60, false);
        v.set(Role::Pan, 1.0);
        s.tick(0, &mut v, FRAME_60, false);
        let after_one = v.get(Role::Pan).unwrap();
        assert!(after_one > 0.0 && after_one < 1.0, "got {after_one}");
        let settled = hold(&mut s, Role::Pan, 1.0, 60);
        assert!((settled - 1.0).abs() < 0.01, "settled at {settled}");
    }

    /// Contract C3. The transient decision is shared across the fixture's color roles, so the
    /// RGB ratio survives the attack. Per-role decisions make a purple flash read red on the
    /// front of every beat.
    #[test]
    fn transient_preserves_hue_across_color_roles() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::Red, 0.0);
        v.set(Role::Blue, 0.0);
        s.tick(0, &mut v, FRAME_60, false);

        // Purple: a large red rise and a small blue one, in the same frame.
        v.set(Role::Red, 0.80);
        v.set(Role::Blue, 0.16);
        s.tick(0, &mut v, FRAME_60, false);

        let r = v.get(Role::Red).unwrap();
        let b = v.get(Role::Blue).unwrap();
        assert!(r > 0.0 && b > 0.0);
        let target_ratio = 0.80 / 0.16;
        let out_ratio = r / b;
        assert!(
            (out_ratio - target_ratio).abs() < 0.01,
            "hue skewed on the attack: target {target_ratio:.3}, got {out_ratio:.3}"
        );
    }

    /// Contract C3. The same time constant must produce the same value after the same
    /// wall-clock time regardless of frame rate.
    #[test]
    fn response_is_frame_rate_independent() {
        let mut fast = RoleSmoother::new(1);
        let mut slow = RoleSmoother::new(1);
        let (mut a, mut b) = (RoleValues::new(), RoleValues::new());
        a.set(Role::Dimmer, 0.0);
        b.set(Role::Dimmer, 0.0);
        fast.tick(0, &mut a, 1.0 / 120.0, false);
        slow.tick(0, &mut b, 1.0 / 30.0, false);

        // One second of wall clock: 120 frames at 120fps, 30 at 30fps.
        a.set(Role::Dimmer, 1.0);
        b.set(Role::Dimmer, 1.0);
        for _ in 0..120 {
            a.set(Role::Dimmer, 1.0);
            fast.tick(0, &mut a, 1.0 / 120.0, false);
        }
        for _ in 0..30 {
            b.set(Role::Dimmer, 1.0);
            slow.tick(0, &mut b, 1.0 / 30.0, false);
        }
        let (x, y) = (a.get(Role::Dimmer).unwrap(), b.get(Role::Dimmer).unwrap());
        assert!(
            (x - y).abs() < 0.001,
            "120fps gave {x}, 30fps gave {y}: the filter is frame-rate dependent"
        );
    }

    /// Smoothing a wheel walks it through every gel between the old and new slot.
    #[test]
    fn discrete_roles_pass_through_untouched() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::ColorWheel, 0.0);
        v.set(Role::GoboWheel, 0.0);
        v.set(Role::Strobe, 0.0);
        s.tick(0, &mut v, FRAME_60, false);
        v.set(Role::ColorWheel, 1.0);
        v.set(Role::GoboWheel, 0.5);
        v.set(Role::Strobe, 0.8);
        s.tick(0, &mut v, FRAME_60, false);
        assert_eq!(v.get(Role::ColorWheel), Some(1.0));
        assert_eq!(v.get(Role::GoboWheel), Some(0.5));
        assert_eq!(v.get(Role::Strobe), Some(0.8));
    }

    /// A transient fast path on a moving head is a snap. Position must stay on its ambient
    /// response no matter how large the jump.
    #[test]
    fn position_never_takes_the_transient_path() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::Pan, 0.0);
        s.tick(0, &mut v, FRAME_60, false);
        v.set(Role::Pan, 1.0);
        s.tick(0, &mut v, FRAME_60, false);
        let moved = v.get(Role::Pan).unwrap();
        let expected = alpha(TAU_POSITION, FRAME_60);
        assert!(
            (moved - expected).abs() < 1e-5,
            "pan moved {moved}, ambient response is {expected}"
        );
        assert!(moved < 0.25, "a full-scale pan jump must not snap");
    }

    #[test]
    fn intensity_rises_faster_on_a_large_jump_than_a_small_one() {
        let step = |rise: f32| {
            let mut s = RoleSmoother::new(1);
            let mut v = RoleValues::new();
            v.set(Role::Dimmer, 0.0);
            s.tick(0, &mut v, FRAME_60, false);
            v.set(Role::Dimmer, rise);
            s.tick(0, &mut v, FRAME_60, false);
            v.get(Role::Dimmer).unwrap() / rise
        };
        let small = step(0.02);
        let large = step(0.9);
        assert!(
            large > small * 1.5,
            "transient did not engage: small {small:.3}, large {large:.3}"
        );
    }

    /// Contract C3. Blackout must be instant, so bypass syncs state without touching output.
    #[test]
    fn bypass_syncs_state_without_modifying_values() {
        let mut s = RoleSmoother::new(1);
        hold(&mut s, Role::Dimmer, 1.0, 30);

        let mut dark = RoleValues::new();
        dark.set(Role::Dimmer, 0.0);
        s.tick(0, &mut dark, FRAME_60, true);
        assert_eq!(
            dark.get(Role::Dimmer),
            Some(0.0),
            "blackout must be instant"
        );

        // Coming out of bypass, the filter resumes from dark rather than snapping back up.
        let mut lit = RoleValues::new();
        lit.set(Role::Dimmer, 1.0);
        s.tick(0, &mut lit, FRAME_60, false);
        assert!(
            lit.get(Role::Dimmer).unwrap() < 1.0,
            "state must have been synced to the blackout value"
        );
    }

    #[test]
    fn a_downward_move_never_takes_the_transient_path() {
        let mut s = RoleSmoother::new(1);
        hold(&mut s, Role::Dimmer, 1.0, 30);
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 0.0);
        s.tick(0, &mut v, FRAME_60, false);
        let after = v.get(Role::Dimmer).unwrap();
        let ambient = 1.0 - alpha(TAU_INTENSITY, FRAME_60);
        assert!(
            (after - ambient).abs() < 1e-5,
            "fall should use the ambient response: got {after}, expected {ambient}"
        );
    }

    #[test]
    fn fixtures_are_smoothed_independently() {
        let mut smoother = RoleSmoother::new(2);
        let mut high = RoleValues::new();
        let mut low = RoleValues::new();
        // Seed opposite starting states.
        high.set(Role::Pan, 1.0);
        low.set(Role::Pan, 0.0);
        smoother.tick(0, &mut high, FRAME_60, false);
        smoother.tick(1, &mut low, FRAME_60, false);

        // Drive both toward the middle from their own ends.
        high.set(Role::Pan, 0.5);
        low.set(Role::Pan, 0.5);
        smoother.tick(0, &mut high, FRAME_60, false);
        smoother.tick(1, &mut low, FRAME_60, false);

        let from_above = high.get(Role::Pan).unwrap();
        let from_below = low.get(Role::Pan).unwrap();
        assert!(
            from_above > 0.5,
            "fixture 0 approaches from above: {from_above}"
        );
        assert!(
            from_below < 0.5,
            "fixture 1 approaches from below: {from_below}"
        );
    }

    #[test]
    fn resize_preserves_state_for_surviving_fixtures() {
        let mut s = RoleSmoother::new(1);
        hold(&mut s, Role::Dimmer, 1.0, 30);
        s.resize(3);
        let mut next = RoleValues::new();
        next.set(Role::Dimmer, 0.0);
        s.tick(0, &mut next, FRAME_60, false);
        assert!(
            next.get(Role::Dimmer).unwrap() > 0.5,
            "fixture 0 must still be near full, not reset"
        );
    }

    #[test]
    fn an_out_of_range_fixture_index_is_ignored() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 1.0);
        s.tick(99, &mut v, FRAME_60, false);
        assert_eq!(v.get(Role::Dimmer), Some(1.0));
    }

    #[test]
    fn zero_dt_does_not_divide_by_zero() {
        let mut s = RoleSmoother::new(1);
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 0.0);
        s.tick(0, &mut v, FRAME_60, false);
        v.set(Role::Dimmer, 1.0);
        s.tick(0, &mut v, 0.0, false);
        assert!(v.get(Role::Dimmer).unwrap().is_finite());
    }
}
