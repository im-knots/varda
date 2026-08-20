//! Shared value types for the engine layer.
//!
//! These types are used in engine trait signatures and snapshot structs.
//! They MUST NOT reference wgpu, egui, winit, or any GPU/UI framework types.
//!
//! Per /spec/engine-value-types.md, this module names its value vocabulary
//! from two places, and never reaches into `internal::{renderer,surface,video}`
//! directly:
//! - **Tier 1** (`engine::value::*`): plain data owned by the engine, whose
//!   *definitions* live in `crate::engine::value` — `renderer`/`surface`/
//!   `video` `pub use` them back to keep their existing call paths working.
//! - **Tier 2** (domain modules below): genuine domain entities that already
//!   lived in pure, framework-free modules (`audio`, `camera`, `channel`,
//!   `deck`, `mixer`, `modulation`, `params`) — out of scope for the Tier 1
//!   relocation; re-exported here as-is.

use serde::{Deserialize, Serialize};

// Tier 2 — pure domain modules, re-exported as-is.
pub use crate::audio::AudioSourceId;
pub use crate::camera::CameraId;
pub use crate::channel::{BlendMode, DeckRenderFps};
pub use crate::deck::ScalingMode;
pub use crate::depth::DepthSensorId;
pub use crate::mixer::CrossfadeEasing;
pub use crate::modulation::{
    ADSRStage, AudioBandPreset, AudioReactMode, LFOWaveform, StepInterpolation,
};
pub use crate::params::ParamValue;

// Tier 1 — engine-owned value types (see `crate::engine::value`).
pub use crate::engine::value::render::OutputSource;
pub use crate::engine::value::surface::{
    CircleHint, ContentMapping, CubicHandle, SurfaceOutputType, SurfacePath, SurfaceReorderOp,
};
pub use crate::engine::value::video::{DeckTransportSync, LoopMode, TransportSyncMode};

/// Identifies which effect chain to operate on.
///
/// Used for chain-scoped operations (append an effect, reorder within a chain).
/// Operations on an *existing* effect address it by its own UUID instead — see
/// [`/spec/api-addressing.md`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum EffectTarget {
    /// A deck's pre-composite chain, by deck UUID.
    Deck(String),
    /// A channel's post-composite chain, by channel UUID.
    Channel(String),
    /// The master output chain.
    Master,
}

/// What a copy is taken from. See [`/spec/clipboard.md`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub enum ClipboardSource {
    Deck(String),
    Channel(String),
    Effect(String),
}

/// Where a paste lands.
///
/// The `After*` forms are what a right-click uses, so a copy arrives directly
/// below the thing the menu was opened on; the `Into*` forms append, which is
/// what the container's own menu and an API caller mean.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub enum PasteTarget {
    /// This deck's channel, directly below it.
    AfterDeck(String),
    /// The end of this channel.
    IntoChannel(String),
    /// This effect's chain, directly after it.
    AfterEffect(String),
    /// The end of a deck, channel, or master chain.
    IntoChain(EffectTarget),
    /// A new channel at the end of the mixer.
    NewChannel,
}

/// What the clipboard is holding, for a menu that has to name it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ClipboardSummary {
    pub kind: ClipboardKind,
    /// The object's own name, for "Paste deck 'ripple'".
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum ClipboardKind {
    Deck,
    Channel,
    Effect,
}

/// Per-frame engine state snapshot — plain data, no GPU types, no lifetimes.
///
/// Produced by `VardaApp` each frame. Distributed to consumers via watch channel.
/// `UIData` is derived from this for the egui UI consumer.
// Serialized DTO: the flags mirror independent engine toggles, not a state enum.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Serialize)]
pub struct EngineState {
    pub mixer: MixerSnapshot,
    pub audio: AudioSnapshot,
    pub modulation: ModulationSnapshot,
    pub outputs: OutputSnapshot,
    pub registry: RegistrySnapshot,
    pub midi: MidiSnapshot,
    pub cameras: CameraSnapshot,
    pub depth_sensors: DepthSensorSnapshot,
    pub screen_capture: ScreenCaptureSnapshot,
    pub clock: ClockSnapshot,
    pub transport: TransportSnapshot,
    pub timecode: TimecodeSnapshot,
    /// Present only when the scene has an arrangement.
    pub arrangement: Option<ArrangementSnapshot>,
    pub fps: f32,
    pub frame_count: u64,
    /// Target FPS (0 = uncapped)
    pub target_fps: u32,
    /// Discovered NDI sources (names)
    pub ndi_sources: Vec<String>,
    /// Whether NDI runtime is available
    pub ndi_available: bool,
    /// Discovered Syphon servers (names)
    pub syphon_sources: Vec<String>,
    /// Whether Syphon framework is available
    pub syphon_available: bool,
    /// Active stream receiver configs (url, mode, connected)
    pub stream_receivers: Vec<StreamReceiverSnapshot>,
    pub analyzers: Vec<AnalyzerTypeInfo>,
    /// User-defined macro controls (one control → many parameter targets).
    pub macros: Vec<crate::macros::Macro>,
    /// Whether the undo timeline has an undoable action (shared UI/API timeline).
    pub can_undo: bool,
    /// Whether the redo timeline has a redoable action (shared UI/API timeline).
    pub can_redo: bool,
}

