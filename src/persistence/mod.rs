//! Workspace persistence — save/load `.varda/` directory.
//!
//! The workspace is the current working directory. All state lives in `.varda/`:
//! - `scene.json` — channels, decks, effects, modulation (show-specific, shareable)
//! - `stage.json` — surfaces, outputs, warp, editor prefs (venue-specific)
//! - `midi.json`  — MIDI controller mappings (device-name-keyed)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Stage configuration persisted in `.varda/stage.json`.
/// Contains venue-specific data: surfaces, outputs, and editor preferences.
/// Kept separate from scene.json so users can share deck layouts without stage geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePrefs {
    #[serde(default = "default_grid_size")]
    pub grid_size: f32,
    #[serde(default = "default_true")]
    pub snap: bool,
    #[serde(default)]
    pub library_panel_open: bool,
    #[serde(default)]
    pub stage_editor_open: bool,
    /// 2D stage surface layout
    #[serde(default)]
    pub surfaces: crate::surface::SurfaceManager,
    /// Output window configurations (surface assignments, warp calibration)
    #[serde(default)]
    pub outputs: Vec<crate::scene::OutputConfig>,
}

fn default_grid_size() -> f32 { 0.05 }
fn default_true() -> bool { true }

impl Default for StagePrefs {
    fn default() -> Self {
        Self {
            grid_size: 0.05,
            snap: true,
            library_panel_open: false,
            stage_editor_open: false,
            surfaces: crate::surface::SurfaceManager::default(),
            outputs: Vec::new(),
        }
    }
}

impl StagePrefs {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read stage prefs: {}", path.as_ref().display()))?;
        let prefs: StagePrefs = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse stage prefs: {}", path.as_ref().display()))?;
        Ok(prefs)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize stage prefs")?;
        std::fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write stage prefs: {}", path.as_ref().display()))?;
        Ok(())
    }
}

/// Workspace directory manager — handles `.varda/` paths and directory creation.
pub struct Workspace {
    /// Root of the workspace (current working directory)
    root: PathBuf,
}

impl Workspace {
    /// Create a workspace rooted at the given directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Create a workspace rooted at the current working directory.
    pub fn from_cwd() -> Result<Self> {
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        Ok(Self::new(cwd))
    }

    /// Path to the `.varda/` directory.
    pub fn varda_dir(&self) -> PathBuf {
        self.root.join(".varda")
    }

    /// Path to `scene.json`.
    pub fn scene_path(&self) -> PathBuf {
        self.varda_dir().join("scene.json")
    }

    /// Path to `midi.json`.
    pub fn midi_path(&self) -> PathBuf {
        self.varda_dir().join("midi.json")
    }

    /// Path to `stage.json`.
    pub fn stage_path(&self) -> PathBuf {
        self.varda_dir().join("stage.json")
    }

    /// Whether `.varda/` exists in this workspace.
    pub fn exists(&self) -> bool {
        self.varda_dir().is_dir()
    }

    /// Ensure the `.varda/` directory exists.
    pub fn ensure_dir(&self) -> Result<()> {
        let dir = self.varda_dir();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create .varda directory: {}", dir.display()))?;
            log::info!("Created workspace directory: {}", dir.display());
        }
        Ok(())
    }

    /// Check if a scene file exists.
    pub fn has_scene(&self) -> bool {
        self.scene_path().is_file()
    }

    /// Check if a MIDI config file exists.
    pub fn has_midi(&self) -> bool {
        self.midi_path().is_file()
    }

    /// Check if stage prefs file exists.
    pub fn has_stage(&self) -> bool {
        self.stage_path().is_file()
    }

    /// Root directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

// ── Snapshot: Live State → Config ───────────────────────────────────

use crate::mixer::Mixer;
use crate::renderer::context::{OutputTarget, OutputWindow};
use crate::scene::*;

