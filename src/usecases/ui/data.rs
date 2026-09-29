//! Per-frame view model: engine state projected into plain data for the panels.
//!
//! Built once per frame by [`super::build_ui_data`]. Panels read this and never
//! touch the engine directly.

use super::{CameraDetectMode, panels};
use crate::BlendMode;
use crate::audio::AudioSourceId;
use crate::channel::DeckRenderFps;
use crate::modulation::{ADSRStage, AudioReactMode, LFOWaveform, StepInterpolation};
use crate::params::ParamValue;
use crate::renderer::context::OutputSource;
use crate::renderer::slicer::{DomeGeometry, DomePreset};
use crate::surface::detect::DetectedContour;
use crate::surface::{CircleHint, ContentMapping, SurfaceOutputType, SurfacePath};

/// Parameter info for UI rendering, collected before egui to avoid borrow conflicts.
#[derive(Clone)]
pub struct ParamUIInfo {
    pub name: String,
    pub label: Option<String>,
    pub value: ParamValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// Inspector section. `None` params are ungrouped and render first, without a
    /// header.
    pub group: Option<String>,
    /// Options for a `long` param, empty for every other type.
    pub choices: Vec<ParamChoiceUI>,
}

/// One option of a `long` (enum) parameter.
#[derive(Clone)]
pub struct ParamChoiceUI {
    pub value: i32,
    pub label: String,
}

/// Shader parameters for the UI (generator or effect).
#[derive(Clone)]
pub struct ShaderParamsUI {
    pub shader_name: String,
    pub params: Vec<ParamUIInfo>,
}

/// Modulation source snapshot for the UI, with its UUID.
#[derive(Clone)]
pub struct ModSourceUIEntry {
    pub uuid: String,
    pub source: ModSourceUI,
    /// Which clock this source follows.
    pub timebase: crate::timebase::Timebase,
}

impl ModSourceUIEntry {
    /// Short label for this source wherever a modulator is listed or shown as a
    /// value's origin. `idx` is the source's position in the full modulation list,
    /// which also picks its color.
    pub fn label(&self, idx: usize) -> String {
        match &self.source {
            ModSourceUI::LFO { .. } => format!("LFO {}", idx + 1),
            ModSourceUI::Audio {
                freq_low,
                freq_high,
                ..
            } => format!("Audio {freq_low:.0}-{freq_high:.0}Hz"),
            ModSourceUI::ADSR { .. } => format!("ADSR {}", idx + 1),
            ModSourceUI::StepSequencer { .. } => format!("StepSeq {}", idx + 1),
            ModSourceUI::Analyzer { analyzer_type, .. } => {
                format!("Analyzer {} {}", analyzer_type, idx + 1)
            }
            ModSourceUI::Envelope { .. } => format!("Automation {}", idx + 1),
        }
    }
}

/// Modulation source snapshot for the UI.
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
    /// Automation curve, edited in its arrangement lane rather than as a card.
    Envelope {
        breakpoints: Vec<crate::modulation::Breakpoint>,
    },
}