/// Snapshot of an active stream receiver for UI consumption.
#[derive(Clone, Serialize)]
pub struct StreamReceiverSnapshot {
    pub url: String,
    pub mode: String,
    pub connected: bool,
}

// ── Clock Snapshot ──────────────────────────────────────────────

/// A detected MIDI clock source for UI display.
#[derive(Clone, Debug, Serialize)]
pub struct DetectedClockSourceSnapshot {
    pub device_id: crate::midi::DeviceId,
    pub device_name: String,
    pub bpm: Option<f32>,
}

/// Snapshot of the unified clock state for UI display.
#[derive(Clone, Serialize)]
pub struct ClockSnapshot {
    /// Current BPM from the resolved clock source.
    pub bpm: Option<f32>,
    /// Beat phase 0.0–1.0.
    pub beat_phase: f32,
    /// Which source is active: "Audio", "MIDI", "OSC", or "None".
    pub source_label: String,
    /// Device name (for MIDI clock source).
    pub device_name: Option<String>,
    /// Whether a valid clock source is active.
    pub active: bool,
    /// All MIDI devices currently detected as sending clock ticks.
    pub detected_midi_sources: Vec<DetectedClockSourceSnapshot>,
    /// Whether OSC clock is currently active.
    pub osc_active: bool,
    /// Current OSC BPM (if active).
    pub osc_bpm: Option<f32>,
    /// Current audio BPM (always available as fallback).
    pub audio_bpm: Option<f32>,
    /// Current preference label: "Auto", "`ForceMidi`(<name>)", "`ForceOsc`", "`ForceAudio`", "`ForceManual`".
    pub preference_label: String,
    /// Device ID if preference is `ForceMidi`.
    pub preference_force_device_id: Option<crate::midi::DeviceId>,
    /// Manual BPM value (if preference is `ForceManual`).
    pub manual_bpm: Option<f32>,
    /// How many modulation sources are locked to the beat. Drives the readout's
    /// emphasis and answers "what stops if this clock goes away".
    /// See /spec/transport.md § Tempo and position are both shown.
    pub beat_followers: usize,
}

/// Snapshot of arrangement mode. See /spec/arrangement.md.
///
/// Absent when the scene has no arrangement, which is how a Performance-only
/// scene stays free of arrangement concepts entirely.
#[derive(Clone, Serialize, Default)]
pub struct ArrangementSnapshot {
    /// The authored lanes and idle behaviour, verbatim.
    pub config: crate::arrangement::ArrangementConfig,
    /// Whether the arrangement is currently driving decks. False before the
    /// transport has run, so a scene that opens in Performance mode stays there.
    pub engaged: bool,
    /// Modulation keys a performer has taken by hand. Drives the "held" badge
    /// and the re-arm affordance.
    pub overridden_params: Vec<String>,
    /// Latest position covered by any region, for the ruler's default extent.
    pub duration: f64,
}

/// Snapshot of the absolute show position. See /spec/transport.md.
///
/// Distinct from [`ClockSnapshot`], which is tempo. Both can be live at once.
#[derive(Clone, Serialize)]
pub struct TransportSnapshot {
    /// Absolute position in seconds. `f64` because shows conventionally start
    /// at hour 1.
    pub position: f64,
    pub running: bool,
    /// Whether the transport has advanced at least once this session. Until it
    /// has, position-locked features stay inert.
    pub has_run: bool,
    pub source: crate::transport::TransportSource,
    /// Why the transport is or is not moving, so idle and broken are
    /// distinguishable on a dark stage.
    pub status_label: String,
    pub loop_region: Option<crate::transport::LoopRegion>,
    /// Frame rate positions are displayed at.
    pub timecode_rate: crate::transport::TimecodeRate,
    /// Position pre-rendered as `HH:MM:SS:FF`, so every consumer shows the
    /// same string rather than each reimplementing drop-frame.
    pub timecode: String,
    /// How many modulation sources are locked to the transport. Counterpart to
    /// [`ClockSnapshot::beat_followers`].
    pub followers: usize,
    /// Whether live parameter writes are being kept as automation. Armed and
    /// recording are different states: nothing is written until the position
    /// moves. See /spec/automation-recording.md.
    pub record_armed: bool,
    /// Parameter keys with a take open right now, so the lanes catching a pass
    /// can say so. Filled in by the snapshot builder, which can see the
    /// recorder.
    pub recording_params: Vec<String>,
}

