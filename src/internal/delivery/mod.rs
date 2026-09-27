//! Delivering rendered frames to where they are going: ffmpeg for recordings
//! and network streams. The renderer produces read-back frames; this module
//! decides what a target can carry and sends each frame on.
//! The renderer never names it. See /spec/vardapp-decomposition.md.

pub mod ffmpeg;
pub mod presentation;

pub use ffmpeg::*;

/// A live audio passthrough subscription held by an active output, used to
/// unsubscribe on stop and to report passthrough health (dropped chunks).
/// See spec/audio-passthrough.md.
pub struct AudioPassthrough {
    /// The audio source this output is tee'd from.
    pub source_id: crate::audio::AudioSourceId,
    /// Subscription token, for unsubscribe on stop.
    pub token: crate::audio::PcmToken,
    /// PCM chunks dropped on backpressure (producer side health stat).
    pub dropped: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// Audio passthrough health: frames the encoder took, chunks dropped on
/// backpressure, and silence spliced in for gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioHealth {
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub silence_spliced: u64,
}

/// Encoder health: frames written, dropped when the encoder fell behind, and
/// padded to hold the output frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderHealth {
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub frames_padded: u64,
}
