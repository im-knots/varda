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
            let prefix = format!("outputs[{}]", i);
            if output.name.trim().is_empty() {
                errors.push(format!("{}: name is empty", prefix));
            }
            for (j, sa) in output.surface_assignments.iter().enumerate() {
                if let crate::renderer::warp::WarpMode::CornerPin { corners } = &sa.warp_mode {
                    for (c, corner) in corners.iter().enumerate() {
                        for (k, v) in corner.iter().enumerate() {
                            if !v.is_finite() {
                                errors.push(format!(
                                    "{}/surface_assignments[{}]: warp corner[{}][{}] is not finite",
                                    prefix, j, c, k
                                ));
                            }
                        }
                    }
                }
            }
        }
        errors
    }

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

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Stage prefs save: {}", e);
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
use crate::scene::*;

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

/// Build a SceneConfig snapshot from live app state (show-specific: channels, effects, modulation).
pub fn snapshot_scene(mixer: &Mixer, render_width: u32, render_height: u32) -> SceneConfig {
    let channels = mixer
        .channels()
        .iter()
        .map(|ch| {
            let decks = ch
                .decks
                .iter()
                .map(|slot| {
                    let source = match slot.deck.source_type() {
                        "shader" => {
                            let path = slot.deck.source_path().unwrap_or_default().to_string();
                            SourceConfig::Shader {
                                path,
                                params: slot.deck.generator_params.values.clone(),
                            }
                        }
                        "video" => {
                            let pb = slot.deck.playback_snapshot();
                            SourceConfig::Video {
                                path: slot.deck.source_path().unwrap_or_default().to_string(),
                                loop_mode: pb.as_ref().map(|p| p.loop_mode).unwrap_or_default(),
                                speed: pb.as_ref().map(|p| p.speed).unwrap_or(1.0),
                                in_point: pb.as_ref().map(|p| p.in_point).unwrap_or(0.0),
                                out_point: pb.as_ref().map(|p| p.out_point).unwrap_or(0.0),
                            }
                        }
                        "image" => SourceConfig::Image {
                            path: slot.deck.source_path().unwrap_or_default().to_string(),
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
                        blend_mode: slot.blend_mode.into(),
                        mute: slot.mute,
                        solo: slot.solo,
                        z_index: slot.z_index,
                        render_fps: slot.render_fps,
                        auto_transition,
                        modulation: vec![],
                    })
                })
                .flatten()
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
                            from_ch: *from_ch,
                            to_ch: *to_ch,
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
        version: 3,
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
        OutputTarget::SrtStream { url, codec } => OutputTargetConfig::SrtStream {
            url: url.clone(),
            codec: codec.to_string(),
        },
        OutputTarget::HlsStream {
            name,
            codec,
            low_latency,
        } => OutputTargetConfig::HlsStream {
            name: name.clone(),
            codec: codec.to_string(),
            low_latency: *low_latency,
        },
        OutputTarget::DashStream { name, codec } => OutputTargetConfig::DashStream {
            name: name.clone(),
            codec: codec.to_string(),
        },
        OutputTarget::RtmpStream { url, codec } => OutputTargetConfig::RtmpStream {
            url: url.clone(),
            codec: codec.to_string(),
        },
        OutputTarget::NdiSend { sender_name } => OutputTargetConfig::NdiSend {
            sender_name: sender_name.clone(),
        },
        OutputTarget::SyphonServer { server_name } => OutputTargetConfig::SyphonServer {
            server_name: server_name.clone(),
        },
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
                "prores" | "ProRes" | "ProRes 422" => RecordingCodec::ProRes,
                "h265" | "H265" | "H.265 (HEVC)" => RecordingCodec::H265,
                "av1" | "AV1" => RecordingCodec::AV1,
                "hap" | "Hap" | "HAP" => RecordingCodec::Hap,
                "hap_alpha" | "HapAlpha" | "HAP Alpha" => RecordingCodec::HapAlpha,
                "hapq" | "HapQ" | "HAP Q" => RecordingCodec::HapQ,
                _ => RecordingCodec::H264,
            },
        },
        OutputTargetConfig::SrtStream { url, codec } => OutputTarget::SrtStream {
            url: url.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::SrtCodec::H265,
                _ => crate::renderer::context::SrtCodec::H264,
            },
        },
        OutputTargetConfig::HlsStream {
            name,
            codec,
            low_latency,
        } => OutputTarget::HlsStream {
            name: name.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::StreamingCodec::H265,
                "AV1" | "av1" => crate::renderer::context::StreamingCodec::AV1,
                _ => crate::renderer::context::StreamingCodec::H264,
            },
            low_latency: *low_latency,
        },
        OutputTargetConfig::DashStream { name, codec } => OutputTarget::DashStream {
            name: name.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::StreamingCodec::H265,
                "AV1" | "av1" => crate::renderer::context::StreamingCodec::AV1,
                _ => crate::renderer::context::StreamingCodec::H264,
            },
        },
        OutputTargetConfig::RtmpStream { url, codec } => OutputTarget::RtmpStream {
            url: url.clone(),
            codec: match codec.as_str() {
                "H.265 (HEVC)" | "H265" | "h265" => crate::renderer::context::StreamingCodec::H265,
                "AV1" | "av1" => crate::renderer::context::StreamingCodec::AV1,
                _ => crate::renderer::context::StreamingCodec::H264,
            },
        },
        OutputTargetConfig::NdiSend { sender_name } => OutputTarget::NdiSend {
            sender_name: sender_name.clone(),
        },
        OutputTargetConfig::SyphonServer { server_name } => OutputTarget::SyphonServer {
            server_name: server_name.clone(),
        },
    }
}

