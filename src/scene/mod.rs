//! Scene - Saveable preset for VJ configurations

use crate::params::ParamValue;
use crate::channel::BlendMode;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Scene configuration - serializable snapshot of a VJ setup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfig {
    /// Scene name
    pub name: String,
    
    /// Scene description
    #[serde(default)]
    pub description: String,
    
    /// Deck configurations
    pub decks: Vec<DeckConfig>,
    
    /// Master effect chain (shader paths)
    #[serde(default)]
    pub master_effects: Vec<String>,
    
    /// Scene version for compatibility
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 { 1 }

/// Deck configuration - serializable deck state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckConfig {
    /// Deck name/label
    #[serde(default)]
    pub name: String,
    
    /// Source configuration
    pub source: SourceConfig,
    
    /// Effect chain (shader paths)
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
}

fn default_opacity() -> f32 { 1.0 }

/// Source configuration - what generates the base image
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    /// ISF shader generator
    Shader {
        /// Path to shader file
        path: String,
        /// Parameter values (name -> value)
        #[serde(default)]
        params: std::collections::HashMap<String, ParamValue>,
    },
    /// Video file
    Video {
        /// Path to video file
        path: String,
        /// Loop playback
        #[serde(default = "default_true")]
        loop_playback: bool,
    },
    /// Static image
    Image {
        /// Path to image file
        path: String,
    },
}

fn default_true() -> bool { true }

/// Effect configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectConfig {
    /// Path to effect shader
    pub path: String,
    /// Whether effect is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Parameter values
    #[serde(default)]
    pub params: std::collections::HashMap<String, ParamValue>,
}

/// Blend mode for serialization
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

impl SceneConfig {
    /// Create a new empty scene
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            decks: Vec::new(),
            master_effects: Vec::new(),
            version: 1,
        }
    }

    /// Load a scene from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read scene file: {}", path.display()))?;

        let scene: SceneConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse scene file: {}", path.display()))?;

        Ok(scene)
    }

    /// Save the scene to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize scene")?;

        std::fs::write(path, content)
            .with_context(|| format!("Failed to write scene file: {}", path.display()))?;

        Ok(())
    }

    /// Add a deck configuration
    pub fn add_deck(&mut self, deck: DeckConfig) {
        self.decks.push(deck);
    }
}

impl DeckConfig {
    /// Create a new deck config with a shader source
    pub fn from_shader(shader_path: impl Into<String>) -> Self {
        Self {
            name: String::new(),
            source: SourceConfig::Shader {
                path: shader_path.into(),
                params: std::collections::HashMap::new(),
            },
            effects: Vec::new(),
            opacity: 1.0,
            blend_mode: BlendModeConfig::Normal,
            mute: false,
            solo: false,
            z_index: 0,
        }
    }

    /// Create a new deck config with a video source
    pub fn from_video(video_path: impl Into<String>) -> Self {
        Self {
            name: String::new(),
            source: SourceConfig::Video {
                path: video_path.into(),
                loop_playback: true,
            },
            effects: Vec::new(),
            opacity: 1.0,
            blend_mode: BlendModeConfig::Normal,
            mute: false,
            solo: false,
            z_index: 0,
        }
    }

    /// Add an effect to this deck
    pub fn add_effect(&mut self, effect_path: impl Into<String>) {
        self.effects.push(EffectConfig {
            path: effect_path.into(),
            enabled: true,
            params: std::collections::HashMap::new(),
        });
    }
}

