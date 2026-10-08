//! Shared value types for the engine layer.
//!
//! Used in engine trait signatures and snapshot structs. Must not reference
//! wgpu, egui, winit, or any GPU or UI framework type.
//!
//! Types come from `crate::engine::value` and from the framework-free domain
//! modules (`audio`, `camera`, `channel`, `deck`, `mixer`, `modulation`,
//! `params`), never from `internal::{renderer,surface,video}`.

use serde::{Deserialize, Serialize};

// Framework-free domain modules.
pub use crate::audio::AudioSourceId;
pub use crate::camera::CameraId;
pub use crate::channel::{BlendMode, DeckRenderFps};
pub use crate::depth::DepthSensorId;
pub use crate::mixer::CrossfadeEasing;
pub use crate::modulation::{
    ADSRStage, AudioBandPreset, AudioReactMode, LFOWaveform, StepInterpolation,
};
pub use crate::params::ParamValue;
pub use crate::source::ScalingMode;

// Engine value types.
pub use crate::engine::value::render::OutputSource;
pub use crate::engine::value::source::{DeckTransportSync, TransportSyncMode};
pub use crate::engine::value::surface::{
    CircleHint, ContentMapping, CubicHandle, SurfaceOutputType, SurfacePath, SurfaceReorderOp,
};
pub use crate::engine::value::video::LoopMode;

pub use crate::engine::value::entity::EffectTarget;

/// What a copy is taken from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub enum ClipboardSource {
    Deck(String),
    Channel(String),
    Effect(String),
}

/// Where a paste lands.
///
/// `After*` places the copy directly below the right-clicked item; `Into*`
/// appends to the container.
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

/// Per-frame engine state snapshot, built by `VardaApp` and sent to consumers
/// on a watch channel. The egui `UIData` is derived from it.
// Serialized DTO: the flags mirror independent engine toggles, not a state enum.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Serialize)]
pub struct EngineState {
    pub mixer: MixerSnapshot,
    /// Deck loads in flight, then loads that failed in the last minute.
    pub deck_loads: Vec<DeckLoadSnapshot>,
    /// Dome projection the domemaster is rendered for.
    pub dome: crate::engine::value::dome::DomeConfig,
    pub audio: AudioSnapshot,
    pub modulation: ModulationSnapshot,
    pub outputs: OutputSnapshot,
    pub registry: RegistrySnapshot,
    pub midi: MidiSnapshot,
    /// Cameras, for the stage editor's surface detection.
    pub cameras: CameraSnapshot,
    /// Every registered deck source type: its controls and library. Shared
    /// with the engine's cache, so cloning a snapshot does not copy it.
    pub sources: std::sync::Arc<Vec<crate::engine::value::provider::ProviderTypeSnapshot>>,
    /// Every registered output sink type: its settings and library.
    pub sinks: std::sync::Arc<Vec<crate::engine::value::provider::ProviderTypeSnapshot>>,
    pub clock: ClockSnapshot,
    pub transport: TransportSnapshot,
    pub timecode: TimecodeSnapshot,
    /// Present only when the scene has an arrangement.
    pub arrangement: Option<ArrangementSnapshot>,
    pub fps: f32,
    pub frame_count: u64,
    /// Target FPS (0 = uncapped)
    pub target_fps: u32,
    pub analyzers: Vec<AnalyzerTypeInfo>,
    /// User-defined macro controls, each driving many parameters.
    pub macros: Vec<crate::macros::Macro>,
    /// Whether there is an action to undo. The UI and API share one timeline.
    pub can_undo: bool,
    /// Whether there is an action to redo.
    pub can_redo: bool,
    pub keymap: KeymapSnapshot,
    pub presets: PresetsSnapshot,
    /// Notifications currently shown to the performer.
    pub notifications: Vec<NotificationSnapshot>,
    /// What the clipboard is holding, if anything.
    pub clipboard: Option<ClipboardSummary>,
    pub render: RenderSnapshot,
    pub system: SystemSnapshot,
}

