//! Workspace persistence in `.varda/`:
//! - `scene.json` — channels, decks, effects, modulation (show-specific, shareable)
//! - `stage.json` — surfaces, outputs, warp, editor prefs (venue-specific)
//! - `midi.json`  — MIDI controller mappings (device-name-keyed)
//! - `presets/`   — saved deck and channel presets

pub mod presets;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Venue data in `.varda/stage.json`: surfaces, outputs, and editor
/// preferences. Separate from scene.json so scenes can be shared without stage geometry.
// Independent persisted toggles; bundling them would change the JSON.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePrefs {
    #[serde(default = "default_grid_size")]
    pub grid_size: f32,
    #[serde(default = "default_true")]
    pub snap: bool,
    #[serde(default)]
    pub library_panel_open: bool,
    #[serde(default = "default_true")]
    pub right_panel_open: bool,
    #[serde(default)]
    pub stage_editor_open: bool,
    #[serde(default)]
    pub dome_preview_open: bool,
    /// Whether the stage editor is in 3D Dome mode.
    #[serde(default)]
    pub dome_mode_active: bool,
    /// Active dome preset.
    #[serde(default = "default_dome_preset")]
    pub dome_preset: crate::renderer::slicer::DomePreset,
    /// Active dome geometry.
    #[serde(default)]
    pub dome_geometry: crate::renderer::slicer::DomeGeometry,
    /// Domemaster render size. Belongs to the dome, so it is stage data.
    #[serde(default)]
    pub domemaster_resolution: crate::renderer::dome::DomemasterResolution,
    /// 2D stage surface layout.
    #[serde(default)]
    pub surfaces: crate::surface::SurfaceManager,
    /// Output configurations.
    #[serde(default)]
    pub outputs: Vec<crate::scene::OutputConfig>,
    /// Which SMPTE input the transport follows, and where LTC is patched.
    #[serde(default)]
    pub timecode: crate::timecode::TimecodeConfig,
}

fn default_grid_size() -> f32 {
    0.05
}
fn default_true() -> bool {
    true
}
fn default_dome_preset() -> crate::renderer::slicer::DomePreset {
    crate::renderer::slicer::DomePreset::Quad
}

impl Default for StagePrefs {
    fn default() -> Self {
        Self {
            grid_size: 0.05,
            snap: true,
            library_panel_open: false,
            right_panel_open: true,
            stage_editor_open: false,
            dome_preview_open: false,
            dome_mode_active: false,
            dome_preset: crate::renderer::slicer::DomePreset::Quad,
            dome_geometry: crate::renderer::slicer::DomeGeometry::default(),
            domemaster_resolution: crate::renderer::dome::DomemasterResolution::default(),
            surfaces: crate::surface::SurfaceManager::default(),
            outputs: Vec::new(),
            timecode: crate::timecode::TimecodeConfig::default(),
        }
    }
}

impl StagePrefs {
    /// The stage editor preferences this file carries for the GUI.
    pub fn editor_prefs(&self) -> crate::engine::value::editor::EditorPrefs {
        crate::engine::value::editor::EditorPrefs {
            grid_size: self.grid_size,
            snap: self.snap,
            library_panel_open: self.library_panel_open,
            right_panel_open: self.right_panel_open,
            stage_editor_open: self.stage_editor_open,
            dome_preview_open: self.dome_preview_open,
            dome_mode_active: self.dome_mode_active,
        }
    }

    /// The dome projection this file carries.
    pub fn dome_config(&self) -> crate::engine::value::dome::DomeConfig {
        crate::engine::value::dome::DomeConfig {
            preset: self.dome_preset,
            geometry: self.dome_geometry,
        }
    }

    /// Validate stage prefs. Returns a list of errors; empty means valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if !self.grid_size.is_finite() || self.grid_size <= 0.0 {
            errors.push(format!(
                "grid_size {} must be > 0 and finite",
                self.grid_size
            ));
        }
        for (i, output) in self.outputs.iter().enumerate() {
            let prefix = format!("outputs[{i}]");
            if output.name.trim().is_empty() {
                errors.push(format!("{prefix}: name is empty"));
            }
        }
        // Surface warp corner pins must be finite.
        for (i, surface) in self.surfaces.surfaces.iter().enumerate() {
            if let Some(crate::surface::warp::WarpMode::CornerPin { corners }) = &surface.warp {
                for (c, corner) in corners.iter().enumerate() {
                    for (k, v) in corner.iter().enumerate() {
                        if !v.is_finite() {
                            errors.push(format!(
                                "surfaces[{i}]: warp corner[{c}][{k}] is not finite"
                            ));
                        }
                    }
                }
            }
        }
        errors
    }

    /// Load stage prefs from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` cannot be read or is not valid [`StagePrefs`]
    /// JSON. Validation problems are only logged.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read stage prefs: {}", path.as_ref().display()))?;
        let prefs: StagePrefs = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse stage prefs: {}", path.as_ref().display()))?;
        let warnings = prefs.validate();
        for w in &warnings {
            log::warn!("Stage prefs {}: {}", path.as_ref().display(), w);
        }
        Ok(prefs)
    }

    /// Save stage prefs to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the atomic write fails.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Stage prefs save: {e}");
        }
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize stage prefs")?;
        crate::files::atomic_write(path.as_ref(), &content)?;
        Ok(())
    }
}

