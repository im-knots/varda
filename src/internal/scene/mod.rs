//! The data model for `.varda/scene.json`: channels, decks, effects and
//! modulation. Surfaces and outputs live in `stage.json`.

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

/// The root of `.varda/scene.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfig {
    /// File format version; see [`Self::migrate`].
    #[serde(default = "default_version")]
    pub version: u32,

    #[serde(default)]
    pub channels: Vec<ChannelConfig>,

    /// Crossfader position (0.0 = Ch 0, 1.0 = Ch 1).
    #[serde(default)]
    pub crossfader: f32,

    /// Active transition shader name (`None` = opacity crossfade).
    #[serde(default)]
    pub active_transition: Option<String>,

    #[serde(default)]
    pub master_effects: Vec<EffectConfig>,

    /// Modulation sources and assignments.
    #[serde(default)]
    pub modulation: ModulationEngine,

    /// Macro controls, each driving many parameters. Scenes before v4 have none.
    #[serde(default)]
    pub macros: MacroBank,

    /// Named transition sequences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transition_sequences: Vec<TransitionSequenceConfig>,

    /// Master render width (1920 if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_width: Option<u32>,

    /// Master render height (1080 if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_height: Option<u32>,

    /// Tonemap mode (ACES if absent).
    #[serde(default)]
    pub tonemap_mode: crate::renderer::tonemap::TonemapMode,

    /// Active LUT filename, relative to `.varda/luts/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_lut: Option<String>,

    /// Arrangement mode data. Absent in Performance-only scenes and before v7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<crate::arrangement::ArrangementConfig>,

    /// Frame rate and loop range. Outside `arrangement` because the position
    /// readout needs a rate without one.
    #[serde(default)]
    pub transport: TransportConfig,
}

/// The saved part of the transport. Position and run state are not saved.
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
    pub const CURRENT_VERSION: u32 = 9;

    /// Bring an older scene up to [`Self::CURRENT_VERSION`] in place. Runs on
    /// load, before validation; steps run in version order.
    ///
    /// v8 scenes store a component index on modulation assignments; v9 puts it
    /// in the key (`.../color/r`). The app rewrites them once targets exist
    /// ([`crate::mixer::Mixer::rekey_legacy_modulation`]), since only the
    /// target knows whether index 0 means `r` or `x`.
    ///
    /// v6 scenes have no arrangement or transport settings; both default.
    pub fn migrate(&mut self) {
        if self.version < 6 {
            self.migrate_v5_bipolar_amplitude();
        }
        if self.version < 8 {
            self.migrate_v7_modulation_keys();
        }
        self.version = Self::CURRENT_VERSION;
    }

    /// v7 to v8: pre-v8 scenes key modulation assignments (envelopes included)
    /// as `deck_<u>:<name>`; v8 uses router paths.
    ///
    /// Pre-v8 scenes used the `opacity` key for the deck's opacity, not a
    /// shader parameter. Unrecognized keys are kept.
    fn migrate_v7_modulation_keys(&mut self) {
        let assignments = std::mem::take(&mut self.modulation.assignments);
        let mut rekeyed = 0;
        for (key, mods) in assignments {
            let key = if let Some(address) =
                crate::engine::value::param::ParamAddress::from_legacy_modulation_key(&key)
            {
                rekeyed += 1;
                address.to_string()
            } else {
                log::warn!("Scene migration v7→v8: kept unrecognized modulation key '{key}'");
                key
            };
            self.modulation
                .assignments
                .entry(key)
                .or_default()
                .extend(mods);
        }
        if rekeyed > 0 {
            log::info!("Scene migration v7→v8: re-keyed {rekeyed} modulation target(s)");
        }
        // Pre-v8 macro targets are router paths but may use retired spellings
        // (`video/seek`, owner-qualified effect paths).
        for macro_control in self.macros.macros_mut() {
            for target in &mut macro_control.targets {
                target.path = crate::engine::value::param::canonical_path(&target.path);
            }
        }
    }

    /// v5 to v6: bipolar contributions have a 0.5 weight, so a full-amplitude
    /// bipolar LFO sweeps the range exactly. Pre-v6 scenes scaled by the whole
    /// range and clipped.
    ///
    /// LFO amplitude is doubled and clamped to 1.0, which reproduces the
    /// unclipped motion. Step sequencers have no amplitude and are left as is.
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

    /// Modulation on this channel's own effects, as recipes. Empty in
    /// `scene.json`; filled for presets and the clipboard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modulation: Vec<ModulationRecipe>,
}