impl Default for TransportSnapshot {
    fn default() -> Self {
        Self {
            position: 0.0,
            running: false,
            has_run: false,
            source: crate::transport::TransportSource::default(),
            status_label: crate::transport::TransportStatus::Idle.label().to_string(),
            loop_region: None,
            timecode_rate: crate::transport::TimecodeRate::default(),
            timecode: crate::transport::TimecodeRate::default().format(0.0),
            followers: 0,
            record_armed: false,
            recording_params: Vec::new(),
        }
    }
}

impl From<&crate::transport::Transport> for TransportSnapshot {
    /// `followers` is left at zero here: the count lives in the modulation
    /// engine, which the transport has no reference to. The snapshot builder
    /// fills it in.
    fn from(t: &crate::transport::Transport) -> Self {
        Self {
            position: t.position(),
            running: t.running(),
            has_run: t.has_run(),
            source: t.source(),
            status_label: t.status().label().to_string(),
            loop_region: t.loop_region(),
            timecode_rate: t.timecode_rate(),
            timecode: t.formatted_position(),
            followers: 0,
            record_armed: false,
            recording_params: Vec::new(),
        }
    }
}

// ── Timecode Snapshot ──────────────────────────────────────────────

/// One timecode input, resolved or not. See /spec/timecode.md.
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct TimecodeInputSnapshot {
    /// Stable name, `ltc` or `mtc:<device>`, and what `resolved` names.
    pub key: String,
    /// For a readout: "LTC (channel 2)", "MTC (Tascam Model 12)".
    pub label: String,
    pub position: f64,
    /// Position as `HH:MM:SS:FF` at this input's own rate, which is not
    /// necessarily the rate the ruler is drawn at.
    pub timecode: String,
    pub rate: crate::transport::TimecodeRate,
    pub running: bool,
    /// Coasting through a dropout rather than reading frames.
    pub freewheeling: bool,
    /// Measured against wall time; 1.0 while a master plays forwards.
    pub speed: f64,
}

/// Every timecode input being listened to, and which one is driving.
///
/// A list rather than an object even though one signal drives the transport: a
/// performer chasing a bad cable needs to see the input that is *not*
/// resolving. See /spec/timecode.md § Dual simultaneous inputs.
#[derive(Clone, Serialize, Default, utoipa::ToSchema)]
pub struct TimecodeSnapshot {
    pub inputs: Vec<TimecodeInputSnapshot>,
    /// `key` of the input driving the transport, if any.
    pub resolved: Option<String>,
    pub preference: crate::timecode::TimecodePreference,
    /// The audio input LTC is expected on, while one is patched.
    pub ltc_input: Option<crate::timecode::LtcInput>,
}

// ── Registry Snapshot ──────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct RegistrySnapshot {
    /// Generator shaders: (name, index)
    pub generators: Vec<(String, usize)>,
    /// Filter shaders: (name, index)
    pub filters: Vec<(String, usize)>,
    /// Total shader count
    pub shader_count: usize,
}

// ── Mixer Snapshot ──────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct MixerSnapshot {
    pub channels: Vec<ChannelSnapshot>,
    pub crossfader: f32,
    pub auto_crossfade_active: bool,
    pub auto_crossfade_progress: f32,
    pub master_effects: Vec<EffectSnapshot>,
    pub active_transition_name: Option<String>,
    pub transition_names: Vec<String>,
    pub sequences: Vec<SequenceSnapshot>,
    pub tonemap_mode: crate::engine::value::render::TonemapMode,
    pub active_lut: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ChannelSnapshot {
    pub idx: usize,
    pub uuid: String,
    pub name: String,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub decks: Vec<DeckSnapshot>,
    pub effects: Vec<EffectSnapshot>,
    /// Smoothed render time for this channel in milliseconds
    pub render_time_ms: f32,
    /// Number of active (rendered) decks in the last frame
    pub active_deck_count: u32,
}