/// `.varda/` paths and directory creation.
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Create a workspace rooted at the given directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Create a workspace rooted at the current working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the current directory cannot be determined.
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

    /// Path to the `luts/` directory inside `.varda/`.
    pub fn luts_dir(&self) -> PathBuf {
        self.varda_dir().join("luts")
    }

    /// Path to `keymap.json`.
    pub fn keymap_path(&self) -> PathBuf {
        self.varda_dir().join("keymap.json")
    }

    /// Path to `osc.json`.
    pub fn osc_path(&self) -> PathBuf {
        self.varda_dir().join("osc.json")
    }

    /// Check if a keymap config file exists.
    pub fn has_keymap(&self) -> bool {
        self.keymap_path().is_file()
    }

    /// Check if an OSC config file exists.
    pub fn has_osc(&self) -> bool {
        self.osc_path().is_file()
    }

    /// Path to the MIDI `controller-profiles/` directory.
    pub fn controller_profiles_dir(&self) -> PathBuf {
        self.varda_dir().join("controller-profiles")
    }

    /// Path to `presets/` directory.
    pub fn presets_dir(&self) -> PathBuf {
        self.varda_dir().join("presets")
    }

    /// Path to `shaders/` directory for workspace-local ISF shaders.
    pub fn shaders_dir(&self) -> PathBuf {
        self.varda_dir().join("shaders")
    }

    /// Path to `presets/decks/` directory.
    pub fn deck_presets_dir(&self) -> PathBuf {
        self.presets_dir().join("decks")
    }

    /// Path to `presets/channels/` directory.
    pub fn channel_presets_dir(&self) -> PathBuf {
        self.presets_dir().join("channels")
    }

    /// Ensure preset directories exist.
    ///
    /// # Errors
    ///
    /// Returns an error if `.varda/`, `presets/decks/` or `presets/channels/`
    /// cannot be created.
    pub fn ensure_preset_dirs(&self) -> Result<()> {
        self.ensure_dir()?;
        let dirs = [self.deck_presets_dir(), self.channel_presets_dir()];
        for dir in &dirs {
            if !dir.exists() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("Failed to create preset dir: {}", dir.display()))?;
            }
        }
        Ok(())
    }

    /// Whether `.varda/` exists in this workspace.
    pub fn exists(&self) -> bool {
        self.varda_dir().is_dir()
    }

    /// Ensure the `.varda/` directory exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
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
use crate::scene::{
    AutoTransitionConfig, ChannelConfig, DeckConfig, EffectConfig, OutputConfig, SceneConfig,
    SourceConfig, SurfaceAssignmentConfig, TriggerConfig,
};

// ── DurationSpec ↔ DurationSpecConfig helpers ───────────────────────

fn duration_spec_to_config(
    spec: &crate::channel::DurationSpec,
) -> crate::scene::DurationSpecConfig {
    use crate::channel::DurationSpec;
    use crate::scene::DurationSpecConfig;
    match spec {
        DurationSpec::Beats(v) => DurationSpecConfig::Beats(*v),
        DurationSpec::Seconds(v) => DurationSpecConfig::Seconds(*v),
        DurationSpec::Minutes(v) => DurationSpecConfig::Minutes(*v),
        DurationSpec::Hours(v) => DurationSpecConfig::Hours(*v),
    }
}

fn duration_config_to_spec(
    config: &crate::scene::DurationSpecConfig,
) -> crate::channel::DurationSpec {
    use crate::channel::DurationSpec;
    use crate::scene::DurationSpecConfig;
    match config {
        DurationSpecConfig::Beats(v) => DurationSpec::Beats(*v),
        DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(*v),
        DurationSpecConfig::Minutes(v) => DurationSpec::Minutes(*v),
        DurationSpecConfig::Hours(v) => DurationSpec::Hours(*v),
    }
}