fn default_opacity() -> f32 {
    1.0
}

// ── Deck ───────────────────────────────────────────────────────────

fn generate_default_uuid() -> String {
    crate::ids::generate_short_uuid()
}

/// Serializable deck state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,

    #[serde(default)]
    pub name: String,

    pub source: SourceConfig,

    #[serde(default)]
    pub effects: Vec<EffectConfig>,

    /// Deck opacity (0.0-1.0).
    #[serde(default = "default_opacity")]
    pub opacity: f32,

    /// Preserve source alpha instead of flattening over black.
    #[serde(default)]
    pub transparent: bool,

    #[serde(default)]
    pub blend_mode: BlendModeConfig,

    #[serde(default)]
    pub mute: bool,

    #[serde(default)]
    pub solo: bool,

    /// Z-index for layer ordering.
    #[serde(default)]
    pub z_index: i32,

    /// Per-deck render FPS cap (default: adaptive).
    #[serde(default)]
    pub render_fps: DeckRenderFps,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_transition: Option<AutoTransitionConfig>,

    /// Modulation recipes, for presets and the clipboard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modulation: Vec<ModulationRecipe>,
}

/// A modulation source and the params it targets, as stored in a preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationRecipe {
    #[serde(default = "crate::ids::generate_short_uuid")]
    pub source_uuid: String,
    pub source: crate::modulation::ModulationSource,
    /// Which clock the source follows. Stored on the engine entry, not the source.
    #[serde(default)]
    pub timebase: crate::timebase::Timebase,
    /// Assignments with owner-relative param keys.
    pub assignments: Vec<ModulationRecipeAssignment>,
}

impl ModulationRecipe {
    /// Rewrite pre-v8 assignment keys; see [`canonical_recipe_param`].
    pub fn canonicalize(&mut self) {
        for assignment in &mut self.assignments {
            assignment.param = canonical_recipe_param(&assignment.param);
        }
    }
}

/// The current spelling of a recipe assignment's parameter.
///
/// Deck-owned entries are relative to `deck/<uuid>/` (`param/speed`,
/// `opacity`, `video/speed`); effect entries are full
/// `effect/<uuid>/param/<name>` keys. Pre-v8 presets hold `speed`,
/// `video_speed`, or `fx_<uuid>:<name>`. Idempotent.
pub fn canonical_recipe_param(param: &str) -> String {
    use crate::engine::value::param::ParamAddress;
    if param.starts_with("fx_") {
        return ParamAddress::from_legacy_modulation_key(param)
            .map_or_else(|| param.to_string(), |address| address.to_string());
    }
    if param.contains('/') || param == "opacity" || param == "scaling_mode" {
        return param.to_string();
    }
    match param {
        "video_speed" => "video/speed".to_string(),
        "video_position" => "video/position".to_string(),
        "video_play" => "video/play".to_string(),
        "video_loop_mode" => "video/loop_mode".to_string(),
        name => format!("param/{name}"),
    }
}

impl DeckConfig {
    /// [`ModulationRecipe::canonicalize`] every recipe this deck carries.
    pub fn canonicalize_modulation(&mut self) {
        self.modulation
            .iter_mut()
            .for_each(ModulationRecipe::canonicalize);
    }
}

impl ChannelConfig {
    /// [`ModulationRecipe::canonicalize`] the channel's recipes and its decks'.
    pub fn canonicalize_modulation(&mut self) {
        self.modulation
            .iter_mut()
            .for_each(ModulationRecipe::canonicalize);
        self.decks
            .iter_mut()
            .for_each(DeckConfig::canonicalize_modulation);
    }
}

/// A single assignment within a modulation recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationRecipeAssignment {
    /// Relative to the owning deck (`param/brightness`, `opacity`), or a full
    /// `effect/<uuid>/param/<name>` key for an effect parameter.
    pub param: String,
    pub amount: f32,
    /// Component index in pre-v9 files. Read, never written; see
    /// [`crate::modulation::ParamModulation::legacy_component`].
    #[serde(default, rename = "component", skip_serializing)]
    pub legacy_component: Option<usize>,
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

    /// Transition shader name (`None` = opacity fade).
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

