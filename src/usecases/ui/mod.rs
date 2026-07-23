pub mod notifications;
pub mod panels;
pub mod runner;
mod snapshot;
pub mod widgets;

pub(crate) use snapshot::build_ui_data;

use crate::audio::AudioSourceId;
use crate::camera::CameraId;
use crate::channel::DeckRenderFps;
use crate::modulation::{ADSRStage, AudioReactMode, LFOWaveform, StepInterpolation};
use crate::params::ParamValue;
use crate::renderer::context::OutputSource;
use crate::renderer::slicer::{DomeGeometry, DomePreset};
use crate::surface::detect::{DetectedContour, DetectionParams};
use crate::surface::{CircleHint, ContentMapping, SurfaceOutputType, SurfacePath};
use crate::{BlendMode, ScalingMode, ShaderParams};

// Re-export default render resolution constants from the engine layer
pub use crate::app::{DEFAULT_RENDER_HEIGHT, DEFAULT_RENDER_WIDTH};

/// UI-consumer-owned layout and selection state.
///
/// These fields are presentation concerns that don't belong in the engine.
/// Each UI consumer (egui, CLI, HTTP API) maintains its own instance.
/// Persisted in `stage.json` via the `StagePrefs` struct.
#[derive(Clone, Debug)]
pub struct UILayoutState {
    /// Currently selected deck for detail view in bottom bar (ch_idx, deck_idx)
    pub selected_deck: Option<(usize, usize)>,
    /// Currently selected channel for detail view in bottom bar (ch_idx)
    pub selected_channel: Option<usize>,
    /// Whether the master output is selected for detail view in bottom bar
    pub selected_master: bool,
    /// Currently selected sequence for detail view in bottom bar (seq_idx)
    pub selected_sequence: Option<usize>,
    /// Currently selected step within the selected sequence (seq_idx, step_idx)
    pub selected_sequence_step: Option<(usize, usize)>,
    /// Currently selected macro (by UUID) for detail view in bottom bar
    pub selected_macro: Option<String>,
    /// Whether the full-screen stage editor is open (replaces deck view)
    pub stage_editor_open: bool,
    /// Stage editor grid size (normalized, e.g. 0.05 = 20 divisions)
    pub stage_editor_grid_size: f32,
    /// Whether snap-to-grid is enabled in the stage editor
    pub stage_editor_snap: bool,
    /// Whether the library panel (left sidebar) is open
    pub library_panel_open: bool,
    /// Whether the right panel (master output sidebar) is open
    pub right_panel_open: bool,
    /// Whether the 3D dome preview is open in the stage editor
    pub dome_preview_open: bool,
    /// Whether the stage editor is in 3D Dome mode (vs 2D Polygon mode)
    pub dome_mode_active: bool,
    /// Active dome preset
    pub dome_preset: DomePreset,
    /// Active dome geometry (radius, truncation, tilt)
    pub dome_geometry: DomeGeometry,
    /// Camera detection mode state
    pub camera_detect_mode: CameraDetectMode,
}

impl Default for UILayoutState {
    fn default() -> Self {
        Self {
            selected_deck: None,
            selected_channel: None,
            selected_master: false,
            selected_sequence: None,
            selected_sequence_step: None,
            selected_macro: None,
            stage_editor_open: false,
            stage_editor_grid_size: 0.05,
            stage_editor_snap: true,
            library_panel_open: true,
            right_panel_open: true,
            dome_preview_open: false,
            dome_mode_active: false,
            dome_preset: DomePreset::Quad,
            dome_geometry: DomeGeometry::default(),
            camera_detect_mode: CameraDetectMode::Off,
        }
    }
}

/// Camera detection mode state machine.
///
/// Off → Live (camera feed) → Preview (frozen frame with contour selection) → Off
#[derive(Debug, Clone, Default)]
pub enum CameraDetectMode {
    #[default]
    Off,
    Live {
        camera_id: CameraId,
        params: DetectionParams,
    },
    Preview {
        camera_id: CameraId,
        contours: Vec<DetectedContour>,
        selected: Vec<bool>,
    },
}

/// Actions emitted by the camera detection UI.
#[derive(Debug, Clone)]
pub enum CameraDetectAction {
    Enter { camera_id: CameraId },
    Exit,
    UpdateParams(DetectionParams),
    Capture,
    ToggleContour(usize),
    SelectAll(bool),
    Accept,
}

impl UILayoutState {
    /// Apply selection actions from UIActions.
    pub fn apply_selections(&mut self, ui_actions: &UIActions) {
        let session = &ui_actions.session;
        if let Some(sel) = session.select_deck {
            self.selected_deck = Some(sel);
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
        }
        if let Some(ch) = session.select_channel {
            self.selected_channel = Some(ch);
            self.selected_deck = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
        }
        if session.select_master {
            self.selected_master = true;
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
        }
        if let Some(seq) = session.select_sequence {
            self.selected_sequence = Some(seq);
            self.selected_sequence_step = None;
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_macro = None;
        }
        if let Some(step) = session.select_sequence_step {
            self.selected_sequence_step = Some(step);
            // Ensure sequence is also selected
            self.selected_sequence = Some(step.0);
        }
        if let Some(uuid) = &session.select_macro {
            self.selected_macro = Some(uuid.clone());
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
        }
        if session.deselect_macro {
            self.selected_macro = None;
        }
        if session.toggle_stage_editor {
            self.stage_editor_open = !self.stage_editor_open;
        }
        if let Some(size) = session.set_grid_size {
            self.stage_editor_grid_size = size;
        }
        if session.toggle_snap {
            self.stage_editor_snap = !self.stage_editor_snap;
        }
        if session.toggle_library_panel {
            self.library_panel_open = !self.library_panel_open;
        }
        if session.toggle_right_panel {
            self.right_panel_open = !self.right_panel_open;
        }
        if session.toggle_dome_preview {
            self.dome_preview_open = !self.dome_preview_open;
        }
        // Dome mode actions
        for action in &session.dome_actions {
            match action {
                DomeAction::SetMode(active) => {
                    self.dome_mode_active = *active;
                    // When entering dome mode, also open dome preview
                    if *active {
                        self.dome_preview_open = true;
                    }
                }
                DomeAction::SetPreset(preset) => self.dome_preset = *preset,
                DomeAction::SetRadius(r) => self.dome_geometry.radius = *r,
                DomeAction::SetTruncation(deg) => self.dome_geometry.truncation_degrees = *deg,
                DomeAction::SetTilt(deg) => self.dome_geometry.tilt_degrees = *deg,
                DomeAction::SetContentAzimuth(deg) => {
                    self.dome_geometry.content_azimuth_degrees = *deg
                }
                DomeAction::SetContentElevation(deg) => {
                    self.dome_geometry.content_elevation_degrees = *deg
                }
                DomeAction::SetContentRoll(deg) => self.dome_geometry.content_roll_degrees = *deg,
                DomeAction::RotateCamera { .. }
                | DomeAction::ZoomCamera { .. }
                | DomeAction::ResetCamera => {
                    // Camera actions are handled by the runner, not layout state
                }
            }
        }
    }

