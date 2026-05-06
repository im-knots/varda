//! Shared value types for the engine layer.
//!
//! These types are used in engine trait signatures and snapshot structs.
//! They MUST NOT reference wgpu, egui, winit, or any GPU/UI framework types.

// Re-export existing clean value types from domain modules
pub use crate::channel::BlendMode;
pub use crate::deck::ScalingMode;
pub use crate::mixer::CrossfadeEasing;
pub use crate::video::LoopMode;
pub use crate::modulation::{LFOWaveform, AudioReactMode, ADSRStage, StepInterpolation, AudioBandPreset};
pub use crate::audio::AudioSourceId;
pub use crate::camera::CameraId;
pub use crate::params::ParamValue;
pub use crate::renderer::context::OutputSource;
pub use crate::surface::{ContentMapping, SurfaceOutputType, CircleHint};

/// Identifies where to apply an effect in the signal chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EffectTarget {
    /// Effect on a specific deck: (channel_idx, deck_idx)
    Deck(usize, usize),
    /// Effect on a channel's post-composite chain: (channel_idx)
    Channel(usize),
    /// Effect on the master output chain
    Master,
}

/// Per-frame engine state snapshot — plain data, no GPU types, no lifetimes.
///
/// Produced by VardaApp each frame. Distributed to consumers via watch channel.
/// UIData is derived from this for the egui UI consumer.
#[derive(Clone)]
pub struct EngineState {
    pub mixer: MixerSnapshot,
    pub audio: AudioSnapshot,
    pub modulation: ModulationSnapshot,
    pub outputs: OutputSnapshot,
    pub registry: RegistrySnapshot,
    pub midi: MidiSnapshot,
    pub cameras: CameraSnapshot,
    pub fps: f32,
    pub frame_count: u64,
}

// ── Registry Snapshot ──────────────────────────────────────────────

#[derive(Clone)]
pub struct RegistrySnapshot {
    /// Generator shaders: (name, index)
    pub generators: Vec<(String, usize)>,
    /// Filter shaders: (name, index)
    pub filters: Vec<(String, usize)>,
    /// Total shader count
    pub shader_count: usize,
}

// ── Mixer Snapshot ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct MixerSnapshot {
    pub channels: Vec<ChannelSnapshot>,
    pub crossfader: f32,
    pub auto_crossfade_active: bool,
    pub auto_crossfade_progress: f32,
    pub master_effects: Vec<EffectSnapshot>,
    pub active_transition_name: Option<String>,
    pub transition_names: Vec<String>,
    pub sequences: Vec<SequenceSnapshot>,
}

#[derive(Clone)]
pub struct ChannelSnapshot {
    pub idx: usize,
    pub name: String,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub decks: Vec<DeckSnapshot>,
    pub effects: Vec<EffectSnapshot>,
}

#[derive(Clone)]
pub struct DeckSnapshot {
    pub idx: usize,
    pub name: String,
    pub opacity: f32,
    pub effective_opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    pub scaling_mode: Option<ScalingMode>,
    pub generator: ShaderParamsSnapshot,
    pub effects: Vec<EffectSnapshot>,
    pub video_playback: Option<VideoPlaybackSnapshot>,
    pub auto_transition: Option<AutoTransitionSnapshot>,
}

#[derive(Clone)]
pub struct EffectSnapshot {
    pub name: String,
    pub enabled: bool,
    pub params: ShaderParamsSnapshot,
}

#[derive(Clone)]
pub struct ShaderParamsSnapshot {
    pub shader_name: String,
    pub params: Vec<ParamSnapshot>,
}

#[derive(Clone)]
pub struct ParamSnapshot {
    pub name: String,
    pub label: Option<String>,
    pub value: ParamValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

#[derive(Clone)]
pub struct VideoPlaybackSnapshot {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub loop_mode: LoopMode,
    pub in_point: f64,
    pub out_point: f64,
    pub frame_rate: f64,
}

#[derive(Clone)]
pub struct AutoTransitionSnapshot {
    pub enabled: bool,
    pub trigger_is_clip_end: bool,
    pub play_duration_value: f64,
    pub play_duration_is_beats: bool,
    pub transition_duration_value: f64,
    pub transition_duration_is_beats: bool,
    pub transition_shader_name: Option<String>,
    pub phase: crate::channel::DeckTransitionPhase,
}

// ── Audio Snapshot ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct AudioSnapshot {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub bpm: Option<f32>,
    pub beat_phase: f32,
    pub enabled: bool,
    pub devices: Vec<AudioDeviceSnapshot>,
    pub fft: Vec<f32>,
    pub sample_rate: f32,
}