/// Modulation source color, unique for any index via binary hue subdivision.
///
/// Same algorithm as channel colors, offset by half the hue wheel and with
/// higher saturation and different lightness bands, so modulator colors stay
/// distinct from channel colors.
pub fn modulator_color(idx: usize) -> egui::Color32 {
    // Opposite side of the hue wheel from channel colors (0.76 + 0.5 = 0.26).
    const HUE_OFFSET: f32 = 0.26;

    // Brighter and more saturated than channel styles, to stand out on a dark UI.
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

/// Modulation assignment snapshot for the UI.
#[derive(Clone)]
pub struct ModAssignmentUI {
    pub source_id: String,
    pub amount: f32,
}

/// Effect info for the UI: (uuid, name, enabled, params).
pub type EffectInfo = (String, String, bool, ShaderParamsUI);

/// Auto-transition state snapshot for the UI.
// Mirrors independent engine-side flags one-for-one; collapsing them would obscure the mapping.
#[allow(clippy::struct_excessive_bools)]
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

/// Normalized (`0..1`) depth-preprocessor params backing the bottom-bar faders.
#[derive(Clone)]
pub struct DepthPreproUI {
    pub sensor_name: String,
    pub near: f32,
    pub far: f32,
    pub smoothing: f32,
    pub hole_fill: f32,
    pub mask_feather: f32,
    pub motion_gain: f32,
    pub mirror: bool,
}

/// Deck info for the UI.
// Flat projection of independent deck flags (solo/mute/transparent/source kind).
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone)]
pub struct DeckUIInfo {
    pub deck_idx: usize,
    pub uuid: String,
    pub name: String,
    /// The deck's source: its type and the state of its controls. The type's
    /// schema is in [`UIData::sources`].
    pub source: crate::engine::value::source::DeckSourceSnapshot,
    /// Depth-preprocessor controls (None = no `depth_sensor` preprocessor).
    pub depth_prepro: Option<DepthPreproUI>,
    /// True while the deck's interactive window is open.
    pub is_interactive: bool,
    pub opacity: f32,
    /// Opacity including auto-transition state, for display only.
    pub effective_opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    /// True when this deck preserves source alpha (transparent compositing).
    pub transparent: bool,
    pub generator: ShaderParamsUI,
    pub effects: Vec<EffectInfo>,
    /// Auto-transition state (None = not configured).
    pub auto_transition: Option<AutoTransitionUI>,
    /// Per-deck render FPS setting.
    pub render_fps: DeckRenderFps,
    /// Render rate the deck is achieving.
    pub effective_render_fps: f32,
    /// Smoothed render cost in microseconds.
    pub render_cost_us: f32,
    /// GPU-measured render cost in microseconds (0 = not available).
    pub gpu_render_cost_us: f32,
}

/// Channel info for the UI.
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

/// Audio input device info for the UI.
#[derive(Clone)]
pub struct AudioDeviceUI {
    pub id: AudioSourceId,
    pub name: String,
    pub active: bool,
}

/// Audio snapshot for the UI.
#[derive(Clone)]
pub struct AudioUIData {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub bpm: Option<f32>,
    pub beat_phase: f32,
    pub enabled: bool,
    /// Available audio input devices.
    pub devices: Vec<AudioDeviceUI>,
    /// FFT spectrum of the primary source, 256 bins.
    pub fft: Vec<f32>,
    /// Sample rate of the primary source.
    pub sample_rate: f32,
}

/// Notification snapshot, so egui rendering doesn't borrow `NotificationSystem`.
#[derive(Clone)]
pub struct NotificationUI {
    /// Stable id to dismiss this notification by.
    pub id: u64,
    pub level: crate::notifications::NotificationLevel,
    pub message: String,
    pub progress: f32,
}

/// Per-channel render statistics for the FPS popover.
pub struct ChannelRenderStats {
    pub name: String,
    /// Average FPS across active decks in this channel, from deck render timing.
    pub avg_deck_fps: f32,
    /// Number of active (rendered) decks.
    pub active_deck_count: u32,
    /// Total channel render time in milliseconds.
    pub render_time_ms: f32,
}

