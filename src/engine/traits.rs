//! Domain-specific traits for engine operations.
//!
//! Interface Segregation: one trait pair (Commands + Queries) per domain.
//! Consumers import only what they need.
//!
//! Traits MUST NOT expose wgpu, egui, or internal implementation types.
//! Parameters use primitives, strings, and engine-defined value types.

use anyhow::Result;
use super::types::*;

// ── Mixer ───────────────────────────────────────────────────────────

/// Commands for controlling the mixer, channels, decks, and effects.
pub trait MixerCommands {
    fn set_crossfader(&mut self, position: f32);
    fn snap_crossfader(&mut self, position: f32);
    fn start_auto_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing);
    fn start_beat_crossfade(&mut self, target: f32, beats: f32);
    fn add_deck(&mut self, channel_idx: usize, shader_name: &str) -> Result<()>;
    fn add_image_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()>;
    fn add_video_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()>;
    fn add_solid_color_deck(&mut self, channel_idx: usize, color: [f32; 4]) -> Result<()>;
    fn add_camera_deck(&mut self, channel_idx: usize, camera_id: CameraId) -> Result<()>;
    fn remove_deck(&mut self, channel_idx: usize, deck_idx: usize) -> Result<()>;
    fn move_deck(&mut self, src_ch: usize, src_deck: usize, dst_ch: usize) -> Result<()>;
    fn set_deck_opacity(&mut self, channel_idx: usize, deck_idx: usize, opacity: f32);
    fn set_deck_blend_mode(&mut self, channel_idx: usize, deck_idx: usize, mode: BlendMode);
    fn set_deck_solo(&mut self, channel_idx: usize, deck_idx: usize, solo: bool);
    fn set_deck_mute(&mut self, channel_idx: usize, deck_idx: usize, mute: bool);
    fn set_deck_scaling_mode(&mut self, channel_idx: usize, deck_idx: usize, mode: ScalingMode);
    fn set_channel_opacity(&mut self, channel_idx: usize, opacity: f32);
    fn set_channel_blend_mode(&mut self, channel_idx: usize, mode: BlendMode);
    fn add_channel(&mut self) -> Result<usize>;
    fn remove_channel(&mut self, channel_idx: usize) -> Result<()>;
    fn add_effect(&mut self, target: EffectTarget, shader_name: &str) -> Result<()>;
    fn remove_effect(&mut self, target: EffectTarget, effect_idx: usize);
    fn toggle_effect(&mut self, target: EffectTarget, effect_idx: usize);
    fn move_effect(&mut self, target: EffectTarget, from_idx: usize, to_idx: usize);
    fn set_transition(&mut self, shader_name: Option<&str>) -> Result<()>;
    fn set_param(&mut self, path: &str, value: ParamValue);
}

/// Read-only queries for mixer state.
pub trait MixerQueries {
    fn mixer_snapshot(&self) -> MixerSnapshot;
}

// ── Audio ───────────────────────────────────────────────────────────

/// Commands for controlling audio input.
pub trait AudioCommands {
    fn open_audio_source(&mut self, source_id: AudioSourceId) -> Result<()>;
    fn close_audio_source(&mut self, source_id: AudioSourceId);
    fn scan_audio_devices(&mut self);
}

/// Read-only queries for audio state.
pub trait AudioQueries {
    fn audio_snapshot(&self) -> AudioSnapshot;
}

// ── Modulation ──────────────────────────────────────────────────────

/// Commands for controlling the modulation engine.
pub trait ModulationCommands {
    fn add_lfo(&mut self, waveform: LFOWaveform, frequency: f32) -> usize;
    fn add_audio_band(&mut self, preset: AudioBandPreset, source_id: Option<AudioSourceId>) -> usize;
    fn add_adsr(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> usize;
    fn add_step_sequencer(&mut self, num_steps: usize, rate: f32) -> usize;
    fn remove_modulation_source(&mut self, idx: usize);
    fn assign_modulation(&mut self, target: &str, source_idx: usize, amount: f32);
    fn clear_modulation(&mut self, target: &str);
}

/// Read-only queries for modulation state.
pub trait ModulationQueries {
    fn modulation_snapshot(&self) -> ModulationSnapshot;
}

// ── Output ──────────────────────────────────────────────────────────

/// Commands for controlling outputs and surfaces.
pub trait OutputCommands {
    fn request_create_output(&mut self);
    fn close_output(&mut self, idx: usize);
    fn set_output_display(&mut self, idx: usize, monitor_name: &str);
}

/// Read-only queries for output state.
pub trait OutputQueries {
    fn output_snapshot(&self) -> OutputSnapshot;
}
