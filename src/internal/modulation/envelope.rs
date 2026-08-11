//! Automation envelopes: a parameter value as a pure function of timebase position.
//!
//! See /spec/automation.md. The property that matters here is that evaluation
//! carries no accumulated state, which is what lets automation survive locates,
//! loops, and timecode jumps with no resync logic.

use serde::{Deserialize, Serialize};

/// Segment shape from one breakpoint to the next.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub enum CurveKind {
    /// Hold this breakpoint's value until the next one.
    Step,
    /// Straight line. `tension` 0.0 is linear; negative eases in, positive eases out.
    Linear { tension: f32 },
    /// Cubic smoothstep, matching `StepSequencer`'s smooth interpolation.
    Smooth,
}

impl Default for CurveKind {
    fn default() -> Self {
        Self::Linear { tension: 0.0 }
    }
}

/// A single point on an automation curve.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct Breakpoint {
    /// Position in timebase units. Seconds for `Transport`, beats for `Beat`.
    pub position: f64,
    /// Normalized value, 0.0–1.0.
    pub value: f32,
    /// Segment shape from this breakpoint to the next.
    #[serde(default)]
    pub curve: CurveKind,
}

impl Breakpoint {
    pub fn new(position: f64, value: f32) -> Self {
        Self {
            position,
            value,
            curve: CurveKind::default(),
        }
    }

    #[must_use]
    pub fn with_curve(mut self, curve: CurveKind) -> Self {
        self.curve = curve;
        self
    }
}

/// Shape the 0–1 segment fraction according to the curve.
fn shape(curve: CurveKind, t: f32) -> f32 {
    match curve {
        // Handled by the caller, which never interpolates a Step segment.
        CurveKind::Step => 0.0,
        CurveKind::Linear { tension } => {
            if tension == 0.0 {
                t
            } else {
                // 2^-tension: positive tension pulls the exponent below 1 for a
                // fast start and slow finish, negative pushes it above 1.
                t.powf(2.0f32.powf(-tension))
            }
        }
        CurveKind::Smooth => t * t * (3.0 - 2.0 * t),
    }
}

/// Locate the segment containing `position`, starting from a cached index.
///
/// Position is monotonic on almost every frame, so the cached segment and its
/// successor are checked before falling back to a binary search. The cache is
/// an optimization only: a stale index can never produce a wrong value, because
/// every path re-verifies the bracket before using it.
fn segment_index(breakpoints: &[Breakpoint], position: f64, cursor: &mut usize) -> usize {
    let last = breakpoints.len() - 2;
    let brackets =
        |i: usize| breakpoints[i].position <= position && position < breakpoints[i + 1].position;

    let cached = (*cursor).min(last);
    if brackets(cached) {
        return cached;
    }
    if cached < last && brackets(cached + 1) {
        *cursor = cached + 1;
        return cached + 1;
    }

    // partition_point gives the count of breakpoints starting at or before
    // `position`; the segment is the one beginning at the last of them.
    let found = breakpoints
        .partition_point(|bp| bp.position <= position)
        .saturating_sub(1)
        .min(last);
    *cursor = found;
    found
}

/// Value of the curve at `position`.
///
/// Outside the drawn range the first and last values are held rather than
/// falling to zero: an envelope that collapsed at its edges would black out
/// every automated parameter before and after the arranged section.
#[doc(alias = "evaluate_envelope")]
pub fn evaluate(breakpoints: &[Breakpoint], position: f64, cursor: &mut usize) -> f32 {
    match breakpoints {
        [] => 0.0,
        [only] => only.value,
        [first, .., last] => {
            if position <= first.position {
                return first.value;
            }
            if position >= last.position {
                return last.value;
            }

            let i = segment_index(breakpoints, position, cursor);
            let (a, b) = (breakpoints[i], breakpoints[i + 1]);
            if matches!(a.curve, CurveKind::Step) {
                return a.value;
            }

            let span = b.position - a.position;
            if span <= 0.0 {
                return b.value;
            }
            let t = ((position - a.position) / span) as f32;
            a.value + (b.value - a.value) * shape(a.curve, t)
        }
    }
}