/// Build a StagePrefs snapshot from live app state (venue-specific: surfaces, outputs, editor prefs).
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
                                warp_mode: a.warp_mode.clone(),
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
                            warp_mode: a.warp_mode.clone(),
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
    stream_manager: &mut crate::stream::StreamManager,
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
        if !ch_config.uuid.is_empty() {
            channel.set_uuid(ch_config.uuid.clone());
        }
        channel.opacity = ch_config.opacity;
        channel.blend_mode = ch_config.blend_mode.into();

        for deck_config in &ch_config.decks {
            match restore_deck(
                deck_config,
                context,
                registry,
                camera_manager,
                ndi_manager,
                stream_manager,
                render_width,
                render_height,
            ) {
                Ok(deck) => {
                    let mut slot = crate::channel::DeckSlot::new(deck);
                    slot.opacity = deck_config.opacity;
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
                        at.transition_shader_name = at_config.transition_shader.clone();
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
                                        "Failed to restore deck transition shader '{}': {}",
                                        shader_name,
                                        e
                                    );
                                }
                            } else {
                                log::warn!(
                                    "Deck transition shader '{}' not found in registry",
                                    shader_name
                                );
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
                    let msg = format!(
                        "Failed to restore channel effect '{}': {}",
                        eff_config.path, e
                    );
                    log::warn!("{}", msg);
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
        .map(|n| n + 1)
        .unwrap_or(mixer.channel_count());
    mixer.set_next_channel_index(max_idx);

    // Restore master effects
    for eff_config in &config.master_effects {
        match restore_effect(eff_config, context, context.texture_format) {
            Ok(eff) => mixer.master_effects_mut().push(eff),
            Err(e) => {
                let msg = format!(
                    "Failed to restore master effect '{}': {}",
                    eff_config.path, e
                );
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
        if let Some(shader) = registry
            .transitions()
            .iter()
            .find(|s| s.name() == *transition_name)
        {
            match mixer.set_transition(context, (*shader).clone()) {
                Ok(()) => {}
                Err(e) => {
                    let msg = format!("Failed to restore transition '{}': {}", transition_name, e);
                    log::warn!("{}", msg);
                    warnings.push(msg);
                }
            }
        } else {
            warnings.push(format!(
                "Transition '{}' not found in registry",
                transition_name
            ));
        }
    }

    // Restore transition sequences
    let channel_count = mixer.channels().len();
    for seq_config in &config.transition_sequences {
        use crate::mixer::{SequencerState, StepKind, TransitionSequence, TransitionStep};
        use crate::scene::TransitionStepConfig;
        let steps = seq_config.steps.iter().filter_map(|step| {
            let kind = match step {
                TransitionStepConfig::Fade { from_ch, to_ch, duration, easing, transition_shader, target_amount } => {
                    if *from_ch >= channel_count || *to_ch >= channel_count {
                        log::warn!(
                            "Transition step references channel {} or {} but only {} channels exist; skipping",
                            from_ch, to_ch, channel_count
                        );
                        return None;
                    }
                    StepKind::Fade {
                        from_ch: *from_ch,
                        to_ch: *to_ch,
                        duration: duration_config_to_spec(duration),
                        easing: (*easing).into(),
                        transition_shader: transition_shader.clone(),
                        target_amount: *target_amount,
                    }
                }
                TransitionStepConfig::Wait { duration } => {
                    StepKind::Wait {
                        duration: duration_config_to_spec(duration),
                    }
                }
                TransitionStepConfig::GoTo { step_index } => {
                    StepKind::GoTo { step_index: *step_index }
                }
            };
            Some(TransitionStep { kind })
        }).collect();
        mixer.transition_sequences_mut().push(TransitionSequence {
            name: seq_config.name.clone(),
            steps,
            enabled: seq_config.enabled,
            state: SequencerState::new(),
        });
    }

    Ok(RestoreResult { mixer, warnings })
}

/// Restore a single deck from config.
pub(crate) fn restore_deck(
    config: &DeckConfig,
    context: &GpuContext,
    _registry: &crate::registry::ShaderRegistry,
    camera_manager: &mut crate::camera::CameraManager,
    ndi_manager: &mut crate::ndi::NdiManager,
    stream_manager: &mut crate::stream::StreamManager,
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
        SourceConfig::Video {
            path,
            loop_mode,
            speed,
            in_point,
            out_point,
        } => {
            let deck = Deck::new_from_video(context, path, render_width, render_height)?;
            deck.video_set_loop_mode(*loop_mode);
            deck.video_set_speed(*speed);
            deck.video_set_in_point(*in_point);
            deck.video_set_out_point(*out_point);
            deck
        }
        SourceConfig::Image { path } => {
            Deck::new_from_image(context, path, render_width, render_height)?
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
                .ok_or_else(|| anyhow::anyhow!("Camera '{}' not found — is it connected?", name))?;
            let camera_id = device.id;
            let cam_name = device.name.clone();

            let (src_w, src_h) = camera_manager
                .open_camera(camera_id, &context.device)
                .with_context(|| format!("Failed to open camera '{}'", name))?;

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
                    "NDI source '{}' not available for restore",
                    name
                ));
            }
        },
        SourceConfig::Syphon { name } => {
            // Syphon sources are resolved at runtime — skip if not on macOS
            log::warn!(
                "Syphon source '{}' restoration not yet implemented (needs SyphonManager)",
                name
            );
            return Err(anyhow::anyhow!(
                "Syphon source '{}' not available for restore",
                name
            ));
        }
        SourceConfig::Srt { url, mode } => {
            let srt_mode = match mode.as_str() {
                "listener" => crate::stream::SrtMode::Listener,
                "caller" => crate::stream::SrtMode::Caller,
                other => {
                    log::warn!("Unknown SRT mode '{}', defaulting to Caller", other);
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
                        "SRT source '{}' not available for restore",
                        url
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
                        "HLS source '{}' not available for restore",
                        url
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
                        "DASH source '{}' not available for restore",
                        url
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
                        "RTMP source '{}' not available for restore",
                        url
                    ));
                }
            }
        }
    };

    // Restore UUID from config
    if !config.uuid.is_empty() {
        deck.set_uuid(config.uuid.clone());
    }

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
pub(crate) fn restore_effect(
    config: &EffectConfig,
    context: &GpuContext,
    target_format: wgpu::TextureFormat,
) -> Result<Effect> {
    let shader = ISFShader::from_file(&config.path)
        .with_context(|| format!("Failed to load effect shader: {}", config.path))?;
    let mut effect = Effect::new_with_format(context, shader, target_format)?;
    effect.uuid = config.uuid.clone();
    effect.enabled = config.enabled;
    // Restore parameter values
    for (name, value) in &config.params {
        effect.params.set(name, *value);
    }
    Ok(effect)
}