/// Everything needed to render the UI.
// Aggregate view model; its bools are unrelated engine states, not a state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct UIData {
    pub filters: Vec<(String, usize)>,
    pub shader_count: usize,
    pub channels: Vec<ChannelUIInfo>,
    pub master_effect_info: Vec<EffectInfo>,
    pub modulation_sources: Vec<ModSourceUIEntry>,
    /// Current value of each modulation source, by UUID.
    pub modulation_current_values: std::collections::HashMap<String, f32>,
    /// Modulation assignments: `param_key` -> list of (`source_id`, amount).
    pub modulation_assignments: std::collections::HashMap<String, Vec<ModAssignmentUI>>,
    /// User-defined macro controls (one control → many parameter targets).
    pub macros: Vec<crate::macros::Macro>,
    pub audio: AudioUIData,
    /// Deck preview textures keyed by deck UUID, which survives reordering and
    /// removal.
    pub deck_preview_textures: std::collections::HashMap<String, egui::TextureId>,
    /// Channel preview textures keyed by `ch_idx`.
    pub channel_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    /// Output preview textures keyed by output index.
    pub output_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    pub main_output_texture: Option<egui::TextureId>,
    pub notifications: Vec<NotificationUI>,
    /// Crossfader position (0.0 = A, 1.0 = B).
    pub crossfader: f32,
    /// Whether an auto-crossfade is running.
    pub auto_crossfade_active: bool,
    /// Auto-crossfade progress (0.0–1.0), if active.
    pub auto_crossfade_progress: f32,
    /// Current tonemap mode.
    pub tonemap_mode: crate::renderer::tonemap::TonemapMode,
    /// Active LUT filename, if any.
    pub active_lut_filename: Option<String>,
    /// Scene-referred look LUT filename, applied before every output transform.
    pub look_lut_filename: Option<String>,
    /// LUT files available in .varda/luts/.
    pub available_luts: std::sync::Arc<[String]>,
    /// Whether MIDI learn mode is active.
    pub midi_learn_active: bool,
    /// Parameter path waiting for MIDI learn.
    pub midi_learn_target: Option<String>,
    /// Whether keyboard learn mode is active.
    pub keyboard_learn_active: bool,
    /// Display string for the keyboard learn target.
    pub keyboard_learn_target: Option<String>,
    /// Current keybindings, read-only, for dispatch and the settings panel.
    pub keymap_bindings:
        std::collections::HashMap<crate::keymap::KeyCombo, crate::keymap::KeyTarget>,
    /// Transition shader names from the registry.
    pub transition_names: Vec<String>,
    /// Active transition name, if any.
    pub active_transition_name: Option<String>,
    /// Selected deck for the bottom-bar detail view (`ch_idx`, `deck_idx`).
    pub selected_deck: Option<(usize, usize)>,
    /// Selected channel for the bottom-bar detail view (`ch_idx`).
    pub selected_channel: Option<usize>,
    /// Whether master output is selected for the bottom-bar detail view.
    pub selected_master: bool,
    /// Selected sequence for the bottom-bar detail view (`seq_idx`).
    pub selected_sequence: Option<usize>,
    /// Selected step within the selected sequence (`seq_idx`, `step_idx`).
    pub selected_sequence_step: Option<(usize, usize)>,
    /// Selected macro (by UUID) for the bottom-bar detail view.
    pub selected_macro: Option<String>,
    /// All outputs, windowed and headless.
    pub outputs: Vec<OutputUI>,
    /// Surfaces in the stage layout.
    pub surfaces: Vec<SurfaceUI>,
    /// Whether the full-screen stage editor is open (replaces the deck view).
    pub stage_editor_open: bool,
    /// Whether the central area shows the arrangement timeline instead of the
    /// mixer.
    pub arrangement_mode_open: bool,
    /// The scene's arrangement, absent in a Performance-only scene.
    pub arrangement: Option<crate::engine::types::ArrangementSnapshot>,
    /// Timeline horizontal zoom, in pixels per second of show time.
    pub arrangement_pixels_per_second: f32,
    /// Show position at the timeline's left edge.
    pub arrangement_scroll: f64,
    /// Rows scrolled off the top of the timeline, in pixels.
    pub arrangement_scroll_y: f32,
    /// Whether timeline edits round to whole frames at the ruler's rate.
    pub arrangement_snap: bool,
    /// The part of the show being worked on. Seeded from the transport's loop for
    /// a scene saved with one.
    pub arrangement_focus: Option<crate::usecases::ui::state::FocusRange>,
    /// What the clipboard holds, so menus can name it and disable Paste when it
    /// doesn't fit.
    pub clipboard: Option<crate::engine::ClipboardSummary>,
    /// Whether the 3D dome preview is open in the stage editor.
    pub dome_preview_open: bool,
    /// Dome preview texture (rendered 3D hemisphere).
    pub dome_preview_texture: Option<egui::TextureId>,
    /// Whether the stage editor is in 3D Dome mode (vs 2D Polygon mode).
    pub dome_mode_active: bool,
    /// Active dome preset.
    pub dome_preset: DomePreset,
    /// Active dome geometry (radius, truncation, tilt).
    pub dome_geometry: DomeGeometry,
    /// Domemaster render size (square).
    pub domemaster_resolution: crate::renderer::dome::DomemasterResolution,
    /// Live camera feed texture for camera detection.
    pub camera_detect_texture: Option<egui::TextureId>,
    /// Camera detection mode state.
    pub camera_detect_mode: CameraDetectMode,
    /// Contours detected in the current frame, for the overlay.
    pub camera_detect_contours: Vec<DetectedContour>,
    /// Whether the library panel (left sidebar) is open.
    pub library_panel_open: bool,
    /// Whether the right panel (master output sidebar) is open.
    pub right_panel_open: bool,
    /// Stage editor grid size (normalized; 0.05 = 20 divisions).
    pub stage_editor_grid_size: f32,
    /// Whether snap-to-grid is on in the stage editor.
    pub stage_editor_snap: bool,
    /// Available display monitors, refreshed each frame.
    pub available_monitors: Vec<MonitorInfo>,
    /// Connected MIDI devices.
    pub midi_devices: Vec<MidiDeviceUI>,
    /// Current MIDI mappings.
    pub midi_mappings: Vec<MidiMappingUI>,
    /// Available camera devices (name, id).
    pub cameras: Vec<(String, crate::camera::CameraId)>,
    /// Every registered deck source type: its controls and library.
    pub sources: std::sync::Arc<Vec<crate::engine::value::provider::ProviderTypeSnapshot>>,
    /// Every output sink type: its settings and library.
    pub sinks: std::sync::Arc<Vec<crate::engine::value::provider::ProviderTypeSnapshot>>,
    /// Transition sequences.
    pub sequences: Vec<SequenceUIData>,
    /// Number of channels, for the sequence builder's channel dropdowns.
    pub channel_count: usize,
    /// Average of per-channel FPS, from deck render timing.
    pub fps: f32,
    /// Per-channel render stats.
    pub channel_render_stats: Vec<ChannelRenderStats>,
    /// GPU device name (e.g. "Apple M1 Pro").
    pub gpu_device_name: String,
    /// GPU backend (e.g. "Metal", "Vulkan", "Dx12").
    pub gpu_backend: String,
    /// GPU driver name.
    pub gpu_driver: String,
    /// GPU driver version info.
    pub gpu_driver_info: String,
    /// GPU device type (e.g. "`DiscreteGpu`", "`IntegratedGpu`").
    pub gpu_device_type: String,
    /// GPU utilization % (0–100), from GPU timestamps.
    pub gpu_utilization: f32,
    /// CPU usage % (0–100).
    pub cpu_usage: f32,
    /// RAM used, in bytes.
    pub ram_used: u64,
    /// RAM total, in bytes.
    pub ram_total: u64,
    /// Clock sync source label ("Audio", "MIDI", "OSC", "None").
    pub clock_source: String,
    /// Clock sync BPM, if active.
    pub clock_bpm: Option<f32>,
    /// Whether clock sync is active.
    pub clock_active: bool,
    /// Clock MIDI device name, if the source is MIDI.
    pub clock_device_name: Option<String>,
    /// Detected MIDI clock sources for the popover.
    pub clock_detected_midi: Vec<crate::engine::types::DetectedClockSourceSnapshot>,
    /// Whether OSC clock is active.
    pub clock_osc_active: bool,
    /// OSC BPM, if active.
    pub clock_osc_bpm: Option<f32>,
    /// Audio BPM (fallback).
    pub clock_audio_bpm: Option<f32>,
    /// Current clock preference label.
    pub clock_preference: String,
    /// Device ID when the preference is `ForceMidi`.
    pub clock_preference_force_device_id: Option<crate::midi::DeviceId>,
    /// Manual BPM when the preference is `ForceManual`.
    pub clock_manual_bpm: Option<f32>,
    /// Number of modulation sources locked to the beat; sets the tempo readout's
    /// emphasis.
    pub clock_beat_followers: usize,
    /// Absolute show position, separate from the tempo clock.
    pub transport: crate::engine::types::TransportSnapshot,
    /// Every timecode input being listened to, and which one is driving. The
    /// transport popover shows it so a bad cable can be told from a stopped show.
    pub timecode: crate::engine::types::TimecodeSnapshot,
    /// Master render width.
    pub render_width: u32,
    /// Master render height.
    pub render_height: u32,
    /// GPU's maximum 2D texture dimension, the only limit on custom render
    /// resolution.
    pub max_render_dimension: u32,
    /// Target FPS (0 = uncapped).
    pub target_fps: u32,
    /// Whether undo is available.
    pub can_undo: bool,
    /// Whether redo is available.
    pub can_redo: bool,
    /// Number of decks loading in background threads.
    pub pending_deck_loads: usize,
    /// Deck preset names from `PresetLibrary`.
    pub deck_presets: Vec<String>,
    /// Channel preset names from `PresetLibrary`.
    pub channel_presets: Vec<String>,
}

