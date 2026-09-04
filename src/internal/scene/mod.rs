//! Scene configuration — serializable snapshot of the full VJ performance state.
//!
//! This is the data model for `.varda/scene.json`. It captures everything needed
//! to reconstruct a show: channels, decks, effects, modulation.
//! Surfaces and outputs live in `stage.json` (venue-specific, not show-specific).

use crate::channel::{BlendMode, DeckRenderFps};
use crate::macros::MacroBank;
use crate::modulation::ModulationEngine;
use crate::params::ParamValue;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub mod reidentify;

// ── Scene (top-level) ──────────────────────────────────────────────

/// Full scene configuration — the root of `.varda/scene.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfig {
    /// File format version (for future migrations)
    #[serde(default = "default_version")]
    pub version: u32,

    /// Channel configurations (ordered)
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,

    /// Crossfader position (0.0 = Ch 0, 1.0 = Ch 1)
    #[serde(default)]
    pub crossfader: f32,

    /// Active transition shader name (None = opacity crossfade)
    #[serde(default)]
    pub active_transition: Option<String>,

    /// Master effect chain
    #[serde(default)]
    pub master_effects: Vec<EffectConfig>,

    /// Modulation engine state (sources + assignments, already Serialize/Deserialize)
    #[serde(default)]
    pub modulation: ModulationEngine,

    /// Macro controls (user-defined knobs/faders/buttons → many parameter targets).
    /// Additive since scene v4; pre-macro scenes default to an empty bank.
    #[serde(default)]
    pub macros: MacroBank,

    /// Transition sequences (channel-to-channel automation). Multiple named sequences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transition_sequences: Vec<TransitionSequenceConfig>,

    /// Master render width (defaults to 1920 if absent in old files)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_width: Option<u32>,

    /// Master render height (defaults to 1080 if absent in old files)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_height: Option<u32>,

    /// Tonemap mode (defaults to ACES if absent)
    #[serde(default)]
    pub tonemap_mode: crate::renderer::tonemap::TonemapMode,

    /// Active LUT filename (relative to `.varda/luts/`), if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_lut: Option<String>,

    /// Arrangement mode data. Absent in scenes authored in Performance mode
    /// only. Additive since scene v7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<crate::arrangement::ArrangementConfig>,

    /// How this show counts frames and where it loops. Held with the scene
    /// rather than inside `arrangement`, because the position readout needs a
    /// rate before any arrangement exists.
    #[serde(default)]
    pub transport: TransportConfig,
}

/// The persisted half of the transport. Position and run state are deliberately
/// absent: a scene should open where it was authored to start, not wherever it
/// happened to be stopped when it was saved.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(default)]
    pub timecode_rate: crate::transport::TimecodeRate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_region: Option<crate::transport::LoopRegion>,
}

fn default_version() -> u32 {
    3
}

impl SceneConfig {
    /// Version written by this build. Bump when adding a migration below.
    pub const CURRENT_VERSION: u32 = 7;

    /// Bring an older scene up to [`Self::CURRENT_VERSION`] in place.
    ///
    /// Runs on load, before validation. Each step is guarded by the version it
    /// upgrades *from*, so a scene several versions behind walks through them in
    /// order.
    ///
    /// v6 → v7 adds the arrangement and the persisted transport settings. Both
    /// are optional and serde-defaulted, so there is no transformation step:
    /// a v6 scene loads as a Performance-only show, which is exactly what it is.
    pub fn migrate(&mut self) {
        if self.version < 6 {
            self.migrate_v5_bipolar_amplitude();
        }
        self.version = Self::CURRENT_VERSION;
    }

    /// v5 → v6: bipolar sources stopped double-sweeping their target's range.
    ///
    /// Before v6 a bipolar source's -1..1 output was scaled by the *whole*
    /// parameter range, giving twice the excursion a fader can hold: the value
    /// hung against both ends and rushed through the middle. Bipolar
    /// contributions now carry a 0.5 weight, so a full-amplitude bipolar LFO
    /// sweeps the range exactly, centred on the base value.
    ///
    /// That halves the excursion of existing patches. An LFO can compensate —
    /// amplitude 0.5 was the only setting that did *not* clip before, and
    /// doubling it reproduces the old motion exactly. Anything above 0.5 was
    /// clipping regardless; clamping it to full amplitude gives the whole fader,
    /// which is the closest thing to what the patch asked for.
    ///
    /// Step sequencers have no amplitude control — their steps already span the
    /// full output range — so they cannot be compensated. They were clipping
    /// before and are simply correct now.
    fn migrate_v5_bipolar_amplitude(&mut self) {
        let mut rescaled = 0;
        for entry in &mut self.modulation.sources {
            if let crate::modulation::ModulationSource::LFO {
                amplitude,
                bipolar: true,
                ..
            } = &mut entry.source
            {
                *amplitude = (*amplitude * 2.0).min(1.0);
                rescaled += 1;
            }
        }
        if rescaled > 0 {
            log::info!(
                "Scene migration v5→v6: rescaled amplitude on {rescaled} bipolar LFO(s) \
                 so they keep their existing sweep depth"
            );
        }
    }
}

// ── Channel ────────────────────────────────────────────────────────

/// Serializable channel state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,

    pub name: String,

    #[serde(default = "default_opacity")]
    pub opacity: f32,

    #[serde(default)]
    pub blend_mode: BlendModeConfig,

    #[serde(default)]
    pub decks: Vec<DeckConfig>,

    #[serde(default)]
    pub effects: Vec<EffectConfig>,

    /// Modulation on this channel's own effects, in the portable recipe form.
    /// Empty in `scene.json`, where the modulation engine is serialized whole;
    /// filled when a channel travels on its own, as a preset or on the
    /// clipboard, since its effects' assignments would otherwise be left behind.
    /// See /spec/clipboard.md.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modulation: Vec<ModulationRecipe>,
}

fn default_opacity() -> f32 {
    1.0
}
fn default_video_speed() -> f64 {
    1.0
}

// ── Deck ───────────────────────────────────────────────────────────

fn generate_default_uuid() -> String {
    crate::deck::generate_short_uuid()
}

/// Serializable deck state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,

    /// Display name
    #[serde(default)]
    pub name: String,

    /// Source configuration
    pub source: SourceConfig,

    /// Effect chain
    #[serde(default)]
    pub effects: Vec<EffectConfig>,

    /// Deck opacity (0.0 - 1.0)
    #[serde(default = "default_opacity")]
    pub opacity: f32,

    /// Transparent compositing: preserve source alpha instead of flattening over
    /// black. Defaults to false for backward compatibility with existing scenes.
    #[serde(default)]
    pub transparent: bool,

    /// Blend mode for compositing
    #[serde(default)]
    pub blend_mode: BlendModeConfig,

    /// Mute state
    #[serde(default)]
    pub mute: bool,

    /// Solo state
    #[serde(default)]
    pub solo: bool,

    /// Z-index for layer ordering
    #[serde(default)]
    pub z_index: i32,

    /// Per-deck render FPS cap (default: auto adaptive)
    #[serde(default)]
    pub render_fps: DeckRenderFps,

    /// Auto-transition configuration (None = no auto-transition)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_transition: Option<AutoTransitionConfig>,

    /// Modulation recipes (for preset portability)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modulation: Vec<ModulationRecipe>,
}

/// A modulation recipe stored in a preset.
/// Contains a source definition and which params it targets (relative keys).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationRecipe {
    /// UUID of the modulation source
    #[serde(default = "crate::deck::generate_short_uuid")]
    pub source_uuid: String,
    /// The modulation source definition
    pub source: crate::modulation::ModulationSource,
    /// Which clock the source follows. It lives on the engine's entry rather
    /// than inside the source, so a recipe that omitted it restored an
    /// arrangement curve as free-running. See /spec/timebase.md.
    #[serde(default)]
    pub timebase: crate::timebase::Timebase,
    /// Assignments using relative param keys (no ch/deck prefix)
    pub assignments: Vec<ModulationRecipeAssignment>,
}

/// A single assignment within a modulation recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationRecipeAssignment {
    /// Relative param key: "brightness" for generator, "fx0:amount" for effects
    pub param: String,
    /// Modulation amount
    pub amount: f32,
    /// Component index for multi-component params (e.g., color channels)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<usize>,
}

// ── Auto-Transition ────────────────────────────────────────────────