/// Convert saved sequence steps into runtime steps, resolving fade channel
/// references against the restored channels.
///
/// A fade whose channels don't resolve becomes a `Wait` of the same duration,
/// since `GoTo` steps address steps by position.
pub fn restore_sequence_steps(
    steps: &[crate::scene::TransitionStepConfig],
    channel_uuids: &[String],
    warnings: &mut Vec<String>,
) -> Vec<crate::mixer::TransitionStep> {
    use crate::mixer::{StepKind, TransitionStep};
    use crate::scene::TransitionStepConfig;

    steps
        .iter()
        .map(|step| {
            let kind = match step {
                TransitionStepConfig::Fade {
                    from_ch,
                    to_ch,
                    duration,
                    easing,
                    transition_shader,
                    target_amount,
                } => {
                    let resolved = from_ch
                        .resolve(channel_uuids)
                        .zip(to_ch.resolve(channel_uuids))
                        .filter(|(from, to)| {
                            channel_uuids.contains(from) && channel_uuids.contains(to)
                        });
                    if let Some((from_ch, to_ch)) = resolved {
                        StepKind::Fade {
                            from_ch,
                            to_ch,
                            duration: duration_config_to_spec(duration),
                            easing: (*easing).into(),
                            transition_shader: transition_shader.clone(),
                            target_amount: *target_amount,
                        }
                    } else {
                        let msg = "Fade step references a channel that no longer exists; \
                                   kept as a wait so later GoTo targets stay valid"
                            .to_string();
                        log::warn!("{msg}");
                        warnings.push(msg);
                        StepKind::Wait {
                            duration: duration_config_to_spec(duration),
                        }
                    }
                }
                TransitionStepConfig::Wait { duration } => StepKind::Wait {
                    duration: duration_config_to_spec(duration),
                },
                TransitionStepConfig::GoTo { step_index } => StepKind::GoTo {
                    step_index: *step_index,
                },
            };
            TransitionStep { kind }
        })
        .collect()
}

/// Build a `SceneConfig` from live state. Only the transport's authored
/// settings (frame rate, loop range) are saved, not position or run state.
pub fn snapshot_scene(
    mixer: &Mixer,
    transport: Option<&crate::scene::TransportConfig>,
    render_width: u32,
    render_height: u32,
) -> SceneConfig {
    let channels = mixer
        .channels()
        .iter()
        .map(|ch| {
            let decks = ch
                .decks
                .iter()
                .map(|slot| {
                    let source = slot.deck.source_config();

                    let effects = slot
                        .deck
                        .effects
                        .iter()
                        .map(|eff| EffectConfig {
                            uuid: eff.uuid().to_owned(),
                            path: eff.shader.file_path.clone().unwrap_or_default(),
                            enabled: eff.enabled,
                            params: eff.params.values.clone(),
                        })
                        .collect();

                    let auto_transition = slot
                        .auto_transition
                        .as_ref()
                        .filter(|at| at.enabled)
                        .map(|at| {
                            use crate::channel::TransitionTrigger;
                            AutoTransitionConfig {
                                enabled: at.enabled,
                                trigger: match at.trigger {
                                    TransitionTrigger::Timer => TriggerConfig::Timer,
                                    TransitionTrigger::ClipEnd => TriggerConfig::ClipEnd,
                                },
                                play_duration: duration_spec_to_config(&at.play_duration),
                                transition_duration: duration_spec_to_config(
                                    &at.transition_duration,
                                ),
                                transition_shader: at.transition_shader_name.clone(),
                            }
                        });

                    DeckConfig {
                        uuid: slot.deck.uuid().to_string(),
                        name: slot.deck.source_name().to_string(),
                        source,
                        effects,
                        opacity: slot.opacity,
                        transparent: slot.deck.transparent(),
                        blend_mode: slot.blend_mode.into(),
                        mute: slot.mute,
                        solo: slot.solo,
                        z_index: slot.z_index,
                        render_fps: slot.render_fps,
                        auto_transition,
                        modulation: vec![],
                    }
                })
                .collect();

            let effects = ch
                .effects
                .iter()
                .map(|eff| EffectConfig {
                    uuid: eff.uuid().to_owned(),
                    path: eff.shader.file_path.clone().unwrap_or_default(),
                    enabled: eff.enabled,
                    params: eff.params.values.clone(),
                })
                .collect();

            ChannelConfig {
                uuid: ch.uuid().to_string(),
                name: ch.name.clone(),
                opacity: ch.opacity,
                blend_mode: ch.blend_mode.into(),
                decks,
                effects,
                // A scene saves the whole modulation engine; recipes are only
                // for a channel copied or saved alone.
                modulation: Vec::new(),
            }
        })
        .collect();

    let master_effects = mixer
        .master_effects()
        .iter()
        .map(|eff| EffectConfig {
            uuid: eff.uuid().to_owned(),
            path: eff.shader.file_path.clone().unwrap_or_default(),
            enabled: eff.enabled,
            params: eff.params.values.clone(),
        })
        .collect();

    let active_transition = mixer.active_transition().as_ref().map(|t| t.name.clone());

    let transition_sequences = mixer
        .transition_sequences()
        .iter()
        .map(|seq| {
            use crate::scene::{TransitionSequenceConfig, TransitionStepConfig};
            TransitionSequenceConfig {
                uuid: seq.uuid.clone(),
                name: seq.name.clone(),
                enabled: seq.enabled,
                steps: seq
                    .steps
                    .iter()
                    .map(|step| match &step.kind {
                        crate::mixer::StepKind::Fade {
                            from_ch,
                            to_ch,
                            duration,
                            easing,
                            transition_shader,
                            target_amount,
                        } => TransitionStepConfig::Fade {
                            from_ch: from_ch.clone().into(),
                            to_ch: to_ch.clone().into(),
                            duration: duration_spec_to_config(duration),
                            easing: (*easing).into(),
                            transition_shader: transition_shader.clone(),
                            target_amount: *target_amount,
                        },
                        crate::mixer::StepKind::Wait { duration } => TransitionStepConfig::Wait {
                            duration: duration_spec_to_config(duration),
                        },
                        crate::mixer::StepKind::GoTo { step_index } => TransitionStepConfig::GoTo {
                            step_index: *step_index,
                        },
                    })
                    .collect(),
            }
        })
        .collect();

    SceneConfig {
        version: SceneConfig::CURRENT_VERSION,
        channels,
        crossfader: mixer.crossfader(),
        active_transition,
        master_effects,
        modulation: mixer.modulation().clone(),
        macros: mixer.macros().clone(),
        transition_sequences,
        render_width: Some(render_width),
        render_height: Some(render_height),
        tonemap_mode: mixer.tonemap_mode(),
        active_lut: mixer
            .active_lut_filename()
            .map(std::string::ToString::to_string),
        arrangement: mixer.arrangement().cloned(),
        transport: transport.cloned().unwrap_or_default(),
    }
}

