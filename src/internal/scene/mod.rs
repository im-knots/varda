//! Scene configuration — serializable snapshot of the full VJ performance state.
//!
//! This is the data model for `.varda/scene.json`. It captures everything needed
//! to reconstruct a show: channels, decks, effects, modulation.
//! Surfaces and outputs live in `stage.json` (venue-specific, not show-specific).

use crate::channel::{BlendMode, DeckRenderFps};
use crate::modulation::ModulationEngine;
use crate::params::ParamValue;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
}

fn default_version() -> u32 {
    3
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

/// A single step in a transition sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TransitionStepConfig {
    Fade {
        from_ch: usize,
        to_ch: usize,
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
        }
    }
}

/// Per-surface warp calibration in an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceAssignmentConfig {
    pub surface_uuid: String,
    pub warp_mode: crate::renderer::warp::WarpMode,
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
                    errors.push(format!("{}: shader path is empty", prefix));
                }
            }
            SourceConfig::Video { path, .. } => {
                if path.trim().is_empty() {
                    errors.push(format!("{}: video path is empty", prefix));
                }
            }
            SourceConfig::Image { path, .. } => {
                if path.trim().is_empty() {
                    errors.push(format!("{}: image path is empty", prefix));
                }
            }
            SourceConfig::SolidColor { color } => {
                for (i, c) in color.iter().enumerate() {
                    if !c.is_finite() {
                        errors.push(format!("{}: color[{}] is not finite", prefix, i));
                    }
                }
            }
            SourceConfig::Camera { name } => {
                if name.trim().is_empty() {
                    errors.push(format!("{}: camera name is empty", prefix));
                }
            }
            SourceConfig::Ndi { name } => {
                if name.trim().is_empty() {
                    errors.push(format!("{}: NDI name is empty", prefix));
                }
            }
            SourceConfig::Syphon { name } => {
                if name.trim().is_empty() {
                    errors.push(format!("{}: Syphon name is empty", prefix));
                }
            }
            SourceConfig::Srt { url, .. } => {
                if url.trim().is_empty() {
                    errors.push(format!("{}: SRT url is empty", prefix));
                }
            }
            SourceConfig::Hls { url } => {
                if url.trim().is_empty() {
                    errors.push(format!("{}: HLS url is empty", prefix));
                }
            }
            SourceConfig::Dash { url } => {
                if url.trim().is_empty() {
                    errors.push(format!("{}: DASH url is empty", prefix));
                }
            }
            SourceConfig::Rtmp { url, .. } => {
                if url.trim().is_empty() {
                    errors.push(format!("{}: RTMP url is empty", prefix));
                }
            }
            SourceConfig::Html { url } => {
                if url.trim().is_empty() {
                    errors.push(format!("{}: HTML url is empty", prefix));
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
            errors.push(format!("{}: effect path is empty", prefix));
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
        errors.extend(self.source.validate(&format!("{}/source", prefix)));
        for (i, fx) in self.effects.iter().enumerate() {
            errors.extend(fx.validate(&format!("{}/effects[{}]", prefix, i)));
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
            errors.extend(deck.validate(&format!("{}/decks[{}]", prefix, i)));
        }
        for (i, fx) in self.effects.iter().enumerate() {
            errors.extend(fx.validate(&format!("{}/effects[{}]", prefix, i)));
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
        if let Some(w) = self.render_width {
            if w == 0 {
                errors.push("render_width is 0".into());
            }
        }
        if let Some(h) = self.render_height {
            if h == 0 {
                errors.push("render_height is 0".into());
            }
        }
        for (i, ch) in self.channels.iter().enumerate() {
            errors.extend(ch.validate(&format!("channels[{}]", i)));
        }
        for (i, fx) in self.master_effects.iter().enumerate() {
            errors.extend(fx.validate(&format!("master_effects[{}]", i)));
        }
        errors
    }

    /// Load from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read scene file: {}", path.as_ref().display()))?;
        let scene: SceneConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse scene file: {}", path.as_ref().display()))?;
        let warnings = scene.validate();
        for w in &warnings {
            log::warn!("Scene config {}: {}", path.as_ref().display(), w);
        }
        Ok(scene)
    }

    /// Save to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Scene config save: {}", e);
        }
        let content = serde_json::to_string_pretty(self).context("Failed to serialize scene")?;
        crate::persistence::atomic_write(path.as_ref(), &content)?;
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
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
                    },
                    effects: vec![],
                    opacity: 0.8,
                    blend_mode: BlendModeConfig::Add,
                    mute: false,
                    solo: false,
                    z_index: 0,
                    auto_transition: None,
                    modulation: vec![],
                    render_fps: DeckRenderFps::default(),
                }],
                effects: vec![],
            }],
            crossfader: 0.0,
            active_transition: Some("dissolve".into()),
            master_effects: vec![],
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
            } => {
                assert_eq!(path, "clips/intro.mov");
                assert_eq!(loop_mode, crate::video::LoopMode::Loop);
                assert!((speed - 1.0).abs() < 1e-5);
                assert!((in_point - 0.0).abs() < 1e-5);
                assert!((out_point - 0.0).abs() < 1e-5);
                assert_eq!(scaling_mode, crate::deck::ScalingMode::Fit);
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
    }

    // ── BlendModeConfig conversion ───────────────────────────────────

    #[test]
    fn blend_mode_config_roundtrip() {
        for mode in BlendMode::all() {
            let config: BlendModeConfig = (*mode).into();
            let back: BlendMode = config.into();
            assert_eq!(back, *mode, "Roundtrip failed for {:?}", mode);
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
        let seq = TransitionSequenceConfig {
            name: "Show Loop".into(),
            enabled: true,
            steps: vec![
                TransitionStepConfig::Fade {
                    from_ch: 0,
                    to_ch: 1,
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
        let restored: TransitionSequenceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "Show Loop");
        assert_eq!(restored.steps.len(), 3);
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
            }],
            crossfader: 0.42,
            active_transition: None,
            master_effects: vec![],
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: Some(1920),
            render_height: Some(1080),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
                    },
                    effects: vec![],
                    opacity: 0.5,
                    blend_mode: BlendModeConfig::Normal,
                    mute: false,
                    solo: false,
                    z_index: 0,
                    auto_transition: None,
                    modulation: vec![],
                    render_fps: DeckRenderFps::default(),
                }],
                effects: vec![],
            }],
            crossfader: 0.5,
            active_transition: None,
            master_effects: vec![],
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: Some(1920),
            render_height: Some(1080),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: Some(0),
            render_height: Some(0),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
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
            },
            effects: vec![],
            opacity: -0.5,
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
            path: "".into(),
            params: HashMap::new(),
        };
        assert!(!s.validate("src").is_empty());
        let s = SourceConfig::Video {
            path: " ".into(),
            loop_mode: Default::default(),
            speed: 1.0,
            in_point: 0.0,
            out_point: 0.0,
            scaling_mode: Default::default(),
        };
        assert!(!s.validate("src").is_empty());
        let s = SourceConfig::Image {
            path: "".into(),
            scaling_mode: Default::default(),
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
            path: "".into(),
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
    fn scene_config_roundtrip_rtmp_output() {
        let target = OutputTargetConfig::RtmpStream {
            url: "rtmp://live.twitch.tv/app/key".to_string(),
            codec: "H.264".to_string(),
            audio_device: None,
        };
        let json = serde_json::to_string(&target).unwrap();
        let restored: OutputTargetConfig = serde_json::from_str(&json).unwrap();
        match restored {
            OutputTargetConfig::RtmpStream {
                url,
                codec,
                audio_device,
            } => {
                assert_eq!(url, "rtmp://live.twitch.tv/app/key");
                assert_eq!(codec, "H.264");
                assert_eq!(audio_device, None);
            }
            _ => panic!("Expected RtmpStream target"),
        }
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
}