/// Serializable auto-transition config for a deck.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTransitionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_timer_trigger")]
    pub trigger: TriggerConfig,

    pub play_duration: DurationSpecConfig,
    pub transition_duration: DurationSpecConfig,

    /// Transition shader name (None = opacity fade)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_shader: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value")]
pub enum DurationSpecConfig {
    #[serde(rename = "beats")]
    Beats(f64),
    #[serde(rename = "seconds")]
    Seconds(f64),
    #[serde(rename = "minutes")]
    Minutes(f64),
    #[serde(rename = "hours")]
    Hours(f64),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerConfig {
    Timer,
    ClipEnd,
}

fn default_timer_trigger() -> TriggerConfig {
    TriggerConfig::Timer
}

// ── Transition Sequence ──────────────────────────────────────────────

/// Serializable transition sequence (channel-to-channel automation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionSequenceConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,

    #[serde(default = "default_sequence_name")]
    pub name: String,

    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default)]
    pub steps: Vec<TransitionStepConfig>,
}

fn default_sequence_name() -> String {
    "Sequence 1".to_string()
}

/// How a fade step names a channel. Scenes at v5 and later always write a UUID;
/// `Index` exists only to read v4-and-earlier scenes, where fade steps stored a
/// positional channel index. `resolve` turns either form into a UUID.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelRef {
    Uuid(String),
    Index(usize),
}

impl ChannelRef {
    /// Resolve to a channel UUID, using `channel_uuids` (scene channel order) to
    /// interpret a legacy index. Returns `None` if the index is out of range.
    pub fn resolve(&self, channel_uuids: &[String]) -> Option<String> {
        match self {
            ChannelRef::Uuid(uuid) => Some(uuid.clone()),
            ChannelRef::Index(idx) => channel_uuids.get(*idx).cloned(),
        }
    }
}

impl From<String> for ChannelRef {
    fn from(uuid: String) -> Self {
        ChannelRef::Uuid(uuid)
    }
}

/// A single step in a transition sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TransitionStepConfig {
    Fade {
        from_ch: ChannelRef,
        to_ch: ChannelRef,
        duration: DurationSpecConfig,
        #[serde(default = "default_easing")]
        easing: EasingConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition_shader: Option<String>,
        #[serde(default = "default_target_amount")]
        target_amount: f32,
    },
    Wait {
        duration: DurationSpecConfig,
    },
    GoTo {
        step_index: usize,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EasingConfig {
    Linear,
    EaseInOut,
    EaseIn,
    EaseOut,
}

fn default_easing() -> EasingConfig {
    EasingConfig::EaseInOut
}
fn default_target_amount() -> f32 {
    1.0
}

impl From<crate::mixer::CrossfadeEasing> for EasingConfig {
    fn from(e: crate::mixer::CrossfadeEasing) -> Self {
        match e {
            crate::mixer::CrossfadeEasing::Linear => EasingConfig::Linear,
            crate::mixer::CrossfadeEasing::EaseInOut => EasingConfig::EaseInOut,
            crate::mixer::CrossfadeEasing::EaseIn => EasingConfig::EaseIn,
            crate::mixer::CrossfadeEasing::EaseOut => EasingConfig::EaseOut,
        }
    }
}

impl From<EasingConfig> for crate::mixer::CrossfadeEasing {
    fn from(e: EasingConfig) -> Self {
        match e {
            EasingConfig::Linear => crate::mixer::CrossfadeEasing::Linear,
            EasingConfig::EaseInOut => crate::mixer::CrossfadeEasing::EaseInOut,
            EasingConfig::EaseIn => crate::mixer::CrossfadeEasing::EaseIn,
            EasingConfig::EaseOut => crate::mixer::CrossfadeEasing::EaseOut,
        }
    }
}

// ── Source ──────────────────────────────────────────────────────────

/// What generates the base image for a deck.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    /// ISF shader generator
    Shader {
        path: String,
        #[serde(default)]
        params: HashMap<String, ParamValue>,
        /// Depth-sensor preprocessor binding, present only when the shader
        /// declares a `depth_sensor` PREPROCESSOR. Absent on every legacy scene
        /// and on every shader that does not use one, so `.varda/` directories
        /// written by earlier builds are unaffected.
        /// See spec/depth-sensor-preprocessor.md § Persistence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth_prepro: Option<DepthPreproConfig>,
    },
    /// Video file (ffmpeg or HAP)
    Video {
        path: String,
        /// Loop mode (default: Loop)
        #[serde(default)]
        loop_mode: crate::video::LoopMode,
        /// Playback speed multiplier (default: 1.0)
        #[serde(default = "default_video_speed")]
        speed: f64,
        /// In-point in seconds (default: 0.0 = start)
        #[serde(default)]
        in_point: f64,
        /// Out-point in seconds (default: 0.0 = end of file)
        #[serde(default)]
        out_point: f64,
        /// How the video is scaled to the deck (default: Fill)
        #[serde(default)]
        scaling_mode: crate::deck::ScalingMode,
        /// Mapping onto the show transport. Default Auto: chase while the
        /// transport is running. See /spec/timecode.md § Consumer 2.
        #[serde(default)]
        transport_sync: crate::video::DeckTransportSync,
    },
    /// Static image
    Image {
        path: String,
        /// How the image is scaled to the deck (default: Fill)
        #[serde(default)]
        scaling_mode: crate::deck::ScalingMode,
    },
    /// Solid color fill
    SolidColor { color: [f32; 4] },
    /// Live camera feed (matched by name on restore)
    Camera { name: String },
    /// NDI network video source (matched by name on restore)
    Ndi { name: String },
    /// Syphon inter-app video source (matched by server name on restore, macOS only)
    Syphon { name: String },
    /// SRT network video source (url + mode, reconnected on restore)
    Srt { url: String, mode: String },
    /// HLS stream source (reconnected on restore)
    Hls { url: String },
    /// DASH stream source (reconnected on restore)
    Dash { url: String },
    /// RTMP stream source (reconnected on restore)
    Rtmp { url: String, mode: String },
    /// HTML content source (URL or file path, rendered via Servo)
    Html { url: String },
    /// Depth sensor (Kinect/LIDAR point cloud, matched by name on restore)
    DepthSensor {
        name: String,
        /// Point-cloud view params (None on legacy scenes → engine defaults)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<DepthParamsConfig>,
    },
    /// OS display or application window capture. The target is matched by
    /// **name** on restore, never by platform handle — display ids and window
    /// numbers are ephemeral across reboots. See spec/screen-capture.md.
    ScreenCapture {
        target: CaptureTargetConfig,
        /// Capture frames per second (1–120).
        #[serde(default = "default_capture_rate")]
        rate: f32,
        /// Normalized crop within the target. Absent means the full frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        crop: Option<CaptureCropConfig>,
        #[serde(default)]
        show_cursor: bool,
        /// `None` means "use the per-target default": exclude Varda from a
        /// display capture, include it when the target *is* a Varda window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude_varda: Option<bool>,
        #[serde(default)]
        scaling_mode: crate::deck::ScalingMode,
    },
    /// Varda's own program or a channel composite, re-entered as a source.
    /// See spec/program-tap.md.
    Tap {
        source: TapSourceConfig,
        #[serde(default)]
        scaling_mode: crate::deck::ScalingMode,
    },
}

/// The tap point a scene records. Channels are referenced by UUID so a tap
/// survives reordering, which already carries semantic weight in the mixer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TapSourceConfig {
    MasterProgram,
    Channel { uuid: String },
}

impl From<&crate::deck::TapSource> for TapSourceConfig {
    fn from(source: &crate::deck::TapSource) -> Self {
        match source {
            crate::deck::TapSource::MasterProgram => Self::MasterProgram,
            crate::deck::TapSource::Channel(uuid) => Self::Channel { uuid: uuid.clone() },
        }
    }
}

impl From<&TapSourceConfig> for crate::deck::TapSource {
    fn from(cfg: &TapSourceConfig) -> Self {
        match cfg {
            TapSourceConfig::MasterProgram => Self::MasterProgram,
            TapSourceConfig::Channel { uuid } => Self::Channel(uuid.clone()),
        }
    }
}

fn default_capture_rate() -> f32 {
    crate::screen_capture::backend::DEFAULT_CAPTURE_RATE
}

/// A capture target in handle-free form, so a scene survives a reboot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTargetConfig {
    Display { name: String },
    Window { app: String, title: String },
}

