pub mod notifications;
pub mod panels;
pub mod state;
pub mod widgets;

use crate::mixer::CrossfadeEasing;
use crate::modulation::{LFOWaveform, AudioBand, ADSRStage, StepInterpolation};
use crate::params::ParamValue;
use crate::renderer::context::OutputSource;
use crate::{BlendMode, ScalingMode, ShaderParams};

/// Fixed render resolution for all decks and stage output (Full HD 1080p)
pub const RENDER_WIDTH: u32 = 1920;
pub const RENDER_HEIGHT: u32 = 1080;

/// Parameter info for UI rendering (collected before egui to avoid borrow conflicts)
#[derive(Clone)]
pub struct ParamUIInfo {
    pub name: String,
    pub label: Option<String>,
    pub value: ParamValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

/// Shader parameters info for UI (generator or effect)
#[derive(Clone)]
pub struct ShaderParamsUI {
    pub shader_name: String,
    pub params: Vec<ParamUIInfo>,
}

/// Parameter update to apply after egui
pub enum ParamUpdate {
    GeneratorFloat { ch_idx: usize, deck_idx: usize, name: String, value: f32 },
    GeneratorBool { ch_idx: usize, deck_idx: usize, name: String, value: bool },
    GeneratorColor { ch_idx: usize, deck_idx: usize, name: String, value: [f32; 4] },
    GeneratorResetToDefaults { ch_idx: usize, deck_idx: usize },
    EffectFloat { ch_idx: usize, deck_idx: usize, effect_idx: usize, name: String, value: f32 },
    EffectBool { ch_idx: usize, deck_idx: usize, effect_idx: usize, name: String, value: bool },
    EffectColor { ch_idx: usize, deck_idx: usize, effect_idx: usize, name: String, value: [f32; 4] },
    ChannelEffectFloat { ch_idx: usize, effect_idx: usize, name: String, value: f32 },
    ChannelEffectBool { ch_idx: usize, effect_idx: usize, name: String, value: bool },
    ChannelEffectColor { ch_idx: usize, effect_idx: usize, name: String, value: [f32; 4] },
    MasterEffectFloat { effect_idx: usize, name: String, value: f32 },
    MasterEffectBool { effect_idx: usize, name: String, value: bool },
    MasterEffectColor { effect_idx: usize, name: String, value: [f32; 4] },
}

/// Modulation action from UI
pub enum ModulationAction {
    AddLFO { waveform: LFOWaveform, frequency: f32 },
    AddAudioBand { band: AudioBand },
    AddADSR { attack: f32, decay: f32, sustain: f32, release: f32 },
    AddStepSequencer { num_steps: usize, rate: f32 },
    RemoveSource { idx: usize },
    UpdateLFOFrequency { idx: usize, frequency: f32 },
    UpdateLFOWaveform { idx: usize, waveform: LFOWaveform },
    UpdateLFOPhase { idx: usize, phase: f32 },
    UpdateLFOAmplitude { idx: usize, amplitude: f32 },
    UpdateLFOBipolar { idx: usize, bipolar: bool },
    UpdateAudioSmoothing { idx: usize, smoothing: f32 },
    // ADSR updates
    UpdateADSRAttack { idx: usize, attack: f32 },
    UpdateADSRDecay { idx: usize, decay: f32 },
    UpdateADSRSustain { idx: usize, sustain: f32 },
    UpdateADSRRelease { idx: usize, release: f32 },
    TriggerADSR { idx: usize },
    ReleaseADSR { idx: usize },
    // Step sequencer updates
    UpdateStepValue { idx: usize, step_idx: usize, value: f32 },
    UpdateStepRate { idx: usize, rate: f32 },
    UpdateStepInterpolation { idx: usize, interpolation: StepInterpolation },
    UpdateStepBipolar { idx: usize, bipolar: bool },
    // Mod-on-mod: assign a modulator to another modulator's parameter
    AssignModOnMod { target_source_idx: usize, param_name: String, modulator_idx: usize, amount: f32 },
    RemoveModOnMod { target_source_idx: usize, param_name: String },
    AssignModulation { ch_idx: usize, deck_idx: usize, param_name: String, source_idx: usize, amount: f32 },
    RemoveAssignment { ch_idx: usize, deck_idx: usize, param_name: String, source_idx: usize },
    AssignEffectModulation { ch_idx: usize, deck_idx: usize, effect_idx: usize, param_name: String, source_idx: usize, amount: f32 },
    RemoveEffectAssignment { ch_idx: usize, deck_idx: usize, effect_idx: usize, param_name: String },
    AssignChannelEffectModulation { ch_idx: usize, effect_idx: usize, param_name: String, source_idx: usize, amount: f32 },
    RemoveChannelEffectAssignment { ch_idx: usize, effect_idx: usize, param_name: String },
    AssignMasterEffectModulation { effect_idx: usize, param_name: String, source_idx: usize, amount: f32 },
    RemoveMasterEffectAssignment { effect_idx: usize, param_name: String },
}

/// Modulation source data snapshot for UI display
#[derive(Clone)]
pub enum ModSourceUI {
    LFO { waveform: LFOWaveform, frequency: f32, phase: f32, amplitude: f32, bipolar: bool },
    Audio { band: AudioBand, smoothing: f32 },
    ADSR { attack: f32, decay: f32, sustain: f32, release: f32, stage: ADSRStage },
    StepSequencer { steps: Vec<f32>, rate: f32, interpolation: StepInterpolation, bipolar: bool },
}

/// Fixed color palette for modulation sources — each modulator gets a unique, consistent color
pub const MODULATOR_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0, 220, 220),    // Cyan
    egui::Color32::from_rgb(220, 60, 220),    // Magenta
    egui::Color32::from_rgb(220, 200, 40),    // Yellow
    egui::Color32::from_rgb(100, 220, 60),    // Lime
    egui::Color32::from_rgb(240, 140, 40),    // Orange
    egui::Color32::from_rgb(240, 100, 140),   // Pink
    egui::Color32::from_rgb(80, 160, 240),    // Sky Blue
    egui::Color32::from_rgb(240, 120, 80),    // Coral
];