/// Keyboard shortcuts and keyboard learn.
#[derive(Clone, Default, Serialize)]
pub struct KeymapSnapshot {
    /// Every binding, as a list because a key combination cannot be a JSON key.
    pub bindings: Vec<KeyBindingSnapshot>,
    pub learn_active: bool,
    /// What the next key pressed will be bound to, while learning.
    pub learn_target: Option<crate::engine::value::keymap::KeyTarget>,
}

#[derive(Clone, Serialize)]
pub struct KeyBindingSnapshot {
    pub combo: crate::engine::value::keymap::KeyCombo,
    pub target: crate::engine::value::keymap::KeyTarget,
}

/// Saved deck and channel presets, by name.
#[derive(Clone, Default, Serialize)]
pub struct PresetsSnapshot {
    pub deck: Vec<String>,
    pub channel: Vec<String>,
}

/// A notification shown to the performer.
#[derive(Clone, Serialize)]
pub struct NotificationSnapshot {
    /// Stable id to dismiss it by.
    pub id: u64,
    pub level: crate::engine::value::notification::NotificationLevel,
    pub message: String,
    /// How far through its display time it is, 0 to 1.
    pub progress: f32,
}

/// What the engine renders at.
#[derive(Clone, Default, Serialize)]
pub struct RenderSnapshot {
    pub width: u32,
    pub height: u32,
    /// The largest render dimension the GPU adapter allows.
    pub max_dimension: u32,
    pub domemaster_resolution: crate::engine::value::dome::DomemasterResolution,
}

/// Load on the machine running the engine, and the GPU it runs on.
#[derive(Clone, Default, Serialize)]
pub struct SystemSnapshot {
    /// Process CPU use, percent.
    pub cpu_usage: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    /// Share of the frame the GPU spent rendering, 0 to 1.
    pub gpu_utilization: f32,
    pub gpu: GpuInfoSnapshot,
}

/// The GPU adapter, fixed for the session.
#[derive(Clone, Default, Serialize)]
pub struct GpuInfoSnapshot {
    pub name: String,
    pub backend: String,
    pub driver: String,
    pub driver_info: String,
    pub device_type: String,
}

// ── Clock Snapshot ──────────────────────────────────────────────

/// A detected MIDI clock source for UI display.
#[derive(Clone, Debug, Serialize)]
pub struct DetectedClockSourceSnapshot {
    pub device_id: crate::engine::value::midi::DeviceId,
    pub device_name: String,
    pub bpm: Option<f32>,
}

/// Snapshot of the unified clock state for UI display.
#[derive(Clone, Serialize)]
pub struct ClockSnapshot {
    /// BPM from the active clock source.
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
    /// Audio BPM, always available as a fallback.
    pub audio_bpm: Option<f32>,
    /// Current preference label: "Auto", "`ForceMidi`(<name>)", "`ForceOsc`", "`ForceAudio`", "`ForceManual`".
    pub preference_label: String,
    /// Device ID if preference is `ForceMidi`.
    pub preference_force_device_id: Option<crate::engine::value::midi::DeviceId>,
    /// Manual BPM value (if preference is `ForceManual`).
    pub manual_bpm: Option<f32>,
    /// How many modulation sources are locked to the beat.
    pub beat_followers: usize,
}

/// Snapshot of arrangement mode. Absent when the scene has no arrangement.
#[derive(Clone, Serialize, Default)]
pub struct ArrangementSnapshot {
    /// The authored lanes and idle behavior.
    pub config: crate::arrangement::ArrangementConfig,
    /// Whether the arrangement is driving decks. False until the transport
    /// first runs.
    pub engaged: bool,
    /// Modulation keys the performer has overridden by hand.
    pub overridden_params: Vec<String>,
    /// Latest position covered by any region, for the ruler's default extent.
    pub duration: f64,
}

