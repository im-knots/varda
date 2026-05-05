pub mod notifications;
pub mod panels;
pub mod state;
pub mod widgets;

use crate::mixer::CrossfadeEasing;
use crate::modulation::{LFOWaveform, AudioBand, ADSRStage, StepInterpolation};
use crate::params::ParamValue;
use crate::renderer::context::OutputSource;
use crate::surface::{CircleHint, ContentMapping, SurfaceOutputType};
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
    /// Surfaces in the stage layout
    pub surfaces: Vec<SurfaceUI>,
    /// Whether the full-screen stage editor is open (replaces deck view)
    pub stage_editor_open: bool,
    /// Whether the library panel (left sidebar) is open
    pub library_panel_open: bool,
    /// Stage editor grid size (normalized, e.g. 0.05 = 20 divisions)
    pub stage_editor_grid_size: f32,
    /// Whether snap-to-grid is enabled in the stage editor
    pub stage_editor_snap: bool,
    /// Available display monitors (refreshed each frame)
    pub available_monitors: Vec<MonitorInfo>,
    /// Connected MIDI devices
    pub midi_devices: Vec<MidiDeviceUI>,
    /// Current MIDI mappings (for display)
    pub midi_mappings: Vec<MidiMappingUI>,
    /// Available camera devices (name, id)
    pub cameras: Vec<(String, crate::camera::CameraId)>,
}