impl CaptureTargetConfig {
    pub fn label(&self) -> String {
        match self {
            Self::Display { name } => name.clone(),
            Self::Window { app, title } if title.is_empty() => app.clone(),
            Self::Window { app, title } => format!("{app} — {title}"),
        }
    }

    /// Whether this target is a display, which is what decides the
    /// `exclude_varda` default when the scene did not record one.
    pub fn is_display(&self) -> bool {
        matches!(self, Self::Display { .. })
    }
}

impl From<&crate::screen_capture::backend::TargetIdentity> for CaptureTargetConfig {
    fn from(id: &crate::screen_capture::backend::TargetIdentity) -> Self {
        use crate::screen_capture::backend::TargetIdentity;
        match id {
            TargetIdentity::Display { label } => Self::Display {
                name: label.clone(),
            },
            TargetIdentity::Window { app, title } => Self::Window {
                app: app.clone(),
                title: title.clone(),
            },
        }
    }
}

impl From<&CaptureTargetConfig> for crate::screen_capture::backend::TargetIdentity {
    fn from(cfg: &CaptureTargetConfig) -> Self {
        match cfg {
            CaptureTargetConfig::Display { name } => Self::Display {
                label: name.clone(),
            },
            CaptureTargetConfig::Window { app, title } => Self::Window {
                app: app.clone(),
                title: title.clone(),
            },
        }
    }
}

/// Normalized crop rectangle (0.0–1.0) within a capture target.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CaptureCropConfig {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl From<crate::screen_capture::backend::CropRect> for CaptureCropConfig {
    fn from(c: crate::screen_capture::backend::CropRect) -> Self {
        Self {
            x: c.x,
            y: c.y,
            w: c.w,
            h: c.h,
        }
    }
}

impl From<CaptureCropConfig> for crate::screen_capture::backend::CropRect {
    fn from(c: CaptureCropConfig) -> Self {
        Self {
            x: c.x,
            y: c.y,
            w: c.w,
            h: c.h,
        }
    }
}

/// Serializable depth-sensor preprocessor binding for shader decks.
///
/// The sensor is matched by **name** on restore, matching the convention used by
/// cameras and depth-sensor decks — device ids are not stable across replugs.
/// If no matching sensor is present the deck is skipped with a warning, because
/// `depth_sensor` is a required preprocessor.
///
/// Params are stored denormalized (physical units), matching the runtime
/// `DepthPreprocessParams`. Every field is `#[serde(default)]` so scenes written
/// before a field existed still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthPreproConfig {
    pub sensor_name: String,
    #[serde(default)]
    pub near_mm: f32,
    #[serde(default)]
    pub far_mm: f32,
    #[serde(default)]
    pub smoothing: f32,
    #[serde(default)]
    pub hole_fill: f32,
    #[serde(default)]
    pub mask_feather: f32,
    #[serde(default)]
    pub motion_gain: f32,
    #[serde(default)]
    pub mirror: bool,
}

/// Serializable point-cloud view params for depth-sensor decks. All fields are
/// `#[serde(default)]` so older scenes (and scenes written before a field was
/// added) deserialize cleanly. Stored denormalized (physical units), matching
/// the runtime `PointCloudParams`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthParamsConfig {
    #[serde(default)]
    pub orbit_yaw: f32,
    #[serde(default)]
    pub orbit_pitch: f32,
    #[serde(default)]
    pub zoom: f32,
    #[serde(default)]
    pub point_size: f32,
    /// 0 = Rgb, 1 = `DepthRamp`, 2 = Solid
    #[serde(default)]
    pub color_mode: u8,
    #[serde(default)]
    pub depth_min_mm: f32,
    #[serde(default)]
    pub depth_max_mm: f32,
    #[serde(default)]
    pub solid_color: [f32; 3],
    #[serde(default)]
    pub seed: f32,
    #[serde(default)]
    pub drift: f32,
    #[serde(default)]
    pub disruption: f32,
}

// ── Effect ─────────────────────────────────────────────────────────

/// Serializable effect (ISF filter) state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,
    /// Path to the ISF shader file
    pub path: String,
    /// Whether effect is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Parameter values (name -> value)
    #[serde(default)]
    pub params: HashMap<String, ParamValue>,
}

fn default_true() -> bool {
    true
}

// ── Output ─────────────────────────────────────────────────────────

/// Serializable output target configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(Default)]
pub enum OutputTargetConfig {
    #[default]
    Windowed,
    Display {
        name: String,
    },
    Recording {
        path: String,
        codec: String,
        /// Audio passthrough device name (None = silent). See spec/audio-passthrough.md.
        #[serde(default)]
        audio_device: Option<String>,
    },
    SrtStream {
        url: String,
        #[serde(default)]
        codec: String,
        #[serde(default)]
        audio_device: Option<String>,
    },
    HlsStream {
        name: String,
        #[serde(default)]
        codec: String,
        #[serde(default)]
        low_latency: bool,
        #[serde(default)]
        audio_device: Option<String>,
    },
    DashStream {
        name: String,
        #[serde(default)]
        codec: String,
        #[serde(default)]
        audio_device: Option<String>,
    },
    RtmpStream {
        url: String,
        #[serde(default)]
        codec: String,
        #[serde(default)]
        codec_contract: crate::renderer::context::RtmpCodecContract,
        #[serde(default)]
        audio_device: Option<String>,
    },
    NdiSend {
        sender_name: String,
    },
    SyphonServer {
        server_name: String,
    },
}

/// Serializable output configuration (unified model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,
    pub name: String,
    /// The output target type and config.
    #[serde(default)]
    pub target: OutputTargetConfig,
    /// Legacy field — Display target name. Kept for backwards compat during migration.
    /// Ignored if `target` is present and not Windowed.
    #[serde(default, skip_serializing)]
    pub target_display: Option<String>,
    /// Surface assignments with warp calibration
    #[serde(default)]
    pub surface_assignments: Vec<SurfaceAssignmentConfig>,
    /// Saved window position [x, y] in physical pixels (for Windowed targets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_position: Option<[i32; 2]>,
    /// Saved window size [width, height] in physical pixels (for Windowed targets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<[u32; 2]>,
    /// Whether edge blend is auto-computed or manually configured.
    #[serde(default)]
    pub edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode,
    /// Edge blending configuration for multi-projector overlap zones.
    #[serde(default)]
    pub edge_blend: crate::renderer::edge_blend::EdgeBlendConfig,
    /// Per-output rotation (0°/90°/180°/270°).
    #[serde(default)]
    pub rotation: crate::renderer::context::OutputRotation,
    /// Requested SDR precision and deterministic presentation dithering.
    #[serde(default, flatten)]
    pub presentation: crate::engine::value::render::PresentationRequest,
}

impl OutputConfig {
    /// Create a default windowed output config with an auto-generated name.
    pub fn default_windowed() -> Self {
        Self {
            uuid: crate::deck::generate_short_uuid(),
            name: String::new(),
            target: OutputTargetConfig::Windowed,
            target_display: None,
            surface_assignments: Vec::new(),
            window_position: None,
            window_size: None,
            edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode::default(),
            edge_blend: crate::renderer::edge_blend::EdgeBlendConfig::default(),
            rotation: crate::renderer::context::OutputRotation::default(),
            presentation: crate::engine::value::render::PresentationRequest::default(),
        }
    }
}

/// Membership of a surface in an output (persisted). Warp now lives on the
/// surface (`Surface.warp`); `legacy_warp_mode` exists only to migrate
/// pre-8i.5 files that stored warp here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceAssignmentConfig {
    pub surface_uuid: String,
    /// LEGACY (pre-8i.5): warp used to live on the assignment. Read at load for
    /// one-time migration onto `Surface.warp`, then dropped (never re-saved).
    #[serde(default, rename = "warp_mode", skip_serializing)]
    pub legacy_warp_mode: Option<crate::renderer::warp::WarpMode>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

// ── Blend mode ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlendModeConfig {
    #[default]
    Normal,
    Add,
    Subtract,
    Multiply,
    Screen,
    Overlay,
    #[serde(rename = "softlight")]
    SoftLight,
    #[serde(rename = "hardlight")]
    HardLight,
    #[serde(rename = "colordodge")]
    ColorDodge,
    #[serde(rename = "colorburn")]
    ColorBurn,
    Difference,
    Exclusion,
    Darken,
    Lighten,
    #[serde(rename = "linearburn")]
    LinearBurn,
}