impl UIData {
    /// The schema of a source type, from the snapshot.
    pub fn source_type(
        &self,
        source_type: &str,
    ) -> Option<&crate::engine::value::provider::ProviderTypeSnapshot> {
        self.sources.iter().find(|t| t.type_id == source_type)
    }

    /// The schema of an output sink type, from the snapshot.
    pub fn sink_type(
        &self,
        type_id: &str,
    ) -> Option<&crate::engine::value::provider::ProviderTypeSnapshot> {
        self.sinks.iter().find(|t| t.type_id == type_id)
    }

    /// A surface source by channel and deck names instead of UUIDs. A reference to
    /// something that no longer exists reads as missing, not as the master.
    pub fn surface_source_label(&self, source: &crate::renderer::context::OutputSource) -> String {
        use crate::renderer::context::OutputSource;
        let channel = |uuid: &str| {
            self.channels
                .iter()
                .find(|c| c.uuid == uuid)
                .map_or_else(|| "(missing channel)".to_string(), |c| c.name.clone())
        };
        match source {
            OutputSource::Master => "Master".into(),
            OutputSource::Domemaster => "Domemaster".into(),
            OutputSource::Channel(uuid) => channel(uuid),
            OutputSource::Channels(uuids) => uuids
                .iter()
                .map(|u| channel(u))
                .collect::<Vec<_>>()
                .join("+"),
            OutputSource::Deck(uuid) => self
                .channels
                .iter()
                .find_map(|c| {
                    c.decks
                        .iter()
                        .find(|d| d.uuid == *uuid)
                        .map(|d| format!("{} / {}", c.name, d.name))
                })
                .unwrap_or_else(|| "(missing deck)".into()),
        }
    }

