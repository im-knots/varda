//! Workspace persistence — save/load `.varda/` directory.
//!
//! The workspace is the current working directory. All state lives in `.varda/`:
//! - `scene.json` — channels, decks, effects, modulation (show-specific, shareable)
//! - `stage.json` — surfaces, outputs, warp, editor prefs (venue-specific)
//! - `midi.json`  — MIDI controller mappings (device-name-keyed)
//! - `presets/`   — saved deck and channel presets

pub mod presets;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Atomic file write: writes to a `.tmp` sibling then renames into place.
/// Prevents data loss if the process crashes mid-write.
///
/// # Errors
///
/// Returns an error if the temporary sibling file cannot be written (missing
/// parent directory, permissions, disk full) or if renaming it over `path`
/// fails.
pub fn atomic_write<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    let path = path.as_ref();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)
        .with_context(|| format!("Failed to write temp file: {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Stage configuration persisted in `.varda/stage.json`.
/// Contains venue-specific data: surfaces, outputs, and editor preferences.
/// Kept separate from scene.json so users can share deck layouts without stage geometry.
// Flags are independent persisted UI toggles; bundling them would change scene/stage JSON.
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
    /// Whether the stage editor is in 3D Dome mode
    #[serde(default)]
    pub dome_mode_active: bool,
    /// Active dome preset
    #[serde(default = "default_dome_preset")]
    pub dome_preset: crate::renderer::slicer::DomePreset,
    /// Active dome geometry
    #[serde(default)]
    pub dome_geometry: crate::renderer::slicer::DomeGeometry,
    /// 2D stage surface layout
    #[serde(default)]
    pub surfaces: crate::surface::SurfaceManager,
    /// Output window configurations (surface assignments, warp calibration)
    #[serde(default)]
    pub outputs: Vec<crate::scene::OutputConfig>,
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
            surfaces: crate::surface::SurfaceManager::default(),
            outputs: Vec::new(),
        }
    }
}