impl From<BlendMode> for BlendModeConfig {
    fn from(mode: BlendMode) -> Self {
        match mode {
            BlendMode::Normal => BlendModeConfig::Normal,
            BlendMode::Add => BlendModeConfig::Add,
            BlendMode::Subtract => BlendModeConfig::Subtract,
            BlendMode::Multiply => BlendModeConfig::Multiply,
            BlendMode::Screen => BlendModeConfig::Screen,
            BlendMode::Overlay => BlendModeConfig::Overlay,
            BlendMode::SoftLight => BlendModeConfig::SoftLight,
            BlendMode::HardLight => BlendModeConfig::HardLight,
            BlendMode::ColorDodge => BlendModeConfig::ColorDodge,
            BlendMode::ColorBurn => BlendModeConfig::ColorBurn,
            BlendMode::Difference => BlendModeConfig::Difference,
            BlendMode::Exclusion => BlendModeConfig::Exclusion,
            BlendMode::Darken => BlendModeConfig::Darken,
            BlendMode::Lighten => BlendModeConfig::Lighten,
            BlendMode::LinearBurn => BlendModeConfig::LinearBurn,
        }
    }
}

impl From<BlendModeConfig> for BlendMode {
    fn from(config: BlendModeConfig) -> Self {
        match config {
            BlendModeConfig::Normal => BlendMode::Normal,
            BlendModeConfig::Add => BlendMode::Add,
            BlendModeConfig::Subtract => BlendMode::Subtract,
            BlendModeConfig::Multiply => BlendMode::Multiply,
            BlendModeConfig::Screen => BlendMode::Screen,
            BlendModeConfig::Overlay => BlendMode::Overlay,
            BlendModeConfig::SoftLight => BlendMode::SoftLight,
            BlendModeConfig::HardLight => BlendMode::HardLight,
            BlendModeConfig::ColorDodge => BlendMode::ColorDodge,
            BlendModeConfig::ColorBurn => BlendMode::ColorBurn,
            BlendModeConfig::Difference => BlendMode::Difference,
            BlendModeConfig::Exclusion => BlendMode::Exclusion,
            BlendModeConfig::Darken => BlendMode::Darken,
            BlendModeConfig::Lighten => BlendMode::Lighten,
            BlendModeConfig::LinearBurn => BlendMode::LinearBurn,
        }
    }
}

// ── Validation ─────────────────────────────────────────────────────

impl SourceConfig {
    /// Validate source config. Returns a list of errors (empty = valid).
    pub fn validate(&self, prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        match self {
            SourceConfig::Shader { path, .. } => {
                if path.trim().is_empty() {
                    errors.push(format!("{prefix}: shader path is empty"));
                }
            }
            SourceConfig::Video { path, .. } => {
                if path.trim().is_empty() {
                    errors.push(format!("{prefix}: video path is empty"));
                }
            }
            SourceConfig::Image { path, .. } => {
                if path.trim().is_empty() {
                    errors.push(format!("{prefix}: image path is empty"));
                }
            }
            SourceConfig::SolidColor { color } => {
                for (i, c) in color.iter().enumerate() {
                    if !c.is_finite() {
                        errors.push(format!("{prefix}: color[{i}] is not finite"));
                    }
                }
            }
            SourceConfig::Camera { name } => {
                if name.trim().is_empty() {
                    errors.push(format!("{prefix}: camera name is empty"));
                }
            }
            SourceConfig::Ndi { name } => {
                if name.trim().is_empty() {
                    errors.push(format!("{prefix}: NDI name is empty"));
                }
            }
            SourceConfig::Syphon { name } => {
                if name.trim().is_empty() {
                    errors.push(format!("{prefix}: Syphon name is empty"));
                }
            }
            SourceConfig::Srt { url, .. } => {
                if url.trim().is_empty() {
                    errors.push(format!("{prefix}: SRT url is empty"));
                }
            }
            SourceConfig::Hls { url } => {
                if url.trim().is_empty() {
                    errors.push(format!("{prefix}: HLS url is empty"));
                }
            }
            SourceConfig::Dash { url } => {
                if url.trim().is_empty() {
                    errors.push(format!("{prefix}: DASH url is empty"));
                }
            }
            SourceConfig::Rtmp { url, .. } => {
                if url.trim().is_empty() {
                    errors.push(format!("{prefix}: RTMP url is empty"));
                }
            }
            SourceConfig::Html { url } => {
                if url.trim().is_empty() {
                    errors.push(format!("{prefix}: HTML url is empty"));
                }
            }
            SourceConfig::DepthSensor { name, .. } => {
                if name.trim().is_empty() {
                    errors.push(format!("{prefix}: depth sensor name is empty"));
                }
            }
            SourceConfig::ScreenCapture {
                target, rate, crop, ..
            } => {
                match target {
                    CaptureTargetConfig::Display { name } if name.trim().is_empty() => {
                        errors.push(format!("{prefix}: capture display name is empty"));
                    }
                    CaptureTargetConfig::Window { app, title } => {
                        // A window with neither an app nor a title can never be
                        // matched back to a live target, so it is unrecoverable
                        // rather than merely stale.
                        if app.trim().is_empty() && title.trim().is_empty() {
                            errors.push(format!("{prefix}: capture window has no app or title"));
                        }
                    }
                    CaptureTargetConfig::Display { .. } => {}
                }
                if !rate.is_finite()
                    || *rate < crate::screen_capture::backend::MIN_CAPTURE_RATE
                    || *rate > crate::screen_capture::backend::MAX_CAPTURE_RATE
                {
                    errors.push(format!(
                        "{prefix}: capture rate {rate} is outside {}–{}",
                        crate::screen_capture::backend::MIN_CAPTURE_RATE,
                        crate::screen_capture::backend::MAX_CAPTURE_RATE
                    ));
                }
                if let Some(c) = crop {
                    if !(c.x.is_finite() && c.y.is_finite() && c.w.is_finite() && c.h.is_finite()) {
                        errors.push(format!("{prefix}: capture crop is not finite"));
                    } else if c.x + c.w > 1.0 + f32::EPSILON || c.y + c.h > 1.0 + f32::EPSILON {
                        errors.push(format!("{prefix}: capture crop extends outside the target"));
                    }
                }
            }
            SourceConfig::Tap { source, .. } => {
                if let TapSourceConfig::Channel { uuid } = source {
                    // A missing channel is a restore-time warning, not a
                    // validation error; an empty UUID can never match anything.
                    if uuid.trim().is_empty() {
                        errors.push(format!("{prefix}: tap channel uuid is empty"));
                    }
                }
            }
        }
        errors
    }
}

impl EffectConfig {
    /// Validate effect config. Returns a list of errors (empty = valid).
    pub fn validate(&self, prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if self.path.trim().is_empty() {
            errors.push(format!("{prefix}: effect path is empty"));
        }
        errors
    }
}

impl DeckConfig {
    /// Validate deck config. Returns a list of errors (empty = valid).
    pub fn validate(&self, prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if !(0.0..=1.0).contains(&self.opacity) {
            errors.push(format!(
                "{}: opacity {} out of range 0.0-1.0",
                prefix, self.opacity
            ));
        }
        errors.extend(self.source.validate(&format!("{prefix}/source")));
        for (i, fx) in self.effects.iter().enumerate() {
            errors.extend(fx.validate(&format!("{prefix}/effects[{i}]")));
        }
        errors
    }
}

impl ChannelConfig {
    /// Validate channel config. Returns a list of errors (empty = valid).
    pub fn validate(&self, prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if !(0.0..=1.0).contains(&self.opacity) {
            errors.push(format!(
                "{}: opacity {} out of range 0.0-1.0",
                prefix, self.opacity
            ));
        }
        for (i, deck) in self.decks.iter().enumerate() {
            errors.extend(deck.validate(&format!("{prefix}/decks[{i}]")));
        }
        for (i, fx) in self.effects.iter().enumerate() {
            errors.extend(fx.validate(&format!("{prefix}/effects[{i}]")));
        }
        errors
    }
}

// ── I/O ────────────────────────────────────────────────────────────

