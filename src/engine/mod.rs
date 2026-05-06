//! Engine layer — domain contracts (traits + types).
//!
//! This module defines the public API for the Varda engine.
//! NO implementation, NO GPU types. Pure contracts.
//!
//! Consumers (UI, HTTP API, CLI) program against these traits.
//! The concrete implementation lives in `src/app/`.

pub mod traits;
pub mod types;

pub use traits::*;
pub use types::*;

/// Cross-thread command envelope for message-passing consumers.
///
/// Each variant mirrors a trait method 1:1. Cross-thread consumers
/// (HTTP API, CLI) send these via `mpsc::Sender<EngineCommand>`.
/// The engine processes them once per frame.
#[derive(Debug)]
pub enum EngineCommand {
    // ── Mixer ──────────────────────────────────────────────────
    SetCrossfader(f32),
    AutoCrossfade { target: f32, duration_secs: f32, easing: CrossfadeEasing },
    BeatCrossfade { target: f32, beats: f32 },
    AddDeck { channel_idx: usize, shader_name: String },
    AddImageDeck { channel_idx: usize, path: std::path::PathBuf },
    AddVideoDeck { channel_idx: usize, path: std::path::PathBuf },
    AddSolidColorDeck { channel_idx: usize, color: [f32; 4] },
    AddCameraDeck { channel_idx: usize, camera_id: CameraId },
    RemoveDeck { channel_idx: usize, deck_idx: usize },
    MoveDeck { src_ch: usize, src_deck: usize, dst_ch: usize },
    SetDeckOpacity { channel_idx: usize, deck_idx: usize, opacity: f32 },
    SetDeckBlendMode { channel_idx: usize, deck_idx: usize, mode: BlendMode },
    SetDeckSolo { channel_idx: usize, deck_idx: usize, solo: bool },
    SetDeckMute { channel_idx: usize, deck_idx: usize, mute: bool },
    SetDeckScalingMode { channel_idx: usize, deck_idx: usize, mode: ScalingMode },
    SetChannelOpacity { channel_idx: usize, opacity: f32 },
    SetChannelBlendMode { channel_idx: usize, mode: BlendMode },
    AddChannel,
    RemoveChannel { channel_idx: usize },
    AddEffect { target: EffectTarget, shader_name: String },
    RemoveEffect { target: EffectTarget, effect_idx: usize },
    ToggleEffect { target: EffectTarget, effect_idx: usize },
    MoveEffect { target: EffectTarget, from_idx: usize, to_idx: usize },
    SetTransition { shader_name: Option<String> },
    SetParam { path: String, value: ParamValue },

    // ── Audio ──────────────────────────────────────────────────
    OpenAudioSource { source_id: AudioSourceId },
    CloseAudioSource { source_id: AudioSourceId },
    ScanAudioDevices,

    // ── Modulation ─────────────────────────────────────────────
    AddLfo { waveform: LFOWaveform, frequency: f32 },
    AddAudioBand { preset: AudioBandPreset, source_id: Option<AudioSourceId> },
    AddAdsr { attack: f32, decay: f32, sustain: f32, release: f32 },
    AddStepSequencer { num_steps: usize, rate: f32 },
    RemoveModulationSource { idx: usize },
    AssignModulation { target: String, source_idx: usize, amount: f32 },
    ClearModulation { target: String },

    // ── Output ─────────────────────────────────────────────────
    CreateOutput,
    CloseOutput { idx: usize },
    SetOutputDisplay { idx: usize, monitor_name: String },
}
