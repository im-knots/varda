//! When the arrangement is allowed to drive, and what a performer's hand does
//! to that.
//!
//! The rule that matters is negative: authority does **not** engage until the
//! transport has actually advanced. An arrangement whose regions begin at hour
//! one, sitting at position zero with every deck off, is behaving perfectly and
//! produces exactly the same black output as an unplugged timecode cable. Since
//! correct-and-idle is indistinguishable from broken on the output, engagement
//! is gated on evidence that the show has started rather than on the
//! arrangement merely existing.
//!
//! See /spec/transport.md § Engagement and /spec/arrangement.md § Authority.

use super::ArrangementConfig;
use crate::timebase::TransportSample;

/// How long a re-armed lane takes to reach the automated value.
///
/// A jump to the envelope's value is the correct *state* and the wrong *look*,
/// and this happens live in front of an audience.
pub const DEFAULT_REARM_SECONDS: f64 = 0.5;

/// Whether the arrangement is driving this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authority {
    /// Performance mode holds everything and the scene renders as saved.
    Inert,
    /// Lanes with content drive their decks, unless individually overridden.
    Engaged,
}

impl Authority {
    pub fn is_engaged(self) -> bool {
        matches!(self, Authority::Engaged)
    }

    /// Resolve authority for this frame.
    ///
    /// `transport` is `None` until the transport has advanced at least once,
    /// which is the whole engagement gate: a cold start with no timecode shows
    /// the saved scene, live and visible, and nothing about a missing cable or
    /// a wrong input can black the output.
    pub fn resolve(
        arrangement: Option<&ArrangementConfig>,
        transport: Option<&TransportSample>,
    ) -> Self {
        match (arrangement, transport) {
            (Some(a), Some(_)) if a.drives_anything() => Authority::Engaged,
            _ => Authority::Inert,
        }
    }
}

impl ArrangementConfig {
    /// Decks the arrangement drives, whose per-deck auto-transition must be
    /// suspended while authority holds.
    ///
    /// Auto-transitions are *relative*: their phase depends on when a deck
    /// became active rather than on transport position, so they cannot be
    /// resolved from an arbitrary position and would fight the regions. They
    /// partition cleanly per deck, so a deck with no lane keeps its own.
    pub fn arranged_decks(&self) -> impl Iterator<Item = &str> {
        self.lanes
            .iter()
            .filter(|l| l.drives_anything())
            .map(|l| l.deck_uuid.as_str())
    }

    /// Whether this deck's activity is the arrangement's to decide.
    pub fn drives_deck(&self, deck_uuid: &str) -> bool {
        self.lane(deck_uuid)
            .is_some_and(super::LaneConfig::drives_anything)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrangement::{LaneConfig, RegionConfig};

    fn sample(position: f64) -> TransportSample {
        TransportSample {
            position,
            running: true,
            discontinuity: false,
        }
    }

    fn arrangement_with_content() -> ArrangementConfig {
        let mut lane = LaneConfig::new("deck-1");
        lane.regions.push(RegionConfig::new(0.0, 10.0));
        ArrangementConfig {
            lanes: vec![lane],
            ..ArrangementConfig::default()
        }
    }

    /// The cold-start rule, stated as a test because it is the one failure that
    /// looks identical to correct behaviour on the output.
    #[test]
    fn authority_stays_inert_until_the_transport_has_run() {
        let arrangement = arrangement_with_content();
        assert_eq!(
            Authority::resolve(Some(&arrangement), None),
            Authority::Inert,
            "a cold start must render the saved scene, not the arrangement"
        );
    }

    #[test]
    fn authority_engages_once_the_transport_has_run() {
        let arrangement = arrangement_with_content();
        assert_eq!(
            Authority::resolve(Some(&arrangement), Some(&sample(5.0))),
            Authority::Engaged
        );
    }

    /// Engagement survives a stop: envelopes freeze at their last value rather
    /// than releasing, because releasing would cut the look the instant someone
    /// trips over a cable.
    #[test]
    fn authority_holds_while_the_transport_is_stopped_after_running() {
        let arrangement = arrangement_with_content();
        let stopped = TransportSample {
            position: 5.0,
            running: false,
            discontinuity: false,
        };
        assert_eq!(
            Authority::resolve(Some(&arrangement), Some(&stopped)),
            Authority::Engaged
        );
    }

    #[test]
    fn no_arrangement_is_never_engaged() {
        assert_eq!(
            Authority::resolve(None, Some(&sample(5.0))),
            Authority::Inert
        );
    }

    /// Adding an empty row must not take the deck away from Performance mode.
    #[test]
    fn an_arrangement_of_empty_lanes_never_engages() {
        let arrangement = ArrangementConfig {
            lanes: vec![LaneConfig::new("deck-1")],
            ..ArrangementConfig::default()
        };
        assert_eq!(
            Authority::resolve(Some(&arrangement), Some(&sample(5.0))),
            Authority::Inert
        );
    }

    #[test]
    fn only_lanes_with_content_claim_their_deck() {
        let mut arranged = LaneConfig::new("arranged");
        arranged.regions.push(RegionConfig::new(0.0, 1.0));

        let arrangement = ArrangementConfig {
            lanes: vec![arranged, LaneConfig::new("empty")],
            ..ArrangementConfig::default()
        };

        let claimed: Vec<&str> = arrangement.arranged_decks().collect();
        assert_eq!(claimed, vec!["arranged"]);
        assert!(arrangement.drives_deck("arranged"));
        assert!(!arrangement.drives_deck("empty"));
        assert!(!arrangement.drives_deck("not-in-the-arrangement"));
    }
}