// Serialized DTO: the flags mirror independent deck toggles, not a state enum.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Serialize)]
pub struct DeckSnapshot {
    pub idx: usize,
    pub uuid: String,
    pub name: String,
    /// True when this deck's source is an HTML/Servo instance.
    pub is_html: bool,
    /// True when the interactive window is currently open for this deck.
    pub is_html_interactive: bool,
    /// True when this deck's source is a depth sensor (point-cloud) source.
    pub is_depth_sensor: bool,
    /// Point-cloud controls (None = not a depth-sensor source).
    pub point_cloud_params: Option<PointCloudParamsSnapshot>,
    /// True when this deck has a `depth_sensor` shader preprocessor attached.
    pub has_depth_prepro: bool,
    /// Depth-preprocessor controls (None = no preprocessor attached).
    pub depth_prepro_params: Option<DepthPreproParamsSnapshot>,
    /// Screen-capture controls (None = not a screen-capture source).
    pub screen_capture: Option<ScreenCaptureDeckSnapshot>,
    /// Tap controls (None = not a tap source).
    pub tap: Option<TapDeckSnapshot>,
    pub opacity: f32,
    pub effective_opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    /// True when this deck preserves source alpha (transparent compositing).
    pub transparent: bool,
    pub scaling_mode: Option<ScalingMode>,
    pub generator: ShaderParamsSnapshot,
    pub effects: Vec<EffectSnapshot>,
    pub video_playback: Option<VideoPlaybackSnapshot>,
    pub auto_transition: Option<AutoTransitionSnapshot>,
    /// Configured render FPS (Auto or fixed value)
    pub render_fps: DeckRenderFps,
    /// Effective render rate (actual FPS this deck is rendering at)
    pub effective_render_fps: f32,
    /// Smoothed render cost in microseconds
    pub render_cost_us: f32,
    /// GPU-measured render cost in microseconds (0 = not available)
    pub gpu_render_cost_us: f32,
    /// Smoothed FPS from actual deck render pipeline timing
    pub fps: f32,
    /// True while the arrangement has this deck's source asleep because no
    /// region or curve will show it soon. A sleeping video holds its frame and
    /// resumes from there, which is why a frozen clip is worth reporting rather
    /// than leaving someone to wonder. See /spec/deck-residency.md.
    pub source_asleep: bool,
    pub running_analyzers: Vec<RunningAnalyzerSnapshot>,
}

/// Router-exposed `deck/<uuid>/depth/*` values, normalized to `0..1` so a
/// consumer can render faders without reaching into the engine.
/// See spec/depth-sensors.md.
#[derive(Clone, Serialize)]
pub struct PointCloudParamsSnapshot {
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

/// Router-exposed `deck/<uuid>/depth_prepro/*` values, normalized to `0..1` so a
/// consumer can render faders without reaching into the engine.
/// See spec/depth-sensor-preprocessor.md.
#[derive(Clone, Serialize)]
pub struct DepthPreproParamsSnapshot {
    /// Name of the sensor the preprocessor acquired.
    pub sensor_name: String,
    pub near: f32,
    pub far: f32,
    pub smoothing: f32,
    pub hole_fill: f32,
    pub mask_feather: f32,
    pub motion_gain: f32,
    /// Bucketed from the normalized `mirror` fader.
    pub mirror: bool,
}

#[derive(Clone, Serialize)]
pub struct EffectSnapshot {
    pub uuid: String,
    pub name: String,
    pub enabled: bool,
    pub params: ShaderParamsSnapshot,
}

#[derive(Clone, Serialize)]
pub struct ShaderParamsSnapshot {
    pub shader_name: String,
    pub params: Vec<ParamSnapshot>,
}

#[derive(Clone, Serialize)]
pub struct ParamSnapshot {
    pub name: String,
    pub label: Option<String>,
    pub value: ParamValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

#[derive(Clone, Serialize)]
pub struct VideoPlaybackSnapshot {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub loop_mode: LoopMode,
    pub in_point: f64,
    pub out_point: f64,
    pub frame_rate: f64,
    pub transport_sync: crate::engine::value::video::DeckTransportSync,
}

// Serialized DTO: each flag pairs with its own value field (beats vs seconds).
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
pub struct AudioDeviceSnapshot {
    pub id: AudioSourceId,
    pub name: String,
    pub active: bool,
}

// ── Modulation Snapshot ─────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ModulationSnapshot {
    pub sources: Vec<ModulationSourceSnapshotEntry>,
    pub current_values: std::collections::HashMap<String, f32>,
    pub assignments: std::collections::HashMap<String, Vec<ModulationAssignmentSnapshot>>,
}

#[derive(Clone, Serialize)]
pub struct ModulationSourceSnapshotEntry {
    pub uuid: String,
    pub source: ModulationSourceSnapshot,
    /// Which notion of time this source follows. See /spec/timebase.md.
    pub timebase: crate::timebase::Timebase,
}

#[derive(Clone, Serialize)]
pub enum ModulationSourceSnapshot {
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
    Envelope {
        breakpoints: Vec<crate::modulation::Breakpoint>,
    },
}

#[derive(Clone, Serialize)]
pub struct ModulationAssignmentSnapshot {
    pub source_id: String,
    pub amount: f32,
}

// ── Sequence Snapshot ───────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct SequenceSnapshot {
    pub uuid: String,
    pub name: String,
    pub enabled: bool,
    pub playing: bool,
    pub current_step: usize,
    pub step_elapsed: f64,
    pub steps: Vec<SequenceStepSnapshot>,
}