/// Snapshot of the absolute show position. Tempo is in [`ClockSnapshot`].
#[derive(Clone, Serialize)]
pub struct TransportSnapshot {
    /// Absolute position in seconds. `f64` because shows often start at hour 1.
    pub position: f64,
    pub running: bool,
    /// Whether the transport has advanced this session. Position-locked
    /// features stay inactive until it has.
    pub has_run: bool,
    pub source: crate::transport::TransportSource,
    /// Why the transport is or is not moving.
    pub status_label: String,
    pub loop_region: Option<crate::transport::LoopRegion>,
    /// Frame rate positions are displayed at.
    pub timecode_rate: crate::transport::TimecodeRate,
    /// Position as `HH:MM:SS:FF`, drop-frame applied.
    pub timecode: String,
    /// How many modulation sources are locked to the transport.
    pub followers: usize,
    /// Whether automation recording is armed. Nothing is written until the
    /// position moves.
    pub record_armed: bool,
    /// Parameter keys currently being recorded. Filled in by the snapshot
    /// builder.
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
    /// Leaves `followers` at zero; the snapshot builder fills it from the
    /// modulation engine.
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

/// One timecode input, resolved or not.
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct TimecodeInputSnapshot {
    /// Stable name, `ltc` or `mtc:<device>`, and what `resolved` names.
    pub key: String,
    /// For a readout: "LTC (channel 2)", "MTC (Tascam Model 12)".
    pub label: String,
    pub position: f64,
    /// Position as `HH:MM:SS:FF` at this input's own rate.
    pub timecode: String,
    pub rate: crate::transport::TimecodeRate,
    pub running: bool,
    /// Coasting through a dropout rather than reading frames.
    pub freewheeling: bool,
    /// Measured against wall time; 1.0 while a master plays forwards.
    pub speed: f64,
}

/// A deck being built in the background, or one that failed to build.
///
/// Deck-creating commands return the new deck's UUID immediately; the deck
/// joins its channel once built.
#[derive(Clone, Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub struct DeckLoadSnapshot {
    /// The UUID the deck has, or would have had.
    pub uuid: String,
    pub channel_uuid: String,
    /// Shader name or media file name.
    pub name: String,
    pub status: DeckLoadStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, utoipa::ToSchema)]
pub enum DeckLoadStatus {
    Loading,
    Failed { message: String },
}

/// Every timecode input being listened to, and which one drives the
/// transport. Unresolved inputs are listed too.
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
    /// Scene-referred look LUT filename, applied before every output transform.
    pub look_lut: Option<String>,
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
    /// The deck's source: its type, and the state of its controls. The type's
    /// schema is in `EngineState::sources`.
    pub source: crate::engine::value::source::DeckSourceSnapshot,
    /// True when this deck's interactive window is open.
    pub is_interactive: bool,
    /// True when this deck has a `depth_sensor` shader preprocessor attached.
    pub has_depth_prepro: bool,
    /// Depth-preprocessor controls (None = no preprocessor attached).
    pub depth_prepro_params: Option<DepthPreproParamsSnapshot>,
    pub opacity: f32,
    pub effective_opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    /// True when this deck preserves source alpha (transparent compositing).
    pub transparent: bool,
    pub generator: ShaderParamsSnapshot,
    pub effects: Vec<EffectSnapshot>,
    pub auto_transition: Option<AutoTransitionSnapshot>,
    /// Configured render FPS (Auto or fixed value)
    pub render_fps: DeckRenderFps,
    /// Actual render FPS.
    pub effective_render_fps: f32,
    /// Smoothed render cost in microseconds
    pub render_cost_us: f32,
    /// GPU-measured render cost in microseconds (0 = not available)
    pub gpu_render_cost_us: f32,
    /// Smoothed FPS from actual deck render pipeline timing
    pub fps: f32,
    /// True while the arrangement has put this deck's source to sleep because
    /// nothing will show it soon. A sleeping video holds its frame.
    pub source_asleep: bool,
    /// A source swap is building; the deck draws its current source until it is ready.
    pub source_pending: bool,
    pub running_analyzers: Vec<RunningAnalyzerSnapshot>,
}