/// Info about an available display monitor (for UI display selector)
#[derive(Clone)]
pub struct MonitorInfo {
    pub name: String,
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

/// MIDI device info for UI display.
#[derive(Clone)]
pub struct MidiDeviceUI {
    pub id: crate::midi::DeviceId,
    pub name: String,
    pub enabled: bool,
    pub has_output: bool,
    pub profile: String,
}

/// MIDI mapping entry for UI display.
#[derive(Clone)]
pub struct MidiMappingUI {
    pub key: crate::midi::MidiKey,
    pub key_display: String,
    pub device_name: String,
    pub param_path: String,
}

/// Output window action from UI
pub enum OutputAction {
    /// Create a new output window
    Create,
    /// Close an output window by index
    Close { idx: usize },
    /// Set the display target for an output window (Windowed or a specific display)
    SetTarget { idx: usize, target: crate::renderer::context::OutputTarget },
    /// Assign a surface to this output (adds a SurfaceAssignment)
    AssignSurface { output_idx: usize, surface_idx: usize },
    /// Remove a surface assignment from this output
    UnassignSurface { output_idx: usize, assignment_idx: usize },
    /// Toggle calibration mode on an output
    ToggleCalibration { idx: usize },
    /// Update a warp corner for a surface assignment
    SetWarpCorner { output_idx: usize, assignment_idx: usize, corner_idx: usize, position: [f32; 2] },
    /// Reset warp to identity for a surface assignment
    ResetWarp { output_idx: usize, assignment_idx: usize },
}

/// Snapshot of a surface assignment for UI display
#[derive(Clone)]
pub struct SurfaceAssignmentUI {
    pub surface_idx: usize,
    pub surface_name: String,
    pub warp_corners: [[f32; 2]; 4],
    pub enabled: bool,
}

/// Snapshot of an output window's state for UI display
#[derive(Clone)]
pub struct OutputWindowUI {
    pub name: String,
    /// Current display target label (e.g. "Windowed", "HDMI-1 (1920x1080)")
    pub target_label: String,
    /// Whether currently targeting a display (vs windowed)
    pub is_on_display: bool,
    pub surface_assignments: Vec<SurfaceAssignmentUI>,
    pub calibration_mode: bool,
}

/// Surface action from UI
pub enum SurfaceAction {
    /// Add a new rectangular surface
    Add { name: String, source: OutputSource },
    /// Add a polygon surface with specific vertices
    AddPolygon { name: String, vertices: Vec<[f32; 2]>, source: OutputSource },
    /// Remove a surface by index
    Remove { idx: usize },
    /// Update the vertices of a surface (specific contour: 0=primary, 1+=extra)
    UpdateVertices { idx: usize, contour: usize, vertices: Vec<[f32; 2]> },
    /// Move a surface by a delta (moves all contours)
    MoveDelta { idx: usize, dx: f32, dy: f32 },
    /// Change the content source for a surface
    SetSource { idx: usize, source: OutputSource },
    /// Change the output type for a surface
    SetOutputType { idx: usize, output_type: SurfaceOutputType },
    /// Change the content mapping mode for a surface
    SetContentMapping { idx: usize, mapping: ContentMapping },
    /// Rename a surface
    Rename { idx: usize, name: String },
    /// Duplicate a surface (offset slightly so it's visible)
    Duplicate { idx: usize },
    /// Flip a surface horizontally (mirror around its bounding box center X)
    FlipHorizontal { idx: usize },
    /// Flip a surface vertically (mirror around its bounding box center Y)
    FlipVertical { idx: usize },
    /// Insert a vertex on an edge (after vertex at `after_vert_idx`)
    InsertVertex { idx: usize, after_vert_idx: usize, position: [f32; 2] },
    /// Add a circle surface with a CircleHint (vertices generated from hint)
    AddCircle { name: String, hint: CircleHint, source: OutputSource },
    /// Update a circle's radius and regenerate vertices
    SetCircleRadius { idx: usize, radius: f32 },
    /// Update a circle's side count and regenerate vertices
    SetCircleSides { idx: usize, sides: u32 },
    /// Convert a circle surface to a plain polygon (drop circle hint)
    ConvertToPolygon { idx: usize },
    /// Combine multiple surfaces into one (overlapping → merge, non-overlapping → multi-contour)
    Combine { indices: Vec<usize> },
}

/// Snapshot of a surface for UI display
#[derive(Clone)]
pub struct SurfaceUI {
    pub name: String,
    pub vertices: Vec<[f32; 2]>,
    pub extra_contours: Vec<Vec<[f32; 2]>>,
    pub source: OutputSource,
    pub content_mapping: ContentMapping,
    pub output_type: SurfaceOutputType,
    pub circle_hint: Option<CircleHint>,
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
    /// Channel index to open an image file dialog for (deferred to outside egui frame)
    pub open_image_dialog_for_channel: Option<usize>,
    /// (ch_idx, path) — add a video file as a new deck to channel
    pub video_to_add: Option<(usize, std::path::PathBuf)>,
    /// Channel index to open a video file dialog for (deferred to outside egui frame)
    pub open_video_dialog_for_channel: Option<usize>,
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
    /// MIDI learn: toggle learn mode on/off
    pub midi_learn_toggle: bool,
    /// MIDI learn: select a parameter as learn target (in learn mode, clicking a param)
    pub midi_learn_select: Option<String>,
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
    /// Surface actions
    pub surface_actions: Vec<SurfaceAction>,
    /// Toggle stage editor open/closed
    pub toggle_stage_editor: bool,
    /// Set stage editor grid size (normalized)
    pub set_grid_size: Option<f32>,
    /// Toggle snap-to-grid
    pub toggle_snap: bool,
    /// MIDI: rescan devices
    pub midi_rescan: bool,
    /// MIDI: toggle device enabled/disabled (device_id, enabled)
    pub midi_device_toggles: Vec<(crate::midi::DeviceId, bool)>,
    /// MIDI: clear all mappings
    pub midi_clear_mappings: bool,
    /// MIDI: remove a specific mapping
    pub midi_remove_mapping: Vec<crate::midi::MidiKey>,
    /// Toggle library panel open/closed
    pub toggle_library_panel: bool,
    /// Move an effect within a deck's chain: (ch_idx, deck_idx, from_idx, to_idx)
    pub effect_to_move: Option<(usize, usize, usize, usize)>,
    /// Move a channel effect within its chain: (ch_idx, from_idx, to_idx)
    pub ch_effect_to_move: Option<(usize, usize, usize)>,
    /// Move a master effect within its chain: (from_idx, to_idx)
    pub master_effect_to_move: Option<(usize, usize)>,
    /// (ch_idx, camera_id) — add a camera as a new deck to channel
    pub camera_to_add: Option<(usize, crate::camera::CameraId)>,
    /// Rescan for camera devices
    pub camera_rescan: bool,
    /// Save workspace requested (Ctrl+S / Cmd+S)
    pub save_requested: bool,
}

impl UIActions {
    pub fn new() -> Self {
        Self {
            shader_to_add: None,
            image_to_add: None,
            open_image_dialog_for_channel: None,
            video_to_add: None,
            open_video_dialog_for_channel: None,
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
            midi_learn_toggle: false,
            midi_learn_select: None,
            set_transition: None,
            select_deck: None,
            select_channel: None,
            select_master: false,
            add_channel: false,
            remove_channel: None,
            deck_to_move: None,
            output_actions: Vec::new(),
            surface_actions: Vec::new(),
            toggle_stage_editor: false,
            set_grid_size: None,
            toggle_snap: false,
            midi_rescan: false,
            midi_device_toggles: Vec::new(),
            midi_clear_mappings: false,
            midi_remove_mapping: Vec::new(),
            toggle_library_panel: false,
            effect_to_move: None,
            ch_effect_to_move: None,
            master_effect_to_move: None,
            camera_to_add: None,
            camera_rescan: false,
            save_requested: false,
        }
    }
}

/// Drag payload types for library drag-and-drop
#[derive(Debug, Clone)]
pub enum LibraryDrag {
    /// Generator shader from library (registry index)
    Generator(usize),
    /// Effect/filter shader from library (registry index)
    Effect(usize),
    /// Camera device from library (CameraId)
    Camera(crate::camera::CameraId),
}

/// Drag payload for effect reordering within a chain
#[derive(Debug, Clone, Copy)]
pub enum EffectDrag {
    /// Deck effect: (ch_idx, deck_idx, effect_idx)
    Deck(usize, usize, usize),
    /// Channel effect: (ch_idx, effect_idx)
    Channel(usize, usize),
    /// Master effect: (effect_idx)
    Master(usize),
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