/// Build a `StagePrefs` from live state.
pub fn snapshot_stage(
    surface_manager: &crate::surface::SurfaceManager,
    outputs_list: &[crate::output::Output],
    editor: &crate::engine::value::editor::EditorPrefs,
    dome: &crate::engine::value::dome::DomeConfig,
    domemaster_resolution: crate::renderer::dome::DomemasterResolution,
) -> StagePrefs {
    let outputs = outputs_list
        .iter()
        .map(|output| OutputConfig {
            uuid: output.uuid.clone(),
            name: output.name.clone(),
            // A window's position and size are part of its sink's settings.
            target: output.sink().config(),
            target_display: None,
            surface_assignments: output
                .surface_assignments
                .iter()
                .map(|a| SurfaceAssignmentConfig {
                    surface_uuid: a.surface_uuid.clone(),
                    legacy_warp_mode: None,
                    enabled: a.enabled,
                })
                .collect(),
            window_position: None,
            window_size: None,
            edge_blend_mode: output.edge_blend_mode,
            edge_blend: output.edge_blend,
            rotation: output.rotation,
            presentation: output.presentation_request(),
            tonemap_override: output.tonemap_override,
            calibration_mode: output.calibration_mode,
            unassigned: output.unassigned,
        })
        .collect();

    StagePrefs {
        grid_size: editor.grid_size,
        snap: editor.snap,
        library_panel_open: editor.library_panel_open,
        right_panel_open: editor.right_panel_open,
        stage_editor_open: editor.stage_editor_open,
        dome_preview_open: editor.dome_preview_open,
        dome_mode_active: editor.dome_mode_active,
        dome_preset: dome.preset,
        dome_geometry: dome.geometry,
        domemaster_resolution,
        surfaces: surface_manager.clone(),
        outputs,
        // Filled by the caller.
        timecode: crate::timecode::TimecodeConfig::default(),
    }
}

// ── Restore: Config → Live State ────────────────────────────────────

use crate::deck::{Deck, Effect};
use crate::isf::ISFShader;
use crate::renderer::GpuContext;
/// The restored mixer. Surfaces and outputs load separately from stage.json.
pub struct RestoreResult {
    pub mixer: Mixer,
    pub warnings: Vec<String>,
}