#[derive(Clone, Serialize)]
pub struct SequenceStepSnapshot {
    pub label: String,
    pub kind: SequenceStepKindSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub enum SequenceStepKindSnapshot {
    Fade {
        /// Source channel UUID. Resolve against the channel list for a name; a
        /// UUID that no longer resolves means the channel was deleted.
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

// ── Output Snapshot ─────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct OutputSnapshot {
    pub windows: Vec<OutputWindowSnapshot>,
    pub surfaces: Vec<SurfaceSnapshot>,
    pub monitors: Vec<MonitorSnapshot>,
}

#[derive(Clone, Serialize)]
pub struct OutputWindowSnapshot {
    pub uuid: String,
    pub name: String,
    /// Full output target (carries `audio_device` for ffmpeg-backed outputs).
    pub target: crate::engine::value::render::OutputTarget,
    pub target_label: String,
    pub is_on_display: bool,
    /// Whether a headless output is actively recording/streaming.
    pub is_active: bool,
    pub surface_assignments: Vec<SurfaceAssignmentSnapshot>,
    pub calibration_mode: crate::engine::value::render::CalibrationMode,
    /// Persisted precision and dithering request.
    pub presentation_request: crate::engine::value::render::PresentationRequest,
    /// Runtime format selected by the active output adapter.
    pub resolved_presentation: crate::engine::value::render::ResolvedPresentation,
    /// Live audio passthrough health for an active ffmpeg output (None = video-only).
    pub audio_passthrough: Option<AudioPassthroughSnapshot>,
}

#[derive(Clone, Serialize)]
pub struct AudioPassthroughSnapshot {
    /// Selected capture device name.
    pub device: String,
    /// PCM chunks written to ffmpeg so far.
    pub frames_written: u64,
    /// PCM chunks dropped on backpressure.
    pub frames_dropped: u64,
}

#[derive(Clone, Serialize)]
pub struct SurfaceAssignmentSnapshot {
    pub surface_uuid: String,
    pub surface_name: String,
    pub enabled: bool,
}

#[derive(Clone, Serialize)]
pub struct SurfaceSnapshot {
    pub uuid: String,
    pub name: String,
    pub vertices: Vec<[f32; 2]>,
    pub extra_contours: Vec<Vec<[f32; 2]>>,
    pub source: OutputSource,
    pub content_mapping: ContentMapping,
    pub output_type: SurfaceOutputType,
    pub circle_hint: Option<CircleHint>,
    /// Effective warp (auto-conforming to the shape while `warp_bound`).
    pub warp: Option<crate::engine::value::warp::WarpMode>,
    /// Whether the warp auto-conforms to the surface shape (auto-warp).
    pub warp_bound: bool,
    /// Curve authoring path, when the surface is bezier-edited.
    pub path: Option<SurfacePath>,
    /// Subtractive cut-out holes (8i.7).
    pub holes: Vec<SurfacePath>,
    /// Flattened hole contours (canvas coords), derived from `holes`.
    pub hole_contours: Vec<Vec<[f32; 2]>>,
}

#[derive(Clone, Serialize)]
pub struct MonitorSnapshot {
    pub name: String,
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

// ── MIDI Snapshot ───────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct MidiSnapshot {
    pub devices: Vec<MidiDeviceSnapshot>,
    pub mappings: Vec<MidiMappingSnapshot>,
    pub learn_active: bool,
    pub learn_target: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct MidiDeviceSnapshot {
    pub id: crate::midi::DeviceId,
    pub name: String,
    pub enabled: bool,
    pub has_output: bool,
    pub profile: String,
}

#[derive(Clone, Serialize)]
pub struct MidiMappingSnapshot {
    pub key: crate::midi::MidiKey,
    pub key_display: String,
    pub device_name: String,
    pub param_path: String,
}

// ── Camera Snapshot ─────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct CameraSnapshot {
    pub devices: Vec<(String, CameraId)>,
}

// ── Depth Sensor Snapshot ───────────────────────────────────────────

/// Per-sensor runtime state for GUI/API/WS consumers. Plain data — no GPU
/// types. See spec/depth-sensors.md.
#[derive(Clone, Serialize)]
pub struct DepthSensorSnapshot {
    /// Detected sensors as `(name, id)`, for the Library panel.
    pub devices: Vec<(String, DepthSensorId)>,
}

/// Per-deck screen-capture controls, for the deck detail panel and the API.
// The flags are independent facts about one capture, not a state machine:
// cursor and Varda-exclusion are user settings, bound and connected are two
// distinct failure modes a performer needs told apart.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct ScreenCaptureDeckSnapshot {
    /// The target this deck captures, in its persisted display form.
    pub target_label: String,
    /// Display targets get the `exclude_varda` toggle; window targets do not.
    pub is_display: bool,
    /// Capture rate as a 0–1 fraction of the 1–120 fps range, matching what
    /// `deck/<uuid>/capture/rate` accepts.
    pub rate_norm: f32,
    /// The same rate in fps, for display.
    pub rate_fps: f32,
    /// Normalized crop as `[x, y, w, h]`.
    pub crop: [f32; 4],
    pub show_cursor: bool,
    pub exclude_varda: bool,
    /// False when a restored scene named a target that is not currently on
    /// screen. The deck keeps its effects and mappings and renders black.
    pub bound: bool,
    /// Whether frames are currently arriving.
    pub connected: bool,
}

/// Per-deck tap controls, for the deck detail panel and the API.
/// See spec/program-tap.md.
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct TapDeckSnapshot {
    /// `"master_program"` or `"channel"`.
    pub kind: String,
    /// Channel UUID when `kind` is `"channel"`.
    pub channel_uuid: Option<String>,
    /// Resolved display name for the tap point.
    pub label: String,
    /// False when the tapped channel no longer exists. The deck keeps its
    /// effects and mappings and renders black.
    pub bound: bool,
}

// ── Screen Capture Snapshot ─────────────────────────────────────────

/// One capturable display or window, for the Library panel and the API.
/// Plain data — no platform handles, since the UI addresses targets by name.
/// See spec/screen-capture.md.
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct CaptureTargetSnapshot {
    /// `"display"` or `"window"`.
    pub kind: String,
    /// Human-readable name, e.g. `"Display 1"` or `"Ableton Live — Set 3"`.
    pub label: String,
    /// Owning application bundle id or process name. Windows only.
    pub app: Option<String>,
    /// Window title at enumeration time. Windows only.
    pub title: Option<String>,
    pub width: u32,
    pub height: u32,
    /// This target is one of Varda's own windows — capturing it is a deliberate
    /// self-capture, which the UI marks so it is an informed choice.
    pub is_varda: bool,
}

/// Screen-capture subsystem state for GUI/API/WS consumers.
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct ScreenCaptureSnapshot {
    /// Targets found by the last scan. Manual — never polled.
    pub targets: Vec<CaptureTargetSnapshot>,
    /// `granted` / `denied` / `not_determined` / `not_required`. This is why a
    /// capture deck can be black, so it is reported rather than inferred.
    pub permission: String,
    /// False when built without the `screen-capture` feature or started with
    /// `--no-screen-capture`.
    pub available: bool,
    /// Platform backend in use, e.g. `"ScreenCaptureKit"`.
    pub backend: String,
    /// Number of live capture sessions (one per target, shared by N decks).
    pub active_captures: usize,
}

impl Default for ScreenCaptureSnapshot {
    /// The state a build without a capture backend reports: nothing to scan,
    /// and no permission to ask for.
    fn default() -> Self {
        Self {
            targets: vec![],
            permission: "not_required".into(),
            available: false,
            backend: "none".into(),
            active_captures: 0,
        }
    }
}

// ── Analyzer Snapshot ──────────────────────────────────────────────

/// Info about an available analyzer type (for UI discovery).
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AnalyzerTypeInfo {
    pub analyzer_type: String,
    pub scalar_outputs: Vec<AnalyzerScalarInfo>,
    pub texture_outputs: Vec<String>,
}

/// Info about a scalar output an analyzer produces.
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AnalyzerScalarInfo {
    pub name: String,
    pub description: String,
    pub range: (f32, f32),
    pub default_smoothing: f32,
}

#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RunningAnalyzerSnapshot {
    pub analyzer_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── EffectTarget tests ───────────────────────────────────────────

