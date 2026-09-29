//! UUID → index resolution.
//!
//! Write commands address targets by UUID. The returned indices are valid only until the
//! containing collection is mutated, so resolve immediately before use and never store them.
//!
//! An unresolvable UUID is always an error, never a no-op: the caller's view is stale.

use super::{EffectLocation, Mixer};
use crate::engine::value::entity::{EffectTarget, Resolved, UnknownEntity};

/// A resolved effect chain, without a position inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectChain {
    Deck { channel_idx: usize, deck_idx: usize },
    Channel { channel_idx: usize },
    Master,
}

impl Mixer {
    /// The sorted, deduplicated positions of the channels `uuids` names; the key a sub-mix is
    /// prepared and looked up under. Missing channels are left out.
    pub fn channel_positions(&self, uuids: &[String]) -> Vec<usize> {
        let mut positions: Vec<usize> = uuids
            .iter()
            .filter_map(|uuid| self.find_channel_by_uuid(uuid))
            .collect();
        positions.sort_unstable();
        positions.dedup();
        positions
    }

    /// Resolve a channel UUID to its current index.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names nothing.
    pub fn resolve_channel(&self, uuid: &str) -> Resolved<usize> {
        self.find_channel_by_uuid(uuid)
            .ok_or_else(|| UnknownEntity::new("channel", uuid))
    }

    /// Resolve a deck UUID to its current `(channel_idx, deck_idx)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names nothing.
    pub fn resolve_deck(&self, uuid: &str) -> Resolved<(usize, usize)> {
        self.find_deck_by_uuid(uuid)
            .ok_or_else(|| UnknownEntity::new("deck", uuid))
    }

    /// Resolve an effect UUID to its owning chain and position.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names nothing.
    pub fn resolve_effect(&self, uuid: &str) -> Resolved<EffectLocation> {
        self.find_effect_by_uuid(uuid)
            .ok_or_else(|| UnknownEntity::new("effect", uuid))
    }

    /// Resolve a transition-sequence UUID to its current index.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names nothing.
    pub fn resolve_sequence(&self, uuid: &str) -> Resolved<usize> {
        self.transition_sequences()
            .iter()
            .position(|s| s.uuid == uuid)
            .ok_or_else(|| UnknownEntity::new("sequence", uuid))
    }

    /// Resolve the chain an [`EffectTarget`] names. Used by append and reorder, which act on the
    /// whole chain.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID the target carries names nothing.
    pub fn resolve_effect_target(&self, target: &EffectTarget) -> Resolved<EffectChain> {
        match target {
            EffectTarget::Deck(deck_uuid) => {
                let (channel_idx, deck_idx) = self.resolve_deck(deck_uuid)?;
                Ok(EffectChain::Deck {
                    channel_idx,
                    deck_idx,
                })
            }
            EffectTarget::Channel(channel_uuid) => Ok(EffectChain::Channel {
                channel_idx: self.resolve_channel(channel_uuid)?,
            }),
            EffectTarget::Master => Ok(EffectChain::Master),
        }
    }
}