/// Reconstruct live state from a `SceneConfig`.
///
/// # Errors
///
/// Returns an error if the mixer or a deck or effect cannot be built on the
/// GPU. Recoverable problems go to [`RestoreResult::warnings`].
// Writes into many independent live-state targets.
#[allow(clippy::too_many_arguments)]
pub fn restore_scene(
    config: &SceneConfig,
    sources: &mut crate::source::SourceRegistry,
    env: &mut crate::source::SourceEnv,
) -> Result<RestoreResult> {
    let mut warnings = Vec::new();
    let context = env.gpu;
    let registry = env.shaders;
    let (render_width, render_height) = (env.width, env.height);
    let mut mixer = Mixer::new(context, render_width, render_height)?;

    // Replace the default channels.
    mixer.channels_mut().clear();

    for ch_config in &config.channels {
        let mut channel = crate::channel::Channel::new(
            ch_config.name.clone(),
            context,
            render_width,
            render_height,
        )?;
        if !ch_config.uuid.is_empty() {
            channel.set_uuid(ch_config.uuid.clone());
        }
        channel.opacity = ch_config.opacity;
        channel.blend_mode = ch_config.blend_mode.into();

        for deck_config in &ch_config.decks {
            let (deck, warning) = restore_deck(deck_config, sources, env);
            if let Some(reason) = warning {
                let msg = format!(
                    "Deck '{}' kept as a placeholder: {reason}",
                    deck_config.name
                );
                log::warn!("{msg}");
                warnings.push(msg);
            }
            let mut slot = crate::channel::DeckSlot::new(deck);
            slot.opacity = deck_config.opacity;
            slot.deck.set_transparent(deck_config.transparent);
            slot.blend_mode = deck_config.blend_mode.into();
            slot.mute = deck_config.mute;
            slot.solo = deck_config.solo;
            slot.z_index = deck_config.z_index;
            slot.render_fps = deck_config.render_fps;

            if let Some(at_config) = &deck_config.auto_transition {
                use crate::channel::{DeckAutoTransition, TransitionTrigger};
                let mut at = DeckAutoTransition::new();
                at.enabled = at_config.enabled;
                at.trigger = match at_config.trigger {
                    TriggerConfig::Timer => TransitionTrigger::Timer,
                    TriggerConfig::ClipEnd => TransitionTrigger::ClipEnd,
                };
                at.play_duration = duration_config_to_spec(&at_config.play_duration);
                at.transition_duration = duration_config_to_spec(&at_config.transition_duration);
                at.transition_shader_name
                    .clone_from(&at_config.transition_shader);
                slot.auto_transition = Some(at);

                if let Some(shader_name) = &at_config.transition_shader {
                    if let Some(shader) = registry
                        .transitions()
                        .iter()
                        .find(|s| s.name() == *shader_name)
                    {
                        if let Err(e) = slot.set_transition_shader(context, (*shader).clone()) {
                            log::warn!(
                                "Failed to restore deck transition shader '{shader_name}': {e}"
                            );
                        }
                    } else {
                        log::warn!("Deck transition shader '{shader_name}' not found in registry");
                    }
                }
            }

            channel.add_deck_slot(slot);
        }

        for eff_config in &ch_config.effects {
            match restore_effect(eff_config, context, context.compositing_format) {
                Ok(eff) => channel.add_effect(eff),
                Err(e) => {
                    let msg = format!(
                        "Failed to restore channel effect '{}': {}",
                        eff_config.path, e
                    );
                    log::warn!("{msg}");
                    warnings.push(msg);
                }
            }
        }

        mixer.channels_mut().push(channel);
    }

    // Continue "Ch N" numbering after the highest restored index.
    let max_idx = mixer
        .channels()
        .iter()
        .filter_map(|ch| {
            ch.name
                .strip_prefix("Ch ")
                .and_then(|s| s.parse::<usize>().ok())
        })
        .max()
        .map_or(mixer.channel_count(), |n| n + 1);
    mixer.set_next_channel_index(max_idx);

    for eff_config in &config.master_effects {
        match restore_effect(eff_config, context, context.compositing_format) {
            Ok(eff) => mixer.master_effects_mut().push(eff),
            Err(e) => {
                let msg = format!(
                    "Failed to restore master effect '{}': {}",
                    eff_config.path, e
                );
                log::warn!("{msg}");
                warnings.push(msg);
            }
        }
    }

    mixer.set_crossfader(config.crossfader);

    mixer.set_modulation(config.modulation.clone());

    mixer.set_macros(config.macros.clone());

    // Also drops live overrides.
    mixer.set_arrangement(config.arrangement.clone());

    if let Some(transition_name) = &config.active_transition {
        if let Some(shader) = registry
            .transitions()
            .iter()
            .find(|s| s.name() == *transition_name)
        {
            match mixer.set_transition(context, (*shader).clone()) {
                Ok(()) => {}
                Err(e) => {
                    let msg = format!("Failed to restore transition '{transition_name}': {e}");
                    log::warn!("{msg}");
                    warnings.push(msg);
                }
            }
        } else {
            warnings.push(format!(
                "Transition '{transition_name}' not found in registry"
            ));
        }
    }

    // Fade steps address channels by UUID. Scenes at v4 and earlier store
    // indices, which `ChannelRef::resolve` maps through the restored order.
    let channel_uuids: Vec<String> = mixer
        .channels()
        .iter()
        .map(|ch| ch.uuid().to_string())
        .collect();
    for seq_config in &config.transition_sequences {
        let steps = restore_sequence_steps(&seq_config.steps, &channel_uuids, &mut warnings);
        mixer
            .transition_sequences_mut()
            .push(crate::mixer::TransitionSequence::with_uuid(
                seq_config.uuid.clone(),
                seq_config.name.clone(),
                steps,
                seq_config.enabled,
            ));
    }

    mixer.set_tonemap_mode(&context.queue, config.tonemap_mode);

    if let Some(lut_filename) = &config.active_lut {
        let lut_path = std::env::current_dir()
            .unwrap_or_default()
            .join(".varda/luts")
            .join(lut_filename);
        match crate::renderer::lut::parse_lut_file(&lut_path) {
            Ok(parsed) => {
                mixer.load_lut(
                    &context.device,
                    &context.queue,
                    &parsed,
                    lut_filename.clone(),
                );
                log::info!("Restored LUT: {lut_filename}");
            }
            Err(e) => {
                let msg = format!("Failed to restore LUT '{lut_filename}': {e}");
                log::warn!("{msg}");
                warnings.push(msg);
            }
        }
    }

    Ok(RestoreResult { mixer, warnings })
}

