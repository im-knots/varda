//! Deck source values beyond the shared provider types in
//! [`crate::engine::value::provider`].

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
    /// False when this build cannot run the type; the deck is then a
    /// placeholder holding its config.
    pub available: bool,
    /// The source sets its own alpha, so the deck's transparent flag is ignored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owns_alpha: bool,
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

/// Whether a deck that follows a timeline (a clip, timed text) maps it onto
/// the show transport.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
pub enum TransportSyncMode {
    /// Chase while the transport is running; free-run (with loop modes) when it is not.
    #[default]
    Auto,
    /// Chase even while the transport is stopped: freeze on the mapped frame.
    Always,
    /// Never chase.
    Never,
}

impl TransportSyncMode {
    /// Whether this mode chases given the transport's running flag.
    #[must_use]
    pub const fn is_chasing(self, transport_running: bool) -> bool {
        match self {
            Self::Auto => transport_running,
            Self::Always => true,
            Self::Never => false,
        }
    }

    /// Short label for the deck-detail combo.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Always => "Always",
            Self::Never => "Never",
        }
    }
}

/// Per-deck mapping of a clip or timed text onto the show transport.
///
/// `offset` is independent of arrangement regions. `delay_frames` is in
/// transport displayed frames, not clip frames.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct DeckTransportSync {
    #[serde(default)]
    pub mode: TransportSyncMode,
    /// Transport seconds at which the clip's in-point or the text's time zero
    /// sits.
    #[serde(default)]
    pub offset: f64,
    /// Signed latency in transport displayed frames.
    #[serde(default)]
    pub delay_frames: i32,
}

impl Default for DeckTransportSync {
    fn default() -> Self {
        Self {
            mode: TransportSyncMode::Auto,
            offset: 0.0,
            delay_frames: 0,
        }
    }
}