    #[test]
    fn effect_target_deck_equality() {
        let a = EffectTarget::Deck("deck-a".into());
        let b = EffectTarget::Deck("deck-a".into());
        assert_eq!(a, b);
    }

    #[test]
    fn effect_target_deck_inequality() {
        assert_ne!(
            EffectTarget::Deck("deck-a".into()),
            EffectTarget::Deck("deck-b".into())
        );
        assert_ne!(
            EffectTarget::Deck("uuid-1".into()),
            EffectTarget::Channel("uuid-1".into())
        );
        assert_ne!(EffectTarget::Channel("ch-a".into()), EffectTarget::Master);
    }

    #[test]
    fn effect_target_debug() {
        assert!(format!("{:?}", EffectTarget::Master).contains("Master"));
        assert!(format!("{:?}", EffectTarget::Channel("ch-2".into())).contains("ch-2"));
        assert!(format!("{:?}", EffectTarget::Deck("deck-1".into())).contains("deck-1"));
    }

    #[test]
    fn effect_target_clone() {
        let original = EffectTarget::Deck("deck-5".into());
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn effect_target_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(EffectTarget::Master);
        set.insert(EffectTarget::Channel("ch-a".into()));
        set.insert(EffectTarget::Channel("ch-a".into())); // duplicate
        assert_eq!(set.len(), 2);
    }