impl StagePrefs {
    /// Validate stage prefs for semantic correctness. Returns a list of errors.
    /// An empty list means the config is valid.
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
        // Warp now lives on surfaces — validate their corner-pin finiteness.
        for (i, surface) in self.surfaces.surfaces.iter().enumerate() {
            if let Some(crate::renderer::warp::WarpMode::CornerPin { corners }) = &surface.warp {
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
    /// Returns an error if `path` cannot be read (missing file, permissions) or
    /// if its contents are not valid JSON for a [`StagePrefs`]. Validation
    /// problems in an otherwise-parseable file are logged as warnings only.
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
    /// Returns an error if the prefs cannot be serialized to JSON, or if the
    /// atomic write fails (temp file write or rename).
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Stage prefs save: {e}");
        }
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize stage prefs")?;
        atomic_write(path.as_ref(), &content)?;
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
    ///
    /// # Errors
    ///
    /// Returns an error if the current working directory cannot be determined
    /// (e.g. it was deleted, or the process lacks permission to read it).
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

    /// Path to `controller-profiles/` directory for MIDI controller profiles.
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
    /// Returns an error if `.varda/` or either of the `presets/decks/` and
    /// `presets/channels/` directories cannot be created.
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
    /// Returns an error if the directory does not exist and cannot be created
    /// (permissions, or a non-directory file already at that path).
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
use crate::renderer::context::{OutputTarget, RecordingCodec, UnifiedOutput};
use crate::scene::{
    AutoTransitionConfig, ChannelConfig, DeckConfig, EffectConfig, OutputConfig,
    OutputTargetConfig, SceneConfig, SourceConfig, SurfaceAssignmentConfig, TriggerConfig,
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

/// Convert persisted sequence steps into runtime steps, resolving fade steps'
/// channel references against the channels as restored.
///
/// A fade whose channels no longer resolve degrades to a `Wait` of the same
/// duration rather than being dropped. `GoTo` steps address other steps by
/// position, so removing a step would silently retarget every jump past it —
/// keeping the slot preserves both the jump targets and the sequence's timing.
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

/// Serialize a deck's depth-sensor preprocessor binding, if it has one.
///
/// Stores the device *name* rather than its id, matching how cameras and
/// depth-sensor decks restore — ids shift when devices are replugged.
fn depth_prepro_config(deck: &Deck) -> Option<crate::scene::DepthPreproConfig> {
    let state = deck.depth_prepro.as_ref()?;
    let p = &state.params;
    Some(crate::scene::DepthPreproConfig {
        sensor_name: state.sensor_name.clone(),
        near_mm: p.near_mm,
        far_mm: p.far_mm,
        smoothing: p.smoothing,
        hole_fill: p.hole_fill,
        mask_feather: p.mask_feather,
        motion_gain: p.motion_gain,
        mirror: p.mirror,
    })
}

/// Build a `SceneConfig` snapshot from live app state (show-specific: channels, effects, modulation).
pub fn snapshot_scene(mixer: &Mixer, render_width: u32, render_height: u32) -> SceneConfig {
    let channels = mixer
        .channels()
        .iter()
        .map(|ch| {
            let decks = ch
                .decks
                .iter()
                .filter_map(|slot| {
                    let source = match slot.deck.source_type() {
                        "shader" => {
                            let path = slot.deck.source_path().unwrap_or_default().to_string();
                            SourceConfig::Shader {
                                path,
                                params: slot.deck.generator_params.values.clone(),
                                depth_prepro: depth_prepro_config(&slot.deck),
                            }
                        }
                        "video" => {
                            let pb = slot.deck.playback_snapshot();
                            SourceConfig::Video {
                                path: slot.deck.source_path().unwrap_or_default().to_string(),
                                loop_mode: pb.as_ref().map(|p| p.loop_mode).unwrap_or_default(),
                                speed: pb.as_ref().map_or(1.0, |p| p.speed),
                                in_point: pb.as_ref().map_or(0.0, |p| p.in_point),
                                out_point: pb.as_ref().map_or(0.0, |p| p.out_point),
                                scaling_mode: slot.deck.scaling_mode().unwrap_or_default(),
                            }
                        }
                        "image" => SourceConfig::Image {
                            path: slot.deck.source_path().unwrap_or_default().to_string(),
                            scaling_mode: slot.deck.scaling_mode().unwrap_or_default(),
                        },
                        "solid_color" => {
                            let color = slot.deck.solid_color().unwrap_or([0.0, 0.0, 0.0, 1.0]);
                            SourceConfig::SolidColor { color }
                        }
                        "camera" => {
                            // Store the camera display name (strip the 📹 prefix we add)
                            let name = slot
                                .deck
                                .source_name()
                                .trim_start_matches("📹 ")
                                .to_string();
                            SourceConfig::Camera { name }
                        }
                        "ndi" => {
                            // Store the NDI source name (strip the 📡 prefix we add)
                            let name = slot
                                .deck
                                .source_name()
                                .trim_start_matches("📡 ")
                                .to_string();
                            SourceConfig::Ndi { name }
                        }
                        "syphon" => {
                            // Store the Syphon server name (strip the 🔗 prefix we add)
                            let name = slot
                                .deck
                                .source_name()
                                .trim_start_matches("🔗 ")
                                .to_string();
                            SourceConfig::Syphon { name }
                        }
                        "srt" => {
                            let url = slot
                                .deck
                                .source_name()
                                .trim_start_matches("📺 ")
                                .to_string();
                            let mode = "caller".to_string();
                            SourceConfig::Srt { url, mode }
                        }
                        "hls" => {
                            let url = slot
                                .deck
                                .source_name()
                                .trim_start_matches("📡 ")
                                .to_string();
                            SourceConfig::Hls { url }
                        }
                        "dash" => {
                            let url = slot
                                .deck
                                .source_name()
                                .trim_start_matches("📡 ")
                                .to_string();
                            SourceConfig::Dash { url }
                        }
                        "rtmp" => {
                            let url = slot
                                .deck
                                .source_name()
                                .trim_start_matches("📺 ")
                                .to_string();
                            SourceConfig::Rtmp {
                                url,
                                mode: "pull".to_string(),
                            }
                        }
                        "html" => {
                            let url = slot
                                .deck
                                .source_name()
                                .trim_start_matches("🌐 ")
                                .to_string();
                            SourceConfig::Html { url }
                        }
                        "depth_sensor" => {
                            // Store the sensor display name (strip the 🛰 prefix we add)
                            let name = slot.deck.source_name().trim_start_matches("🛰 ").to_string();
                            let p = &slot.deck.point_cloud_params;
                            let params = Some(crate::scene::DepthParamsConfig {
                                orbit_yaw: p.orbit_yaw,
                                orbit_pitch: p.orbit_pitch,
                                zoom: p.zoom,
                                point_size: p.point_size,
                                color_mode: p.color_mode.as_f32() as u8,
                                depth_min_mm: p.depth_min_mm,
                                depth_max_mm: p.depth_max_mm,
                                solid_color: p.solid_color,
                                seed: p.seed,
                                drift: p.drift,
                                disruption: p.disruption,
                            });
                            SourceConfig::DepthSensor { name, params }
                        }
                        _ => return None,
                    };

                    let effects = slot
                        .deck
                        .effects
                        .iter()
                        .map(|eff| EffectConfig {
                            uuid: eff.uuid.clone(),
                            path: eff.shader.file_path.clone().unwrap_or_default(),
                            enabled: eff.enabled,
                            params: eff.params.values.clone(),
                        })
                        .collect();

                    // Snapshot auto-transition config
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

                    Some(DeckConfig {
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
                    })
                })
                .collect();

            let effects = ch
                .effects
                .iter()
                .map(|eff| EffectConfig {
                    uuid: eff.uuid.clone(),
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
            }
        })
        .collect();

    let master_effects = mixer
        .master_effects()
        .iter()
        .map(|eff| EffectConfig {
            uuid: eff.uuid.clone(),
            path: eff.shader.file_path.clone().unwrap_or_default(),
            enabled: eff.enabled,
            params: eff.params.values.clone(),
        })
        .collect();

    let active_transition = mixer.active_transition().as_ref().map(|t| t.name.clone());

    // Snapshot transition sequences
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
        version: 5,
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
    }
}

/// Convert a live `OutputTarget` to a serializable `OutputTargetConfig`.
fn target_to_config(target: &OutputTarget) -> OutputTargetConfig {
    match target {
        OutputTarget::Windowed => OutputTargetConfig::Windowed,
        OutputTarget::Display { name, .. } => OutputTargetConfig::Display { name: name.clone() },
        OutputTarget::Recording {
            path,
            codec,
            audio_device,
        } => OutputTargetConfig::Recording {
            path: path.clone(),
            codec: codec.to_string(),
            audio_device: audio_device.clone(),
        },
        OutputTarget::SrtStream {
            url,
            codec,
            audio_device,
        } => OutputTargetConfig::SrtStream {
            url: url.clone(),
            codec: codec.to_string(),
            audio_device: audio_device.clone(),
        },
        OutputTarget::HlsStream {
            name,
            codec,
            low_latency,
            audio_device,
        } => OutputTargetConfig::HlsStream {
            name: name.clone(),
            codec: codec.to_string(),
            low_latency: *low_latency,
            audio_device: audio_device.clone(),
        },
        OutputTarget::DashStream {
            name,
            codec,
            audio_device,
        } => OutputTargetConfig::DashStream {
            name: name.clone(),
            codec: codec.to_string(),
            audio_device: audio_device.clone(),
        },
        OutputTarget::RtmpStream {
            url,
            codec,
            audio_device,
        } => OutputTargetConfig::RtmpStream {
            url: url.clone(),
            codec: codec.to_string(),
            audio_device: audio_device.clone(),
        },
        OutputTarget::NdiSend { sender_name } => OutputTargetConfig::NdiSend {
            sender_name: sender_name.clone(),
        },
        OutputTarget::SyphonServer { server_name } => OutputTargetConfig::SyphonServer {
            server_name: server_name.clone(),
        },
    }
}

/// Convert a serializable `OutputTargetConfig` back to a live `OutputTarget`.
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
        OutputTargetConfig::Recording {
            path,
            codec,
            audio_device,
        } => OutputTarget::Recording {
            path: path.clone(),
            codec: match codec.as_str() {
                "prores" | "ProRes" | "ProRes 422" => RecordingCodec::ProRes,
                "prores_4444" | "ProRes4444" | "ProRes 4444" => RecordingCodec::ProRes4444,
                "h265" | "H265" | "H.265 (HEVC)" => RecordingCodec::H265,
                "av1" | "AV1" => RecordingCodec::AV1,
                "hap" | "Hap" | "HAP" => RecordingCodec::Hap,
                "hap_alpha" | "HapAlpha" | "HAP Alpha" => RecordingCodec::HapAlpha,
                "hapq" | "HapQ" | "HAP Q" => RecordingCodec::HapQ,
                _ => RecordingCodec::H264,
            },
            audio_device: audio_device.clone(),
        },
        OutputTargetConfig::SrtStream {
            url,
            codec,
            audio_device,
        } => OutputTarget::SrtStream {
            url: url.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::SrtCodec::H265,
                _ => crate::renderer::context::SrtCodec::H264,
            },
            audio_device: audio_device.clone(),
        },
        OutputTargetConfig::HlsStream {
            name,
            codec,
            low_latency,
            audio_device,
        } => OutputTarget::HlsStream {
            name: name.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::StreamingCodec::H265,
                "AV1" | "av1" => crate::renderer::context::StreamingCodec::AV1,
                _ => crate::renderer::context::StreamingCodec::H264,
            },
            low_latency: *low_latency,
            audio_device: audio_device.clone(),
        },
        OutputTargetConfig::DashStream {
            name,
            codec,
            audio_device,
        } => OutputTarget::DashStream {
            name: name.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::StreamingCodec::H265,
                "AV1" | "av1" => crate::renderer::context::StreamingCodec::AV1,
                _ => crate::renderer::context::StreamingCodec::H264,
            },
            audio_device: audio_device.clone(),
        },
        OutputTargetConfig::RtmpStream {
            url,
            codec,
            audio_device,
        } => OutputTarget::RtmpStream {
            url: url.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::StreamingCodec::H265,
                "AV1" | "av1" => crate::renderer::context::StreamingCodec::AV1,
                _ => crate::renderer::context::StreamingCodec::H264,
            },
            audio_device: audio_device.clone(),
        },
        OutputTargetConfig::NdiSend { sender_name } => OutputTarget::NdiSend {
            sender_name: sender_name.clone(),
        },
        OutputTargetConfig::SyphonServer { server_name } => OutputTarget::SyphonServer {
            server_name: server_name.clone(),
        },
    }
}

