//! Deck source values: what crosses the engine boundary about a deck's
//! source beyond the shared provider vocabulary in
//! [`crate::engine::value::provider`].
//!
//! See /spec/deck-source-providers.md.

use serde::{Deserialize, Serialize};

use super::provider::{ControlStatus, ProviderConfig};

/// A deck source as it is saved and sent: a type id plus that type's fields.
pub type SourceConfig = ProviderConfig;

/// A deck's source in a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct DeckSourceSnapshot {
    /// Source type id; its schema is in [`ProviderTypeSnapshot`](super::provider::ProviderTypeSnapshot).
    #[serde(rename = "type")]
    pub source_type: String,
    /// False when this build cannot run the type, and the deck is a
    /// placeholder holding its config. See /spec/deck-source-providers.md
    /// Decision 4.
    pub available: bool,
    pub status: ControlStatus,
}

/// Scaling mode for sources that are drawn onto the deck from a texture of
/// their own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum ScalingMode {
    /// Scale to fill the entire target, cropping edges if aspect ratio differs
    #[default]
    Fill,
    /// Scale to fit within the target, letterboxing if aspect ratio differs
    Fit,
    /// Stretch to exactly match target dimensions (may distort)
    Stretch,
    /// No scaling, center at native resolution
    Center,
}

impl ScalingMode {
    pub const ALL: [Self; 4] = [Self::Fill, Self::Fit, Self::Stretch, Self::Center];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
            Self::Stretch => "Stretch",
            Self::Center => "Center",
        }
    }
}