/// Get the color for a modulation source by index
pub fn modulator_color(idx: usize) -> egui::Color32 {
    MODULATOR_COLORS[idx % MODULATOR_COLORS.len()]
}

/// Modulation assignment snapshot for UI display
#[derive(Clone)]
pub struct ModAssignmentUI {
    pub source_idx: usize,
    pub amount: f32,
}

/// Effect info tuple for UI: (name, enabled, params)
pub type EffectInfo = (String, bool, ShaderParamsUI);

/// Deck info for UI display
#[derive(Clone)]
pub struct DeckUIInfo {
    pub deck_idx: usize,
    pub name: String,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    pub scaling_mode: Option<ScalingMode>,
    pub generator: ShaderParamsUI,
    pub effects: Vec<EffectInfo>,
}

/// Channel info for UI display
#[derive(Clone)]
pub struct ChannelUIInfo {
    pub ch_idx: usize,
    pub name: String,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub decks: Vec<DeckUIInfo>,
    pub effects: Vec<EffectInfo>,
}

/// Audio data snapshot for UI display
#[derive(Clone)]
pub struct AudioUIData {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub bpm: Option<f32>,
    pub beat_phase: f32,
    pub enabled: bool,
}

/// Notification snapshot for UI rendering (avoids borrowing NotificationSystem during egui)
#[derive(Clone)]
pub struct NotificationUI {
    pub level: notifications::NotificationLevel,
    pub message: String,
    pub progress: f32,
}