    /// Channels to force-render for preview, derived from the current selection.
    ///
    /// Selecting a deck or a channel cues that channel so its off-air preview
    /// updates live (see /spec/channel-preview.md). Master or no selection cues
    /// nothing. Returned as a set (0 or 1 today) to leave room for multi-cue.
    pub fn preview_channels(&self) -> Vec<usize> {
        if let Some((ch, _)) = self.selected_deck {
            vec![ch]
        } else if let Some(ch) = self.selected_channel {
            vec![ch]
        } else {
            Vec::new()
        }
    }

    /// Fix up selection indices after a channel is removed.
    pub fn fixup_channel_removal(&mut self, removed_ch: usize) {
        if let Some((sel_ch, _)) = self.selected_deck {
            if sel_ch == removed_ch {
                self.selected_deck = None;
            } else if sel_ch > removed_ch {
                // sel_ch > removed_ch, so selected_deck must be Some (we matched it above)
                if let Some((_, deck_idx)) = self.selected_deck {
                    self.selected_deck = Some((sel_ch - 1, deck_idx));
                }
            }
        }
        if let Some(sel_ch) = self.selected_channel {
            if sel_ch == removed_ch {
                self.selected_channel = None;
            } else if sel_ch > removed_ch {
                self.selected_channel = Some(sel_ch - 1);
            }
        }
    }
}

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

/// Modulation source data snapshot for UI display (paired with UUID)
#[derive(Clone)]
pub struct ModSourceUIEntry {
    pub uuid: String,
    pub source: ModSourceUI,
}

/// Modulation source data snapshot for UI display
#[derive(Clone)]
pub enum ModSourceUI {
    LFO {
        waveform: LFOWaveform,
        frequency: f32,
        phase: f32,
        amplitude: f32,
        bipolar: bool,
    },
    Audio {
        source_id: Option<AudioSourceId>,
        freq_low: f32,
        freq_high: f32,
        gain: f32,
        smoothing: f32,
        mode: AudioReactMode,
        noise_gate: f32,
    },
    ADSR {
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
        stage: ADSRStage,
    },
    StepSequencer {
        steps: Vec<f32>,
        rate: f32,
        interpolation: StepInterpolation,
        bipolar: bool,
    },
    Analyzer {
        deck_id: String,
        analyzer_type: String,
        output_name: String,
        smoothing: f32,
    },
}

