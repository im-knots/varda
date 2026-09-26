//! Delivering rendered frames to where they are going: ffmpeg for recordings
//! and network streams, and the NDI sender. The renderer produces read-back
//! frames; this module decides what a target can carry and sends each frame on.
//! The renderer never names it. See /spec/vardapp-decomposition.md.

pub mod ffmpeg;
pub mod presentation;

pub use ffmpeg::*;

use crate::engine::value::render::OutputTarget;

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

/// Result of delivering a frame to an output target.
pub enum DeliveryResult {
    /// Frame delivered successfully (or no-op for unhandled targets).
    Ok,
    /// Subprocess write failed — output should be deactivated.
    Failed(String),
    /// SRT client disconnected: the old subprocess has been stopped and the
    /// caller must respawn the listener. The caller owns the respawn so it can
    /// re-subscribe audio passthrough, which needs the audio manager.
    SrtNeedsRestart,
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

/// What an active headless output sends through: the ffmpeg process a
/// recording or stream runs, and the audio feeding it.
#[derive(Default)]
pub struct Delivery {
    /// The encoder, for recording and ffmpeg stream targets.
    pub subprocess: Option<FfmpegSubprocess>,
    /// The audio passthrough subscription, when the target carries audio.
    pub audio: Option<AudioPassthrough>,
}

impl Delivery {
    /// Hand one read-back frame to `target`.
    ///
    /// With an encoder running, the frame goes to it; a frame it cannot take
    /// ends the encode, and an SRT listener is handed back for a restart. With
    /// no encoder, an NDI target publishes through `ndi`, declaring `fps`, the
    /// master render rate. `name` labels a failure.
    pub fn deliver(
        &mut self,
        target: &OutputTarget,
        name: &str,
        frame: &crate::renderer::ReadbackFrame,
        ndi: &mut crate::ndi::NdiManager,
        fps: u32,
    ) -> DeliveryResult {
        if let Some(sub) = &mut self.subprocess {
            if sub.feed_readback_frame(frame) {
                return DeliveryResult::Ok;
            }
            if let Some(mut sub) = self.subprocess.take() {
                sub.stop();
            }
            return if matches!(target, OutputTarget::SrtStream { .. }) {
                DeliveryResult::SrtNeedsRestart
            } else {
                DeliveryResult::Failed(format!("FFmpeg frame contract failed for '{name}'"))
            };
        }
        match target {
            OutputTarget::NdiSend { sender_name } => {
                ndi.send_frame(
                    sender_name,
                    frame.bytes(),
                    frame.width(),
                    frame.height(),
                    fps,
                );
                DeliveryResult::Ok
            }
            // Syphon output is published GPU-side (zero-copy) in the headless
            // render loop before this point, so on macOS it never reaches here.
            #[cfg(not(target_os = "macos"))]
            OutputTarget::SyphonServer { .. } => {
                log::warn!("Syphon output not supported on this platform");
                DeliveryResult::Ok
            }
            _ => DeliveryResult::Ok,
        }
    }

    /// How the audio passthrough is keeping up, when the output carries audio.
    pub fn audio_health(&self) -> Option<AudioHealth> {
        let audio = self.audio.as_ref()?;
        let sub = self.subprocess.as_ref();
        Some(AudioHealth {
            frames_written: sub
                .and_then(FfmpegSubprocess::audio_frames_written)
                .unwrap_or(0),
            frames_dropped: audio.dropped.load(std::sync::atomic::Ordering::Relaxed),
            silence_spliced: sub
                .and_then(FfmpegSubprocess::audio_silence_spliced)
                .unwrap_or(0),
        })
    }

    /// How the encoder is keeping up, when one is running.
    pub fn encoder_health(&self) -> Option<EncoderHealth> {
        self.subprocess.as_ref().map(|sub| EncoderHealth {
            frames_written: sub.frames_written(),
            frames_dropped: sub.frames_dropped(),
            frames_padded: sub.frames_padded(),
        })
    }

    /// How long the encoder has been running, if there is one.
    pub fn duration(&self) -> Option<std::time::Duration> {
        self.subprocess.as_ref().map(FfmpegSubprocess::duration)
    }

    /// Stop the encoder, if any. Returns the audio subscription so the caller
    /// can unsubscribe it from the audio manager.
    pub fn stop(&mut self) -> Option<AudioPassthrough> {
        if let Some(mut sub) = self.subprocess.take() {
            sub.stop();
        }
        self.audio.take()
    }
}