/// Reacquire and attach a shader deck's depth-sensor preprocessor on restore.
///
/// Resolves the sensor by saved name, else by the ISF header's device
/// selection (older scenes save no name). Returns `Err` when no sensor is
/// available, so the caller keeps a placeholder.
fn restore_depth_preprocessor(
    deck: &mut Deck,
    saved: Option<&crate::deck::DepthPreproConfig>,
    metadata: &crate::isf::ISFMetadata,
    shader_path: &str,
    env: &mut crate::source::SourceEnv,
) -> Result<()> {
    use crate::depth::preprocess::{DepthPreprocessParams, DepthPreprocessPipeline};

    if crate::depth::preprocess::requested_device(metadata).is_none() {
        return Ok(());
    }
    let context = env.gpu;
    let depth_manager = env
        .services
        .get_mut::<crate::depth::DepthSensorManager>()
        .context("depth_sensor preprocessor declared but no depth sensors are available")?;

    let params = saved.map_or_else(DepthPreprocessParams::default, |c| DepthPreprocessParams {
        near_mm: c.near_mm,
        far_mm: c.far_mm.max(c.near_mm + 1.0),
        smoothing: c.smoothing,
        hole_fill: c.hole_fill,
        mask_feather: c.mask_feather,
        motion_gain: c.motion_gain,
        mirror: c.mirror,
    });

    // Saved device name first, then the header's selection.
    let by_name = saved.and_then(|c| {
        depth_manager
            .devices()
            .iter()
            .find(|d| d.name == c.sensor_name)
            .cloned()
    });

    let (id, name, width, height) = if let Some(info) = by_name {
        let (w, h) = crate::depth::open_depth_sensor(depth_manager, info.id, &context.device)
            .with_context(|| format!("Failed to open depth sensor '{}'", info.name))?;
        (info.id, info.name, w, h)
    } else {
        let sensor = crate::depth::preprocess::acquire_for_shader(
            depth_manager,
            &context.device,
            metadata,
            shader_path,
        )?
        .context("depth_sensor preprocessor declared but not acquired")?;
        deck.attach_depth_preprocessor(sensor.id, sensor.name, sensor.pipeline, params);
        return Ok(());
    };

    deck.attach_depth_preprocessor(
        id,
        name,
        DepthPreprocessPipeline::new(&context.device, width, height),
        params,
    );
    Ok(())
}

