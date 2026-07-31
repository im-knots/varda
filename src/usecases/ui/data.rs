//! Per-frame view model: the engine state projected into plain data the panels
//! render from.
//!
//! Built once per frame by [`super::build_ui_data`]. Panels read this and never
//! touch the engine directly (/spec/ui-engine-boundary.md).

use super::{panels, CameraDetectMode};
use crate::audio::AudioSourceId;
use crate::channel::DeckRenderFps;
use crate::modulation::{ADSRStage, AudioReactMode, LFOWaveform, StepInterpolation};
use crate::params::ParamValue;
use crate::renderer::context::OutputSource;
use crate::renderer::slicer::{DomeGeometry, DomePreset};
use crate::surface::detect::DetectedContour;
use crate::surface::{CircleHint, ContentMapping, SurfaceOutputType, SurfacePath};
use crate::{BlendMode, ScalingMode};

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

/// Normalized (`0..1`) point-cloud params backing the bottom-bar faders.
/// See spec/depth-sensors.md.
#[derive(Clone)]
pub struct PointCloudUI {
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub zoom: f32,
    pub point_size: f32,
    pub depth_min: f32,
    pub depth_max: f32,
    pub seed: f32,
    pub drift: f32,
    pub disruption: f32,
    /// 0 = Rgb, 1 = `DepthRamp`, 2 = Solid.
    pub color_mode: u8,
}

/// Normalized (`0..1`) depth-preprocessor params backing the bottom-bar faders.
/// See spec/depth-sensor-preprocessor.md.
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

/// Deck info for UI display
// Flat projection of independent deck flags (solo/mute/transparent/source kind).
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone)]
pub struct DeckUIInfo {
    pub deck_idx: usize,
    pub uuid: String,
    pub name: String,
    /// True when this deck's source is an HTML/Servo instance.
    pub is_html: bool,
    /// True when this deck's source is a depth sensor (point-cloud) source.
    pub is_depth_sensor: bool,
    /// Point-cloud controls (None = not a depth-sensor source).
    pub point_cloud: Option<PointCloudUI>,
    /// Depth-preprocessor controls (None = no `depth_sensor` preprocessor).
    pub depth_prepro: Option<DepthPreproUI>,
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

/// Notification snapshot for UI rendering (avoids borrowing `NotificationSystem` during egui)
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
// Aggregate view model; its bools are unrelated engine states, not a state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct UIData {
    pub generators: Vec<(String, usize)>,
    pub filters: Vec<(String, usize)>,
    pub shader_count: usize,
    pub channels: Vec<ChannelUIInfo>,
    pub master_effect_info: Vec<EffectInfo>,
    pub modulation_sources: Vec<ModSourceUIEntry>,
    /// Current computed values for each modulation source by UUID
    pub modulation_current_values: std::collections::HashMap<String, f32>,
    /// Modulation assignments: `param_key` -> list of (`source_id`, amount)
    pub modulation_assignments: std::collections::HashMap<String, Vec<ModAssignmentUI>>,
    /// User-defined macro controls (one control → many parameter targets).
    pub macros: Vec<crate::macros::Macro>,
    pub audio: AudioUIData,
    /// Deck preview textures keyed by deck UUID. UUID keys survive reordering
    /// and removal, so the map needs no reindex pass — see
    /// [`/spec/api-addressing.md`].
    pub deck_preview_textures: std::collections::HashMap<String, egui::TextureId>,
    /// Channel preview textures keyed by `ch_idx`
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
    /// Currently selected deck for detail view in bottom bar (`ch_idx`, `deck_idx`)
    pub selected_deck: Option<(usize, usize)>,
    /// Currently selected channel for detail view in bottom bar (`ch_idx`)
    pub selected_channel: Option<usize>,
    /// Whether the master output is selected for detail view in bottom bar
    pub selected_master: bool,
    /// Currently selected sequence for detail view in bottom bar (`seq_idx`)
    pub selected_sequence: Option<usize>,
    /// Currently selected step within the selected sequence (`seq_idx`, `step_idx`)
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
    /// Detected depth sensors (name, id)
    pub depth_sensors: Vec<(String, crate::depth::DepthSensorId)>,
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
    /// Pipeline-derived FPS: average of per-channel FPSes (from deck render timing)
    pub fps: f32,
    /// Per-channel render stats: (`channel_name`, `avg_deck_fps`, `active_deck_count`, `render_time_ms`)
    pub channel_render_stats: Vec<ChannelRenderStats>,
    /// GPU device name (e.g. "Apple M1 Pro")
    pub gpu_device_name: String,
    /// GPU backend (e.g. "Metal", "Vulkan", "Dx12")
    pub gpu_backend: String,
    /// GPU driver info string
    pub gpu_driver: String,
    /// GPU driver version/info
    pub gpu_driver_info: String,
    /// GPU device type (e.g. "`DiscreteGpu`", "`IntegratedGpu`")
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
    /// Device ID if preference is `ForceMidi`
    pub clock_preference_force_device_id: Option<crate::midi::DeviceId>,
    /// Manual BPM value (if preference is `ForceManual`)
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
    /// Loaded deck preset names (from `PresetLibrary`)
    pub deck_presets: Vec<String>,
    /// Loaded channel preset names (from `PresetLibrary`)
    pub channel_presets: Vec<String>,
}

/// Read-only snapshot of a single transition sequence
#[derive(Clone)]
pub struct SequenceUIData {
    /// Stable UUID — the address for every sequence command.
    pub uuid: String,
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