/// `deck/<uuid>/depth_prepro/*` values, normalized to `0..1`.
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
    /// Whether the effect's shader is built; a building effect passes the picture through.
    pub status: crate::engine::value::effect::EffectStatus,
    pub params: ShaderParamsSnapshot,
}

#[derive(Clone, Serialize)]
pub struct ShaderParamsSnapshot {
    pub shader_name: String,
    pub params: Vec<ParamSnapshot>,
    /// The shader's `COLUMNS`: groups laid out as columns of their own.
    pub columns: Vec<ParamColumnSnapshot>,
}

/// One column of a generator's controls: a title and the groups it holds.
#[derive(Clone, Serialize)]
pub struct ParamColumnSnapshot {
    pub title: String,
    pub groups: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct ParamSnapshot {
    pub name: String,
    pub label: Option<String>,
    pub value: ParamValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// Inspector section, from the shader's `GROUP` key. `None` groups with the
    /// other ungrouped params.
    pub group: Option<String>,
    /// Selectable values for a `long` input, paired with their labels.
    pub choices: Option<Vec<ParamChoice>>,
    /// An `event` input: setting it true fires it for one rendered frame.
    pub event: bool,
}

/// One option of a `long` (enum) parameter.
#[derive(Clone, Serialize)]
pub struct ParamChoice {
    pub value: i32,
    pub label: String,
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
    /// Which timebase this source follows.
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
        /// Source channel UUID. One that does not resolve names a deleted
        /// channel.
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

/// An output's sink in a snapshot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OutputSinkSnapshot {
    /// Sink type id; its schema is in [`EngineState::sinks`].
    #[serde(rename = "type")]
    pub type_id: String,
    /// What the sink is sending to, for display.
    pub label: String,
    /// False when this run cannot drive the type; the output is then a
    /// placeholder holding its settings.
    pub available: bool,
    /// Whether the sink is started and stopped rather than always showing.
    pub startable: bool,
    pub status: crate::engine::value::provider::ControlStatus,
}

#[derive(Clone, Serialize)]
pub struct OutputWindowSnapshot {
    pub uuid: String,
    pub name: String,
    /// Where the output delivers: its sink type, settings and state.
    pub sink: OutputSinkSnapshot,
    /// Whether a startable sink is delivering. Always false for a window.
    pub is_active: bool,
    /// What the output shows with no surfaces assigned: the user's choice, or
    /// the sink's default.
    pub unassigned: crate::engine::value::render::Unassigned,
    pub surface_assignments: Vec<SurfaceAssignmentSnapshot>,
    pub calibration_mode: crate::engine::value::render::CalibrationMode,
    /// Persisted precision and dithering request.
    pub presentation_request: crate::engine::value::render::PresentationRequest,
    /// Runtime format selected by the active output adapter.
    pub resolved_presentation: crate::engine::value::render::ResolvedPresentation,
    /// Every presentation mode, with the reason this output cannot deliver it.
    /// A picker should disable blocked modes and show the reason.
    pub mode_availability: Vec<crate::engine::value::render::ModeAvailability>,
    /// Per-output tonemap override. `None` inherits the mixer's show-wide curve.
    pub tonemap_override: Option<crate::engine::value::render::TonemapMode>,
    /// Live audio passthrough health for an active ffmpeg output (None = video-only).
    pub audio_passthrough: Option<AudioPassthroughSnapshot>,
    /// Live ffmpeg video health (None = this output has no subprocess).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryHealthSnapshot>,
    pub edge_blend_mode: crate::engine::value::render::EdgeBlendMode,
    pub edge_blend: crate::engine::value::render::EdgeBlendConfig,
    pub rotation: crate::engine::value::render::OutputRotation,
    /// Seconds a headless output has been sending; zero for a window.
    pub active_seconds: f64,
    /// Resolution the output renders at.
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Serialize)]
pub struct AudioPassthroughSnapshot {
    /// Selected capture device name.
    pub device: String,
    /// PCM chunks written to ffmpeg so far.
    pub frames_written: u64,
    /// PCM chunks dropped on backpressure.
    pub frames_dropped: u64,
    /// Silence spliced in to cover gaps in the input.
    pub silence_spliced: u64,
}

