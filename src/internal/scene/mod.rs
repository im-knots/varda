//! Scene configuration — serializable snapshot of the full VJ performance state.
//!
//! This is the data model for `.varda/scene.json`. It captures everything needed
//! to reconstruct a show: channels, decks, effects, modulation.
//! Surfaces and outputs live in `stage.json` (venue-specific, not show-specific).

use crate::channel::BlendMode;
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
}

fn default_version() -> u32 { 2 }

// ── Channel ────────────────────────────────────────────────────────

/// Serializable channel state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
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

fn default_opacity() -> f32 { 1.0 }

// ── Deck ───────────────────────────────────────────────────────────

/// Serializable deck state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckConfig {
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

    /// Auto-transition configuration (None = no auto-transition)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_transition: Option<AutoTransitionConfig>,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerConfig {
    Timer,
    ClipEnd,
}

fn default_timer_trigger() -> TriggerConfig { TriggerConfig::Timer }

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

fn default_sequence_name() -> String { "Sequence 1".to_string() }

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

fn default_easing() -> EasingConfig { EasingConfig::EaseInOut }

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
    },
    /// Static image
    Image {
        path: String,
    },
    /// Solid color fill
    SolidColor {
        color: [f32; 4],
    },
    /// Live camera feed (matched by name on restore)
    Camera {
        name: String,
    },
    /// NDI network video source (matched by name on restore)
    Ndi {
        name: String,
    },
    /// Syphon inter-app video source (matched by server name on restore, macOS only)
    Syphon {
        name: String,
    },
    /// SRT network video source (url + mode, reconnected on restore)
    Srt {
        url: String,
        mode: String,
    },
}

// ── Effect ─────────────────────────────────────────────────────────

/// Serializable effect (ISF filter) state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectConfig {
    /// Path to the ISF shader file
    pub path: String,
    /// Whether effect is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Parameter values (name -> value)
    #[serde(default)]
    pub params: HashMap<String, ParamValue>,
}

fn default_true() -> bool { true }

// ── Output ─────────────────────────────────────────────────────────

/// Serializable output target configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputTargetConfig {
    Windowed,
    Display { name: String },
    Recording { path: String, codec: String },
    SrtStream { url: String },
    NdiSend { sender_name: String },
    SyphonServer { server_name: String },
}

impl Default for OutputTargetConfig {
    fn default() -> Self { Self::Windowed }
}

/// Serializable output configuration (unified model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
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
}

impl OutputConfig {
    /// Create a default windowed output config with an auto-generated name.
    pub fn default_windowed() -> Self {
        Self {
            name: String::new(), // Will be assigned a name at creation time
            target: OutputTargetConfig::Windowed,
            target_display: None,
            surface_assignments: Vec::new(),
        }
    }
}


/// Per-surface warp calibration in an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceAssignmentConfig {
    pub surface_idx: usize,
    pub warp_corners: [[f32; 2]; 4],
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
    Multiply,
    Screen,
    Overlay,
    Difference,
}

impl From<BlendMode> for BlendModeConfig {
    fn from(mode: BlendMode) -> Self {
        match mode {
            BlendMode::Normal => BlendModeConfig::Normal,
            BlendMode::Add => BlendModeConfig::Add,
            BlendMode::Multiply => BlendModeConfig::Multiply,
            BlendMode::Screen => BlendModeConfig::Screen,
            BlendMode::Overlay => BlendModeConfig::Overlay,
            BlendMode::Difference => BlendModeConfig::Difference,
        }
    }
}

impl From<BlendModeConfig> for BlendMode {
    fn from(config: BlendModeConfig) -> Self {
        match config {
            BlendModeConfig::Normal => BlendMode::Normal,
            BlendModeConfig::Add => BlendMode::Add,
            BlendModeConfig::Multiply => BlendMode::Multiply,
            BlendModeConfig::Screen => BlendMode::Screen,
            BlendModeConfig::Overlay => BlendMode::Overlay,
            BlendModeConfig::Difference => BlendMode::Difference,
        }
    }
}

// ── I/O ────────────────────────────────────────────────────────────

impl SceneConfig {
    /// Load from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read scene file: {}", path.as_ref().display()))?;
        let scene: SceneConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse scene file: {}", path.as_ref().display()))?;
        Ok(scene)
    }

    /// Save to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize scene")?;
        std::fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write scene file: {}", path.as_ref().display()))?;
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
            channels: vec![
                ChannelConfig {
                    name: "Ch 0".into(),
                    opacity: 1.0,
                    blend_mode: BlendModeConfig::Normal,
                    decks: vec![
                        DeckConfig {
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
                        },
                    ],
                    effects: vec![],
                },
            ],
            crossfader: 0.0,
            active_transition: Some("dissolve".into()),
            master_effects: vec![],
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
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
        };
        let json = serde_json::to_string_pretty(&scene).unwrap();
        let restored: SceneConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.master_effects.len(), 1);
        assert!(restored.master_effects[0].enabled);
    }

    #[test]
    fn scene_config_roundtrip_solid_color_source() {
        let source = SourceConfig::SolidColor { color: [1.0, 0.0, 0.0, 1.0] };
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
        let source = SourceConfig::Video { path: "clips/intro.mov".into() };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Video { path } => assert_eq!(path, "clips/intro.mov"),
            _ => panic!("Expected Video"),
        }
    }

    #[test]
    fn scene_config_roundtrip_image_source() {
        let source = SourceConfig::Image { path: "images/logo.png".into() };
        let json = serde_json::to_string(&source).unwrap();
        let restored: SourceConfig = serde_json::from_str(&json).unwrap();
        match restored {
            SourceConfig::Image { path } => assert_eq!(path, "images/logo.png"),
            _ => panic!("Expected Image"),
        }
    }

    #[test]
    fn scene_config_roundtrip_camera_source() {
        let source = SourceConfig::Camera { name: "FaceTime HD".into() };
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
        let modes = [
            (BlendMode::Normal, BlendModeConfig::Normal),
            (BlendMode::Add, BlendModeConfig::Add),
            (BlendMode::Multiply, BlendModeConfig::Multiply),
            (BlendMode::Screen, BlendModeConfig::Screen),
            (BlendMode::Overlay, BlendModeConfig::Overlay),
            (BlendMode::Difference, BlendModeConfig::Difference),
        ];
        for (mode, config) in &modes {
            let converted: BlendModeConfig = (*mode).into();
            assert_eq!(std::mem::discriminant(&converted), std::mem::discriminant(config));
            let back: BlendMode = converted.into();
            assert_eq!(back, *mode);
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
}