/// Build a `StagePrefs` snapshot from live app state (venue-specific: surfaces, outputs, editor prefs).
// Aggregates many independent live-state sources into one snapshot; no shared invariant to bundle.
// The bools mirror independent persisted `StagePrefs` toggles one-for-one.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub fn snapshot_stage(
    surface_manager: &crate::surface::SurfaceManager,
    outputs_list: &[UnifiedOutput],
    grid_size: f32,
    snap: bool,
    library_panel_open: bool,
    right_panel_open: bool,
    stage_editor_open: bool,
    dome_preview_open: bool,
    dome_mode_active: bool,
    dome_preset: crate::renderer::slicer::DomePreset,
    dome_geometry: crate::renderer::slicer::DomeGeometry,
) -> StagePrefs {
    let outputs = outputs_list
        .iter()
        .map(|unified| {
            let (name, target, surface_assignments, window_position, window_size) = match unified {
                UnifiedOutput::Window(w) => {
                    // Capture window position and size for restoration
                    let pos = w.window.outer_position().ok().map(|p| [p.x, p.y]);
                    let sz = {
                        let s = w.window.inner_size();
                        if s.width > 0 && s.height > 0 {
                            Some([s.width, s.height])
                        } else {
                            None
                        }
                    };
                    (
                        w.name.clone(),
                        target_to_config(&w.target),
                        w.surface_assignments
                            .iter()
                            .map(|a| SurfaceAssignmentConfig {
                                surface_uuid: a.surface_uuid.clone(),
                                legacy_warp_mode: None,
                                enabled: a.enabled,
                            })
                            .collect(),
                        pos,
                        sz,
                    )
                }
                UnifiedOutput::Headless(h) => (
                    h.name.clone(),
                    target_to_config(&h.target),
                    h.surface_assignments
                        .iter()
                        .map(|a| SurfaceAssignmentConfig {
                            surface_uuid: a.surface_uuid.clone(),
                            legacy_warp_mode: None,
                            enabled: a.enabled,
                        })
                        .collect(),
                    None,
                    None,
                ),
            };
            let edge_blend_mode = unified.edge_blend_mode();
            let edge_blend = unified.edge_blend();
            OutputConfig {
                uuid: unified.uuid().to_string(),
                name,
                target,
                target_display: None,
                surface_assignments,
                window_position,
                window_size,
                edge_blend_mode,
                edge_blend,
                rotation: unified.rotation(),
            }
        })
        .collect();

    StagePrefs {
        grid_size,
        snap,
        library_panel_open,
        right_panel_open,
        stage_editor_open,
        dome_preview_open,
        dome_mode_active,
        dome_preset,
        dome_geometry,
        surfaces: surface_manager.clone(),
        outputs,
    }
}

