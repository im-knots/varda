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
    /// Restart from the in-point at the out-point.
    #[default]
    Loop,
    /// Play forward then reverse repeatedly.
    PingPong,
    /// Play once and stop at the out-point.
    OneShot,
    /// Play once and hold the last frame.
    HoldLast,
}
