//! Which decks the arrangement still needs frames from.
//!
//! The existing zero-opacity cull skips a deck's render pass but not its decode
//! thread, which keeps running so a fader can bring anything up instantly. That
//! is right in Performance mode and wrong in a two-hour arrangement, where a
//! deck's next region can be forty minutes away.
//!
//! The signal is the deck's opacity envelope rather than a separate window
//! model: it is already a pure function of position, so "visible anywhere in the
//! next few seconds" is a query against data that exists, and it covers
//! hand-drawn opacity curves that no region produced.
//!
//! See /spec/deck-residency.md.

use crate::modulation::{Breakpoint, envelope_active_between};

/// How far ahead of the playhead a deck must start decoding.
///
/// Suspension pauses an already-running decoder rather than tearing it down, so
/// resuming costs a command wake plus one frame. A second is generous cover for
/// a slow disk and a 4K frame, and a second of wasted decode per region is
/// nothing against a show's length.
pub const PREROLL_SECONDS: f64 = 1.0;

/// How long a deck keeps decoding after it stops being visible.
///
/// Expressed as a backward extension of the same window rather than a per-deck
/// timer, which keeps the predicate pure: a run of short regions cannot thrash
/// the decoder, and there is no state to get stuck.
pub const RELEASE_DELAY_SECONDS: f64 = 2.0;

/// What the arrangement predicts about one deck's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceDemand {
    /// Nothing is scheduling this deck, so every source keeps running exactly
    /// as it always has. The default, and what all of Performance mode uses.
    #[default]
    Unscheduled,
    /// Visible now, or soon enough that its frames are needed.
    Needed,
    /// Not visible, and not about to be. Safe to stop pulling frames.
    Idle,
}

impl SourceDemand {
    /// Whether frames should keep flowing. Only an explicit `Idle` stops them,
    /// so any gap in the reasoning above leaves a deck running rather than dark.
    pub fn wants_frames(self) -> bool {
        self != SourceDemand::Idle
    }
}

/// The demand for a deck whose opacity is driven by `drivers`, at `position`
/// show seconds.
///
/// Each driver is one modulation assigned to the deck's opacity: `Some` for an
/// automation curve laid out against show position, and `None` for anything
/// residency cannot read off the timeline. One `None` is enough to make the
/// deck unschedulable, because an LFO or an audio band can raise it at any
/// moment. A deck with no drivers at all is a plain performance deck.
pub fn demand<'a>(
    drivers: impl IntoIterator<Item = Option<&'a [Breakpoint]>>,
    position: f64,
) -> SourceDemand {
    let from = position - RELEASE_DELAY_SECONDS;
    let to = position + PREROLL_SECONDS;

    let mut any = false;
    for driver in drivers {
        let Some(points) = driver else {
            return SourceDemand::Unscheduled;
        };
        any = true;
        if envelope_active_between(points, from, to) {
            return SourceDemand::Needed;
        }
    }
    if any {
        SourceDemand::Idle
    } else {
        SourceDemand::Unscheduled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single region from 10 s to 20 s.
    fn region_curve() -> Vec<Breakpoint> {
        vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(10.0, 0.0),
            Breakpoint::new(10.0, 1.0),
            Breakpoint::new(20.0, 1.0),
            Breakpoint::new(20.0, 0.0),
            Breakpoint::new(60.0, 0.0),
        ]
    }

    fn demand_at(position: f64) -> SourceDemand {
        let curve = region_curve();
        demand([Some(curve.as_slice())], position)
    }

    #[test]
    fn a_deck_far_from_its_region_is_idle() {
        assert_eq!(demand_at(0.0), SourceDemand::Idle);
        assert_eq!(demand_at(40.0), SourceDemand::Idle);
    }

    /// The whole point: frames must be flowing before the audience sees any.
    #[test]
    fn decoding_starts_before_the_region_does() {
        assert_eq!(
            demand_at(10.0 - PREROLL_SECONDS + 0.01),
            SourceDemand::Needed
        );
        assert_eq!(demand_at(15.0), SourceDemand::Needed);
    }

    #[test]
    fn decoding_continues_briefly_after_the_region_ends() {
        assert_eq!(
            demand_at(20.0 + RELEASE_DELAY_SECONDS - 0.01),
            SourceDemand::Needed
        );
        assert_eq!(
            demand_at(20.0 + RELEASE_DELAY_SECONDS + PREROLL_SECONDS + 0.01),
            SourceDemand::Idle
        );
    }

    /// Back-to-back regions must not flap the decoder off and on between them.
    #[test]
    fn a_gap_shorter_than_the_window_never_suspends() {
        let curve = vec![
            Breakpoint::new(0.0, 1.0),
            Breakpoint::new(10.0, 1.0),
            Breakpoint::new(10.0, 0.0),
            // A one-second hole between two regions.
            Breakpoint::new(11.0, 0.0),
            Breakpoint::new(11.0, 1.0),
            Breakpoint::new(20.0, 1.0),
        ];
        let mut t = 10.0;
        while t <= 11.0 {
            assert_eq!(
                demand([Some(curve.as_slice())], t),
                SourceDemand::Needed,
                "suspended inside a {t}s gap"
            );
            t += 0.05;
        }
    }

    /// Two curves on one deck's opacity: either one wanting it is enough.
    #[test]
    fn any_curve_wanting_the_deck_keeps_it_awake() {
        let early = vec![Breakpoint::new(0.0, 1.0), Breakpoint::new(5.0, 0.0)];
        let late = vec![Breakpoint::new(50.0, 0.0), Breakpoint::new(55.0, 1.0)];
        let both = || [Some(early.as_slice()), Some(late.as_slice())];

        assert_eq!(demand(both(), 52.0), SourceDemand::Needed);
        assert_eq!(demand(both(), 20.0), SourceDemand::Idle);
    }

    /// A modulator residency cannot read off the timeline can raise the deck at
    /// any moment, so nothing else about it matters.
    #[test]
    fn one_live_modulator_makes_a_deck_unschedulable() {
        let dark = vec![Breakpoint::new(0.0, 0.0), Breakpoint::new(60.0, 0.0)];
        assert_eq!(
            demand([Some(dark.as_slice()), None], 30.0),
            SourceDemand::Unscheduled
        );
        assert_eq!(demand([None], 30.0), SourceDemand::Unscheduled);
    }

    /// A deck with nothing on its opacity is a performance deck, and Performance
    /// mode has never gated anything.
    #[test]
    fn a_deck_with_no_drivers_is_unscheduled() {
        assert_eq!(demand([], 5.0), SourceDemand::Unscheduled);
        assert!(SourceDemand::default().wants_frames());
        assert!(SourceDemand::Needed.wants_frames());
        assert!(!SourceDemand::Idle.wants_frames());
    }
}