/// Check if a live deck's source matches a target SourceConfig (same type + same path/name).
/// Used by diff-apply to decide whether a deck can be patched in place or must be rebuilt.
pub(crate) fn source_configs_match(deck: &Deck, config: &SourceConfig) -> bool {
    match (deck.source_type(), config) {
        ("shader", SourceConfig::Shader { path, .. }) => deck.source_path() == Some(path.as_str()),
        ("video", SourceConfig::Video { path, .. }) => deck.source_path() == Some(path.as_str()),
        ("image", SourceConfig::Image { path }) => deck.source_path() == Some(path.as_str()),
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
                loop_mode: Default::default(),
                speed: 1.0,
                in_point: 0.0,
                out_point: 0.0
            }
        ));
        assert!(!source_configs_match(
            &deck,
            &SourceConfig::Shader {
                path: "test.fs".into(),
                params: HashMap::new()
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
                path: "test.png".into()
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
        let mut prefs = StagePrefs::default();
        prefs.grid_size = 0.0;
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
        let mut prefs = StagePrefs::default();
        prefs.outputs.push(crate::scene::OutputConfig {
            uuid: "test0001".into(),
            name: "test".into(),
            target: crate::scene::OutputTargetConfig::Windowed,
            target_display: None,
            surface_assignments: vec![crate::scene::SurfaceAssignmentConfig {
                surface_uuid: "abcd1234".into(),
                warp_mode: crate::renderer::warp::WarpMode::CornerPin {
                    corners: [[0.0, 0.0], [1.0, 0.0], [f32::INFINITY, 1.0], [0.0, 1.0]],
                },
                enabled: true,
            }],
            window_position: None,
            window_size: None,
            edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode::default(),
            edge_blend: crate::renderer::edge_blend::EdgeBlendConfig::default(),
            rotation: crate::renderer::context::OutputRotation::default(),
        });
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