/// All collected data needed to render the UI
pub struct UIData {
    pub generators: Vec<(String, usize)>,
    pub filters: Vec<(String, usize)>,
    pub shader_count: usize,
    pub channels: Vec<ChannelUIInfo>,
    pub master_effect_info: Vec<EffectInfo>,
    pub modulation_sources: Vec<ModSourceUI>,
    /// Current computed values for each modulation source (for visualization)
    pub modulation_current_values: Vec<f32>,
    /// Modulation assignments: param_key -> list of (source_idx, amount)
    pub modulation_assignments: std::collections::HashMap<String, Vec<ModAssignmentUI>>,
    pub audio: AudioUIData,
    /// Deck preview textures keyed by (ch_idx, deck_idx)
    pub deck_preview_textures: std::collections::HashMap<(usize, usize), egui::TextureId>,
    pub main_output_texture: Option<egui::TextureId>,
    pub notifications: Vec<NotificationUI>,
    /// Crossfader position (0.0 = A, 1.0 = B)
    pub crossfader: f32,
    /// Whether an auto-crossfade is currently running
    pub auto_crossfade_active: bool,
    /// Progress of auto-crossfade (0.0–1.0), if active
    pub auto_crossfade_progress: f32,
    /// Whether MIDI learn mode is active
    pub midi_learn_active: bool,
    /// The parameter path currently waiting for MIDI learn
    pub midi_learn_target: Option<String>,
    /// Available transition shader names (from registry)
    pub transition_names: Vec<String>,
    /// Currently active transition name, if any
    pub active_transition_name: Option<String>,
    /// Currently selected deck for detail view in bottom bar (ch_idx, deck_idx)
    pub selected_deck: Option<(usize, usize)>,
    /// Currently selected channel for detail view in bottom bar (ch_idx)
    pub selected_channel: Option<usize>,
    /// Whether the master output is selected for detail view in bottom bar
    pub selected_master: bool,
    /// Output windows state for UI display
    pub output_windows: Vec<OutputWindowUI>,
}

/// Output window action from UI
pub enum OutputAction {
    /// Create a new output window with the given source
    Create { source: OutputSource },
    /// Close an output window by index
    Close { idx: usize },
    /// Change the source routing for an output window
    SetSource { idx: usize, source: OutputSource },
    /// Toggle fullscreen on an output window
    ToggleFullscreen { idx: usize },
}

/// Snapshot of an output window's state for UI display
#[derive(Clone)]
pub struct OutputWindowUI {
    pub name: String,
    pub source: OutputSource,
    pub is_fullscreen: bool,
}

/// Crossfader action from UI
pub enum CrossfaderAction {
    /// Set crossfader manually (drag)
    SetPosition(f32),
    /// Snap to A (0.0) immediately
    SnapA,
    /// Snap to B (1.0) immediately
    SnapB,
    /// Start auto-transition to target over duration with easing
    AutoTransition { target: f32, duration_secs: f32, easing: CrossfadeEasing },
    /// Start beat-synced transition
    BeatTransition { target: f32, beats: f32 },
}

