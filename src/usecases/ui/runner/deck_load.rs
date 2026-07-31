//! Deck preview-texture registration and background deck-load bookkeeping.
//!
//! egui texture handles are a presentation concern the engine never touches, so
//! the deck -> `TextureId` map lives on the delivery side — see
//! `/spec/app-presentation-boundary.md`.

use super::GpuContext;
use crate::app::render::DeckLoadToken;

/// Register a single deck's preview texture, keyed by its UUID.
///
/// Lives here, not on `VardaApp`, because egui texture handles are a
/// presentation concern the engine never touches — see
/// `/spec/app-presentation-boundary.md`.
pub(super) fn register_deck_preview_texture(
    egui_renderer: &mut egui_wgpu::Renderer,
    context: &GpuContext,
    mixer: &crate::mixer::Mixer,
    deck_uuid: &str,
    deck_preview_textures: &mut std::collections::HashMap<String, egui::TextureId>,
) {
    let Some((ch_idx, deck_idx)) = mixer.find_deck_by_uuid(deck_uuid) else {
        log::warn!("No deck {deck_uuid} to register a preview texture for");
        return;
    };
    if let Some(slot) = mixer
        .channels()
        .get(ch_idx)
        .and_then(|ch| ch.decks.get(deck_idx))
    {
        let texture_id = egui_renderer.register_native_texture(
            &context.device,
            &slot.deck.texture_view,
            wgpu::FilterMode::Linear,
        );
        deck_preview_textures.insert(deck_uuid.to_string(), texture_id);
    }
}

/// Apply the egui texture post-step for a frame's `apply_engine_actions`
/// outcomes: register a preview for each newly-created deck. Reorder and
/// removal need no repair pass — the map is UUID-keyed, so positions shifting
/// cannot invalidate an entry.
pub(super) fn apply_deck_texture_outcomes(
    outcomes: &[crate::engine::CommandOutcome],
    egui_renderer: &mut egui_wgpu::Renderer,
    context: &GpuContext,
    mixer: &crate::mixer::Mixer,
    deck_preview_textures: &mut std::collections::HashMap<String, egui::TextureId>,
) {
    for outcome in outcomes {
        if let crate::engine::CommandOutcome::DecksCreated { uuids } = outcome {
            for uuid in uuids {
                register_deck_preview_texture(
                    egui_renderer,
                    context,
                    mixer,
                    uuid,
                    deck_preview_textures,
                );
            }
        }
    }
}

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
