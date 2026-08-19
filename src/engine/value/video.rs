//! Video playback value types.

/// Loop mode for video playback.
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
pub enum LoopMode {
    /// Standard loop — restart from in-point when reaching out-point.
    #[default]
    Loop,
    /// Play forward then reverse repeatedly.
    PingPong,
    /// Play once and stop at the out-point.
    OneShot,
    /// Play once and hold the last frame.
    HoldLast,
}

/// Whether a video deck maps its playhead onto the show transport.
///
/// See /spec/timecode.md § Consumer 2.
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

/// Per-clip mapping of a video deck onto the show transport.
///
/// `offset` is independent of arrangement regions. `delay_frames` is in
/// transport displayed frames, not clip frames.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct DeckTransportSync {
    #[serde(default)]
    pub mode: TransportSyncMode,
    /// Transport seconds at which this clip's in-point sits.
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