/// All UI actions/intents collected during a frame
pub struct UIActions {
    /// (ch_idx, generator_registry_idx) — add a shader as a new deck to channel
    pub shader_to_add: Option<(usize, usize)>,
    /// (ch_idx, path) — add an image file as a new deck to channel
    pub image_to_add: Option<(usize, std::path::PathBuf)>,
    /// (ch_idx, color_rgba) — add a solid color deck to channel
    pub solid_color_to_add: Option<(usize, [f32; 4])>,
    /// (ch_idx, deck_idx) — remove deck from channel
    pub deck_to_remove: Option<(usize, usize)>,
    /// (ch_idx, deck_idx, opacity, blend_mode, solo, mute)
    pub deck_updates: Vec<(usize, usize, f32, BlendMode, bool, bool)>,
    /// (ch_idx, deck_idx, scaling_mode) — change scaling mode for an image deck
    pub scaling_mode_updates: Vec<(usize, usize, ScalingMode)>,
    /// (ch_idx, deck_idx, filter_registry_idx) — add effect to deck
    pub effect_to_add: Option<(usize, usize, usize)>,
    /// (ch_idx, deck_idx, effect_idx) — remove effect from deck
    pub effect_to_remove: Option<(usize, usize, usize)>,
    /// (ch_idx, deck_idx, effect_idx) — toggle effect
    pub effect_to_toggle: Option<(usize, usize, usize)>,
    /// (ch_idx, filter_registry_idx) — add effect to channel
    pub ch_effect_to_add: Option<(usize, usize)>,
    /// (ch_idx, effect_idx) — remove effect from channel
    pub ch_effect_to_remove: Option<(usize, usize)>,
    /// (ch_idx, effect_idx) — toggle channel effect
    pub ch_effect_to_toggle: Option<(usize, usize)>,
    pub param_updates: Vec<ParamUpdate>,
    pub modulation_actions: Vec<ModulationAction>,
    pub master_effect_to_add: Option<usize>,
    pub master_effect_to_remove: Option<usize>,
    pub master_effect_to_toggle: Option<usize>,
    pub notifications_to_dismiss: Vec<usize>,
    pub crossfader_action: Option<CrossfaderAction>,
    /// (ch_idx, opacity, blend_mode) — per-channel updates
    pub channel_updates: Vec<(usize, f32, BlendMode)>,
    /// MIDI learn: start learning for a parameter path
    pub midi_learn_start: Option<String>,
    /// MIDI learn: cancel current learn mode
    pub midi_learn_cancel: bool,
    /// Set a transition shader by name (None = clear/use opacity crossfade)
    pub set_transition: Option<Option<String>>,
    /// Select a deck for detail view in bottom bar (ch_idx, deck_idx)
    pub select_deck: Option<(usize, usize)>,
    /// Select a channel for detail view in bottom bar (ch_idx)
    pub select_channel: Option<usize>,
    /// Select master output for detail view in bottom bar
    pub select_master: bool,
    /// Add a new channel to the mixer
    pub add_channel: bool,
    /// Remove a channel from the mixer (by index)
    pub remove_channel: Option<usize>,
    /// Move a deck between channels: (source_ch_idx, source_deck_idx, target_ch_idx)
    pub deck_to_move: Option<(usize, usize, usize)>,
    /// Output window actions
    pub output_actions: Vec<OutputAction>,
}

impl UIActions {
    pub fn new() -> Self {
        Self {
            shader_to_add: None,
            image_to_add: None,
            solid_color_to_add: None,
            deck_to_remove: None,
            deck_updates: Vec::new(),
            scaling_mode_updates: Vec::new(),
            effect_to_add: None,
            effect_to_remove: None,
            effect_to_toggle: None,
            ch_effect_to_add: None,
            ch_effect_to_remove: None,
            ch_effect_to_toggle: None,
            param_updates: Vec::new(),
            modulation_actions: Vec::new(),
            master_effect_to_add: None,
            master_effect_to_remove: None,
            master_effect_to_toggle: None,
            notifications_to_dismiss: Vec::new(),
            crossfader_action: None,
            channel_updates: Vec::new(),
            midi_learn_start: None,
            midi_learn_cancel: false,
            set_transition: None,
            select_deck: None,
            select_channel: None,
            select_master: false,
            add_channel: false,
            remove_channel: None,
            deck_to_move: None,
            output_actions: Vec::new(),
        }
    }
}

/// Helper to extract params from ShaderParams for UI display
pub fn collect_params(params: &ShaderParams) -> Vec<ParamUIInfo> {
    params.param_order.iter().filter_map(|name| {
        let value = params.values.get(name)?;
        let def = params.definitions.get(name);
        Some(ParamUIInfo {
            name: name.clone(),
            label: def.and_then(|d| d.label.clone()),
            value: *value,
            min: def.and_then(|d| d.min),
            max: def.and_then(|d| d.max),
        })
    }).collect()
}

