//! UUID → position resolution for command handling.
//!
//! Every write command addresses its target by UUID (see
//! [`/spec/api-addressing.md`]). These helpers turn a UUID into the transient
//! index the internal data structures need. The returned indices are valid only
//! until the containing collection is mutated, so resolve immediately before
//! use and never store the result.
//!
//! Failure is always "not found", never a silent no-op: an unresolvable UUID
//! means the caller's view of the world is stale, and the caller needs to know.

use super::VardaApp;
use crate::mixer::EffectLocation;

/// A UUID that named nothing. Carries the entity kind so the command layer can
/// build a useful `404`.
#[derive(Debug, Clone)]
pub struct UnknownEntity {
    pub kind: &'static str,
    pub uuid: String,
}

impl std::fmt::Display for UnknownEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No {} with UUID '{}'", self.kind, self.uuid)
    }
}

impl std::error::Error for UnknownEntity {}

impl From<UnknownEntity> for crate::engine::CommandResult {
    fn from(e: UnknownEntity) -> Self {
        crate::engine::CommandResult::Err {
            code: crate::engine::ErrorCode::NotFound,
            message: e.to_string(),
        }
    }
}

impl UnknownEntity {
    fn new(kind: &'static str, uuid: &str) -> Self {
        Self {
            kind,
            uuid: uuid.to_string(),
        }
    }
}

pub type Resolved<T> = Result<T, UnknownEntity>;

impl VardaApp {
    /// Resolve a channel UUID to its current index.
    pub(crate) fn resolve_channel(&self, uuid: &str) -> Resolved<usize> {
        self.mixer
            .find_channel_by_uuid(uuid)
            .ok_or_else(|| UnknownEntity::new("channel", uuid))
    }

    /// Resolve a deck UUID to its current `(channel_idx, deck_idx)`.
    pub(crate) fn resolve_deck(&self, uuid: &str) -> Resolved<(usize, usize)> {
        self.mixer
            .find_deck_by_uuid(uuid)
            .ok_or_else(|| UnknownEntity::new("deck", uuid))
    }

    /// Resolve an effect UUID to its owning chain and position.
    pub(crate) fn resolve_effect(&self, uuid: &str) -> Resolved<EffectLocation> {
        self.mixer
            .find_effect_by_uuid(uuid)
            .ok_or_else(|| UnknownEntity::new("effect", uuid))
    }

    /// Resolve an output UUID to its current index.
    pub(crate) fn resolve_output(&self, uuid: &str) -> Resolved<usize> {
        self.output
            .outputs
            .iter()
            .position(|o| o.uuid() == uuid)
            .ok_or_else(|| UnknownEntity::new("output", uuid))
    }

    /// Resolve a transition-sequence UUID to its current index.
    pub(crate) fn resolve_sequence(&self, uuid: &str) -> Resolved<usize> {
        self.mixer
            .transition_sequences()
            .iter()
            .position(|s| s.uuid == uuid)
            .ok_or_else(|| UnknownEntity::new("sequence", uuid))
    }

    /// Resolve the chain an [`EffectTarget`] names, without naming an effect.
    /// Used by append and reorder, which are chain-scoped rather than
    /// effect-scoped.
    pub(crate) fn resolve_effect_target(
        &self,
        target: &crate::engine::EffectTarget,
    ) -> Resolved<EffectChain> {
        match target {
            crate::engine::EffectTarget::Deck(deck_uuid) => {
                let (channel_idx, deck_idx) = self.resolve_deck(deck_uuid)?;
                Ok(EffectChain::Deck {
                    channel_idx,
                    deck_idx,
                })
            }
            crate::engine::EffectTarget::Channel(channel_uuid) => Ok(EffectChain::Channel {
                channel_idx: self.resolve_channel(channel_uuid)?,
            }),
            crate::engine::EffectTarget::Master => Ok(EffectChain::Master),
        }
    }
}

/// A resolved effect chain, without a position inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectChain {
    Deck { channel_idx: usize, deck_idx: usize },
    Channel { channel_idx: usize },
    Master,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{CommandResult, ErrorCode};

    #[test]
    fn new_populates_kind_and_uuid() {
        let e = UnknownEntity::new("deck", "abc123");
        assert_eq!(e.kind, "deck");
        assert_eq!(e.uuid, "abc123");
    }

    #[test]
    fn display_names_the_kind_and_uuid() {
        let e = UnknownEntity::new("channel", "ch-7");
        assert_eq!(e.to_string(), "No channel with UUID 'ch-7'");
    }

    #[test]
    fn into_command_result_is_not_found_with_display_message() {
        // Every resolve_* error path relies on this `.into()` to build a 404:
        // the code must be NotFound and the message must match the Display form.
        let e = UnknownEntity::new("effect", "fx-99");
        let expected = e.to_string();
        let result: CommandResult = e.into();
        match result {
            CommandResult::Err { code, message } => {
                assert_eq!(code, ErrorCode::NotFound);
                assert_eq!(message, expected);
            }
            other => panic!("expected Err(NotFound), got {other:?}"),
        }
    }
}