// ── Restore: Config → Live State ────────────────────────────────────

use crate::deck::{Deck, Effect};
use crate::isf::ISFShader;
use crate::renderer::GpuContext;

/// A Syphon deck whose source could not be resolved at restore time, deferred
/// for late binding. Varda is the *client* of externally-owned Syphon servers;
/// on restart the producer may not be publishing yet, so the named server is not in
/// `SyphonServerDirectory`. Rather than fail the restore (the old behaviour:
/// "restoration not yet implemented" → `black_hole` placeholder), we record the
/// intent here and let `VardaApp::reconcile_syphon` auto-attach the real deck
/// the moment the server appears. Startup order becomes irrelevant.
#[derive(Debug, Clone)]
pub struct PendingSyphonDeck {
    /// UUID of the channel this deck belongs to. Binding happens seconds to
    /// minutes after restore, by which point a positional index may point at a
    /// different channel.
    pub channel_uuid: String,
    /// Full persisted deck config (carries the `Syphon { name }` source plus
    /// opacity / blend / mute / solo / z-index to re-apply on bind).
    pub config: crate::scene::DeckConfig,
}

/// Restore result — contains reconstructed mixer.
/// Surfaces and outputs are loaded separately from stage.json.
pub struct RestoreResult {
    pub mixer: Mixer,
    pub warnings: Vec<String>,
    /// Syphon decks deferred for late binding (see `PendingSyphonDeck`).
    pub pending_syphon: Vec<PendingSyphonDeck>,
}