/// Infinite non-colliding modulation source colors via binary hue subdivision.
///
/// Uses the same subdivision algorithm as channel colors but offset by half the
/// hue wheel (0.26 vs 0.76) and with higher saturation / different lightness
/// bands, so modulator colors are always visually distinct from channel colors.
pub fn modulator_color(idx: usize) -> egui::Color32 {
    // Opposite side of the hue wheel from channel colors (0.76 + 0.5 = 0.26)
    const HUE_OFFSET: f32 = 0.26;

    // Brighter / more saturated styles than channels to stand out on dark UI
    const RING_STYLES: [(f32, f32); 6] = [
        (0.90, 0.55), // ring 0: vivid
        (0.85, 0.62), // ring 1: vivid light
        (0.95, 0.48), // ring 2: saturated deep
        (0.70, 0.70), // ring 3: soft bright
        (0.95, 0.42), // ring 4: very saturated dark
        (0.65, 0.75), // ring 5+: pastel
    ];

    let (ring, hue_frac) = panels::utils::hue_subdivision(idx);
    let hue = (HUE_OFFSET + hue_frac) % 1.0;
    let (sat, lit) = RING_STYLES[ring.min(RING_STYLES.len() - 1)];

    let (r, g, b) = panels::utils::hsl_to_rgb(hue, sat, lit);
    egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Modulation assignment snapshot for UI display
#[derive(Clone)]
pub struct ModAssignmentUI {
    pub source_id: String,
    pub amount: f32,
}

/// Effect info tuple for UI: (name, enabled, params)
/// Effect info for UI: (uuid, name, enabled, params)
pub type EffectInfo = (String, String, bool, ShaderParamsUI);

/// Video playback state snapshot for UI display
#[derive(Clone)]
pub struct VideoPlaybackUI {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub loop_mode: crate::video::LoopMode,
    pub in_point: f64,
    pub out_point: f64,
    pub frame_rate: f64,
}

/// Auto-transition state snapshot for UI display
#[derive(Clone)]
pub struct AutoTransitionUI {
    pub enabled: bool,
    pub trigger_is_clip_end: bool,
    pub play_duration_value: f64,
    pub play_duration_is_beats: bool,
    pub transition_duration_value: f64,
    pub transition_duration_is_beats: bool,
    pub transition_shader_name: Option<String>,
    pub phase: crate::channel::DeckTransitionPhase,
}

/// Deck info for UI display
#[derive(Clone)]
pub struct DeckUIInfo {
    pub deck_idx: usize,
    pub uuid: String,
    pub name: String,
    /// True when this deck's source is an HTML/Servo instance.
    pub is_html: bool,
    /// True when the interactive window is currently open for this deck.
    pub is_html_interactive: bool,
    pub opacity: f32,
    /// Effective opacity accounting for auto-transition state (for visual feedback only)
    pub effective_opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    /// True when this deck preserves source alpha (transparent compositing).
    pub transparent: bool,
    pub scaling_mode: Option<ScalingMode>,
    pub generator: ShaderParamsUI,
    pub effects: Vec<EffectInfo>,
    /// Video playback state (only present for video decks)
    pub video_playback: Option<VideoPlaybackUI>,
    /// Auto-transition state (None = no auto-transition configured)
    pub auto_transition: Option<AutoTransitionUI>,
    /// Per-deck render FPS setting
    pub render_fps: DeckRenderFps,
    /// Effective render rate this deck is achieving
    pub effective_render_fps: f32,
    /// Smoothed render cost in microseconds
    pub render_cost_us: f32,
    /// GPU-measured render cost in microseconds (0 = not available)
    pub gpu_render_cost_us: f32,
}

/// Channel info for UI display
#[derive(Clone)]
pub struct ChannelUIInfo {
    pub ch_idx: usize,
    pub uuid: String,
    pub name: String,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub decks: Vec<DeckUIInfo>,
    pub effects: Vec<EffectInfo>,
}

/// Audio input device info for UI display.
#[derive(Clone)]
pub struct AudioDeviceUI {
    pub id: AudioSourceId,
    pub name: String,
    pub active: bool,
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
    /// Available audio input devices
    pub devices: Vec<AudioDeviceUI>,
    /// FFT spectrum of primary source (for spectrum visualization, 256 bins)
    pub fft: Vec<f32>,
    /// Sample rate of primary source
    pub sample_rate: f32,
}

/// Notification snapshot for UI rendering (avoids borrowing NotificationSystem during egui)
#[derive(Clone)]
pub struct NotificationUI {
    pub level: crate::notifications::NotificationLevel,
    pub message: String,
    pub progress: f32,
}

/// Per-channel render statistics for the FPS popover
pub struct ChannelRenderStats {
    pub name: String,
    /// Average FPS across active decks in this channel (from deck render pipeline timing)
    pub avg_deck_fps: f32,
    /// Number of active (rendered) decks
    pub active_deck_count: u32,
    /// Total channel render time in milliseconds
    pub render_time_ms: f32,
}

/// SRT source entry for the library panel config card
#[derive(Clone)]
pub struct SrtLibraryEntry {
    pub url: String,
    pub mode: crate::stream::SrtMode,
    pub connected: bool,
}

/// HLS source entry for the library panel
#[derive(Clone)]
pub struct HlsLibraryEntry {
    pub url: String,
    pub connected: bool,
}

/// DASH source entry for the library panel
#[derive(Clone)]
pub struct DashLibraryEntry {
    pub url: String,
    pub connected: bool,
}

/// RTMP source entry for the library panel
#[derive(Clone)]
pub struct RtmpLibraryEntry {
    pub url: String,
    pub mode: crate::stream::RtmpMode,
    pub connected: bool,
}

/// HTML source entry for the library panel
#[derive(Clone)]
pub struct HtmlLibraryEntry {
    pub url: String,
    pub active: bool,
}

/// All collected data needed to render the UI
pub struct UIData {
    pub generators: Vec<(String, usize)>,
    pub filters: Vec<(String, usize)>,
    pub shader_count: usize,
    pub channels: Vec<ChannelUIInfo>,
    pub master_effect_info: Vec<EffectInfo>,
    pub modulation_sources: Vec<ModSourceUIEntry>,
    /// Current computed values for each modulation source by UUID
    pub modulation_current_values: std::collections::HashMap<String, f32>,
    /// Modulation assignments: param_key -> list of (source_id, amount)
    pub modulation_assignments: std::collections::HashMap<String, Vec<ModAssignmentUI>>,
    /// User-defined macro controls (one control → many parameter targets).
    pub macros: Vec<crate::macros::Macro>,
    pub audio: AudioUIData,
    /// Deck preview textures keyed by (ch_idx, deck_idx)
    pub deck_preview_textures: std::collections::HashMap<(usize, usize), egui::TextureId>,
    /// Channel preview textures keyed by ch_idx
    pub channel_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    /// Output preview textures keyed by output index
    pub output_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    pub main_output_texture: Option<egui::TextureId>,
    pub notifications: Vec<NotificationUI>,
    /// Crossfader position (0.0 = A, 1.0 = B)
    pub crossfader: f32,
    /// Whether an auto-crossfade is currently running
    pub auto_crossfade_active: bool,
    /// Progress of auto-crossfade (0.0–1.0), if active
    pub auto_crossfade_progress: f32,
    /// Current tonemap mode (Bypass or ACES)
    pub tonemap_mode: crate::renderer::tonemap::TonemapMode,
    /// Active LUT filename (if any)
    pub active_lut_filename: Option<String>,
    /// Available LUT files in .varda/luts/
    pub available_luts: Vec<String>,
    /// Whether MIDI learn mode is active
    pub midi_learn_active: bool,
    /// The parameter path currently waiting for MIDI learn
    pub midi_learn_target: Option<String>,
    /// Whether keyboard learn mode is active
    pub keyboard_learn_active: bool,
    /// Display string for current keyboard learn target
    pub keyboard_learn_target: Option<String>,
    /// All current keybindings (read-only snapshot for dispatch + settings panel)
    pub keymap_bindings:
        std::collections::HashMap<crate::keymap::KeyCombo, crate::keymap::KeyTarget>,
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
    /// Currently selected sequence for detail view in bottom bar (seq_idx)
    pub selected_sequence: Option<usize>,
    /// Currently selected step within the selected sequence (seq_idx, step_idx)
    pub selected_sequence_step: Option<(usize, usize)>,
    /// Currently selected macro (by UUID) for detail view in bottom bar
    pub selected_macro: Option<String>,
    /// Unified outputs (windowed + headless) for UI display
    pub outputs: Vec<OutputUI>,
    /// Surfaces in the stage layout
    pub surfaces: Vec<SurfaceUI>,
    /// Whether the full-screen stage editor is open (replaces deck view)
    pub stage_editor_open: bool,
    /// Whether the 3D dome preview is open in the stage editor
    pub dome_preview_open: bool,
    /// Dome preview texture (rendered 3D hemisphere)
    pub dome_preview_texture: Option<egui::TextureId>,
    /// Whether the stage editor is in 3D Dome mode (vs 2D Polygon mode)
    pub dome_mode_active: bool,
    /// Active dome preset
    pub dome_preset: DomePreset,
    /// Active dome geometry (radius, truncation, tilt)
    pub dome_geometry: DomeGeometry,
    /// Camera detection mode texture (live camera feed registered with egui)
    pub camera_detect_texture: Option<egui::TextureId>,
    /// Current camera detection mode state
    pub camera_detect_mode: CameraDetectMode,
    /// Contours detected in current frame (for overlay rendering)
    pub camera_detect_contours: Vec<DetectedContour>,
    /// Whether the library panel (left sidebar) is open
    pub library_panel_open: bool,
    /// Whether the right panel (master output sidebar) is open
    pub right_panel_open: bool,
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
    /// Discovered NDI sources (name)
    pub ndi_sources: Vec<String>,
    /// Whether NDI runtime is available
    pub ndi_available: bool,
    /// Discovered Syphon servers (name)
    pub syphon_sources: Vec<String>,
    /// Whether Syphon framework is available
    pub syphon_available: bool,
    /// SRT library source configs for the library panel
    pub srt_library_configs: Vec<SrtLibraryEntry>,
    /// HLS library source configs
    pub hls_library_configs: Vec<HlsLibraryEntry>,
    /// DASH library source configs
    pub dash_library_configs: Vec<DashLibraryEntry>,
    /// RTMP library source configs
    pub rtmp_library_configs: Vec<RtmpLibraryEntry>,
    /// HTML library source configs
    pub html_library_configs: Vec<HtmlLibraryEntry>,
    // Recording/SRT state is now per-output (see OutputUI.is_active, active_duration)
    /// Transition sequences (multiple named sequences)
    pub sequences: Vec<SequenceUIData>,
    /// Number of channels (for channel dropdowns in sequence builder)
    pub channel_count: usize,
    /// Channel names (for labels in sequence builder)
    pub channel_names: Vec<String>,
    /// Pipeline-derived FPS: average of per-channel FPSes (from deck render timing)
    pub fps: f32,
    /// Per-channel render stats: (channel_name, avg_deck_fps, active_deck_count, render_time_ms)
    pub channel_render_stats: Vec<ChannelRenderStats>,
    /// GPU device name (e.g. "Apple M1 Pro")
    pub gpu_device_name: String,
    /// GPU backend (e.g. "Metal", "Vulkan", "Dx12")
    pub gpu_backend: String,
    /// GPU driver info string
    pub gpu_driver: String,
    /// GPU driver version/info
    pub gpu_driver_info: String,
    /// GPU device type (e.g. "DiscreteGpu", "IntegratedGpu")
    pub gpu_device_type: String,
    /// GPU utilization % (0–100), from GPU timestamp data
    pub gpu_utilization: f32,
    /// CPU usage % (0–100)
    pub cpu_usage: f32,
    /// RAM used in bytes
    pub ram_used: u64,
    /// RAM total in bytes
    pub ram_total: u64,
    /// Clock sync source label ("Audio", "MIDI", "OSC", "None")
    pub clock_source: String,
    /// Clock sync BPM (if active)
    pub clock_bpm: Option<f32>,
    /// Clock sync active
    pub clock_active: bool,
    /// Clock MIDI device name (if source is MIDI)
    pub clock_device_name: Option<String>,
    /// Detected MIDI clock sources for the popover
    pub clock_detected_midi: Vec<crate::engine::types::DetectedClockSourceSnapshot>,
    /// Whether OSC clock is currently active
    pub clock_osc_active: bool,
    /// OSC BPM (if active)
    pub clock_osc_bpm: Option<f32>,
    /// Audio BPM (fallback)
    pub clock_audio_bpm: Option<f32>,
    /// Current clock preference label
    pub clock_preference: String,
    /// Device ID if preference is ForceMidi
    pub clock_preference_force_device_id: Option<crate::midi::DeviceId>,
    /// Manual BPM value (if preference is ForceManual)
    pub clock_manual_bpm: Option<f32>,
    /// Current master render width
    pub render_width: u32,
    /// Current master render height
    pub render_height: u32,
    /// GPU's maximum 2D texture dimension — the only bound on custom render
    /// resolution (Varda imposes no artificial cap).
    pub max_render_dimension: u32,
    /// Target FPS (0 = uncapped)
    pub target_fps: u32,
    /// Whether undo is available
    pub can_undo: bool,
    /// Whether redo is available
    pub can_redo: bool,
    /// Number of decks currently loading in background threads
    pub pending_deck_loads: usize,
    /// Loaded deck preset names (from PresetLibrary)
    pub deck_presets: Vec<String>,
    /// Loaded channel preset names (from PresetLibrary)
    pub channel_presets: Vec<String>,
}

/// Read-only snapshot of a single transition sequence
#[derive(Clone)]
pub struct SequenceUIData {
    /// Display name
    pub name: String,
    /// Whether the sequence is enabled
    pub enabled: bool,
    /// Whether the sequencer is currently playing
    pub playing: bool,
    /// Current step index (while playing)
    pub current_step: usize,
    /// Elapsed time within the current step (seconds)
    pub step_elapsed: f64,
    /// Step descriptions for display
    pub steps: Vec<SequenceStepUI>,
}

/// A single step displayed in the sequence builder
#[derive(Clone)]
pub struct SequenceStepUI {
    pub label: String,
    pub kind: SequenceStepKindUI,
}

/// UI-friendly step kind representation
#[derive(Clone)]
pub enum SequenceStepKindUI {
    Fade {
        from_ch: usize,
        to_ch: usize,
        duration_val: f64,
        duration_unit: crate::channel::DurationUnit,
        easing: String,
        transition_shader: Option<String>,
        target_amount: f32,
    },
    Wait {
        duration_val: f64,
        duration_unit: crate::channel::DurationUnit,
    },
    GoTo {
        step_index: usize,
    },
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

/// Snapshot of a surface assignment for UI display
#[derive(Clone)]
pub struct SurfaceAssignmentUI {
    pub surface_uuid: String,
    pub surface_name: String,
    pub enabled: bool,
    /// Per-surface overlap zones (Auto mode). Empty when Manual or no overlaps.
    pub overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones,
}

/// Snapshot of an output's state for UI display (unified — windowed or headless)
#[derive(Clone)]
pub struct OutputUI {
    pub uuid: String,
    pub name: String,
    /// The output target (unified enum)
    pub target: crate::renderer::context::OutputTarget,
    /// Current display target label (e.g. "Windowed", "Rec: /path", "SRT: srt://...")
    pub target_label: String,
    /// Whether this output is windowed (has an OS window)
    pub is_windowed: bool,
    /// Whether this output is actively recording/streaming (headless only)
    pub is_active: bool,
    /// Duration of active recording/streaming
    pub active_duration: std::time::Duration,
    pub surface_assignments: Vec<SurfaceAssignmentUI>,
    pub calibration_mode: crate::renderer::context::CalibrationMode,
    /// Edge blend mode (Auto / Manual)
    pub edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode,
    /// Edge blending configuration
    pub edge_blend: crate::renderer::edge_blend::EdgeBlendConfig,
    /// Per-output rotation (0°/90°/180°/270°)
    pub rotation: crate::renderer::context::OutputRotation,
    /// Audio passthrough health for an active ffmpeg output (None = video-only).
    pub audio_passthrough: Option<AudioPassthroughUI>,
}

/// Live audio passthrough health for an active output.
#[derive(Clone)]
pub struct AudioPassthroughUI {
    /// Selected capture device name.
    pub device: String,
    /// PCM chunks written to ffmpeg so far.
    pub frames_written: u64,
    /// PCM chunks dropped on backpressure.
    pub frames_dropped: u64,
}

/// Dome-mode UI actions (camera interaction, mode toggle, config changes).
#[derive(Debug, Clone)]
pub enum DomeAction {
    /// Toggle between 2D Polygon mode and 3D Dome mode
    SetMode(bool),
    /// Set dome preset
    SetPreset(DomePreset),
    /// Set dome radius
    SetRadius(f32),
    /// Set dome truncation angle in degrees
    SetTruncation(f32),
    /// Set dome tilt angle in degrees
    SetTilt(f32),
    /// Set content azimuth rotation in degrees
    SetContentAzimuth(f32),
    /// Set content elevation rotation in degrees
    SetContentElevation(f32),
    /// Set content roll rotation in degrees
    SetContentRoll(f32),
    /// Rotate orbit camera by pixel delta
    RotateCamera { delta_x: f32, delta_y: f32 },
    /// Zoom orbit camera by scroll delta
    ZoomCamera { delta: f32 },
    /// Reset orbit camera to default
    ResetCamera,
}

/// Snapshot of a surface for UI display
#[derive(Clone)]
pub struct SurfaceUI {
    pub uuid: String,
    pub name: String,
    pub vertices: Vec<[f32; 2]>,
    pub extra_contours: Vec<Vec<[f32; 2]>>,
    pub source: OutputSource,
    pub content_mapping: ContentMapping,
    pub output_type: SurfaceOutputType,
    pub circle_hint: Option<CircleHint>,
    /// Effective per-surface warp (corner-pin or mesh); `None` = no warp. While
    /// `warp_bound`, this is the shape-conforming warp. Drives the stage
    /// bottom-bar warp editor.
    pub warp: Option<crate::renderer::warp::WarpMode>,
    /// Whether the warp auto-conforms to the surface shape. When `true` the
    /// bottom-bar warp controls are locked (read-only).
    pub warp_bound: bool,
    /// Curve authoring path, when the surface is bezier-edited. Drives the
    /// anchor/handle overlay and edge hit-testing in the stage editor.
    pub path: Option<SurfacePath>,
    /// Subtractive cut-out holes (8i.7), drawn as editable overlay contours.
    pub holes: Vec<SurfacePath>,
    /// Flattened hole contours (canvas coords) for overlay rendering.
    pub hole_contours: Vec<Vec<[f32; 2]>>,
}

/// UI-local session/ephemeral state accumulated during a frame (Population 2).
///
/// This is the half of the frame's UI output that is **not** an engine mutation:
/// selection focus, panel visibility, learn-mode targeting, dialog-open triggers,
/// notification dismissals, gesture continuation, the async shader-load request,
/// and the layout-coupled undo/redo/save triggers. None of it belongs on the
/// command bus (the HTTP/CLI/MIDI consumers neither can nor should express it);
/// it targets UI-local state (`UILayoutState`) or the runner, not the engine.
///
/// See /spec/ui-engine-boundary.md WS4 / Decision #11 — the deliberate split of
/// "what I tell the engine" (`UIActions::commands`) from "my local view state".
pub struct UISession {
    /// (ch_idx, generator_registry_idx) — add a shader as a new deck to channel.
    /// Async request resolved off-frame via `spawn_deck_loads` (not a command).
    pub shader_to_add: Option<(usize, usize)>,
    /// Channel index to open an image file dialog for (deferred to outside egui frame)
    pub open_image_dialog_for_channel: Option<usize>,
    /// Channel index to open a video file dialog for (deferred to outside egui frame)
    pub open_video_dialog_for_channel: Option<usize>,
    pub notifications_to_dismiss: Vec<usize>,
    /// Info notifications to push (e.g. "Copied URL to clipboard")
    pub info_notifications: Vec<String>,
    /// MIDI learn: toggle learn mode on/off
    pub midi_learn_toggle: bool,
    /// MIDI learn: select a parameter as learn target (in learn mode, clicking a param)
    pub midi_learn_select: Option<String>,
    /// Keyboard learn: toggle learn mode on/off
    pub keyboard_learn_toggle: bool,
    /// Keyboard learn: select a target (Action or ParamPath)
    pub keyboard_learn_select: Option<crate::keymap::KeyTarget>,
    /// Keyboard learn: bind a key combo to current target
    pub keyboard_learn_bind: Option<crate::keymap::KeyCombo>,
    /// Select a deck for detail view in bottom bar (ch_idx, deck_idx)
    pub select_deck: Option<(usize, usize)>,
    /// Select a channel for detail view in bottom bar (ch_idx)
    pub select_channel: Option<usize>,
    /// Select master output for detail view in bottom bar
    pub select_master: bool,
    /// Select a sequence for detail view in bottom bar (seq_idx)
    pub select_sequence: Option<usize>,
    /// Select a step within a sequence for editing in bottom bar (seq_idx, step_idx)
    pub select_sequence_step: Option<(usize, usize)>,
    /// Select a macro (by UUID) for detail view in bottom bar
    pub select_macro: Option<String>,
    /// Clear the macro selection (e.g. its macro was deleted, or Close pressed)
    pub deselect_macro: bool,
    /// Remove a channel from the mixer (by index). Retained as a non-command
    /// field because the runner needs the removed index to fix up UI selection
    /// state (`layout.fixup_channel_removal`).
    pub remove_channel: Option<usize>,
    /// Toggle stage editor open/closed
    pub toggle_stage_editor: bool,
    /// Toggle 3D dome preview in stage editor
    pub toggle_dome_preview: bool,
    /// Dome mode actions (camera, config, mode toggle). UI-local layout state
    /// (`DomeLayoutFields`); only committed to the engine via `GenerateDomeSlices`.
    pub dome_actions: Vec<DomeAction>,
    /// Camera detection actions (preview state machine; only `Accept` becomes a command)
    pub camera_detect_actions: Vec<CameraDetectAction>,
    /// Set stage editor grid size (normalized)
    pub set_grid_size: Option<f32>,
    /// Toggle snap-to-grid
    pub toggle_snap: bool,
    /// Toggle library panel open/closed
    pub toggle_library_panel: bool,
    /// Toggle right panel open/closed
    pub toggle_right_panel: bool,
    /// Save workspace requested (Ctrl+S / Cmd+S). Layout-coupled runner trigger
    /// (accepted deviation, /spec/ui-engine-boundary.md Decision #10).
    pub save_requested: bool,
    /// Undo last undoable action. Layout-coupled runner trigger (accepted deviation).
    pub undo_requested: bool,
    /// Redo last undone action. Layout-coupled runner trigger (accepted deviation).
    pub redo_requested: bool,
    /// A mutating stage/warp pointer drag is in progress this frame. Set by the
    /// stage editor and warp editor while dragging a vertex, warp point, bezier
    /// handle, or gizmo, and by any scene param/opacity slider drag. Used to
    /// collapse a continuous drag into a single undo step (snapshot on gesture
    /// start, suppressed while held).
    pub gesture_active: bool,
}

impl Default for UISession {
    fn default() -> Self {
        Self::new()
    }
}

impl UISession {
    pub fn new() -> Self {
        Self {
            shader_to_add: None,
            open_image_dialog_for_channel: None,
            open_video_dialog_for_channel: None,
            notifications_to_dismiss: Vec::new(),
            info_notifications: Vec::new(),
            midi_learn_toggle: false,
            midi_learn_select: None,
            keyboard_learn_toggle: false,
            keyboard_learn_select: None,
            keyboard_learn_bind: None,
            select_deck: None,
            select_channel: None,
            select_master: false,
            select_sequence: None,
            select_sequence_step: None,
            select_macro: None,
            deselect_macro: false,
            remove_channel: None,
            toggle_stage_editor: false,
            toggle_dome_preview: false,
            dome_actions: Vec::new(),
            camera_detect_actions: Vec::new(),
            set_grid_size: None,
            toggle_snap: false,
            toggle_library_panel: false,
            toggle_right_panel: false,
            save_requested: false,
            undo_requested: false,
            redo_requested: false,
            gesture_active: false,
        }
    }
}

/// All UI output collected during a frame, split into two buckets (WS4):
///
/// - [`commands`](Self::commands): the outbound engine-mutation stream. Panels
///   push `EngineCommand`s directly (the single mutation vocabulary shared with
///   the HTTP/CLI consumers); the app-layer drain runs each through the same
///   dispatch as the command bus.
/// - [`session`](Self::session): UI-local ephemeral state (selection, panel
///   visibility, learn mode, dialog triggers, undo/redo/save). See [`UISession`].
pub struct UIActions {
    /// Outbound engine mutations (see /spec/ui-engine-boundary.md WS2).
    pub commands: Vec<crate::engine::EngineCommand>,
    /// UI-local session/ephemeral state (see [`UISession`], WS4).
    pub session: UISession,
}

impl Default for UIActions {
    fn default() -> Self {
        Self::new()
    }
}

impl UIActions {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            session: UISession::new(),
        }
    }

    /// Whether this frame's actions include any undoable mutation carried by a
    /// non-command field. Source-deck adds, deck remove/move/reorder, channel
    /// add, effects, presets, and mixer edits now flow through `commands` and
    /// are gated by `batch_has_undoable`. Only two irreducible residuals remain
    /// here: the async shader load (`shader_to_add`, resolved off-frame and thus
    /// never on `commands`) and channel removal (`remove_channel`, kept a field
    /// so the runner can fix up UI selection with the removed index).
    pub fn has_undoable_action(&self) -> bool {
        self.session.shader_to_add.is_some() || self.session.remove_channel.is_some()
    }

    /// Whether this frame's actions include any undoable *stage* mutation
    /// (surface geometry/warp/holes/combine/reorder, surface→output
    /// assignments, or authored dome changes).
    ///
    /// Deliberately excludes non-authored actions on the same collections:
    /// output-window lifecycle (create/remove/reposition) and dome preview
    /// camera navigation (rotate/zoom/reset) are live/venue controls, not
    /// stage-editor edits, so they must not create history entries.
    ///
    /// Does NOT distinguish continuous vs discrete edits — gesture collapsing is
    /// handled by the `gesture_active` edge in the runner.
    pub fn has_undoable_stage_action(&self) -> bool {
        self.session.dome_actions.iter().any(|a| {
            !matches!(
                a,
                DomeAction::RotateCamera { .. }
                    | DomeAction::ZoomCamera { .. }
                    | DomeAction::ResetCamera
            )
        })
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
    /// NDI network source (source name)
    Ndi(String),
    /// Syphon server (server name)
    Syphon(String),
    /// SRT network source (url, mode)
    Srt(String, crate::stream::SrtMode),
    /// HLS stream source (url)
    Hls(String),
    /// DASH stream source (url)
    Dash(String),
    /// RTMP stream source (url, mode)
    Rtmp(String, crate::stream::RtmpMode),
    /// HTML content source (url)
    Html(String),
    /// Deck preset from library (index into preset_library.deck_presets)
    DeckPreset(usize),
    /// Channel preset from library (index into preset_library.channel_presets)
    ChannelPreset(usize),
}

/// Drag payload for effect reordering within a chain
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectDrag {
    /// Deck effect: (ch_idx, deck_idx, effect_idx)
    Deck(usize, usize, usize),
    /// Channel effect: (ch_idx, effect_idx)
    Channel(usize, usize),
    /// Master effect: (effect_idx)
    Master(usize),
}

/// Drag payload for reordering steps within a sequence (bottom bar only)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SequenceStepDrag {
    pub seq_idx: usize,
    pub step_idx: usize,
}