/// How a fade step names a channel. v5+ scenes write a UUID; v4 and earlier
/// store a channel index (`Index`). `resolve` turns either into a UUID.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelRef {
    Uuid(String),
    Index(usize),
}

impl ChannelRef {
    /// Resolve to a channel UUID; an index is looked up in `channel_uuids`.
    /// `None` if out of range.
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

/// A deck's source: a type id and that type's fields, decoded by the provider.
pub use crate::source::SourceConfig;

// ── Effect ─────────────────────────────────────────────────────────

/// Serializable effect (ISF filter) state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,
    pub path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub params: HashMap<String, ParamValue>,
}

fn default_true() -> bool {
    true
}

// ── Output ─────────────────────────────────────────────────────────

/// Serializable output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Stable UUID (8-char hex)
    #[serde(default = "generate_default_uuid")]
    pub uuid: String,
    pub name: String,
    /// Sink type and settings, saved as `{"type": "<id>", ...}`.
    #[serde(default = "default_sink")]
    pub target: crate::output::SinkConfig,
    /// Display target name in older files. Read only.
    #[serde(default, skip_serializing)]
    pub target_display: Option<String>,
    /// Surface assignments.
    #[serde(default)]
    pub surface_assignments: Vec<SurfaceAssignmentConfig>,
    /// Window position in older files. Read only.
    #[serde(default, skip_serializing)]
    pub window_position: Option<[i32; 2]>,
    /// Window size in older files. Read only.
    #[serde(default, skip_serializing)]
    pub window_size: Option<[u32; 2]>,
    /// Whether edge blend is auto-computed or manually configured.
    #[serde(default)]
    pub edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode,
    /// Edge blending configuration for multi-projector overlap zones.
    #[serde(default)]
    pub edge_blend: crate::renderer::edge_blend::EdgeBlendConfig,
    /// Per-output rotation (0/90/180/270 degrees).
    #[serde(default)]
    pub rotation: crate::renderer::context::OutputRotation,
    /// Requested SDR precision and deterministic presentation dithering.
    #[serde(default, flatten)]
    pub presentation: crate::engine::value::render::PresentationRequest,
    /// Per-output tonemap override. Absent inherits the show-wide curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tonemap_override: Option<crate::engine::value::render::TonemapMode>,
    /// Calibration test cards, on any output. Absent means off.
    #[serde(default, skip_serializing_if = "is_calibration_off")]
    pub calibration_mode: crate::engine::value::render::CalibrationMode,
    /// What the output shows with nothing assigned. Absent means the sink's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unassigned: Option<crate::engine::value::render::Unassigned>,
}