/// Reconstruct live state from a `SceneConfig`.
///
/// # Errors
///
/// Returns an error if the mixer or any deck/effect in `config` cannot be
/// constructed on the GPU — for example a shader or media file that fails to
/// load or compile. Individually recoverable problems are collected into
/// [`RestoreResult::warnings`] instead.
// Writes back into many independent live-state targets; no shared invariant to bundle.
#[allow(clippy::too_many_arguments)]
pub fn restore_scene(
    config: &SceneConfig,
    context: &GpuContext,
    registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
    depth_manager: &mut crate::depth::DepthSensorManager,
    ndi_manager: &mut crate::ndi::NdiManager,
    stream_manager: &mut crate::stream::StreamManager,
    html_manager: &mut crate::html::HtmlManager,
    render_width: u32,
    render_height: u32,
) -> Result<RestoreResult> {
    let mut warnings = Vec::new();
    // Only pushed to under #[cfg(target_os = "macos")]; on other platforms it
    // stays empty, so `mut` would be flagged as unused there.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut pending_syphon: Vec<PendingSyphonDeck> = Vec::new();
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
        if !ch_config.uuid.is_empty() {
            channel.set_uuid(ch_config.uuid.clone());
        }
        channel.opacity = ch_config.opacity;
        channel.blend_mode = ch_config.blend_mode.into();

        for deck_config in &ch_config.decks {
            // Externally-owned Syphon decks are resolved at runtime, not at
            // restore time — the producer may not be publishing yet. Defer to a
            // pending binding the render thread auto-attaches
            // once the named server appears (see VardaApp::reconcile_syphon).
            // This replaces the old hard-fail stub that dropped the channel to a
            // black_hole placeholder and spammed "restoration not yet implemented".
            if let SourceConfig::Syphon { name } = &deck_config.source {
                #[cfg(target_os = "macos")]
                {
                    log::info!(
                        "Syphon deck '{}' on channel {} deferred to late-bind \
                         (auto-attaches when the server appears)",
                        name,
                        channel.name
                    );
                    pending_syphon.push(PendingSyphonDeck {
                        channel_uuid: channel.uuid().to_string(),
                        config: deck_config.clone(),
                    });
                }
                #[cfg(not(target_os = "macos"))]
                {
                    log::debug!("Skipping Syphon deck '{name}' on non-macOS restore");
                }
                continue;
            }
            match restore_deck(
                deck_config,
                context,
                registry,
                camera_manager,
                depth_manager,
                ndi_manager,
                stream_manager,
                html_manager,
                render_width,
                render_height,
            ) {
                Ok(deck) => {
                    let mut slot = crate::channel::DeckSlot::new(deck);
                    slot.opacity = deck_config.opacity;
                    slot.deck.set_transparent(deck_config.transparent);
                    slot.blend_mode = deck_config.blend_mode.into();
                    slot.mute = deck_config.mute;
                    slot.solo = deck_config.solo;
                    slot.z_index = deck_config.z_index;
                    slot.render_fps = deck_config.render_fps;

                    // Restore auto-transition config
                    if let Some(at_config) = &deck_config.auto_transition {
                        use crate::channel::{DeckAutoTransition, TransitionTrigger};
                        let mut at = DeckAutoTransition::new();
                        at.enabled = at_config.enabled;
                        at.trigger = match at_config.trigger {
                            TriggerConfig::Timer => TransitionTrigger::Timer,
                            TriggerConfig::ClipEnd => TransitionTrigger::ClipEnd,
                        };
                        at.play_duration = duration_config_to_spec(&at_config.play_duration);
                        at.transition_duration =
                            duration_config_to_spec(&at_config.transition_duration);
                        at.transition_shader_name
                            .clone_from(&at_config.transition_shader);
                        slot.auto_transition = Some(at);

                        // Compile transition shader if specified
                        if let Some(shader_name) = &at_config.transition_shader {
                            if let Some(shader) = registry
                                .transitions()
                                .iter()
                                .find(|s| s.name() == *shader_name)
                            {
                                if let Err(e) =
                                    slot.set_transition_shader(context, (*shader).clone())
                                {
                                    log::warn!(
                                        "Failed to restore deck transition shader '{shader_name}': {e}"
                                    );
                                }
                            } else {
                                log::warn!(
                                    "Deck transition shader '{shader_name}' not found in registry"
                                );
                            }
                        }
                    }

                    channel.add_deck_slot(slot);
                }
                Err(e) => {
                    let msg = format!("Failed to restore deck '{}': {}", deck_config.name, e);
                    log::warn!("{msg}");
                    warnings.push(msg);
                }
            }
        }

        // Restore channel effects
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

    // Update next_channel_index so new channels don't get duplicate names.
    // Parse existing channel names to find the highest "Ch N" index.
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

    // Restore master effects
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

    // Restore crossfader
    mixer.set_crossfader(config.crossfader);

    // Restore modulation engine
    mixer.set_modulation(config.modulation.clone());

    // Restore macro controls
    mixer.set_macros(config.macros.clone());

    // Restore active transition
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

    // Restore transition sequences. Fade steps address channels by UUID; scenes
    // at v4 and earlier stored indices, which `ChannelRef::resolve` maps through
    // the restored channel order.
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

    // Restore tonemap mode
    mixer.set_tonemap_mode(&context.queue, config.tonemap_mode);

    // Restore active LUT
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

    Ok(RestoreResult {
        mixer,
        warnings,
        pending_syphon,
    })
}

