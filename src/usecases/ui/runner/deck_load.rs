//! Background deck-load bookkeeping.

use crate::app::render::DeckLoadToken;

/// Target channel of each in-flight background deck load, keyed by the token
/// handed to `spawn_deck_loads` and echoed back in `DeckLoadResult::token`.
///
/// A decode or shader compile can outlive the channel it was aimed at, so the
/// target is stored as a UUID and resolved when the load lands, never at spawn.
/// See [`/spec/api-addressing.md`].
#[derive(Default)]
pub(super) struct DeckLoadTargets {
    by_token: std::collections::HashMap<DeckLoadToken, String>,
    next_token: usize,
}

impl DeckLoadTargets {
    /// Record a load's target channel and return the token that carries it back.
    pub(super) fn record(&mut self, channel_uuid: String) -> DeckLoadToken {
        let token = DeckLoadToken(self.next_token);
        self.next_token += 1;
        self.by_token.insert(token, channel_uuid);
        token
    }

    /// Claim a completed load's target. `None` means the token was never issued
    /// or has already been claimed.
    pub(super) fn claim(&mut self, token: DeckLoadToken) -> Option<String> {
        self.by_token.remove(&token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── DeckLoadTargets ─────────────────────────────────────────────
    //
    // Tokens carry a background load's *target channel UUID* rather than an
    // index, because a decode or shader compile can outlive the channel it was
    // aimed at. These pin the issue/claim contract.

    #[test]
    fn deck_load_tokens_are_unique_and_round_trip_their_channel() {
        let mut targets = DeckLoadTargets::default();
        let first = targets.record("ch-aaa".to_string());
        let second = targets.record("ch-bbb".to_string());
        assert_ne!(first, second, "each load gets a distinct token");
        assert_eq!(targets.claim(first), Some("ch-aaa".to_string()));
        assert_eq!(targets.claim(second), Some("ch-bbb".to_string()));
    }

    /// A token is single-use: claiming twice must not resurrect a stale target.
    #[test]
    fn deck_load_token_cannot_be_claimed_twice() {
        let mut targets = DeckLoadTargets::default();
        let token = targets.record("ch-aaa".to_string());
        assert_eq!(targets.claim(token), Some("ch-aaa".to_string()));
        assert_eq!(targets.claim(token), None, "second claim yields nothing");
    }

    #[test]
    fn unissued_deck_load_token_claims_nothing() {
        let mut targets = DeckLoadTargets::default();
        assert_eq!(targets.claim(DeckLoadToken(42)), None);
    }

    /// Claiming out of order must not disturb the other in-flight loads.
    #[test]
    fn deck_load_targets_are_independent() {
        let mut targets = DeckLoadTargets::default();
        let a = targets.record("ch-aaa".to_string());
        let b = targets.record("ch-bbb".to_string());
        let c = targets.record("ch-ccc".to_string());
        assert_eq!(targets.claim(b), Some("ch-bbb".to_string()));
        assert_eq!(targets.claim(c), Some("ch-ccc".to_string()));
        assert_eq!(targets.claim(a), Some("ch-aaa".to_string()));
    }
}