    // ── Snapshot struct construction ─────────────────────────────────

    #[test]
    fn engine_state_can_be_constructed() {
        let state = EngineState {
            mixer: MixerSnapshot {
                channels: vec![],
                crossfader: 0.0,
                auto_crossfade_active: false,
                auto_crossfade_progress: 0.0,
                master_effects: vec![],
                active_transition_name: None,
                transition_names: vec![],
                sequences: vec![],
                tonemap_mode: crate::engine::value::render::TonemapMode::default(),
                active_lut: None,
            },
            audio: AudioSnapshot {
                level: 0.0,
                bass: 0.0,
                mid: 0.0,
                treble: 0.0,
                bpm: None,
                beat_phase: 0.0,
                enabled: false,
                devices: vec![],
                fft: vec![],
                sample_rate: 48000.0,
            },
            modulation: ModulationSnapshot {
                sources: vec![],
                current_values: std::collections::HashMap::default(),
                assignments: std::collections::HashMap::default(),
            },
            outputs: OutputSnapshot {
                windows: vec![],
                surfaces: vec![],
                monitors: vec![],
            },
            registry: RegistrySnapshot {
                generators: vec![],
                filters: vec![],
                shader_count: 0,
            },
            midi: MidiSnapshot {
                devices: vec![],
                mappings: vec![],
                learn_active: false,
                learn_target: None,
            },
            cameras: CameraSnapshot { devices: vec![] },
            depth_sensors: DepthSensorSnapshot { devices: vec![] },
            screen_capture: ScreenCaptureSnapshot::default(),
            transport: TransportSnapshot::default(),
            timecode: TimecodeSnapshot::default(),
            arrangement: None,
            clock: ClockSnapshot {
                bpm: None,
                beat_phase: 0.0,
                source_label: "None".into(),
                device_name: None,
                active: false,
                detected_midi_sources: vec![],
                osc_active: false,
                osc_bpm: None,
                audio_bpm: None,
                preference_label: "Auto".into(),
                preference_force_device_id: None,
                manual_bpm: None,
                beat_followers: 0,
            },
            fps: 60.0,
            frame_count: 0,
            target_fps: 60,
            ndi_sources: vec![],
            ndi_available: false,
            syphon_sources: vec![],
            syphon_available: false,
            stream_receivers: vec![],
            analyzers: vec![],
            can_undo: false,
            can_redo: false,
            macros: vec![],
        };
        assert!((state.fps - 60.0).abs() < 1e-5);
        assert_eq!(state.frame_count, 0);
    }

    #[test]
    fn engine_state_clone() {
        let state = EngineState {
            mixer: MixerSnapshot {
                channels: vec![],
                crossfader: 0.5,
                auto_crossfade_active: false,
                auto_crossfade_progress: 0.0,
                master_effects: vec![],
                active_transition_name: None,
                transition_names: vec![],
                sequences: vec![],
                tonemap_mode: crate::engine::value::render::TonemapMode::default(),
                active_lut: None,
            },
            audio: AudioSnapshot {
                level: 0.0,
                bass: 0.0,
                mid: 0.0,
                treble: 0.0,
                bpm: Some(120.0),
                beat_phase: 0.0,
                enabled: true,
                devices: vec![],
                fft: vec![],
                sample_rate: 48000.0,
            },
            modulation: ModulationSnapshot {
                sources: vec![],
                current_values: std::collections::HashMap::default(),
                assignments: std::collections::HashMap::default(),
            },
            outputs: OutputSnapshot {
                windows: vec![],
                surfaces: vec![],
                monitors: vec![],
            },
            registry: RegistrySnapshot {
                generators: vec![("Sine".into(), 0)],
                filters: vec![],
                shader_count: 1,
            },
            midi: MidiSnapshot {
                devices: vec![],
                mappings: vec![],
                learn_active: false,
                learn_target: None,
            },
            cameras: CameraSnapshot { devices: vec![] },
            depth_sensors: DepthSensorSnapshot { devices: vec![] },
            screen_capture: ScreenCaptureSnapshot::default(),
            transport: TransportSnapshot::default(),
            timecode: TimecodeSnapshot::default(),
            arrangement: None,
            clock: ClockSnapshot {
                bpm: Some(120.0),
                beat_phase: 0.0,
                source_label: "Audio".into(),
                device_name: None,
                active: true,
                detected_midi_sources: vec![],
                osc_active: false,
                osc_bpm: None,
                audio_bpm: Some(120.0),
                preference_label: "Auto".into(),
                preference_force_device_id: None,
                manual_bpm: None,
                beat_followers: 0,
            },
            fps: 59.9,
            frame_count: 42,
            target_fps: 60,
            ndi_sources: vec![],
            ndi_available: false,
            syphon_sources: vec![],
            syphon_available: false,
            stream_receivers: vec![],
            analyzers: vec![],
            can_undo: false,
            can_redo: false,
            macros: vec![],
        };
        let cloned = state.clone();
        assert!((cloned.mixer.crossfader - 0.5).abs() < 1e-5);
        assert_eq!(cloned.audio.bpm, Some(120.0));
        assert_eq!(cloned.registry.shader_count, 1);
        assert_eq!(cloned.frame_count, 42);
    }