/// Helper to extract params from ShaderParams for UI display
pub fn collect_params(params: &ShaderParams) -> Vec<ParamUIInfo> {
    params
        .param_order
        .iter()
        .filter_map(|name| {
            let value = params.values.get(name)?;
            let def = params.definitions.get(name);
            Some(ParamUIInfo {
                name: name.clone(),
                label: def.and_then(|d| d.label.clone()),
                value: *value,
                min: def.and_then(|d| d.min),
                max: def.and_then(|d| d.max),
            })
        })
        .collect()
}

#[cfg(any(test, feature = "test-fixtures"))]
impl UIData {
    /// Representative test fixture for UI testing.
    ///
    /// Contains 2 channels with 2 decks each, effects, modulation, crossfader
    /// at 0.5, library panel open, deck (0,0) selected, and empty but present
    /// collections for MIDI, audio, surfaces, and sequences.
    pub fn test_fixture() -> Self {
        use crate::modulation::LFOWaveform;

        let deck_a0 = DeckUIInfo {
            deck_idx: 0,
            uuid: "a0000001".to_string(),
            name: "test_generator_a".to_string(),
            is_html: false,
            is_html_interactive: false,
            opacity: 1.0,
            effective_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            scaling_mode: Some(ScalingMode::Fit),
            generator: ShaderParamsUI {
                shader_name: "test_generator_a".to_string(),
                params: vec![ParamUIInfo {
                    name: "speed".to_string(),
                    label: Some("Speed".to_string()),
                    value: crate::params::ParamValue::Float(1.0),
                    min: Some(0.0),
                    max: Some(5.0),
                }],
            },
            effects: vec![(
                "dfx00001".to_string(),
                "test_effect".to_string(),
                true,
                ShaderParamsUI {
                    shader_name: "test_effect".to_string(),
                    params: vec![ParamUIInfo {
                        name: "amount".to_string(),
                        label: Some("Amount".to_string()),
                        value: crate::params::ParamValue::Float(0.5),
                        min: Some(0.0),
                        max: Some(1.0),
                    }],
                },
            )],
            video_playback: None,
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let deck_a1 = DeckUIInfo {
            deck_idx: 1,
            uuid: "a0000002".to_string(),
            name: "test_generator_b".to_string(),
            is_html: false,
            is_html_interactive: false,
            opacity: 0.8,
            effective_opacity: 0.8,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            scaling_mode: Some(ScalingMode::Fit),
            generator: ShaderParamsUI {
                shader_name: "test_generator_b".to_string(),
                params: vec![],
            },
            effects: vec![],
            video_playback: None,
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let channel_a = ChannelUIInfo {
            ch_idx: 0,
            uuid: "ca000001".to_string(),
            name: "Ch A".to_string(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            decks: vec![deck_a0, deck_a1],
            effects: vec![(
                "cfx00001".to_string(),
                "ch_effect".to_string(),
                true,
                ShaderParamsUI {
                    shader_name: "ch_effect".to_string(),
                    params: vec![],
                },
            )],
        };

        let deck_b0 = DeckUIInfo {
            deck_idx: 0,
            uuid: "b0000001".to_string(),
            name: "test_generator_c".to_string(),
            is_html: false,
            is_html_interactive: false,
            opacity: 1.0,
            effective_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            scaling_mode: Some(ScalingMode::Fit),
            generator: ShaderParamsUI {
                shader_name: "test_generator_c".to_string(),
                params: vec![],
            },
            effects: vec![],
            video_playback: None,
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let deck_b1 = DeckUIInfo {
            deck_idx: 1,
            uuid: "b0000002".to_string(),
            name: "test_generator_d".to_string(),
            is_html: false,
            is_html_interactive: false,
            opacity: 1.0,
            effective_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            scaling_mode: Some(ScalingMode::Fit),
            generator: ShaderParamsUI {
                shader_name: "test_generator_d".to_string(),
                params: vec![],
            },
            effects: vec![],
            video_playback: None,
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let channel_b = ChannelUIInfo {
            ch_idx: 1,
            uuid: "cb000001".to_string(),
            name: "Ch B".to_string(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            decks: vec![deck_b0, deck_b1],
            effects: vec![],
        };

        UIData {
            generators: vec![
                ("test_generator_a".to_string(), 0),
                ("test_generator_b".to_string(), 1),
                ("test_generator_c".to_string(), 2),
                ("test_generator_d".to_string(), 3),
            ],
            filters: vec![
                ("test_effect".to_string(), 0),
                ("ch_effect".to_string(), 1),
                ("master_effect".to_string(), 2),
            ],
            shader_count: 7,
            channels: vec![channel_a, channel_b],
            master_effect_info: vec![(
                "mfx00001".to_string(),
                "master_effect".to_string(),
                true,
                ShaderParamsUI {
                    shader_name: "master_effect".to_string(),
                    params: vec![],
                },
            )],
            modulation_sources: vec![ModSourceUIEntry {
                uuid: "mod00001".to_string(),
                source: ModSourceUI::LFO {
                    waveform: LFOWaveform::Sine,
                    frequency: 1.0,
                    phase: 0.0,
                    amplitude: 1.0,
                    bipolar: false,
                },
            }],
            modulation_current_values: {
                let mut m = std::collections::HashMap::new();
                m.insert("mod00001".to_string(), 0.5);
                m
            },
            modulation_assignments: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "deck_a0000001:speed".to_string(),
                    vec![ModAssignmentUI {
                        source_id: "mod00001".to_string(),
                        amount: 0.5,
                    }],
                );
                m
            },
            macros: Vec::new(),
            audio: AudioUIData {
                level: 0.0,
                bass: 0.0,
                mid: 0.0,
                treble: 0.0,
                bpm: None,
                beat_phase: 0.0,
                enabled: false,
                devices: vec![],
                fft: vec![0.0; 256],
                sample_rate: 44100.0,
            },
            deck_preview_textures: std::collections::HashMap::new(),
            channel_preview_textures: std::collections::HashMap::new(),
            output_preview_textures: std::collections::HashMap::new(),
            main_output_texture: None,
            notifications: vec![],
            crossfader: 0.5,
            auto_crossfade_active: false,
            auto_crossfade_progress: 0.0,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut_filename: None,
            available_luts: vec![],
            midi_learn_active: false,
            midi_learn_target: None,
            keyboard_learn_active: false,
            keyboard_learn_target: None,
            keymap_bindings: std::collections::HashMap::new(),
            transition_names: vec!["fade".to_string()],
            active_transition_name: None,
            selected_deck: Some((0, 0)),
            selected_channel: None,
            selected_master: false,
            selected_sequence: None,
            selected_sequence_step: None,
            selected_macro: None,
            outputs: vec![],
            surfaces: vec![],
            stage_editor_open: false,
            dome_preview_open: false,
            dome_preview_texture: None,
            dome_mode_active: false,
            dome_preset: DomePreset::Quad,
            dome_geometry: DomeGeometry::default(),
            camera_detect_texture: None,
            camera_detect_mode: CameraDetectMode::Off,
            camera_detect_contours: vec![],
            library_panel_open: true,
            right_panel_open: true,
            stage_editor_grid_size: 0.05,
            stage_editor_snap: true,
            available_monitors: vec![],
            midi_devices: vec![],
            midi_mappings: vec![],
            cameras: vec![],
            ndi_sources: vec![],
            ndi_available: false,
            syphon_sources: vec![],
            syphon_available: false,
            srt_library_configs: vec![],
            hls_library_configs: vec![],
            dash_library_configs: vec![],
            rtmp_library_configs: vec![],
            html_library_configs: vec![],

            sequences: vec![],
            channel_count: 2,
            channel_names: vec!["Ch A".to_string(), "Ch B".to_string()],
            fps: 60.0,
            channel_render_stats: vec![
                ChannelRenderStats {
                    name: "Ch A".to_string(),
                    avg_deck_fps: 60.0,
                    active_deck_count: 2,
                    render_time_ms: 1.5,
                },
                ChannelRenderStats {
                    name: "Ch B".to_string(),
                    avg_deck_fps: 58.0,
                    active_deck_count: 1,
                    render_time_ms: 0.8,
                },
            ],
            gpu_device_name: "Test GPU".to_string(),
            gpu_backend: "Metal".to_string(),
            gpu_driver: "Apple".to_string(),
            gpu_driver_info: "Metal 3".to_string(),
            gpu_device_type: "IntegratedGpu".to_string(),
            gpu_utilization: 45.0,
            cpu_usage: 25.0,
            ram_used: 8 * 1024 * 1024 * 1024,
            ram_total: 16 * 1024 * 1024 * 1024,
            clock_source: "Audio".to_string(),
            clock_bpm: None,
            clock_active: false,
            clock_device_name: None,
            clock_detected_midi: vec![],
            clock_osc_active: false,
            clock_osc_bpm: None,
            clock_audio_bpm: None,
            clock_preference: "Auto".to_string(),
            clock_preference_force_device_id: None,
            clock_manual_bpm: None,
            render_width: 1920,
            render_height: 1080,
            max_render_dimension: 16384,
            target_fps: 60,
            can_undo: false,
            can_redo: false,
            pending_deck_loads: 0,
            deck_presets: vec![],
            channel_presets: vec![],
        }
    }
}

#[cfg(test)]
mod preview_channel_tests {
    use super::*;

    #[test]
    fn selected_deck_cues_its_channel() {
        let layout = UILayoutState {
            selected_deck: Some((1, 3)),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![1]);
    }

    #[test]
    fn selected_channel_cues_itself() {
        let layout = UILayoutState {
            selected_channel: Some(2),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![2]);
    }

    #[test]
    fn selected_master_cues_nothing() {
        let layout = UILayoutState {
            selected_master: true,
            ..Default::default()
        };
        assert!(layout.preview_channels().is_empty());
    }

    #[test]
    fn no_selection_cues_nothing() {
        let layout = UILayoutState::default();
        assert!(layout.preview_channels().is_empty());
    }

    #[test]
    fn deck_takes_precedence_over_channel() {
        // apply_selections keeps these mutually exclusive, but the derivation
        // must be deterministic even if both happen to be set.
        let layout = UILayoutState {
            selected_deck: Some((0, 0)),
            selected_channel: Some(1),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![0]);
    }
}