    /// The label `deck`'s source gives the control at `route`, so every panel names
    /// source controls like the deck's own column.
    pub fn source_param_label(&self, deck: &DeckUIInfo, route: &str) -> Option<&str> {
        self.source_type(&deck.source.source_type)?
            .params
            .iter()
            .find(|spec| spec.route.as_deref() == Some(route))
            .map(|spec| spec.label.as_str())
    }
}

/// Read-only snapshot of one transition sequence.
#[derive(Clone)]
pub struct SequenceUIData {
    /// Stable UUID used to address every sequence command.
    pub uuid: String,
    /// Display name.
    pub name: String,
    /// Whether the sequence is enabled.
    pub enabled: bool,
    /// Whether the sequencer is playing.
    pub playing: bool,
    /// Current step index while playing.
    pub current_step: usize,
    /// Elapsed time within the current step, in seconds.
    pub step_elapsed: f64,
    /// Step descriptions for display.
    pub steps: Vec<SequenceStepUI>,
}

/// One step shown in the sequence builder.
#[derive(Clone)]
pub struct SequenceStepUI {
    pub label: String,
    pub kind: SequenceStepKindUI,
}

/// Step kind for the UI.
#[derive(Clone)]
pub enum SequenceStepKindUI {
    Fade {
        from_ch: String,
        to_ch: String,
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

/// An available display monitor, for the display selector.
#[derive(Clone)]
pub struct MonitorInfo {
    pub name: String,
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

/// MIDI device info for the UI.
#[derive(Clone)]
pub struct MidiDeviceUI {
    pub id: crate::midi::DeviceId,
    pub name: String,
    pub enabled: bool,
    pub has_output: bool,
    pub profile: String,
}

/// MIDI mapping entry for the UI.
#[derive(Clone)]
pub struct MidiMappingUI {
    pub key: crate::midi::MidiKey,
    pub key_display: String,
    pub device_name: String,
    pub param_path: String,
}

/// Surface assignment snapshot for the UI.
#[derive(Clone)]
pub struct SurfaceAssignmentUI {
    pub surface_uuid: String,
    pub surface_name: String,
    pub enabled: bool,
    /// Per-surface overlap zones (Auto mode). Empty when Manual or no overlaps.
    pub overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones,
}

/// Output state snapshot for the UI, windowed or headless.
#[derive(Clone)]
pub struct OutputUI {
    pub uuid: String,
    pub name: String,
    /// Where the output delivers: its sink type, settings and state. The
    /// type's settings schema is in [`UIData::sinks`].
    pub sink: crate::engine::types::OutputSinkSnapshot,
    /// Whether the output is showing: a window always, a startable sink while it
    /// runs.
    pub is_active: bool,
    /// What the output shows with no surfaces assigned.
    pub unassigned: crate::engine::value::render::Unassigned,
    /// Duration of active recording or streaming.
    pub active_duration: std::time::Duration,
    pub surface_assignments: Vec<SurfaceAssignmentUI>,
    pub calibration_mode: crate::renderer::context::CalibrationMode,
    /// Edge blend mode (Auto / Manual).
    pub edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode,
    /// Edge blending configuration.
    pub edge_blend: crate::renderer::edge_blend::EdgeBlendConfig,
    /// Per-output rotation (0°/90°/180°/270°).
    pub rotation: crate::renderer::context::OutputRotation,
    /// Persisted precision and dithering request.
    pub presentation_request: crate::engine::value::render::PresentationRequest,
    /// Runtime format selected by the active adapter.
    pub resolved_presentation: crate::engine::value::render::ResolvedPresentation,
    /// Every presentation mode, with the reason this output cannot deliver it, if
    /// any. The picker disables blocked modes.
    pub mode_availability: Vec<crate::engine::value::render::ModeAvailability>,
    /// Per-output tonemap override. `None` inherits the show-wide curve.
    pub tonemap_override: Option<crate::engine::value::render::TonemapMode>,
    /// Audio passthrough health for an active ffmpeg output (None = video-only).
    pub audio_passthrough: Option<AudioPassthroughUI>,
    /// ffmpeg video health (None = no subprocess on this output).
    pub delivery: Option<DeliveryHealthUI>,
    /// Pixel size of the texture the output panel previews: the render resolution
    /// for a headless output, the (rotated) window size for a windowed one. The
    /// panel sizes the widget from this so a windowed preview isn't stretched.
    pub preview_width: u32,
    pub preview_height: u32,
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
    /// Samples of silence written in place of those drops, which is what the
    /// listener hears.
    pub silence_spliced: u64,
}

/// Live ffmpeg video health for an active recording or stream.
#[derive(Clone)]
pub struct DeliveryHealthUI {
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub frames_padded: u64,
}

/// Dome-mode UI actions: the editor view toggle and preview camera navigation.
/// Dome config itself is engine state, set with `SetDomePreset` and
/// `SetDomeGeometry`.
#[derive(Debug, Clone)]
pub enum DomeAction {
    /// Toggle between 2D Polygon mode and 3D Dome mode.
    SetMode(bool),
    /// Rotate the orbit camera by a pixel delta.
    RotateCamera { delta_x: f32, delta_y: f32 },
    /// Zoom the orbit camera by a scroll delta.
    ZoomCamera { delta: f32 },
    /// Reset the orbit camera.
    ResetCamera,
}

/// Surface snapshot for the UI.
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
    /// `warp_bound`, this is the shape-conforming warp.
    pub warp: Option<crate::surface::warp::WarpMode>,
    /// Whether the warp conforms to the surface shape. When `true` the bottom-bar
    /// warp controls are read-only.
    pub warp_bound: bool,
    /// Curve authoring path for bezier-edited surfaces. Drives the anchor/handle
    /// overlay and edge hit-testing in the stage editor.
    pub path: Option<SurfacePath>,
    /// Subtractive cut-out holes, drawn as editable overlay contours.
    pub holes: Vec<SurfacePath>,
    /// Flattened hole contours (canvas coords) for overlay rendering.
    pub hole_contours: Vec<Vec<[f32; 2]>>,
}