/// Restore one deck from config through its source provider. A source that
/// cannot run here (unknown type, missing device) becomes a placeholder holding
/// its config, and the reason is returned.
pub(crate) fn restore_deck(
    config: &DeckConfig,
    sources: &mut crate::source::SourceRegistry,
    env: &mut crate::source::SourceEnv,
) -> (Deck, Option<String>) {
    let (source, mut warning) = sources.restore(&config.source, env);
    let mut deck = Deck::from_source(env.gpu, source, env.width, env.height);

    // ISF input values, stored under `params`.
    if let Some(params) = config.source.get("params").and_then(|v| {
        serde_json::from_value::<std::collections::HashMap<String, crate::params::ParamValue>>(
            v.clone(),
        )
        .ok()
    }) {
        for (name, value) in params {
            deck.generator_params.set(&name, value);
        }
    }

    // A missing required depth sensor leaves the deck a placeholder.
    if let Some(metadata) = deck.shader().map(|s| s.metadata.clone()) {
        let saved: Option<crate::deck::DepthPreproConfig> = config
            .source
            .get("depth_prepro")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let name = config.source.str("path").unwrap_or_default().to_string();
        if let Err(e) = restore_depth_preprocessor(&mut deck, saved.as_ref(), &metadata, &name, env)
        {
            let reason = format!("{e:#}");
            deck = Deck::from_source(
                env.gpu,
                Box::new(crate::source::UnavailableSource::new(
                    config.source.clone(),
                    reason.clone(),
                )),
                env.width,
                env.height,
            );
            warning = Some(reason);
        }
    }

    if !config.uuid.is_empty() {
        deck.set_uuid(config.uuid.clone());
    }

    for eff_config in &config.effects {
        match restore_effect(eff_config, env.gpu, env.gpu.compositing_format) {
            Ok(eff) => deck.effects.push(eff),
            Err(e) => log::warn!("Failed to restore deck effect '{}': {}", eff_config.path, e),
        }
    }

    (deck, warning)
}

/// Restore one effect from config. `target_format` is
/// `context.compositing_format` for deck, channel and master effects alike.
pub(crate) fn restore_effect(
    config: &EffectConfig,
    context: &GpuContext,
    target_format: wgpu::TextureFormat,
) -> Result<Effect> {
    let shader = ISFShader::from_file(&config.path)
        .with_context(|| format!("Failed to load effect shader: {}", config.path))?;
    let mut effect = Effect::new_with_format(context, shader, target_format)?;
    effect.set_uuid(config.uuid.clone());
    effect.enabled = config.enabled;
    for (name, value) in &config.params {
        effect.params.set(name, *value);
    }
    Ok(effect)
}