/// Restore a single deck from config.
// Needs many independent GPU/context inputs to rebuild a deck; no shared invariant to bundle.
/// Reacquire and attach a shader deck's depth-sensor preprocessor on restore.
///
/// Resolves the sensor by saved name when the scene recorded one, falling back
/// to the ISF header's device selection for scenes written before the binding
/// was persisted. Returns `Err` when the shader needs a sensor and none is
/// available, so the caller skips the deck.
fn restore_depth_preprocessor(
    deck: &mut Deck,
    saved: Option<&crate::scene::DepthPreproConfig>,
    metadata: &crate::isf::ISFMetadata,
    shader_path: &str,
    depth_manager: &mut crate::depth::DepthSensorManager,
    context: &GpuContext,
) -> Result<()> {
    use crate::depth::preprocess::{DepthPreprocessParams, DepthPreprocessPipeline};

    if crate::depth::preprocess::requested_device(metadata).is_none() {
        return Ok(());
    }

    let params = saved.map_or_else(DepthPreprocessParams::default, |c| DepthPreprocessParams {
        near_mm: c.near_mm,
        far_mm: c.far_mm.max(c.near_mm + 1.0),
        smoothing: c.smoothing,
        hole_fill: c.hole_fill,
        mask_feather: c.mask_feather,
        motion_gain: c.motion_gain,
        mirror: c.mirror,
    });

    // Prefer the saved device name; fall back to the header's selection.
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_deck(
    config: &DeckConfig,
    context: &GpuContext,
    _registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
    depth_manager: &mut crate::depth::DepthSensorManager,
    ndi_manager: &mut crate::ndi::NdiManager,
    stream_manager: &mut crate::stream::StreamManager,
    html_manager: &mut crate::html::HtmlManager,
    render_width: u32,
    render_height: u32,
) -> Result<Deck> {
    let mut deck = match &config.source {
        SourceConfig::Shader {
            path,
            params,
            depth_prepro,
        } => {
            let shader = ISFShader::from_file(path)
                .with_context(|| format!("Failed to load shader: {path}"))?;
            let metadata = shader.metadata.clone();
            let mut deck = if shader.metadata.is_compute() {
                Deck::new_from_compute_shader(context, shader, render_width, render_height)?
            } else {
                Deck::new(context, shader, render_width, render_height)?
            };
            // Restore parameter values
            for (name, value) in params {
                deck.generator_params.set(name, *value);
            }
            // Reacquire the depth sensor this shader needs. `depth_sensor` is a
            // required preprocessor, so a missing device fails the restore and
            // the caller skips the deck with a warning — the same handling a
            // missing camera or depth-sensor deck already gets.
            restore_depth_preprocessor(
                &mut deck,
                depth_prepro.as_ref(),
                &metadata,
                path,
                depth_manager,
                context,
            )?;
            deck
        }
        SourceConfig::Video {
            path,
            loop_mode,
            speed,
            in_point,
            out_point,
            scaling_mode,
        } => {
            let mut deck = Deck::new_from_video(context, path, render_width, render_height)?;
            deck.video_set_loop_mode(*loop_mode);
            deck.video_set_speed(*speed);
            deck.video_set_in_point(*in_point);
            deck.video_set_out_point(*out_point);
            deck.set_scaling_mode(*scaling_mode);
            deck
        }
        SourceConfig::Image { path, scaling_mode } => {
            let mut deck = Deck::new_from_image(context, path, render_width, render_height)?;
            deck.set_scaling_mode(*scaling_mode);
            deck
        }
        SourceConfig::SolidColor { color } => {
            Deck::new_solid_color(context, *color, render_width, render_height)?
        }
        SourceConfig::Camera { name } => {
            // Find the camera by name in the manager's device list
            let device = camera_manager
                .devices()
                .iter()
                .find(|d| d.name == *name)
                .ok_or_else(|| anyhow::anyhow!("Camera '{name}' not found — is it connected?"))?;
            let camera_id = device.id;
            let cam_name = device.name.clone();

            let (src_w, src_h) = camera_manager
                .open_camera(camera_id, &context.device)
                .with_context(|| format!("Failed to open camera '{name}'"))?;

            Deck::new_from_camera(
                context,
                camera_id,
                &cam_name,
                src_w,
                src_h,
                render_width,
                render_height,
            )?
        }
        SourceConfig::Ndi { name } => match ndi_manager.start_receive(name, &context.device) {
            Some(receiver_idx) => {
                let (src_w, src_h) = ndi_manager
                    .receiver_dimensions(receiver_idx)
                    .unwrap_or((1920, 1080));
                Deck::new_from_ndi(
                    context,
                    receiver_idx,
                    name,
                    src_w,
                    src_h,
                    render_width,
                    render_height,
                )?
            }
            None => {
                return Err(anyhow::anyhow!(
                    "NDI source '{name}' not available for restore"
                ));
            }
        },
        SourceConfig::Syphon { name } => {
            // Syphon sources are resolved at runtime — skip if not on macOS
            log::warn!(
                "Syphon source '{name}' restoration not yet implemented (needs SyphonManager)"
            );
            return Err(anyhow::anyhow!(
                "Syphon source '{name}' not available for restore"
            ));
        }
        SourceConfig::Srt { url, mode } => {
            let srt_mode = match mode.as_str() {
                "listener" => crate::stream::SrtMode::Listener,
                "caller" => crate::stream::SrtMode::Caller,
                other => {
                    log::warn!("Unknown SRT mode '{other}', defaulting to Caller");
                    crate::stream::SrtMode::Caller
                }
            };
            match stream_manager.start_srt_receive(url, srt_mode, &context.device) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = stream_manager
                        .receiver_dimensions(receiver_idx)
                        .unwrap_or((1920, 1080));
                    Deck::new_from_srt(
                        context,
                        receiver_idx,
                        url,
                        src_w,
                        src_h,
                        render_width,
                        render_height,
                    )?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "SRT source '{url}' not available for restore"
                    ));
                }
            }
        }
        SourceConfig::Hls { url } => {
            match stream_manager.start_receive(
                url,
                crate::stream::StreamProtocol::Hls,
                &context.device,
            ) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = stream_manager
                        .receiver_dimensions(receiver_idx)
                        .unwrap_or((1920, 1080));
                    Deck::new_from_hls(
                        context,
                        receiver_idx,
                        url,
                        src_w,
                        src_h,
                        render_width,
                        render_height,
                    )?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "HLS source '{url}' not available for restore"
                    ));
                }
            }
        }
        SourceConfig::Dash { url } => {
            match stream_manager.start_receive(
                url,
                crate::stream::StreamProtocol::Dash,
                &context.device,
            ) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = stream_manager
                        .receiver_dimensions(receiver_idx)
                        .unwrap_or((1920, 1080));
                    Deck::new_from_dash(
                        context,
                        receiver_idx,
                        url,
                        src_w,
                        src_h,
                        render_width,
                        render_height,
                    )?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "DASH source '{url}' not available for restore"
                    ));
                }
            }
        }
        SourceConfig::Rtmp { url, mode } => {
            let rtmp_mode = match mode.as_str() {
                "listen" | "Listen" => crate::stream::RtmpMode::Listen,
                _ => crate::stream::RtmpMode::Pull,
            };
            match stream_manager.start_rtmp_receive(url, rtmp_mode, &context.device) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = stream_manager
                        .receiver_dimensions(receiver_idx)
                        .unwrap_or((1920, 1080));
                    Deck::new_from_rtmp(
                        context,
                        receiver_idx,
                        url,
                        src_w,
                        src_h,
                        render_width,
                        render_height,
                    )?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "RTMP source '{url}' not available for restore"
                    ));
                }
            }
        }
        SourceConfig::Html { url } => {
            match html_manager.start_render(url, render_width, render_height, &context.device) {
                Some(instance_idx) => {
                    let (src_w, src_h) = html_manager
                        .instance_dimensions(instance_idx)
                        .unwrap_or((1920, 1080));
                    Deck::new_from_html(
                        context,
                        instance_idx,
                        url,
                        src_w,
                        src_h,
                        render_width,
                        render_height,
                    )?
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "HTML source '{url}' not available for restore"
                    ));
                }
            }
        }
        SourceConfig::DepthSensor { name, params } => {
            // Match the sensor by name in the manager's device list, then open it.
            // If absent (e.g. `depth` feature off or unplugged), skip with error.
            let device = depth_manager
                .devices()
                .iter()
                .find(|d| d.name == *name)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!("Depth sensor '{name}' not found — is it connected?")
                })?;
            let (src_w, src_h) =
                crate::depth::open_depth_sensor(depth_manager, device.id, &context.device)
                    .with_context(|| format!("Failed to open depth sensor '{name}'"))?;
            let mut deck = Deck::new_from_depth_sensor(
                context,
                device.id,
                &device.name,
                src_w,
                src_h,
                render_width,
                render_height,
            )?;
            if let Some(p) = params {
                use crate::depth::point_cloud::{ColorMode, PointCloudParams};
                deck.point_cloud_params = PointCloudParams {
                    orbit_yaw: p.orbit_yaw,
                    orbit_pitch: p.orbit_pitch,
                    zoom: p.zoom,
                    point_size: p.point_size,
                    color_mode: ColorMode::from_u8(p.color_mode),
                    depth_min_mm: p.depth_min_mm,
                    depth_max_mm: p.depth_max_mm,
                    solid_color: p.solid_color,
                    seed: p.seed,
                    drift: p.drift,
                    disruption: p.disruption,
                };
            }
            deck
        }
    };

    // Restore UUID from config
    if !config.uuid.is_empty() {
        deck.set_uuid(config.uuid.clone());
    }

    // Restore effects
    for eff_config in &config.effects {
        match restore_effect(eff_config, context, context.compositing_format) {
            Ok(eff) => deck.effects.push(eff),
            Err(e) => log::warn!("Failed to restore deck effect '{}': {}", eff_config.path, e),
        }
    }

    Ok(deck)
}