impl SceneConfig {
    /// Validate the scene config for semantic correctness. Returns a list of errors.
    /// An empty list means the config is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if !(0.0..=1.0).contains(&self.crossfader) {
            errors.push(format!(
                "crossfader {} out of range 0.0-1.0",
                self.crossfader
            ));
        }
        if let Some(w) = self.render_width
            && w == 0
        {
            errors.push("render_width is 0".into());
        }
        if let Some(h) = self.render_height
            && h == 0
        {
            errors.push("render_height is 0".into());
        }
        for (i, ch) in self.channels.iter().enumerate() {
            errors.extend(ch.validate(&format!("channels[{i}]")));
        }
        for (i, fx) in self.master_effects.iter().enumerate() {
            errors.extend(fx.validate(&format!("master_effects[{i}]")));
        }
        errors
    }

    /// Load from a JSON file
    ///
    /// # Errors
    ///
    /// Returns an error if `path` cannot be read (missing file, permissions) or
    /// if its contents are not valid JSON for a [`SceneConfig`]. Validation
    /// problems in an otherwise-parseable scene are logged as warnings, not
    /// returned as errors.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read scene file: {}", path.as_ref().display()))?;
        let mut scene: SceneConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse scene file: {}", path.as_ref().display()))?;
        scene.migrate();
        let warnings = scene.validate();
        for w in &warnings {
            log::warn!("Scene config {}: {}", path.as_ref().display(), w);
        }
        Ok(scene)
    }

    /// Save to a JSON file
    ///
    /// # Errors
    ///
    /// Returns an error if the scene cannot be serialized to JSON, or if the
    /// atomic write fails (temp file creation, write, or rename).
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Scene config save: {e}");
        }
        let content = serde_json::to_string_pretty(self).context("Failed to serialize scene")?;
        crate::persistence::atomic_write(path.as_ref(), &content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Program / channel tap ────────────────────────────────────────

    /// Both tap variants have to survive a save/load cycle unchanged, since a
    /// scene is the only record of what a tap deck was pointed at.
    #[test]
    fn tap_source_roundtrips_through_json() {
        for source in [
            TapSourceConfig::MasterProgram,
            TapSourceConfig::Channel {
                uuid: "a1b2c3d4".into(),
            },
        ] {
            let cfg = SourceConfig::Tap {
                source: source.clone(),
                scaling_mode: crate::deck::ScalingMode::Fit,
            };
            let json = serde_json::to_string(&cfg).expect("serialize");
            let back: SourceConfig = serde_json::from_str(&json).expect("deserialize");
            match back {
                SourceConfig::Tap {
                    source: got,
                    scaling_mode,
                } => {
                    assert_eq!(got, source);
                    assert_eq!(scaling_mode, crate::deck::ScalingMode::Fit);
                }
                other => panic!("expected a tap, got {other:?}"),
            }
        }
    }

    /// `scaling_mode` is `#[serde(default)]`, so a scene written before the
    /// field existed still loads.
    #[test]
    fn tap_source_loads_without_a_scaling_mode() {
        let json = r#"{"type":"Tap","source":{"kind":"master_program"}}"#;
        let cfg: SourceConfig = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(
            cfg,
            SourceConfig::Tap {
                source: TapSourceConfig::MasterProgram,
                ..
            }
        ));
    }

    /// A channel UUID that cannot match anything is a config error; a UUID that
    /// merely names a deleted channel is not, because the deck is meant to
    /// survive unbound. See spec/program-tap.md.
    #[test]
    fn tap_validation_rejects_only_an_empty_channel_uuid() {
        let empty = SourceConfig::Tap {
            source: TapSourceConfig::Channel { uuid: "  ".into() },
            scaling_mode: crate::deck::ScalingMode::default(),
        };
        assert!(!empty.validate("d").is_empty());

        let absent = SourceConfig::Tap {
            source: TapSourceConfig::Channel {
                uuid: "deadbeef".into(),
            },
            scaling_mode: crate::deck::ScalingMode::default(),
        };
        assert!(
            absent.validate("d").is_empty(),
            "a tap naming a channel that is not in this scene must load unbound, not fail"
        );
    }

    // ── Round-trip serialization ─────────────────────────────────────

    #[test]
    fn scene_config_roundtrip_empty() {
        let scene = SceneConfig {
            version: 2,
            channels: vec![],
            crossfader: 0.5,
            active_transition: None,
            master_effects: vec![],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        let json = serde_json::to_string_pretty(&scene).unwrap();
        let restored: SceneConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.version, 2);
        assert!((restored.crossfader - 0.5).abs() < 1e-5);
        assert!(restored.channels.is_empty());
    }

    #[test]
    fn scene_config_roundtrip_with_channels() {
        let scene = SceneConfig {
            version: 2,
            channels: vec![ChannelConfig {
                uuid: crate::deck::generate_short_uuid(),
                name: "Ch 0".into(),
                opacity: 1.0,
                blend_mode: BlendModeConfig::Normal,
                decks: vec![DeckConfig {
                    uuid: crate::deck::generate_short_uuid(),
                    name: "Color Burn".into(),
                    source: SourceConfig::Shader {
                        path: "shaders/color_burn.fs".into(),
                        params: HashMap::new(),
                        depth_prepro: None,
                    },
                    effects: vec![],
                    opacity: 0.8,
                    transparent: false,
                    blend_mode: BlendModeConfig::Add,
                    mute: false,
                    solo: false,
                    z_index: 0,
                    auto_transition: None,
                    modulation: vec![],
                    render_fps: DeckRenderFps::default(),
                }],
                effects: vec![],
                modulation: vec![],
            }],
            crossfader: 0.0,
            active_transition: Some("dissolve".into()),
            master_effects: vec![],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        let json = serde_json::to_string_pretty(&scene).unwrap();
        let restored: SceneConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.channels.len(), 1);
        assert_eq!(restored.channels[0].name, "Ch 0");
        assert_eq!(restored.channels[0].decks.len(), 1);
        assert_eq!(restored.channels[0].decks[0].name, "Color Burn");
        assert!((restored.channels[0].decks[0].opacity - 0.8).abs() < 1e-5);
        assert_eq!(restored.active_transition, Some("dissolve".into()));
    }

    #[test]
    fn scene_config_roundtrip_with_effects() {
        let scene = SceneConfig {
            version: 2,
            channels: vec![],
            crossfader: 0.0,
            active_transition: None,
            master_effects: vec![EffectConfig {
                uuid: "fxtest01".to_string(),
                path: "shaders/blur.fs".into(),
                enabled: true,
                params: {
                    let mut p = HashMap::new();
                    p.insert("amount".into(), ParamValue::Float(0.5));
                    p
                },
            }],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        let json = serde_json::to_string_pretty(&scene).unwrap();
        let restored: SceneConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.master_effects.len(), 1);
        assert!(restored.master_effects[0].enabled);
    }

    #[test]
    fn scene_config_roundtrip_solid_color_source() {
        let source = SourceConfig::SolidColor {
            color: [1.0, 0.0, 0.0, 1.0],
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::SolidColor { color } => {
                assert!((color[0] - 1.0).abs() < 1e-5);
            }
            _ => panic!("Expected SolidColor"),
        }
    }

    #[test]
    fn scene_config_roundtrip_video_source() {
        let source = SourceConfig::Video {
            path: "clips/intro.mov".into(),
            loop_mode: crate::video::LoopMode::Loop,
            speed: 1.0,
            in_point: 0.0,
            out_point: 0.0,
            scaling_mode: crate::deck::ScalingMode::Fit,
            transport_sync: crate::video::DeckTransportSync {
                mode: crate::video::TransportSyncMode::Never,
                offset: 3600.0,
                delay_frames: -2,
            },
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Video {
                path,
                loop_mode,
                speed,
                in_point,
                out_point,
                scaling_mode,
                transport_sync,
            } => {
                assert_eq!(path, "clips/intro.mov");
                assert_eq!(loop_mode, crate::video::LoopMode::Loop);
                assert!((speed - 1.0).abs() < 1e-5);
                assert!((in_point - 0.0).abs() < 1e-5);
                assert!((out_point - 0.0).abs() < 1e-5);
                assert_eq!(scaling_mode, crate::deck::ScalingMode::Fit);
                assert_eq!(transport_sync.mode, crate::video::TransportSyncMode::Never);
                assert!((transport_sync.offset - 3600.0).abs() < 1e-9);
                assert_eq!(transport_sync.delay_frames, -2);
            }
            _ => panic!("Expected Video"),
        }
    }

    #[test]
    fn video_source_defaults_transport_sync_to_auto() {
        let restored: SourceConfig =
            serde_json::from_str(r#"{"type":"Video","path":"clips/intro.mov"}"#).unwrap();
        match restored {
            SourceConfig::Video { transport_sync, .. } => {
                assert_eq!(transport_sync.mode, crate::video::TransportSyncMode::Auto);
                assert_eq!(transport_sync.offset, 0.0);
                assert_eq!(transport_sync.delay_frames, 0);
            }
            _ => panic!("Expected Video"),
        }
    }

    #[test]
    fn scene_config_roundtrip_image_source() {
        let source = SourceConfig::Image {
            path: "images/logo.png".into(),
            scaling_mode: crate::deck::ScalingMode::Center,
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Image { path, scaling_mode } => {
                assert_eq!(path, "images/logo.png");
                assert_eq!(scaling_mode, crate::deck::ScalingMode::Center);
            }
            _ => panic!("Expected Image"),
        }
    }

    #[test]
    fn scene_config_roundtrip_camera_source() {
        let source = SourceConfig::Camera {
            name: "FaceTime HD".into(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Camera { name } => assert_eq!(name, "FaceTime HD"),
            _ => panic!("Expected Camera"),
        }
    }

    // ── Migration ────────────────────────────────────────────────────

    /// Every `SceneConfig` field carries a serde default, so a version stamp is
    /// enough to stand up the same shape `load` parses.
    fn scene_with_sources(
        version: u32,
        sources: Vec<crate::modulation::ModulationSource>,
    ) -> SceneConfig {
        let mut scene: SceneConfig = serde_json::from_str(&format!("{{\"version\": {version}}}"))
            .expect("a bare version stamp is a valid scene");
        for s in sources {
            scene.modulation.add_source(s);
        }
        scene
    }

    fn bipolar_lfo(amplitude: f32) -> crate::modulation::ModulationSource {
        crate::modulation::ModulationSource::LFO {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude,
            bipolar: true,
        }
    }

    fn amplitude_of(scene: &SceneConfig, idx: usize) -> f32 {
        match &scene.modulation.sources[idx].source {
            crate::modulation::ModulationSource::LFO { amplitude, .. } => *amplitude,
            other => panic!("expected an LFO, got {other:?}"),
        }
    }

    /// Amplitude 0.5 was the only pre-v6 setting that did not clip. Doubling it
    /// preserves the patch's motion exactly under the new 0.5 range weight.
    #[test]
    fn migration_v6_doubles_unclipped_bipolar_amplitude() {
        let mut scene = scene_with_sources(5, vec![bipolar_lfo(0.5)]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 1.0).abs() < 1e-6);
        assert_eq!(scene.version, SceneConfig::CURRENT_VERSION);
    }

    /// Anything above 0.5 was clipping before. Full amplitude is the closest
    /// honest reading: the whole fader, without the flat spots.
    #[test]
    fn migration_v6_clamps_overdriven_bipolar_amplitude() {
        let mut scene = scene_with_sources(5, vec![bipolar_lfo(1.0)]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 1.0).abs() < 1e-6);
    }

    /// Unipolar sources never had the doubling problem and must be left alone.
    #[test]
    fn migration_v6_leaves_unipolar_sources_untouched() {
        let unipolar = crate::modulation::ModulationSource::LFO {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 0.4,
            bipolar: false,
        };
        let mut scene = scene_with_sources(5, vec![unipolar]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 0.4).abs() < 1e-6);
    }

    /// Migration is idempotent: a scene already at the current version keeps
    /// its amplitudes, so re-saving and re-loading cannot compound the rescale.
    #[test]
    fn migration_v6_does_not_rerun_on_current_scenes() {
        let mut scene = scene_with_sources(SceneConfig::CURRENT_VERSION, vec![bipolar_lfo(0.25)]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 0.25).abs() < 1e-6);
    }

    // ── Defaults ─────────────────────────────────────────────────────

    #[test]
    fn scene_config_defaults_on_missing_fields() {
        let json = r#"{"version": 2}"#;
        let scene: SceneConfig = serde_json::from_str(json).unwrap();
        assert_eq!(scene.crossfader, 0.0);
        assert!(scene.channels.is_empty());
        assert!(scene.master_effects.is_empty());
        assert!(scene.active_transition.is_none());
    }

    #[test]
    fn deck_config_defaults() {
        let json = r#"{"source": {"type": "SolidColor", "color": [1,0,0,1]}}"#;
        let deck: DeckConfig = serde_json::from_str(json).unwrap();
        assert_eq!(deck.opacity, 1.0); // default
        assert!(!deck.mute);
        assert!(!deck.solo);
        assert_eq!(deck.z_index, 0);
        // Backward compatibility: scenes saved before the transparency feature
        // omit `transparent` and must load as opaque (false). See html-source.md §2.
        assert!(!deck.transparent);
    }

    #[test]
    fn deck_config_transparent_roundtrip() {
        let json = r#"{"source": {"type": "SolidColor", "color": [1,0,0,1]}, "transparent": true}"#;
        let deck: DeckConfig = serde_json::from_str(json).unwrap();
        assert!(deck.transparent);
        let reser = serde_json::to_string(&deck).unwrap();
        let back: DeckConfig = serde_json::from_str(&reser).unwrap();
        assert!(back.transparent);
    }

    // ── BlendModeConfig conversion ───────────────────────────────────

    #[test]
    fn blend_mode_config_roundtrip() {
        for mode in BlendMode::all() {
            let config: BlendModeConfig = (*mode).into();
            let back: BlendMode = config.into();
            assert_eq!(back, *mode, "Roundtrip failed for {mode:?}");
        }
    }

    // ── EasingConfig conversion ──────────────────────────────────────

    #[test]
    fn easing_config_roundtrip() {
        use crate::mixer::CrossfadeEasing;
        let easings = [
            (CrossfadeEasing::Linear, EasingConfig::Linear),
            (CrossfadeEasing::EaseInOut, EasingConfig::EaseInOut),
            (CrossfadeEasing::EaseIn, EasingConfig::EaseIn),
            (CrossfadeEasing::EaseOut, EasingConfig::EaseOut),
        ];
        for (easing, config) in &easings {
            let converted: EasingConfig = (*easing).into();
            assert_eq!(converted, *config);
            let back: CrossfadeEasing = converted.into();
            assert_eq!(back, *easing);
        }
    }

    // ── Transition sequence config ───────────────────────────────────

    #[test]
    fn transition_sequence_config_roundtrip() {
        let from_uuid = "chfrom01".to_string();
        let to_uuid = "chto0001".to_string();
        let seq = TransitionSequenceConfig {
            uuid: "seq00001".into(),
            name: "Show Loop".into(),
            enabled: true,
            steps: vec![
                TransitionStepConfig::Fade {
                    from_ch: from_uuid.clone().into(),
                    to_ch: to_uuid.clone().into(),
                    duration: DurationSpecConfig::Beats(4.0),
                    easing: EasingConfig::EaseInOut,
                    transition_shader: Some("dissolve".into()),
                    target_amount: 1.0,
                },
                TransitionStepConfig::Wait {
                    duration: DurationSpecConfig::Seconds(10.0),
                },
                TransitionStepConfig::GoTo { step_index: 0 },
            ],
        };
        let json = serde_json::to_string_pretty(&seq).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["steps"][0]["from_ch"], serde_json::json!(from_uuid));
        assert_eq!(raw["steps"][0]["to_ch"], serde_json::json!(to_uuid));

        let restored: TransitionSequenceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.uuid, "seq00001");
        assert_eq!(restored.name, "Show Loop");
        assert_eq!(restored.steps.len(), 3);
        match &restored.steps[0] {
            TransitionStepConfig::Fade { from_ch, to_ch, .. } => {
                assert_eq!(from_ch.resolve(&[]), Some(from_uuid));
                assert_eq!(to_ch.resolve(&[]), Some(to_uuid));
            }
            other => panic!("expected a Fade step, got {other:?}"),
        }
    }

    #[test]
    fn fade_step_reads_legacy_channel_index() {
        let json = r#"{
            "steps": [{
                "kind": "Fade",
                "from_ch": 0,
                "to_ch": 1,
                "duration": {"unit": "beats", "value": 4.0}
            }]
        }"#;
        let seq: TransitionSequenceConfig = serde_json::from_str(json).unwrap();
        let channels = vec!["chzero01".to_string(), "chone0001".to_string()];
        match &seq.steps[0] {
            TransitionStepConfig::Fade { from_ch, to_ch, .. } => {
                assert_eq!(from_ch.resolve(&channels).as_deref(), Some("chzero01"));
                assert_eq!(to_ch.resolve(&channels).as_deref(), Some("chone0001"));
                assert!(
                    from_ch.resolve(&[]).is_none(),
                    "an index cannot resolve without the channel order"
                );
            }
            other => panic!("expected a Fade step, got {other:?}"),
        }
    }

    // ── Auto-transition config ───────────────────────────────────────

    #[test]
    fn auto_transition_config_roundtrip() {
        let at = AutoTransitionConfig {
            enabled: true,
            trigger: TriggerConfig::ClipEnd,
            play_duration: DurationSpecConfig::Beats(16.0),
            transition_duration: DurationSpecConfig::Seconds(2.0),
            transition_shader: Some("wipe".into()),
        };
        let json = serde_json::to_string(&at).unwrap();
        let restored: AutoTransitionConfig = serde_json::from_str(&json).unwrap();
        assert!(restored.enabled);
        assert_eq!(restored.trigger, TriggerConfig::ClipEnd);
    }

    // ── File I/O ─────────────────────────────────────────────────────

    #[test]
    fn scene_config_save_and_load() {
        let dir = std::env::temp_dir().join("varda_test_scene");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("test_scene.json");

        let scene = SceneConfig {
            version: 2,
            channels: vec![ChannelConfig {
                uuid: crate::deck::generate_short_uuid(),
                name: "Test Ch".into(),
                opacity: 0.9,
                blend_mode: BlendModeConfig::Add,
                decks: vec![],
                effects: vec![],
                modulation: vec![],
            }],
            crossfader: 0.42,
            active_transition: None,
            master_effects: vec![],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: Some(1920),
            render_height: Some(1080),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        scene.save(&path).unwrap();
        let loaded = SceneConfig::load(&path).unwrap();
        assert_eq!(loaded.channels.len(), 1);
        assert_eq!(loaded.channels[0].name, "Test Ch");
        assert!((loaded.crossfader - 0.42).abs() < 1e-5);

        // Cleanup
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    // ── Validation ──────────────────────────────────────────────────

    #[test]
    fn validate_valid_scene() {
        let scene = SceneConfig {
            version: 2,
            channels: vec![ChannelConfig {
                uuid: crate::deck::generate_short_uuid(),
                name: "Ch 0".into(),
                opacity: 1.0,
                blend_mode: BlendModeConfig::Normal,
                decks: vec![DeckConfig {
                    uuid: crate::deck::generate_short_uuid(),
                    name: "Deck".into(),
                    source: SourceConfig::Shader {
                        path: "test.fs".into(),
                        params: HashMap::new(),
                        depth_prepro: None,
                    },
                    effects: vec![],
                    opacity: 0.5,
                    transparent: false,
                    blend_mode: BlendModeConfig::Normal,
                    mute: false,
                    solo: false,
                    z_index: 0,
                    auto_transition: None,
                    modulation: vec![],
                    render_fps: DeckRenderFps::default(),
                }],
                effects: vec![],
                modulation: vec![],
            }],
            crossfader: 0.5,
            active_transition: None,
            master_effects: vec![],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: Some(1920),
            render_height: Some(1080),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        assert!(scene.validate().is_empty());
    }

    #[test]
    fn validate_crossfader_out_of_range() {
        let mut scene = SceneConfig {
            version: 2,
            channels: vec![],
            crossfader: 1.5,
            active_transition: None,
            master_effects: vec![],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        let errors = scene.validate();
        assert!(errors.iter().any(|e| e.contains("crossfader")));
        scene.crossfader = -0.1;
        assert!(scene.validate().iter().any(|e| e.contains("crossfader")));
    }

    #[test]
    fn validate_render_dims_zero() {
        let scene = SceneConfig {
            version: 2,
            channels: vec![],
            crossfader: 0.0,
            active_transition: None,
            master_effects: vec![],
            modulation: ModulationEngine::default(),
            macros: MacroBank::default(),
            transition_sequences: vec![],
            render_width: Some(0),
            render_height: Some(0),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        let errors = scene.validate();
        assert!(errors.iter().any(|e| e.contains("render_width")));
        assert!(errors.iter().any(|e| e.contains("render_height")));
    }

    #[test]
    fn validate_channel_opacity_out_of_range() {
        let ch = ChannelConfig {
            uuid: crate::deck::generate_short_uuid(),
            name: "Bad".into(),
            opacity: 2.0,
            blend_mode: BlendModeConfig::Normal,
            decks: vec![],
            effects: vec![],
            modulation: vec![],
        };
        let errors = ch.validate("ch[0]");
        assert!(errors.iter().any(|e| e.contains("opacity")));
    }

    #[test]
    fn validate_deck_opacity_out_of_range() {
        let deck = DeckConfig {
            uuid: crate::deck::generate_short_uuid(),
            name: "D".into(),
            source: SourceConfig::Shader {
                path: "ok.fs".into(),
                params: HashMap::new(),
                depth_prepro: None,
            },
            effects: vec![],
            opacity: -0.5,
            transparent: false,
            blend_mode: BlendModeConfig::Normal,
            mute: false,
            solo: false,
            z_index: 0,
            auto_transition: None,
            modulation: vec![],
            render_fps: DeckRenderFps::default(),
        };
        let errors = deck.validate("d[0]");
        assert!(errors.iter().any(|e| e.contains("opacity")));
    }

    #[test]
    fn validate_source_empty_path() {
        let s = SourceConfig::Shader {
            path: String::new(),
            params: HashMap::new(),
            depth_prepro: None,
        };
        assert!(!s.validate("src").is_empty());
        let s = SourceConfig::Video {
            path: " ".into(),
            loop_mode: crate::video::LoopMode::default(),
            speed: 1.0,
            in_point: 0.0,
            out_point: 0.0,
            scaling_mode: crate::deck::ScalingMode::default(),
            transport_sync: crate::video::DeckTransportSync::default(),
        };
        assert!(!s.validate("src").is_empty());
        let s = SourceConfig::Image {
            path: String::new(),
            scaling_mode: crate::deck::ScalingMode::default(),
        };
        assert!(!s.validate("src").is_empty());
    }

    #[test]
    fn validate_source_solid_color_non_finite() {
        let s = SourceConfig::SolidColor {
            color: [1.0, f32::NAN, 0.0, 1.0],
        };
        let errors = s.validate("src");
        assert!(errors.iter().any(|e| e.contains("color[1]")));
    }

    #[test]
    fn validate_effect_empty_path() {
        let fx = EffectConfig {
            uuid: "test0001".into(),
            path: String::new(),
            enabled: true,
            params: HashMap::new(),
        };
        let errors = fx.validate("fx[0]");
        assert!(!errors.is_empty());
    }

    #[test]
    fn scene_config_roundtrip_rtmp_source() {
        let source = SourceConfig::Rtmp {
            url: "rtmp://live.example.com/app/stream".to_string(),
            mode: "pull".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Rtmp { url, mode } => {
                assert_eq!(url, "rtmp://live.example.com/app/stream");
                assert_eq!(mode, "pull");
            }
            _ => panic!("Expected Rtmp source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_ndi_source() {
        let source = SourceConfig::Ndi {
            name: "STUDIO (Camera 1)".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Ndi { name } => assert_eq!(name, "STUDIO (Camera 1)"),
            _ => panic!("Expected Ndi source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_syphon_source() {
        let source = SourceConfig::Syphon {
            name: "Simple Server".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Syphon { name } => assert_eq!(name, "Simple Server"),
            _ => panic!("Expected Syphon source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_srt_source() {
        let source = SourceConfig::Srt {
            url: "srt://192.168.1.10:9000".to_string(),
            mode: "caller".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Srt { url, mode } => {
                assert_eq!(url, "srt://192.168.1.10:9000");
                assert_eq!(mode, "caller");
            }
            _ => panic!("Expected Srt source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_hls_source() {
        let source = SourceConfig::Hls {
            url: "https://cdn.example.com/live/index.m3u8".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Hls { url } => {
                assert_eq!(url, "https://cdn.example.com/live/index.m3u8");
            }
            _ => panic!("Expected Hls source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_dash_source() {
        let source = SourceConfig::Dash {
            url: "https://cdn.example.com/live/manifest.mpd".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Dash { url } => {
                assert_eq!(url, "https://cdn.example.com/live/manifest.mpd");
            }
            _ => panic!("Expected Dash source"),
        }
    }

    /// The `#[serde(tag = "type")]` discriminant must keep the URL-only network
    /// variants (Hls/Dash) distinct — they share the same field shape, so a
    /// mistagged variant would silently deserialize as the wrong source type.
    #[test]
    fn scene_config_url_variants_are_tag_discriminated() {
        let hls_json = serde_json::to_string(&SourceConfig::Hls {
            url: "u".to_string(),
        })
        .unwrap();
        let dash_json = serde_json::to_string(&SourceConfig::Dash {
            url: "u".to_string(),
        })
        .unwrap();
        assert!(hls_json.contains("\"type\":\"Hls\""));
        assert!(dash_json.contains("\"type\":\"Dash\""));
        assert!(matches!(
            serde_json::from_str::<SourceConfig>(&hls_json).unwrap(),
            SourceConfig::Hls { .. }
        ));
        assert!(matches!(
            serde_json::from_str::<SourceConfig>(&dash_json).unwrap(),
            SourceConfig::Dash { .. }
        ));
    }

    #[test]
    fn scene_config_roundtrip_html_source() {
        let source = SourceConfig::Html {
            url: "https://example.com/visuals.html".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Html { url } => {
                assert_eq!(url, "https://example.com/visuals.html");
            }
            _ => panic!("Expected Html source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_depth_sensor_source() {
        let source = SourceConfig::DepthSensor {
            name: "Kinect v1 (#0)".to_string(),
            params: None,
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::DepthSensor { name, .. } => assert_eq!(name, "Kinect v1 (#0)"),
            _ => panic!("Expected DepthSensor source"),
        }
    }

    #[test]
    fn scene_config_roundtrip_depth_sensor_params() {
        let source = SourceConfig::DepthSensor {
            name: "Kinect v1 (#0)".to_string(),
            params: Some(DepthParamsConfig {
                orbit_yaw: 0.3,
                orbit_pitch: -0.2,
                zoom: 1.5,
                point_size: 4.0,
                color_mode: 2,
                depth_min_mm: 500.0,
                depth_max_mm: 3500.0,
                solid_color: [0.1, 0.2, 0.3],
                seed: 0.05,
                drift: 0.4,
                disruption: 0.7,
            }),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::DepthSensor {
                params: Some(p), ..
            } => {
                assert_eq!(p.color_mode, 2);
                assert_eq!(p.seed, 0.05);
                assert_eq!(p.drift, 0.4);
                assert_eq!(p.disruption, 0.7);
            }
            _ => panic!("Expected DepthSensor source with params"),
        }
    }

    #[test]
    fn scene_config_depth_sensor_legacy_json_has_no_params() {
        // A scene written before point-cloud params existed omits the key entirely.
        let json = r#"{"type":"DepthSensor","name":"Kinect v1 (#0)"}"#;
        let restored: SourceConfig = serde_json::from_str(json).unwrap();
        match restored {
            SourceConfig::DepthSensor { name, params } => {
                assert_eq!(name, "Kinect v1 (#0)");
                assert!(
                    params.is_none(),
                    "legacy scenes must default params to None"
                );
            }
            _ => panic!("Expected DepthSensor source"),
        }
    }

    #[test]
    fn legacy_output_defaults_to_eight_bit_dithered_presentation() {
        let output: OutputConfig = serde_json::from_str(r#"{"name":"Main"}"#).unwrap();
        assert_eq!(
            output.presentation,
            crate::engine::value::render::PresentationRequest::default()
        );
    }

    #[test]
    fn output_presentation_fields_are_flat_in_stage_json() {
        let mut output = OutputConfig::default_windowed();
        output.presentation.depth = crate::engine::value::render::PresentationDepth::Sdr10;
        output.presentation.dither = false;

        let value = serde_json::to_value(output).unwrap();
        assert_eq!(value["presentation_depth"], "sdr10");
        assert_eq!(value["dither"], false);
        assert!(value.get("presentation").is_none());
    }

    #[test]
    fn scene_config_roundtrip_rtmp_output() {
        let target = OutputTargetConfig::RtmpStream {
            url: "rtmp://live.twitch.tv/app/key".to_string(),
            codec: "H.264".to_string(),
            codec_contract: crate::renderer::context::RtmpCodecContract::Enhanced,
            audio_device: None,
        };
        let json = serde_json::to_string(&target).unwrap();
        let restored: OutputTargetConfig = serde_json::from_str(&json).unwrap();
        match restored {
            OutputTargetConfig::RtmpStream {
                url,
                codec,
                codec_contract,
                audio_device,
            } => {
                assert_eq!(url, "rtmp://live.twitch.tv/app/key");
                assert_eq!(codec, "H.264");
                assert_eq!(
                    codec_contract,
                    crate::renderer::context::RtmpCodecContract::Enhanced
                );
                assert_eq!(audio_device, None);
            }
            _ => panic!("Expected RtmpStream target"),
        }
    }

    #[test]
    fn legacy_rtmp_target_defaults_codec_contract() {
        let restored: OutputTargetConfig = serde_json::from_str(
            r#"{"type":"rtmp_stream","url":"rtmps://example/live","codec":"H264"}"#,
        )
        .unwrap();
        assert!(matches!(
            restored,
            OutputTargetConfig::RtmpStream {
                codec_contract: crate::renderer::context::RtmpCodecContract::Legacy,
                ..
            }
        ));
    }

    #[test]
    fn scene_config_legacy_output_loads_video_only() {
        // A scene authored before audio passthrough (no `audio_device` field)
        // must still deserialize, defaulting to video-only (None).
        let legacy = r#"{"type":"recording","path":"set.mp4","codec":"H.264"}"#;
        let restored: OutputTargetConfig = serde_json::from_str(legacy).unwrap();
        match restored {
            OutputTargetConfig::Recording {
                path,
                codec,
                audio_device,
            } => {
                assert_eq!(path, "set.mp4");
                assert_eq!(codec, "H.264");
                assert_eq!(audio_device, None, "legacy scene → video-only");
            }
            _ => panic!("Expected Recording target"),
        }
    }

    #[test]
    fn scene_config_roundtrip_recording_with_audio() {
        let target = OutputTargetConfig::Recording {
            path: "set.mp4".to_string(),
            codec: "ProRes 422".to_string(),
            audio_device: Some("Scarlett 2i2".to_string()),
        };
        let json = serde_json::to_string(&target).unwrap();
        let restored: OutputTargetConfig = serde_json::from_str(&json).unwrap();
        match restored {
            OutputTargetConfig::Recording {
                path,
                codec,
                audio_device,
            } => {
                assert_eq!(path, "set.mp4");
                assert_eq!(codec, "ProRes 422");
                assert_eq!(audio_device.as_deref(), Some("Scarlett 2i2"));
            }
            _ => panic!("Expected Recording target"),
        }
    }

    // ── Per-surface warp migration (8i.5) ────────────────────────────

    /// Pre-8i.5 files stored warp on the assignment under `warp_mode`; it must
    /// still deserialize (into `legacy_warp_mode`) so load-time migration can
    /// move it onto the surface.
    #[test]
    fn assignment_config_reads_legacy_warp_mode() {
        let json = r#"{"surface_uuid":"s1","warp_mode":{"type":"CornerPin","corners":[[0,0],[1,0],[1,1],[0,1]]},"enabled":true}"#;
        let cfg: SurfaceAssignmentConfig = serde_json::from_str(json).unwrap();
        assert!(
            matches!(
                cfg.legacy_warp_mode,
                Some(crate::renderer::warp::WarpMode::CornerPin { .. })
            ),
            "legacy warp_mode should deserialize for migration"
        );
    }

    /// New files must NOT re-serialize the legacy warp field.
    #[test]
    fn assignment_config_drops_legacy_warp_on_save() {
        let cfg = SurfaceAssignmentConfig {
            surface_uuid: "s1".into(),
            legacy_warp_mode: Some(crate::renderer::warp::WarpMode::identity_corners([
                0.0, 0.0, 1.0, 1.0,
            ])),
            enabled: true,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(
            !json.contains("warp_mode"),
            "legacy warp_mode must not be re-serialized: {json}"
        );
    }
}
