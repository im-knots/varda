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

    /// Path to `controllers/` directory for MIDI controller profiles.
    pub fn controllers_dir(&self) -> PathBuf {
        self.varda_dir().join("controllers")
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
use crate::renderer::context::{OutputTarget, UnifiedOutput, RecordingCodec};
use crate::scene::*;

/// Build a SceneConfig snapshot from live app state (show-specific: channels, effects, modulation).
pub fn snapshot_scene(
    mixer: &Mixer,
    render_width: u32,
    render_height: u32,
) -> SceneConfig {
    let channels = mixer.channels().iter().map(|ch| {
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
                "ndi" => {
                    // Store the NDI source name (strip the 📡 prefix we add)
                    let name = slot.deck.source_name()
                        .trim_start_matches("📡 ")
                        .to_string();
                    SourceConfig::Ndi { name }
                }
                "syphon" => {
                    // Store the Syphon server name (strip the 🔗 prefix we add)
                    let name = slot.deck.source_name()
                        .trim_start_matches("🔗 ")
                        .to_string();
                    SourceConfig::Syphon { name }
                }
                "srt" => {
                    // Store the SRT URL (strip the 📺 prefix we add)
                    let url = slot.deck.source_name()
                        .trim_start_matches("📺 ")
                        .to_string();
                    // Determine mode from the deck source
                    let mode = "caller".to_string(); // Default; actual mode stored in manager
                    SourceConfig::Srt { url, mode }
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

            // Snapshot auto-transition config
            let auto_transition = slot.auto_transition.as_ref()
                .filter(|at| at.enabled)
                .map(|at| {
                    use crate::channel::{DurationSpec, TransitionTrigger};
                    AutoTransitionConfig {
                        enabled: at.enabled,
                        trigger: match at.trigger {
                            TransitionTrigger::Timer => TriggerConfig::Timer,
                            TransitionTrigger::ClipEnd => TriggerConfig::ClipEnd,
                        },
                        play_duration: match at.play_duration {
                            DurationSpec::Beats(v) => DurationSpecConfig::Beats(v),
                            DurationSpec::Seconds(v) => DurationSpecConfig::Seconds(v),
                        },
                        transition_duration: match at.transition_duration {
                            DurationSpec::Beats(v) => DurationSpecConfig::Beats(v),
                            DurationSpec::Seconds(v) => DurationSpecConfig::Seconds(v),
                        },
                        transition_shader: at.transition_shader_name.clone(),
                    }
                });

            Some(DeckConfig {
                name: slot.deck.source_name().to_string(),
                source,
                effects,
                opacity: slot.opacity,
                blend_mode: slot.blend_mode.into(),
                mute: slot.mute,
                solo: slot.solo,
                z_index: slot.z_index,
                auto_transition,
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

    let master_effects = mixer.master_effects().iter().map(|eff| {
        EffectConfig {
            path: eff.shader.file_path.clone().unwrap_or_default(),
            enabled: eff.enabled,
            params: eff.params.values.clone(),
        }
    }).collect();

    let active_transition = mixer.active_transition().as_ref().map(|t| t.name.clone());

    // Snapshot transition sequences
    let transition_sequences = mixer.transition_sequences().iter().map(|seq| {
        use crate::channel::DurationSpec;
        use crate::scene::{TransitionSequenceConfig, TransitionStepConfig, DurationSpecConfig};
        TransitionSequenceConfig {
            name: seq.name.clone(),
            enabled: seq.enabled,
            steps: seq.steps.iter().map(|step| match &step.kind {
                crate::mixer::StepKind::Fade { from_ch, to_ch, duration, easing, transition_shader } => {
                    TransitionStepConfig::Fade {
                        from_ch: *from_ch,
                        to_ch: *to_ch,
                        duration: match duration {
                            DurationSpec::Beats(v) => DurationSpecConfig::Beats(*v),
                            DurationSpec::Seconds(v) => DurationSpecConfig::Seconds(*v),
                        },
                        easing: (*easing).into(),
                        transition_shader: transition_shader.clone(),
                    }
                }
                crate::mixer::StepKind::Wait { duration } => {
                    TransitionStepConfig::Wait {
                        duration: match duration {
                            DurationSpec::Beats(v) => DurationSpecConfig::Beats(*v),
                            DurationSpec::Seconds(v) => DurationSpecConfig::Seconds(*v),
                        },
                    }
                }
                crate::mixer::StepKind::GoTo { step_index } => {
                    TransitionStepConfig::GoTo { step_index: *step_index }
                }
            }).collect(),
        }
    }).collect();

    SceneConfig {
        version: 2,
        channels,
        crossfader: mixer.crossfader(),
        active_transition,
        master_effects,
        modulation: mixer.modulation().clone(),
        transition_sequences,
        render_width: Some(render_width),
        render_height: Some(render_height),
    }
}


/// Convert a live OutputTarget to a serializable OutputTargetConfig.
fn target_to_config(target: &OutputTarget) -> OutputTargetConfig {
    match target {
        OutputTarget::Windowed => OutputTargetConfig::Windowed,
        OutputTarget::Display { name, .. } => OutputTargetConfig::Display { name: name.clone() },
        OutputTarget::Recording { path, codec } => OutputTargetConfig::Recording {
            path: path.clone(),
            codec: codec.to_string(),
        },
        OutputTarget::SrtStream { url } => OutputTargetConfig::SrtStream { url: url.clone() },
        OutputTarget::NdiSend { sender_name } => OutputTargetConfig::NdiSend { sender_name: sender_name.clone() },
        OutputTarget::SyphonServer { server_name } => OutputTargetConfig::SyphonServer { server_name: server_name.clone() },
    }
}

/// Convert a serializable OutputTargetConfig back to a live OutputTarget.
/// Public variant for use from outputs.rs.
pub fn config_to_target_pub(config: &OutputTargetConfig) -> OutputTarget {
    config_to_target(config)
}

fn config_to_target(config: &OutputTargetConfig) -> OutputTarget {
    match config {
        OutputTargetConfig::Windowed => OutputTarget::Windowed,
        OutputTargetConfig::Display { name } => OutputTarget::Display {
            name: name.clone(),
            monitor_index: 0, // Will be matched at runtime
        },
        OutputTargetConfig::Recording { path, codec } => OutputTarget::Recording {
            path: path.clone(),
            codec: match codec.as_str() {
                "prores" | "ProRes" => RecordingCodec::ProRes,
                "hapq" | "HapQ" => RecordingCodec::HapQ,
                _ => RecordingCodec::H264,
            },
        },
        OutputTargetConfig::SrtStream { url } => OutputTarget::SrtStream { url: url.clone() },
        OutputTargetConfig::NdiSend { sender_name } => OutputTarget::NdiSend { sender_name: sender_name.clone() },
        OutputTargetConfig::SyphonServer { server_name } => OutputTarget::SyphonServer { server_name: server_name.clone() },
    }
}

/// Build a StagePrefs snapshot from live app state (venue-specific: surfaces, outputs, editor prefs).
pub fn snapshot_stage(
    surface_manager: &crate::surface::SurfaceManager,
    outputs_list: &[UnifiedOutput],
    grid_size: f32,
    snap: bool,
    library_panel_open: bool,
    stage_editor_open: bool,
) -> StagePrefs {
    let outputs = outputs_list.iter().map(|unified| {
        let (name, target, surface_assignments) = match unified {
            UnifiedOutput::Window(w) => (
                w.name.clone(),
                target_to_config(&w.target),
                w.surface_assignments.iter().map(|a| {
                    SurfaceAssignmentConfig {
                        surface_idx: a.surface_idx,
                        warp_corners: a.warp_corners,
                        enabled: a.enabled,
                    }
                }).collect(),
            ),
            UnifiedOutput::Headless(h) => (
                h.name.clone(),
                target_to_config(&h.target),
                Vec::new(), // Headless outputs don't have surface assignments
            ),
        };
        OutputConfig {
            name,
            target: target,
            target_display: None,
            surface_assignments,
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
use crate::renderer::GpuContext;

/// Restore result — contains reconstructed mixer.
/// Surfaces and outputs are loaded separately from stage.json.
pub struct RestoreResult {
    pub mixer: Mixer,
    pub warnings: Vec<String>,
}

/// Reconstruct live state from a SceneConfig.
pub fn restore_scene(
    config: &SceneConfig,
    context: &GpuContext,
    registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
    ndi_manager: &mut crate::ndi::NdiManager,
    srt_manager: &mut crate::srt::SrtManager,
    render_width: u32,
    render_height: u32,
) -> Result<RestoreResult> {
    let mut warnings = Vec::new();
    let mut mixer = Mixer::new(context, render_width, render_height)?;

    // Clear default channels — we'll create from config
    mixer.channels_mut().clear();

    for ch_config in &config.channels {
        let mut channel = crate::channel::Channel::new(
            ch_config.name.clone(),
            context,
            render_width,
            render_height,
        )?;
        channel.opacity = ch_config.opacity;
        channel.blend_mode = ch_config.blend_mode.into();

        for deck_config in &ch_config.decks {
            match restore_deck(deck_config, context, registry, camera_manager, ndi_manager, srt_manager, render_width, render_height) {
                Ok(deck) => {
                    let mut slot = crate::channel::DeckSlot::new(deck);
                    slot.opacity = deck_config.opacity;
                    slot.blend_mode = deck_config.blend_mode.into();
                    slot.mute = deck_config.mute;
                    slot.solo = deck_config.solo;
                    slot.z_index = deck_config.z_index;

                    // Restore auto-transition config
                    if let Some(at_config) = &deck_config.auto_transition {
                        use crate::channel::{DeckAutoTransition, DurationSpec, TransitionTrigger};
                        let mut at = DeckAutoTransition::new();
                        at.enabled = at_config.enabled;
                        at.trigger = match at_config.trigger {
                            TriggerConfig::Timer => TransitionTrigger::Timer,
                            TriggerConfig::ClipEnd => TransitionTrigger::ClipEnd,
                        };
                        at.play_duration = match at_config.play_duration {
                            DurationSpecConfig::Beats(v) => DurationSpec::Beats(v),
                            DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(v),
                        };
                        at.transition_duration = match at_config.transition_duration {
                            DurationSpecConfig::Beats(v) => DurationSpec::Beats(v),
                            DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(v),
                        };
                        at.transition_shader_name = at_config.transition_shader.clone();
                        slot.auto_transition = Some(at);

                        // Compile transition shader if specified
                        if let Some(shader_name) = &at_config.transition_shader {
                            if let Some(shader) = registry.transitions().iter()
                                .find(|s| s.name() == *shader_name)
                            {
                                if let Err(e) = slot.set_transition_shader(context, (*shader).clone()) {
                                    log::warn!("Failed to restore deck transition shader '{}': {}", shader_name, e);
                                }
                            } else {
                                log::warn!("Deck transition shader '{}' not found in registry", shader_name);
                            }
                        }
                    }

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
            match restore_effect(eff_config, context, context.texture_format) {
                Ok(eff) => channel.add_effect(eff),
                Err(e) => {
                    let msg = format!("Failed to restore channel effect '{}': {}", eff_config.path, e);
                    log::warn!("{}", msg);
                    warnings.push(msg);
                }
            }
        }

        mixer.channels_mut().push(channel);
    }

    // Update next_channel_index so new channels don't get duplicate names.
    // Parse existing channel names to find the highest "Ch N" index.
    let max_idx = mixer.channels().iter()
        .filter_map(|ch| ch.name.strip_prefix("Ch ").and_then(|s| s.parse::<usize>().ok()))
        .max()
        .map(|n| n + 1)
        .unwrap_or(mixer.channel_count());
    mixer.set_next_channel_index(max_idx);

    // Restore master effects
    for eff_config in &config.master_effects {
        match restore_effect(eff_config, context, context.texture_format) {
            Ok(eff) => mixer.master_effects_mut().push(eff),
            Err(e) => {
                let msg = format!("Failed to restore master effect '{}': {}", eff_config.path, e);
                log::warn!("{}", msg);
                warnings.push(msg);
            }
        }
    }

    // Restore crossfader
    mixer.set_crossfader(config.crossfader);

    // Restore modulation engine
    mixer.set_modulation(config.modulation.clone());

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

    // Restore transition sequences
    for seq_config in &config.transition_sequences {
        use crate::channel::DurationSpec;
        use crate::mixer::{TransitionSequence, TransitionStep, StepKind, SequencerState};
        use crate::scene::{TransitionStepConfig, DurationSpecConfig};
        let steps = seq_config.steps.iter().map(|step| {
            let kind = match step {
                TransitionStepConfig::Fade { from_ch, to_ch, duration, easing, transition_shader } => {
                    StepKind::Fade {
                        from_ch: *from_ch,
                        to_ch: *to_ch,
                        duration: match duration {
                            DurationSpecConfig::Beats(v) => DurationSpec::Beats(*v),
                            DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(*v),
                        },
                        easing: (*easing).into(),
                        transition_shader: transition_shader.clone(),
                    }
                }
                TransitionStepConfig::Wait { duration } => {
                    StepKind::Wait {
                        duration: match duration {
                            DurationSpecConfig::Beats(v) => DurationSpec::Beats(*v),
                            DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(*v),
                        },
                    }
                }
                TransitionStepConfig::GoTo { step_index } => {
                    StepKind::GoTo { step_index: *step_index }
                }
            };
            TransitionStep { kind }
        }).collect();
        mixer.transition_sequences_mut().push(TransitionSequence {
            name: seq_config.name.clone(),
            steps,
            enabled: seq_config.enabled,
            state: SequencerState::new(),
        });
    }

    Ok(RestoreResult {
        mixer,
        warnings,
    })
}

/// Restore a single deck from config.
fn restore_deck(
    config: &DeckConfig,
    context: &GpuContext,
    _registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
    ndi_manager: &mut crate::ndi::NdiManager,
    srt_manager: &mut crate::srt::SrtManager,
    render_width: u32,
    render_height: u32,
) -> Result<Deck> {
    let mut deck = match &config.source {
        SourceConfig::Shader { path, params } => {
            let shader = ISFShader::from_file(path)
                .with_context(|| format!("Failed to load shader: {}", path))?;
            let mut deck = Deck::new(context, shader, render_width, render_height)?;
            // Restore parameter values
            for (name, value) in params {
                deck.generator_params.set(name, *value);
            }
            deck
        }
        SourceConfig::Video { path } => {
            Deck::new_from_video(context, path, render_width, render_height)?
        }
        SourceConfig::Image { path } => {
            Deck::new_from_image(context, path, render_width, render_height)?
        }
        SourceConfig::SolidColor { color } => {
            Deck::new_solid_color(context, *color, render_width, render_height)?
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

            Deck::new_from_camera(context, camera_id, &cam_name, src_w, src_h, render_width, render_height)?
        }
        SourceConfig::Ndi { name } => {
            match ndi_manager.start_receive(name, &context.device) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = ndi_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                    Deck::new_from_ndi(context, receiver_idx, name, src_w, src_h, render_width, render_height)?
                }
                None => {
                    return Err(anyhow::anyhow!("NDI source '{}' not available for restore", name));
                }
            }
        }
        SourceConfig::Syphon { name } => {
            // Syphon sources are resolved at runtime — skip if not on macOS
            log::warn!("Syphon source '{}' restoration not yet implemented (needs SyphonManager)", name);
            return Err(anyhow::anyhow!("Syphon source '{}' not available for restore", name));
        }
        SourceConfig::Srt { url, mode } => {
            let srt_mode = match mode.as_str() {
                "listener" => crate::srt::SrtMode::Listener,
                _ => crate::srt::SrtMode::Caller,
            };
            match srt_manager.start_receive(url, srt_mode, &context.device) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = srt_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                    Deck::new_from_srt(context, receiver_idx, url, src_w, src_h, render_width, render_height)?
                }
                None => {
                    return Err(anyhow::anyhow!("SRT source '{}' not available for restore", url));
                }
            }
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
/// or `context.texture_format` for channel/master effects.
fn restore_effect(config: &EffectConfig, context: &GpuContext, target_format: wgpu::TextureFormat) -> Result<Effect> {
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