    // ── EngineCommand construction ───────────────────────────────────

    #[test]
    fn engine_command_debug() {
        let cmd = crate::engine::EngineCommand::SetCrossfader(0.5);
        assert!(format!("{cmd:?}").contains("SetCrossfader"));
    }

    #[test]
    fn engine_command_add_deck() {
        let cmd = crate::engine::EngineCommand::AddDeck {
            channel_uuid: "ch-0".into(),
            shader_name: "Color Bars".into(),
        };
        match cmd {
            crate::engine::EngineCommand::AddDeck {
                channel_uuid,
                shader_name,
            } => {
                assert_eq!(channel_uuid, "ch-0");
                assert_eq!(shader_name, "Color Bars");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn engine_command_set_param() {
        let cmd = crate::engine::EngineCommand::SetParam {
            path: "ch0:deck0:brightness".into(),
            value: ParamValue::Float(0.8),
        };
        match cmd {
            crate::engine::EngineCommand::SetParam { path, value } => {
                assert_eq!(path, "ch0:deck0:brightness");
                match value {
                    ParamValue::Float(v) => assert!((v - 0.8).abs() < 1e-5),
                    _ => panic!("Expected Float"),
                }
            }
            _ => panic!("Wrong variant"),
        }
    }

    // ── Snapshot field access ────────────────────────────────────────

    #[test]
    fn channel_snapshot_fields() {
        let ch = ChannelSnapshot {
            idx: 0,
            uuid: "test0001".into(),
            name: "Ch 0".into(),
            opacity: 0.75,
            blend_mode: BlendMode::Add,
            decks: vec![],
            effects: vec![],
            render_time_ms: 1.5,
            active_deck_count: 2,
        };
        assert_eq!(ch.idx, 0);
        assert!((ch.opacity - 0.75).abs() < 1e-5);
        assert_eq!(ch.blend_mode, BlendMode::Add);
        assert!((ch.render_time_ms - 1.5).abs() < 1e-5);
        assert_eq!(ch.active_deck_count, 2);
    }

    #[test]
    fn deck_snapshot_fields() {
        let d = DeckSnapshot {
            idx: 0,
            uuid: "test0002".into(),
            name: "Sine Wave".into(),
            is_html: false,
            is_html_interactive: false,
            is_depth_sensor: true,
            point_cloud_params: Some(PointCloudParamsSnapshot {
                orbit_yaw: 0.5,
                orbit_pitch: 0.5,
                zoom: 0.25,
                point_size: 0.1,
                depth_min: 0.05,
                depth_max: 0.5,
                seed: 0.0,
                drift: 0.25,
                disruption: 0.75,
                color_mode: 1,
            }),
            has_depth_prepro: true,
            depth_prepro_params: Some(DepthPreproParamsSnapshot {
                sensor_name: "Kinect".into(),
                near: 0.0625,
                far: 0.5,
                smoothing: 0.5,
                hole_fill: 0.5,
                mask_feather: 0.375,
                motion_gain: 0.4,
                mirror: true,
            }),
            screen_capture: None,
            tap: None,
            opacity: 1.0,
            effective_opacity: 0.5,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: true,
            transparent: false,
            scaling_mode: Some(ScalingMode::default()),
            generator: ShaderParamsSnapshot {
                shader_name: "Sine".into(),
                params: vec![],
            },
            effects: vec![],
            video_playback: None,
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
            fps: 59.5,
            source_asleep: false,
            running_analyzers: vec![],
        };
        assert!(d.mute);
        assert!(!d.solo);
        assert!((d.effective_opacity - 0.5).abs() < 1e-5);
        assert!((d.fps - 59.5).abs() < 1e-5);
        let prepro = d.depth_prepro_params.expect("preprocessor params present");
        assert_eq!(prepro.sensor_name, "Kinect");
        assert!(prepro.mirror);
        assert!((prepro.far - 0.5).abs() < 1e-5);
        let pc = d.point_cloud_params.expect("point-cloud params present");
        assert_eq!(pc.color_mode, 1);
        assert!((pc.disruption - 0.75).abs() < 1e-5);
    }
}
