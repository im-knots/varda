//! UUID addressing. Write commands address their target by UUID; a domain that
//! cannot resolve one returns [`UnknownEntity`], which the command layer maps
//! to "not found".

use serde::{Deserialize, Serialize};

/// Identifies which effect chain to operate on.
///
/// Used for chain-scoped operations (append, reorder). Operations on an
/// existing effect address it by its own UUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub enum EffectTarget {
    /// A deck's pre-composite chain, by deck UUID.
    Deck(String),
    /// A channel's post-composite chain, by channel UUID.
    Channel(String),
    /// The master output chain.
    Master,
}

/// A UUID that named nothing. Carries the entity kind so the command layer can
/// build a useful `404`.
#[derive(Debug, Clone)]
pub struct UnknownEntity {
    pub kind: &'static str,
    pub uuid: String,
}

impl UnknownEntity {
    pub fn new(kind: &'static str, uuid: &str) -> Self {
        Self {
            kind,
            uuid: uuid.to_string(),
        }
    }
}

impl std::fmt::Display for UnknownEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No {} with UUID '{}'", self.kind, self.uuid)
    }
}

impl std::error::Error for UnknownEntity {}

/// The result of resolving a UUID.
pub type Resolved<T> = Result<T, UnknownEntity>;

#[cfg(test)]
mod tests {
    use super::*;

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
}