/// Video frames offered to an ffmpeg subprocess, split by fate.
#[derive(Clone, Serialize)]
pub struct DeliveryHealthSnapshot {
    /// Frames written to the pipe.
    pub frames_written: u64,
    /// Frames dropped because the writer channel was full.
    pub frames_dropped: u64,
    /// Repeated frames inserted to cover renderer gaps.
    pub frames_padded: u64,
}

#[derive(Clone, Serialize)]
pub struct SurfaceAssignmentSnapshot {
    pub surface_uuid: String,
    pub surface_name: String,
    pub enabled: bool,
    /// Where this surface overlaps one on another output, for auto blending.
    pub overlap_zones: crate::engine::value::render::SurfaceOverlapZones,
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
    /// Effective warp; follows the shape while `warp_bound`.
    pub warp: Option<crate::engine::value::warp::WarpMode>,
    /// Whether the warp follows the surface shape.
    pub warp_bound: bool,
    /// Curve authoring path, when the surface is bezier-edited.
    pub path: Option<SurfacePath>,
    /// Cut-out holes.
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
    pub id: crate::engine::value::midi::DeviceId,
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
            deck_loads: Vec::new(),
            dome: crate::engine::value::dome::DomeConfig::default(),
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
                look_lut: None,
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
            sources: std::sync::Arc::default(),
            sinks: std::sync::Arc::default(),
            analyzers: vec![],
            can_undo: false,
            can_redo: false,
            keymap: KeymapSnapshot::default(),
            presets: PresetsSnapshot::default(),
            notifications: Vec::new(),
            clipboard: None,
            render: RenderSnapshot::default(),
            system: SystemSnapshot::default(),
            macros: vec![],
        };
        assert!((state.fps - 60.0).abs() < 1e-5);
        assert_eq!(state.frame_count, 0);
    }

    #[test]
    fn engine_state_clone() {
        let state = EngineState {
            deck_loads: Vec::new(),
            dome: crate::engine::value::dome::DomeConfig::default(),
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
                look_lut: None,
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
            sources: std::sync::Arc::default(),
            sinks: std::sync::Arc::default(),
            analyzers: vec![],
            can_undo: false,
            can_redo: false,
            keymap: KeymapSnapshot::default(),
            presets: PresetsSnapshot::default(),
            notifications: Vec::new(),
            clipboard: None,
            render: RenderSnapshot::default(),
            system: SystemSnapshot::default(),
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
            source: crate::engine::value::source::SourceConfig::new("Shader")
                .with("name", "Color Bars"),
        };
        match cmd {
            crate::engine::EngineCommand::AddDeck {
                channel_uuid,
                source,
            } => {
                assert_eq!(channel_uuid, "ch-0");
                assert_eq!(source.str("name"), Some("Color Bars"));
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
            source: crate::engine::value::source::DeckSourceSnapshot {
                source_type: "DepthSensor".into(),
                available: true,
                owns_alpha: false,
                status: crate::engine::value::provider::ControlStatus::default(),
            },
            is_interactive: false,
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
            opacity: 1.0,
            effective_opacity: 0.5,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: true,
            transparent: false,
            generator: ShaderParamsSnapshot {
                shader_name: "Sine".into(),
                params: vec![],
                columns: vec![],
            },
            effects: vec![],
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
            fps: 59.5,
            source_asleep: false,
            source_pending: false,
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
        assert_eq!(d.source.source_type, "DepthSensor");
    }
}