#[derive(Clone)]
pub struct AudioDeviceSnapshot {
    pub id: AudioSourceId,
    pub name: String,
    pub active: bool,
}

// ── Modulation Snapshot ─────────────────────────────────────────────

#[derive(Clone)]
pub struct ModulationSnapshot {
    pub sources: Vec<ModulationSourceSnapshot>,
    pub current_values: Vec<f32>,
    pub assignments: std::collections::HashMap<String, Vec<ModulationAssignmentSnapshot>>,
}

#[derive(Clone)]
pub enum ModulationSourceSnapshot {
    LFO { waveform: LFOWaveform, frequency: f32, phase: f32, amplitude: f32, bipolar: bool },
    Audio { source_id: Option<AudioSourceId>, freq_low: f32, freq_high: f32, gain: f32, smoothing: f32, mode: AudioReactMode, noise_gate: f32 },
    ADSR { attack: f32, decay: f32, sustain: f32, release: f32, stage: ADSRStage },
    StepSequencer { steps: Vec<f32>, rate: f32, interpolation: StepInterpolation, bipolar: bool },
}

#[derive(Clone)]
pub struct ModulationAssignmentSnapshot {
    pub source_idx: usize,
    pub amount: f32,
}

// ── Sequence Snapshot ───────────────────────────────────────────────

#[derive(Clone)]
pub struct SequenceSnapshot {
    pub name: String,
    pub enabled: bool,
    pub playing: bool,
    pub current_step: usize,
    pub steps: Vec<SequenceStepSnapshot>,
}

#[derive(Clone)]
pub struct SequenceStepSnapshot {
    pub label: String,
    pub kind: SequenceStepKindSnapshot,
}

#[derive(Clone)]
pub enum SequenceStepKindSnapshot {
    Fade { from_ch: usize, to_ch: usize, duration_val: f64, is_beats: bool, easing: String, transition_shader: Option<String> },
    Wait { duration_val: f64, is_beats: bool },
    GoTo { step_index: usize },
}

// ── Output Snapshot ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct OutputSnapshot {
    pub windows: Vec<OutputWindowSnapshot>,
    pub surfaces: Vec<SurfaceSnapshot>,
    pub monitors: Vec<MonitorSnapshot>,
}

#[derive(Clone)]
pub struct OutputWindowSnapshot {
    pub name: String,
    pub target_label: String,
    pub is_on_display: bool,
    pub surface_assignments: Vec<SurfaceAssignmentSnapshot>,
    pub calibration_mode: bool,
}

#[derive(Clone)]
pub struct SurfaceAssignmentSnapshot {
    pub surface_idx: usize,
    pub surface_name: String,
    pub warp_corners: [[f32; 2]; 4],
    pub enabled: bool,
}

#[derive(Clone)]
pub struct SurfaceSnapshot {
    pub name: String,
    pub vertices: Vec<[f32; 2]>,
    pub extra_contours: Vec<Vec<[f32; 2]>>,
    pub source: OutputSource,
    pub content_mapping: ContentMapping,
    pub output_type: SurfaceOutputType,
    pub circle_hint: Option<CircleHint>,
}

#[derive(Clone)]
pub struct MonitorSnapshot {
    pub name: String,
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

// ── MIDI Snapshot ───────────────────────────────────────────────────

#[derive(Clone)]
pub struct MidiSnapshot {
    pub devices: Vec<MidiDeviceSnapshot>,
    pub mappings: Vec<MidiMappingSnapshot>,
    pub learn_active: bool,
    pub learn_target: Option<String>,
}

#[derive(Clone)]
pub struct MidiDeviceSnapshot {
    pub id: crate::midi::DeviceId,
    pub name: String,
    pub enabled: bool,
    pub has_output: bool,
    pub profile: String,
}

#[derive(Clone)]
pub struct MidiMappingSnapshot {
    pub key: crate::midi::MidiKey,
    pub key_display: String,
    pub device_name: String,
    pub param_path: String,
}

// ── Camera Snapshot ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct CameraSnapshot {
    pub devices: Vec<(String, CameraId)>,
}