/// Restore a single effect from config.
/// `target_format` is `context.compositing_format` for every tier — deck,
/// channel, and master effects all target the unified color-path format.
pub(crate) fn restore_effect(
    config: &EffectConfig,
    context: &GpuContext,
    target_format: wgpu::TextureFormat,
) -> Result<Effect> {
    let shader = ISFShader::from_file(&config.path)
        .with_context(|| format!("Failed to load effect shader: {}", config.path))?;
    let mut effect = Effect::new_with_format(context, shader, target_format)?;
    effect.uuid.clone_from(&config.uuid);
    effect.enabled = config.enabled;
    // Restore parameter values
    for (name, value) in &config.params {
        effect.params.set(name, *value);
    }
    Ok(effect)
}

/// Check if a live deck's source matches a target `SourceConfig` (same type + same path/name).
/// Used by diff-apply to decide whether a deck can be patched in place or must be rebuilt.
// Each arm pairs one source_type string with its matching config variant; merging
// same-bodied arms would let mismatched type/config pairs compare equal.
#[allow(clippy::match_same_arms)]
pub(crate) fn source_configs_match(deck: &Deck, config: &SourceConfig) -> bool {
    match (deck.source_type(), config) {
        ("shader", SourceConfig::Shader { path, .. }) => deck.source_path() == Some(path.as_str()),
        ("video", SourceConfig::Video { path, .. }) => deck.source_path() == Some(path.as_str()),
        ("image", SourceConfig::Image { path, .. }) => deck.source_path() == Some(path.as_str()),
        ("solid_color", SourceConfig::SolidColor { .. }) => true,
        ("camera", SourceConfig::Camera { name }) => {
            deck.source_name().trim_start_matches("📹 ") == name
        }
        ("ndi", SourceConfig::Ndi { name }) => deck.source_name().trim_start_matches("📡 ") == name,
        ("syphon", SourceConfig::Syphon { name }) => {
            deck.source_name().trim_start_matches("🔗 ") == name
        }
        ("srt", SourceConfig::Srt { url, .. }) => {
            deck.source_name().trim_start_matches("📺 ") == url
        }
        ("hls", SourceConfig::Hls { url }) => deck.source_name().trim_start_matches("📡 ") == url,
        ("dash", SourceConfig::Dash { url }) => deck.source_name().trim_start_matches("📡 ") == url,
        ("rtmp", SourceConfig::Rtmp { url, .. }) => {
            deck.source_name().trim_start_matches("📺 ") == url
        }
        ("html", SourceConfig::Html { url }) => deck.source_name().trim_start_matches("🌐 ") == url,
        ("depth_sensor", SourceConfig::DepthSensor { name, .. }) => {
            deck.source_name().trim_start_matches("🛰 ") == name
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::GpuContext;
    use std::collections::HashMap;

    fn headless_gpu() -> GpuContext {
        GpuContext::new_headless().expect("headless GPU required for tests")
    }

    #[test]
    fn source_configs_match_solid_color() {
        let gpu = headless_gpu();
        let deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64).unwrap();
        // Any solid color config matches a solid color deck
        assert!(source_configs_match(
            &deck,
            &SourceConfig::SolidColor {
                color: [0.0, 1.0, 0.0, 1.0]
            }
        ));
        // But not other types
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::Video {
                path: "test.mp4".into(),
                loop_mode: crate::video::LoopMode::default(),
                speed: 1.0,
                in_point: 0.0,
                out_point: 0.0,
                scaling_mode: crate::deck::ScalingMode::default()
            }
        ));
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::Shader {
                path: "test.fs".into(),
                params: HashMap::new(),
                depth_prepro: None
            }
        ));
    }

    #[test]
    fn source_configs_match_type_mismatch() {
        let gpu = headless_gpu();
        let deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64).unwrap();
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::Image {
                path: "test.png".into(),
                scaling_mode: crate::deck::ScalingMode::default()
            }
        ));
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::Camera { name: "cam".into() }
        ));
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::Ndi { name: "src".into() }
        ));
    }

    #[test]
    fn snapshot_and_match_solid_color_roundtrip() {
        let gpu = headless_gpu();
        let mut mixer = Mixer::new(&gpu, 64, 64).unwrap();
        // Clear default channels and add one with a solid color deck
        mixer.channels_mut().clear();
        let mut ch = crate::channel::Channel::new("Ch 0".into(), &gpu, 64, 64).unwrap();
        let deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.5, 0.0, 1.0], 64, 64).unwrap();
        ch.add_deck(deck);
        mixer.channels_mut().push(ch);

        // Snapshot and verify source match
        let config = snapshot_scene(&mixer, 64, 64);
        let deck_ref = &mixer.channels()[0].decks[0].deck;
        assert!(source_configs_match(
            deck_ref,
            &config.channels[0].decks[0].source
        ));
    }

    #[test]
    fn restore_effect_pub_crate_accessible() {
        // Just verify the function signature is accessible at pub(crate) level
        let gpu = headless_gpu();
        let cfg = EffectConfig {
            uuid: "test0001".to_string(),
            path: "nonexistent.fs".into(),
            enabled: true,
            params: HashMap::new(),
        };
        // Should fail (file doesn't exist) but shouldn't be a compile error
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
        // Warp is per-surface now — a non-finite corner on a surface's warp
        // must be reported by validation.
        let mut prefs = StagePrefs::default();
        let uuid = prefs
            .surfaces
            .add_surface("s".into(), crate::renderer::context::OutputSource::Master);
        if let Some((_, s)) = prefs.surfaces.find_by_uuid_mut(&uuid) {
            s.warp = Some(crate::renderer::warp::WarpMode::CornerPin {
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

    #[test]
    fn output_audio_device_survives_target_config_roundtrip() {
        // audio_device must round-trip live OutputTarget ↔ persisted config.
        let recording = OutputTarget::Recording {
            path: "set.mov".into(),
            codec: RecordingCodec::ProRes,
            audio_device: Some("Scarlett 2i2".into()),
        };
        let back = config_to_target(&target_to_config(&recording));
        assert_eq!(back.audio_device(), Some("Scarlett 2i2"));

        // None (video-only) round-trips as None.
        let silent = OutputTarget::RtmpStream {
            url: "rtmp://x".into(),
            codec: crate::renderer::context::StreamingCodec::H264,
            audio_device: None,
        };
        assert_eq!(
            config_to_target(&target_to_config(&silent)).audio_device(),
            None
        );
    }
}