/// Build a SceneConfig snapshot from live app state (show-specific: channels, effects, modulation).
pub fn snapshot_scene(
    mixer: &Mixer,
) -> SceneConfig {
    let channels = mixer.channels.iter().map(|ch| {
        let decks = ch.decks.iter().map(|slot| {
            let source = match slot.deck.source_type() {
                "shader" => {
                    let path = slot.deck.source_path()
                        .unwrap_or_default()
                        .to_string();
                    SourceConfig::Shader {
                        path,
                        params: slot.deck.generator_params.values.clone(),
                    }
                }
                "video" => SourceConfig::Video {
                    path: slot.deck.source_path().unwrap_or_default().to_string(),
                },
                "image" => SourceConfig::Image {
                    path: slot.deck.source_path().unwrap_or_default().to_string(),
                },
                "solid_color" => {
                    let color = slot.deck.solid_color().unwrap_or([0.0, 0.0, 0.0, 1.0]);
                    SourceConfig::SolidColor { color }
                }
                "camera" => {
                    // Store the camera display name (strip the 📹 prefix we add)
                    let name = slot.deck.source_name()
                        .trim_start_matches("📹 ")
                        .to_string();
                    SourceConfig::Camera { name }
                }
                _ => return None,
            };

            let effects = slot.deck.effects.iter().map(|eff| {
                EffectConfig {
                    path: eff.shader.file_path.clone().unwrap_or_default(),
                    enabled: eff.enabled,
                    params: eff.params.values.clone(),
                }
            }).collect();

            Some(DeckConfig {
                name: slot.deck.source_name().to_string(),
                source,
                effects,
                opacity: slot.opacity,
                blend_mode: slot.blend_mode.into(),
                mute: slot.mute,
                solo: slot.solo,
                z_index: slot.z_index,
            })
        }).flatten().collect();

        let effects = ch.effects.iter().map(|eff| {
            EffectConfig {
                path: eff.shader.file_path.clone().unwrap_or_default(),
                enabled: eff.enabled,
                params: eff.params.values.clone(),
            }
        }).collect();

        ChannelConfig {
            name: ch.name.clone(),
            opacity: ch.opacity,
            blend_mode: ch.blend_mode.into(),
            decks,
            effects,
        }
    }).collect();

    let master_effects = mixer.master_effects.iter().map(|eff| {
        EffectConfig {
            path: eff.shader.file_path.clone().unwrap_or_default(),
            enabled: eff.enabled,
            params: eff.params.values.clone(),
        }
    }).collect();

    let active_transition = mixer.active_transition.as_ref().map(|t| t.name.clone());

    SceneConfig {
        version: 2,
        channels,
        crossfader: mixer.crossfader,
        active_transition,
        master_effects,
        modulation: mixer.modulation.clone(),
    }
}

/// Build a StagePrefs snapshot from live app state (venue-specific: surfaces, outputs, editor prefs).
pub fn snapshot_stage(
    surface_manager: &crate::surface::SurfaceManager,
    output_windows: &[OutputWindow],
    grid_size: f32,
    snap: bool,
    library_panel_open: bool,
    stage_editor_open: bool,
) -> StagePrefs {
    let outputs = output_windows.iter().map(|o| {
        let target_display = match &o.target {
            OutputTarget::Windowed => None,
            OutputTarget::Display { name, .. } => Some(name.clone()),
        };
        OutputConfig {
            name: o.name.clone(),
            target_display,
            surface_assignments: o.surface_assignments.iter().map(|a| {
                SurfaceAssignmentConfig {
                    surface_idx: a.surface_idx,
                    warp_corners: a.warp_corners,
                    enabled: a.enabled,
                }
            }).collect(),
        }
    }).collect();

    StagePrefs {
        grid_size,
        snap,
        library_panel_open,
        stage_editor_open,
        surfaces: surface_manager.clone(),
        outputs,
    }
}

// ── Restore: Config → Live State ────────────────────────────────────

use crate::deck::{Deck, Effect};
use crate::isf::ISFShader;
use crate::renderer::RenderContext;
use crate::ui::{RENDER_WIDTH, RENDER_HEIGHT};

/// Restore result — contains reconstructed mixer.
/// Surfaces and outputs are loaded separately from stage.json.
pub struct RestoreResult {
    pub mixer: Mixer,
    pub warnings: Vec<String>,
}