/// Whether a live deck's source matches `config`, so a scene diff can patch the
/// deck in place. Each source type defines the match.
pub(crate) fn source_configs_match(
    deck: &Deck,
    config: &SourceConfig,
    sources: &crate::source::SourceRegistry,
) -> bool {
    sources.same_source(&deck.source().config(), config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::GpuContext;
    use std::collections::HashMap;

    fn headless_gpu() -> GpuContext {
        GpuContext::new_headless().expect("headless GPU required for tests")
    }

    /// A restored effect keeps its saved UUID, including the cached
    /// `param_prefix` the render path uses for modulation lookups.
    #[test]
    fn restored_effect_keeps_the_uuid_its_modulation_is_keyed_on() {
        let gpu = headless_gpu();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/invert.fs");
        assert!(path.exists(), "shaders/invert.fs missing");

        let cfg = EffectConfig {
            uuid: "fxsaved1".to_string(),
            path: path.to_string_lossy().into_owned(),
            enabled: true,
            params: HashMap::new(),
        };
        let effect = restore_effect(&cfg, &gpu, crate::renderer::COLOR_PATH_FORMAT)
            .expect("invert.fs restores");

        assert_eq!(effect.uuid(), "fxsaved1");
        assert_eq!(
            effect.param_prefix(),
            "effect/fxsaved1/param",
            "the prefix modulation is looked up under must follow the restored UUID"
        );
    }

    /// `set_uuid` updates the derived param prefix.
    #[test]
    fn setting_an_effect_uuid_moves_its_modulation_prefix() {
        let gpu = headless_gpu();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/invert.fs");
        let shader = crate::isf::ISFShader::from_file(&path).expect("parse invert.fs");
        let mut effect = Effect::new(&gpu, shader).expect("build effect");

        effect.set_uuid("abcd1234".to_string());
        assert_eq!(effect.uuid(), "abcd1234");
        assert_eq!(effect.param_prefix(), "effect/abcd1234/param");
    }

    fn registry() -> crate::source::SourceRegistry {
        let mut r = crate::source::SourceRegistry::new();
        r.register(crate::solid_color::SolidColorProvider)
            .register(crate::video::provider::VideoProvider)
            .register(crate::still::ImageProvider);
        r
    }

    #[test]
    fn any_solid_color_config_patches_a_solid_color_deck_in_place() {
        let gpu = headless_gpu();
        let deck = crate::deck::Deck::solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64);
        let r = registry();
        assert!(source_configs_match(
            &deck,
            &crate::solid_color::SolidColor::config_for([0.0, 1.0, 0.0, 1.0]),
            &r
        ));
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::new("Video").with("path", "test.mp4"),
            &r
        ));
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::new("Image").with("path", "test.png"),
            &r
        ));
    }

    #[test]
    fn snapshot_and_match_solid_color_roundtrip() {
        let gpu = headless_gpu();
        let mut mixer = Mixer::new(&gpu, 64, 64).unwrap();
        mixer.channels_mut().clear();
        let mut ch = crate::channel::Channel::new("Ch 0".into(), &gpu, 64, 64).unwrap();
        ch.add_deck(crate::deck::Deck::solid_color(
            &gpu,
            [1.0, 0.5, 0.0, 1.0],
            64,
            64,
        ));
        mixer.channels_mut().push(ch);

        let config = snapshot_scene(&mixer, None, 64, 64);
        let saved = &config.channels[0].decks[0].source;
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::json!({"type": "SolidColor", "color": [1.0, 0.5, 0.0, 1.0]}),
            "the saved shape is the one every earlier build wrote"
        );
        assert!(source_configs_match(
            &mixer.channels()[0].decks[0].deck,
            saved,
            &registry()
        ));
    }

    /// A deck whose source type this build does not know is kept, renders
    /// black, and saves its config back unchanged.
    #[test]
    fn an_unknown_source_restores_as_a_placeholder_that_keeps_its_config() {
        let gpu = headless_gpu();
        let mut r = registry();
        let mut services = crate::source::Services::new();
        let shaders = crate::registry::ShaderRegistry::new();
        let mut env = crate::source::SourceEnv {
            gpu: &gpu,
            width: 64,
            height: 64,
            services: &mut services,
            shaders: &shaders,
            channels: &[],
        };
        let source = SourceConfig::new("FutureThing").with("knob", 3);
        let config: DeckConfig = serde_json::from_value(serde_json::json!({
            "uuid": "deck0001",
            "name": "later",
            "source": source,
        }))
        .unwrap();
        let (deck, warning) = restore_deck(&config, &mut r, &mut env);
        assert!(warning.is_some());
        assert_eq!(deck.uuid(), "deck0001");
        assert_eq!(deck.source_config(), source);
    }

    #[test]
    fn restore_effect_pub_crate_accessible() {
        let gpu = headless_gpu();
        let cfg = EffectConfig {
            uuid: "test0001".to_string(),
            path: "nonexistent.fs".into(),
            enabled: true,
            params: HashMap::new(),
        };
        // The file doesn't exist.
        assert!(restore_effect(&cfg, &gpu, wgpu::TextureFormat::Rgba8Unorm).is_err());
    }

    #[test]
    fn validate_stage_prefs_valid() {
        let prefs = StagePrefs::default();
        assert!(prefs.validate().is_empty());
    }

    #[test]
    fn validate_stage_prefs_grid_size_invalid() {
        let mut prefs = StagePrefs {
            grid_size: 0.0,
            ..Default::default()
        };
        assert!(prefs.validate().iter().any(|e| e.contains("grid_size")));
        prefs.grid_size = f32::NAN;
        assert!(prefs.validate().iter().any(|e| e.contains("grid_size")));
        prefs.grid_size = -1.0;
        assert!(prefs.validate().iter().any(|e| e.contains("grid_size")));
    }

    #[test]
    fn validate_stage_prefs_output_name_empty() {
        let mut prefs = StagePrefs::default();
        prefs
            .outputs
            .push(crate::scene::OutputConfig::default_windowed());
        let errors = prefs.validate();
        assert!(errors.iter().any(|e| e.contains("name is empty")));
    }

    #[test]
    fn validate_stage_prefs_warp_corners_non_finite() {
        // A non-finite surface warp corner is reported.
        let mut prefs = StagePrefs::default();
        let uuid = prefs
            .surfaces
            .add_surface("s".into(), crate::renderer::context::OutputSource::Master);
        if let Some((_, s)) = prefs.surfaces.find_by_uuid_mut(&uuid) {
            s.warp = Some(crate::surface::warp::WarpMode::CornerPin {
                corners: [[0.0, 0.0], [1.0, 0.0], [f32::INFINITY, 1.0], [0.0, 1.0]],
            });
        }
        let errors = prefs.validate();
        assert!(errors.iter().any(|e| e.contains("warp corner")));
    }

    #[test]
    fn workspace_shaders_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        let shaders_dir = ws.shaders_dir();
        assert_eq!(shaders_dir, tmp.path().join(".varda").join("shaders"));
    }
}