fn default_sink() -> crate::output::SinkConfig {
    crate::output::SinkConfig::new(crate::output::window::WINDOWED)
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_calibration_off(mode: &crate::engine::value::render::CalibrationMode) -> bool {
    *mode == crate::engine::value::render::CalibrationMode::Off
}

impl OutputConfig {
    /// A floating window, named by whoever creates it.
    pub fn default_windowed() -> Self {
        Self::for_sink(default_sink())
    }

    /// A new output delivering through `target`.
    pub fn for_sink(target: crate::output::SinkConfig) -> Self {
        Self {
            uuid: crate::ids::generate_short_uuid(),
            name: String::new(),
            target,
            target_display: None,
            surface_assignments: Vec::new(),
            window_position: None,
            window_size: None,
            edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode::default(),
            edge_blend: crate::renderer::edge_blend::EdgeBlendConfig::default(),
            rotation: crate::renderer::context::OutputRotation::default(),
            presentation: crate::engine::value::render::PresentationRequest::default(),
            tonemap_override: None,
            calibration_mode: crate::engine::value::render::CalibrationMode::Off,
            unassigned: None,
        }
    }

    /// Move fields older files keep outside the sink config into it: a
    /// display's monitor name, and a window's position and size.
    pub fn migrate_legacy(&mut self) {
        use crate::output::window::{DISPLAY, WINDOWED};
        if self.target.type_id() == WINDOWED
            && let Some(name) = self.target_display.take()
        {
            self.target = crate::output::SinkConfig::new(DISPLAY).with("name", name);
        }
        if let Some(position) = self.window_position.take()
            && self.target.get("position").is_none()
        {
            self.target.set("position", position);
        }
        if let Some(size) = self.window_size.take()
            && self.target.get("size").is_none()
        {
            self.target.set("size", size);
        }
    }
}

/// Membership of a surface in an output. Warp lives on `Surface.warp`;
/// `legacy_warp_mode` reads older files that store warp here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceAssignmentConfig {
    pub surface_uuid: String,
    /// Warp from older files, moved onto `Surface.warp` at load. Never saved.
    #[serde(default, rename = "warp_mode", skip_serializing)]
    pub legacy_warp_mode: Option<crate::surface::warp::WarpMode>,
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
    /// Validate the scene. Returns a list of errors; empty means valid.
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

    /// Load from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` cannot be read or is not valid [`SceneConfig`]
    /// JSON. Validation problems are only logged.
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

    /// Save to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the atomic write fails.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Scene config save: {e}");
        }
        let content = serde_json::to_string_pretty(self).context("Failed to serialize scene")?;
        crate::files::atomic_write(path.as_ref(), &content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                uuid: crate::ids::generate_short_uuid(),
                name: "Ch 0".into(),
                opacity: 1.0,
                blend_mode: BlendModeConfig::Normal,
                decks: vec![DeckConfig {
                    uuid: crate::ids::generate_short_uuid(),
                    name: "Color Burn".into(),
                    source: SourceConfig::new("Shader").with("path", "shaders/color_burn.fs"),
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

    // ── Migration ────────────────────────────────────────────────────

    /// Every field has a serde default, so a version stamp is a valid scene.
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

    /// Pre-v6 amplitude 0.5 doubles to 1.0, keeping the same motion.
    #[test]
    fn migration_v6_doubles_unclipped_bipolar_amplitude() {
        let mut scene = scene_with_sources(5, vec![bipolar_lfo(0.5)]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 1.0).abs() < 1e-6);
        assert_eq!(scene.version, SceneConfig::CURRENT_VERSION);
    }

    /// Pre-v6 amplitude above 0.5 clamps to 1.0.
    #[test]
    fn migration_v6_clamps_overdriven_bipolar_amplitude() {
        let mut scene = scene_with_sources(5, vec![bipolar_lfo(1.0)]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 1.0).abs() < 1e-6);
    }

    /// Unipolar sources are unchanged.
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

    /// A current-version scene keeps its amplitudes.
    #[test]
    fn migration_v6_does_not_rerun_on_current_scenes() {
        let mut scene = scene_with_sources(SceneConfig::CURRENT_VERSION, vec![bipolar_lfo(0.25)]);
        scene.migrate();
        assert!((amplitude_of(&scene, 0) - 0.25).abs() < 1e-6);
    }

    fn assignment_keys(scene: &SceneConfig) -> Vec<String> {
        let mut keys: Vec<String> = scene.modulation.assignments.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Each pre-v8 key form becomes the router path for its target.
    #[test]
    fn migration_v8_rekeys_modulation_by_router_path() {
        let mut scene = scene_with_sources(7, vec![]);
        for key in [
            "deck_d1:speed",
            "deck_d1:opacity",
            "deck_d1:video_position",
            "fx_f1:warp",
            "ch_c1:opacity",
            "macro_k1:value",
            "mod:m1:frequency",
        ] {
            scene
                .modulation
                .assignments
                .insert(key.to_string(), Vec::new());
        }
        scene.migrate();
        assert_eq!(
            assignment_keys(&scene),
            [
                "ch/c1/opacity",
                "deck/d1/opacity",
                "deck/d1/param/speed",
                "deck/d1/video/position",
                "effect/f1/param/warp",
                "macro/k1/value",
                "mod/m1/frequency",
            ]
        );
        assert_eq!(scene.version, SceneConfig::CURRENT_VERSION);
    }

    /// An unrecognized key is kept.
    #[test]
    fn migration_v8_keeps_unrecognized_keys() {
        let mut scene = scene_with_sources(7, vec![]);
        scene
            .modulation
            .assignments
            .insert("something_else".to_string(), Vec::new());
        scene.migrate();
        assert_eq!(assignment_keys(&scene), ["something_else"]);
    }

    #[test]
    fn recipe_params_move_to_the_current_spelling_once() {
        for (old, new) in [
            ("speed", "param/speed"),
            ("opacity", "opacity"),
            ("video_speed", "video/speed"),
            ("video_position", "video/position"),
            ("scaling_mode", "scaling_mode"),
            ("fx_f1:warp", "effect/f1/param/warp"),
            ("param/speed", "param/speed"),
            ("effect/f1/param/warp", "effect/f1/param/warp"),
        ] {
            assert_eq!(canonical_recipe_param(old), new, "{old}");
            assert_eq!(canonical_recipe_param(new), new, "{new} must stay put");
        }
    }

    #[test]
    fn migration_v8_does_not_rerun_on_current_scenes() {
        let mut scene = scene_with_sources(SceneConfig::CURRENT_VERSION, vec![]);
        scene
            .modulation
            .assignments
            .insert("deck/d1/param/speed".to_string(), Vec::new());
        scene.migrate();
        assert_eq!(assignment_keys(&scene), ["deck/d1/param/speed"]);
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
        // Older scenes omit `transparent` and load opaque.
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
                uuid: crate::ids::generate_short_uuid(),
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

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    // ── Validation ──────────────────────────────────────────────────

    #[test]
    fn validate_valid_scene() {
        let scene = SceneConfig {
            version: 2,
            channels: vec![ChannelConfig {
                uuid: crate::ids::generate_short_uuid(),
                name: "Ch 0".into(),
                opacity: 1.0,
                blend_mode: BlendModeConfig::Normal,
                decks: vec![DeckConfig {
                    uuid: crate::ids::generate_short_uuid(),
                    name: "Deck".into(),
                    source: SourceConfig::new("Shader").with("path", "test.fs"),
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
        assert_eq!(scene.validate().len(), 0);
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
            uuid: crate::ids::generate_short_uuid(),
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
            uuid: crate::ids::generate_short_uuid(),
            name: "D".into(),
            source: SourceConfig::new("Shader").with("path", "ok.fs"),
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
    fn validate_effect_empty_path() {
        let fx = EffectConfig {
            uuid: "test0001".into(),
            path: String::new(),
            enabled: true,
            params: HashMap::new(),
        };
        let errors = fx.validate("fx[0]");
        assert_ne!(errors.len(), 0);
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
    fn tonemap_override_survives_a_stage_round_trip() {
        use crate::engine::value::render::TonemapMode;
        let mut output: OutputConfig = serde_json::from_str(r#"{"name":"Main"}"#).unwrap();
        output.tonemap_override = Some(TonemapMode::AgX);

        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["tonemap_override"], "AgX");
        let back: OutputConfig = serde_json::from_value(value).unwrap();
        assert_eq!(back.tonemap_override, Some(TonemapMode::AgX));
    }

    #[test]
    fn an_inheriting_output_writes_no_tonemap_override_key() {
        // Inherit is omitted from stage.json.
        let output: OutputConfig = serde_json::from_str(r#"{"name":"Main"}"#).unwrap();
        assert_eq!(output.tonemap_override, None);
        let value = serde_json::to_value(&output).unwrap();
        assert!(
            value.get("tonemap_override").is_none(),
            "an inheriting output should not write the key"
        );
    }

    #[test]
    fn a_stage_written_before_per_output_tonemap_loads_as_inheriting() {
        let loaded: OutputConfig =
            serde_json::from_str(r#"{"name":"Main","presentation_depth":"sdr10"}"#).unwrap();
        assert_eq!(loaded.tonemap_override, None);
    }

    #[test]
    fn hdr_presentation_fields_survive_a_stage_round_trip() {
        use crate::engine::value::render::{PresentationMode, PresentationTransfer};
        let mut output: OutputConfig = serde_json::from_str(r#"{"name":"Main"}"#).unwrap();
        output.presentation = output.presentation.with_mode(PresentationMode::Hdr10);
        output.presentation.peak_nits = 4000;

        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["presentation_depth"], "sdr10");
        assert_eq!(value["transfer"], "hdr10_pq");
        assert_eq!(value["peak_nits"], 4000);

        let back: OutputConfig = serde_json::from_value(value).unwrap();
        assert_eq!(back.presentation.transfer, PresentationTransfer::Hdr10Pq);
        assert_eq!(back.presentation.peak_nits, 4000);
        assert_eq!(back.presentation.mode(), PresentationMode::Hdr10);
    }

    #[test]
    fn a_pre_hdr_stage_file_loads_as_sdr() {
        use crate::engine::value::render::{PresentationMode, PresentationTransfer};
        // An older output with no transfer and no peak.
        let json = serde_json::json!({
            "uuid": "out00001",
            "name": "Main",
            "presentation_depth": "sdr10",
            "dither": true,
        });
        let loaded: OutputConfig = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.presentation.transfer, PresentationTransfer::Sdr);
        assert_eq!(loaded.presentation.mode(), PresentationMode::Sdr10);
        assert_eq!(
            loaded.presentation.peak_nits,
            crate::engine::value::render::HDR_DEFAULT_PEAK_NITS
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

    /// A saved output's target is its sink config, in the same shape older stages use.
    #[test]
    fn a_saved_output_target_reads_back_unchanged() {
        let json = serde_json::json!({
            "uuid": "o1", "name": "Stream",
            "target": {"type": "rtmp_stream", "url": "rtmp://live/app/key", "codec": "H.264", "codec_contract": "enhanced", "audio_device": null},
        });
        let config: OutputConfig = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(config.target.type_id(), "rtmp_stream");
        let back = serde_json::to_value(&config).unwrap();
        assert_eq!(back["target"], json["target"]);
        assert!(
            back.get("unassigned").is_none(),
            "an unset choice is not written"
        );
        assert!(
            back.get("calibration_mode").is_none(),
            "calibration off is not written"
        );
    }

    /// Older stages keep a display's monitor and a window's placement outside
    /// the target; load moves them into it.
    #[test]
    fn legacy_window_fields_migrate_into_the_sink_config() {
        let mut config: OutputConfig = serde_json::from_value(serde_json::json!({
            "uuid": "o1", "name": "Wall",
            "target": {"type": "windowed"},
            "target_display": "HDMI-1",
            "window_position": [10, 20],
            "window_size": [1920, 1080],
        }))
        .unwrap();
        config.migrate_legacy();
        assert_eq!(config.target.type_id(), "display");
        assert_eq!(config.target.str("name"), Some("HDMI-1"));
        assert_eq!(
            config.target.get("position"),
            Some(&serde_json::json!([10, 20]))
        );
        assert_eq!(
            config.target.get("size"),
            Some(&serde_json::json!([1920, 1080]))
        );
        let saved = serde_json::to_value(&config).unwrap();
        assert!(saved.get("window_position").is_none());
        assert!(saved.get("target_display").is_none());
    }

    #[test]
    fn an_output_without_a_target_is_a_window() {
        let config: OutputConfig =
            serde_json::from_value(serde_json::json!({"name": "Out"})).unwrap();
        assert_eq!(config.target.type_id(), "windowed");
    }

    // ── Per-surface warp migration ───────────────────────────────────

    /// Older files store warp on the assignment as `warp_mode`; it reads into
    /// `legacy_warp_mode`.
    #[test]
    fn assignment_config_reads_legacy_warp_mode() {
        let json = r#"{"surface_uuid":"s1","warp_mode":{"type":"CornerPin","corners":[[0,0],[1,0],[1,1],[0,1]]},"enabled":true}"#;
        let cfg: SurfaceAssignmentConfig = serde_json::from_str(json).unwrap();
        assert!(
            matches!(
                cfg.legacy_warp_mode,
                Some(crate::surface::warp::WarpMode::CornerPin { .. })
            ),
            "legacy warp_mode should deserialize for migration"
        );
    }

    /// The legacy warp field is never written.
    #[test]
    fn assignment_config_drops_legacy_warp_on_save() {
        let cfg = SurfaceAssignmentConfig {
            surface_uuid: "s1".into(),
            legacy_warp_mode: Some(crate::surface::warp::WarpMode::identity_corners([
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