/// Whether the curve is non-zero anywhere in `[from, to]`.
///
/// The question residency asks: "will this deck be visible at any point in this
/// window", answered without sampling, so a region shorter than any sample
/// interval cannot be missed. Conservative by construction, since a false
/// negative would black out a deck that was supposed to appear.
///
/// Edge values are held outside the drawn range, matching [`evaluate`].
pub fn active_between(breakpoints: &[Breakpoint], from: f64, to: f64) -> bool {
    match breakpoints {
        [] => false,
        [only] => only.value > 0.0,
        [first, .., last] => {
            if from < first.position && first.value > 0.0 {
                return true;
            }
            if to > last.position && last.value > 0.0 {
                return true;
            }
            // A segment counts when it overlaps the window at all and either end
            // carries value. A Step segment holds its left value throughout, so
            // its right end says nothing about it.
            breakpoints.windows(2).any(|pair| {
                let (a, b) = (pair[0], pair[1]);
                let overlaps = a.position <= to && b.position >= from;
                let carries =
                    a.value > 0.0 || (b.value > 0.0 && !matches!(a.curve, CurveKind::Step));
                overlaps && carries
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Vec<Breakpoint> {
        vec![Breakpoint::new(1.0, 0.0), Breakpoint::new(3.0, 1.0)]
    }

    #[test]
    fn holds_the_edge_values_outside_the_drawn_range() {
        let bps = ramp();
        let mut c = 0;

        assert!((evaluate(&bps, -50.0, &mut c) - 0.0).abs() < 1e-6);
        assert!((evaluate(&bps, 0.5, &mut c) - 0.0).abs() < 1e-6);
        assert!(
            (evaluate(&bps, 900.0, &mut c) - 1.0).abs() < 1e-6,
            "after the last breakpoint the curve holds, it does not collapse to zero"
        );
    }

    #[test]
    fn empty_envelope_is_inert() {
        let mut c = 0;
        assert!((evaluate(&[], 5.0, &mut c) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn single_breakpoint_is_a_constant() {
        let bps = vec![Breakpoint::new(4.0, 0.7)];
        let mut c = 0;
        assert!((evaluate(&bps, 0.0, &mut c) - 0.7).abs() < 1e-6);
        assert!((evaluate(&bps, 4.0, &mut c) - 0.7).abs() < 1e-6);
        assert!((evaluate(&bps, 99.0, &mut c) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn linear_segment_interpolates() {
        let bps = ramp();
        let mut c = 0;
        assert!((evaluate(&bps, 2.0, &mut c) - 0.5).abs() < 1e-6);
        assert!((evaluate(&bps, 1.5, &mut c) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn step_segments_hold_until_the_next_breakpoint() {
        let bps = vec![
            Breakpoint::new(0.0, 0.2).with_curve(CurveKind::Step),
            Breakpoint::new(10.0, 0.9),
        ];
        let mut c = 0;
        assert!((evaluate(&bps, 0.0, &mut c) - 0.2).abs() < 1e-6);
        assert!((evaluate(&bps, 9.999, &mut c) - 0.2).abs() < 1e-6);
        assert!((evaluate(&bps, 10.0, &mut c) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn smooth_segments_are_flat_at_both_ends() {
        let bps = vec![
            Breakpoint::new(0.0, 0.0).with_curve(CurveKind::Smooth),
            Breakpoint::new(1.0, 1.0),
        ];
        let mut c = 0;
        // Smoothstep passes through the midpoint but approaches the ends slower
        // than a straight line does.
        assert!((evaluate(&bps, 0.5, &mut c) - 0.5).abs() < 1e-6);
        assert!(evaluate(&bps, 0.1, &mut c) < 0.1);
        assert!(evaluate(&bps, 0.9, &mut c) > 0.9);
    }

    #[test]
    fn tension_bends_the_segment_without_moving_its_ends() {
        let ease_out = vec![
            Breakpoint::new(0.0, 0.0).with_curve(CurveKind::Linear { tension: 1.0 }),
            Breakpoint::new(1.0, 1.0),
        ];
        let ease_in = vec![
            Breakpoint::new(0.0, 0.0).with_curve(CurveKind::Linear { tension: -1.0 }),
            Breakpoint::new(1.0, 1.0),
        ];
        let mut c = 0;

        assert!(evaluate(&ease_out, 0.25, &mut c) > 0.25, "eases out");
        assert!(evaluate(&ease_in, 0.25, &mut c) < 0.25, "eases in");

        for bps in [&ease_out, &ease_in] {
            assert!((evaluate(bps, 0.0, &mut c) - 0.0).abs() < 1e-6);
            assert!((evaluate(bps, 1.0, &mut c) - 1.0).abs() < 1e-6);
        }
    }

    /// The whole point of a pure function of position: how you arrived does not
    /// matter. This is what makes automation survive a locate or a loop wrap.
    #[test]
    fn arriving_at_a_position_by_any_path_gives_the_same_value() {
        let bps = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(5.0, 1.0).with_curve(CurveKind::Smooth),
            Breakpoint::new(10.0, 0.25),
            Breakpoint::new(20.0, 0.8),
        ];

        let target = 12.5;

        let mut played = 0;
        let mut swept = 0.0;
        while swept < target {
            evaluate(&bps, swept, &mut played);
            swept += 0.016;
        }
        let after_playing = evaluate(&bps, target, &mut played);

        let mut located = 0;
        let after_locate = evaluate(&bps, target, &mut located);

        let mut backwards = 3;
        evaluate(&bps, 19.0, &mut backwards);
        let after_rewind = evaluate(&bps, target, &mut backwards);

        assert!((after_playing - after_locate).abs() < 1e-6);
        assert!((after_playing - after_rewind).abs() < 1e-6);
    }

    /// The cursor is an optimization, so any value it could hold (including one
    /// left over from a longer envelope) must still resolve correctly.
    #[test]
    fn a_stale_cursor_cannot_produce_a_wrong_value() {
        let bps = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(1.0, 1.0),
            Breakpoint::new(2.0, 0.0),
        ];
        let truth = {
            let mut fresh = 0;
            evaluate(&bps, 1.5, &mut fresh)
        };

        for stale in 0..64 {
            let mut cursor = stale;
            assert!(
                (evaluate(&bps, 1.5, &mut cursor) - truth).abs() < 1e-6,
                "stale cursor {stale} changed the result"
            );
        }
    }

    /// The residency predicate must agree with what the curve actually
    /// evaluates to. A window the predicate calls quiet, but that evaluates
    /// non-zero anywhere inside, is a deck that goes black on stage.
    #[test]
    fn active_between_never_disagrees_with_evaluation() {
        // A region-shaped curve: dark, up at 10, down at 14, dark again.
        let bps = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(10.0, 0.0),
            Breakpoint::new(11.0, 1.0),
            Breakpoint::new(13.0, 1.0),
            Breakpoint::new(14.0, 0.0),
            Breakpoint::new(30.0, 0.0),
        ];

        let mut from = 0.0;
        while from < 32.0 {
            let to = from + 1.5;
            let claimed = active_between(&bps, from, to);
            let mut cursor = 0;
            let mut sampled = false;
            let mut t = from;
            while t <= to {
                if evaluate(&bps, t, &mut cursor) > 0.0 {
                    sampled = true;
                    break;
                }
                t += 0.01;
            }
            assert!(
                claimed || !sampled,
                "window [{from}, {to}] evaluates non-zero but was called quiet"
            );
            from += 0.25;
        }
    }

    #[test]
    fn active_between_finds_a_region_shorter_than_any_sample_interval() {
        let flash = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(5.0, 0.0).with_curve(CurveKind::Step),
            Breakpoint::new(5.01, 1.0).with_curve(CurveKind::Step),
            Breakpoint::new(5.02, 0.0),
            Breakpoint::new(20.0, 0.0),
        ];
        assert!(
            active_between(&flash, 4.0, 6.0),
            "a 10 ms flash still counts"
        );
        assert!(!active_between(&flash, 8.0, 12.0));
    }

    /// An empty or entirely dark curve keeps nothing awake.
    #[test]
    fn a_dark_curve_is_never_active() {
        assert!(!active_between(&[], 0.0, 100.0));
        assert!(!active_between(
            &[Breakpoint::new(0.0, 0.0), Breakpoint::new(50.0, 0.0)],
            0.0,
            100.0
        ));
        assert!(active_between(&[Breakpoint::new(0.0, 1.0)], -5.0, 5.0));
    }

    /// Edge values are held, so a curve that ends bright stays bright forever.
    #[test]
    fn held_edges_count_as_active() {
        let ends_bright = vec![Breakpoint::new(0.0, 0.0), Breakpoint::new(10.0, 1.0)];
        assert!(active_between(&ends_bright, 500.0, 501.0));

        let starts_bright = vec![Breakpoint::new(10.0, 1.0), Breakpoint::new(20.0, 0.0)];
        assert!(active_between(&starts_bright, -500.0, -499.0));
    }

    #[test]
    fn zero_width_segments_do_not_divide_by_zero() {
        let bps = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(1.0, 0.3),
            Breakpoint::new(1.0, 0.9),
            Breakpoint::new(2.0, 1.0),
        ];
        let mut c = 0;
        let v = evaluate(&bps, 1.0, &mut c);
        assert!(v.is_finite());
        assert!((0.0..=1.0).contains(&v));
    }
}
