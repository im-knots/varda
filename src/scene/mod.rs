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

    /// Crossfader position (0.0 = Ch 1, 1.0 = Ch 2)
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
    // Note: Camera decks are NOT persisted — cameras are re-detected on startup
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

/// Serializable output window configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub name: String,
    /// Display target name (matched by monitor name on load). None = windowed.
    #[serde(default)]
    pub target_display: Option<String>,
    /// Surface assignments with warp calibration
    #[serde(default)]
    pub surface_assignments: Vec<SurfaceAssignmentConfig>,
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