/// Reconstruct live state from a SceneConfig.
pub fn restore_scene(
    config: &SceneConfig,
    context: &RenderContext,
    registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
) -> Result<RestoreResult> {
    let mut warnings = Vec::new();
    let mut mixer = Mixer::new(context, RENDER_WIDTH, RENDER_HEIGHT)?;

    // Clear default channels — we'll create from config
    mixer.channels.clear();

    for ch_config in &config.channels {
        let mut channel = crate::channel::Channel::new(
            ch_config.name.clone(),
            context,
            RENDER_WIDTH,
            RENDER_HEIGHT,
        )?;
        channel.opacity = ch_config.opacity;
        channel.blend_mode = ch_config.blend_mode.into();

        for deck_config in &ch_config.decks {
            match restore_deck(deck_config, context, registry, camera_manager) {
                Ok(deck) => {
                    let mut slot = crate::channel::DeckSlot::new(deck);
                    slot.opacity = deck_config.opacity;
                    slot.blend_mode = deck_config.blend_mode.into();
                    slot.mute = deck_config.mute;
                    slot.solo = deck_config.solo;
                    slot.z_index = deck_config.z_index;
                    channel.add_deck_slot(slot);
                }
                Err(e) => {
                    let msg = format!("Failed to restore deck '{}': {}", deck_config.name, e);
                    log::warn!("{}", msg);
                    warnings.push(msg);
                }
            }
        }

        // Restore channel effects
        for eff_config in &ch_config.effects {
            match restore_effect(eff_config, context, context.surface_config.format) {
                Ok(eff) => channel.add_effect(eff),
                Err(e) => {
                    let msg = format!("Failed to restore channel effect '{}': {}", eff_config.path, e);
                    log::warn!("{}", msg);
                    warnings.push(msg);
                }
            }
        }

        mixer.channels.push(channel);
    }

    // Restore master effects
    for eff_config in &config.master_effects {
        match restore_effect(eff_config, context, context.surface_config.format) {
            Ok(eff) => mixer.master_effects.push(eff),
            Err(e) => {
                let msg = format!("Failed to restore master effect '{}': {}", eff_config.path, e);
                log::warn!("{}", msg);
                warnings.push(msg);
            }
        }
    }

    // Restore crossfader
    mixer.crossfader = config.crossfader;

    // Restore modulation engine
    mixer.modulation = config.modulation.clone();

    // Restore active transition
    if let Some(transition_name) = &config.active_transition {
        if let Some(shader) = registry.transitions().iter().find(|s| s.name() == *transition_name) {
            match mixer.set_transition(context, (*shader).clone()) {
                Ok(()) => {}
                Err(e) => {
                    let msg = format!("Failed to restore transition '{}': {}", transition_name, e);
                    log::warn!("{}", msg);
                    warnings.push(msg);
                }
            }
        } else {
            warnings.push(format!("Transition '{}' not found in registry", transition_name));
        }
    }

    Ok(RestoreResult {
        mixer,
        warnings,
    })
}

/// Restore a single deck from config.
fn restore_deck(
    config: &DeckConfig,
    context: &RenderContext,
    _registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
) -> Result<Deck> {
    let mut deck = match &config.source {
        SourceConfig::Shader { path, params } => {
            let shader = ISFShader::from_file(path)
                .with_context(|| format!("Failed to load shader: {}", path))?;
            let mut deck = Deck::new(context, shader, RENDER_WIDTH, RENDER_HEIGHT)?;
            // Restore parameter values
            for (name, value) in params {
                deck.generator_params.set(name, *value);
            }
            deck
        }
        SourceConfig::Video { path } => {
            Deck::new_from_video(context, path, RENDER_WIDTH, RENDER_HEIGHT)?
        }
        SourceConfig::Image { path } => {
            Deck::new_from_image(context, path, RENDER_WIDTH, RENDER_HEIGHT)?
        }
        SourceConfig::SolidColor { color } => {
            Deck::new_solid_color(context, *color, RENDER_WIDTH, RENDER_HEIGHT)?
        }
        SourceConfig::Camera { name } => {
            // Find the camera by name in the manager's device list
            let device = camera_manager.devices().iter()
                .find(|d| d.name == *name)
                .ok_or_else(|| anyhow::anyhow!("Camera '{}' not found — is it connected?", name))?;
            let camera_id = device.id;
            let cam_name = device.name.clone();

            let (src_w, src_h) = camera_manager.open_camera(camera_id, &context.device)
                .with_context(|| format!("Failed to open camera '{}'", name))?;

            Deck::new_from_camera(context, camera_id, &cam_name, src_w, src_h, RENDER_WIDTH, RENDER_HEIGHT)?
        }
    };

    // Restore effects
    for eff_config in &config.effects {
        match restore_effect(eff_config, context, wgpu::TextureFormat::Rgba8Unorm) {
            Ok(eff) => deck.effects.push(eff),
            Err(e) => log::warn!("Failed to restore deck effect '{}': {}", eff_config.path, e),
        }
    }

    Ok(deck)
}

/// Restore a single effect from config.
/// `target_format` should be `Rgba8Unorm` for deck effects,
/// or `context.surface_config.format` for channel/master effects.
fn restore_effect(config: &EffectConfig, context: &RenderContext, target_format: wgpu::TextureFormat) -> Result<Effect> {
    let shader = ISFShader::from_file(&config.path)
        .with_context(|| format!("Failed to load effect shader: {}", config.path))?;
    let mut effect = Effect::new_with_format(context, shader, target_format)?;
    effect.enabled = config.enabled;
    // Restore parameter values
    for (name, value) in &config.params {
        effect.params.set(name, *value);
    }
    Ok(effect)
}
