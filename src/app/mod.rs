//! The engine implementation. `VardaApp` owns every engine subsystem and
//! implements the engine traits; window and egui state live elsewhere.

mod actions;
mod classify;
mod commands;
mod deck_loads;
mod engine_impl;
pub(crate) mod history;
mod inputs;
/// Interactive mode for HTML decks (feature `html`).
#[cfg(feature = "html")]
pub(crate) mod interactive;
mod outputs;
pub mod publish;
pub(crate) mod render;
pub(crate) mod resolve;
mod snapshot;
pub(crate) mod sources;
pub(crate) mod state;
mod surfaces;
mod workspace;

pub use render::{FileDialogRequest, FileDialogTarget, RenderTimes};
pub use workspace::WorkspaceLoad;

/// Default render width for decks and stage output.
pub const DEFAULT_RENDER_WIDTH: u32 = 1920;
/// Default render height for decks and stage output.
pub const DEFAULT_RENDER_HEIGHT: u32 = 1080;

/// Clamp each dimension to the GPU's `max_texture_dimension_2d`, the only
/// resolution limit. Zero passes through; callers reject it.
pub(crate) fn clamp_resolution_to_gpu(width: u32, height: u32, max_dim: u32) -> (u32, u32) {
    (width.min(max_dim), height.min(max_dim))
}

/// Session configuration from CLI flags. Flags override saved config for the
/// session without changing files.
// Each bool is an independent `--flag`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, clap::Parser)]
#[command(name = "varda", version, about = "Live visuals engine")]
pub struct AppConfig {
    /// Run without main UI window (API-only control)
    #[arg(long)]
    pub headless: bool,

    /// HTTP API port
    #[arg(long = "port", default_value_t = 8080)]
    pub api_port: u16,

    /// Target render FPS (default: 60, 0 = uncapped)
    #[arg(long = "fps", default_value_t = 60)]
    pub target_fps: u32,

    /// Workspace root directory (default: current working directory)
    #[arg(long = "workspace")]
    pub workspace_root: Option<std::path::PathBuf>,

    /// Scene file to load (overrides workspace default)
    #[arg(long = "scene")]
    pub scene_path: Option<std::path::PathBuf>,

    /// Stage file to load (overrides workspace default)
    #[arg(long = "stage")]
    pub stage_path: Option<std::path::PathBuf>,

    /// OSC input port (overrides osc.json config)
    #[arg(long = "osc-port")]
    pub osc_port: Option<u16>,

    /// OSC feedback target host:port (repeatable)
    #[arg(long = "osc-out")]
    pub osc_targets: Vec<String>,

    /// Disable OSC input entirely
    #[arg(long = "no-osc")]
    pub osc_disabled: bool,

    /// Disable NDI discovery and sending
    #[arg(long = "no-ndi")]
    pub ndi_disabled: bool,

    /// Disable Syphon (macOS only)
    #[arg(long = "no-syphon")]
    pub syphon_disabled: bool,

    /// Disable Spout (Windows only)
    #[arg(long = "no-spout")]
    pub spout_disabled: bool,

    /// Disable HTML deck sources (skips Servo rendering)
    #[arg(long = "no-html")]
    pub html_disabled: bool,

    /// Disable screen / window capture deck sources (no Screen Recording
    /// permission is requested)
    #[arg(long = "no-screen-capture")]
    pub screen_capture_disabled: bool,

    /// Additional shader library directory (repeatable). Overrides built-in
    /// shaders of the same name.
    #[arg(long = "shader-dir")]
    pub shader_dirs: Vec<std::path::PathBuf>,
}

impl AppConfig {
    /// Resolve the workspace root, first match wins:
    /// 1. `--workspace`
    /// 2. The current directory, if it contains `.varda/`
    /// 3. The home directory (`~/.varda/`)
    pub fn effective_workspace_root(&self) -> std::path::PathBuf {
        Self::resolve_workspace_root(
            self.workspace_root.as_deref(),
            std::env::current_dir().ok().as_deref(),
            dirs::home_dir().as_deref(),
        )
    }

    /// Workspace resolution without reading the environment, for tests.
    fn resolve_workspace_root(
        explicit: Option<&std::path::Path>,
        cwd: Option<&std::path::Path>,
        home: Option<&std::path::Path>,
    ) -> std::path::PathBuf {
        if let Some(ws) = explicit {
            return ws.to_path_buf();
        }
        if let Some(cwd) = cwd
            && cwd.join(".varda").is_dir()
        {
            return cwd.to_path_buf();
        }
        if let Some(home) = home {
            return home.to_path_buf();
        }
        std::path::PathBuf::from(".")
    }
}

use crate::audio::AudioManager;
use crate::camera::CameraManager;
use crate::depth::DepthSensorManager;
use crate::keymap::KeymapStore;
use crate::midi;
use crate::mixer::Mixer;
use crate::notifications::NotificationSystem;
use crate::osc::{OscConfig, OscFeedbackSender, OscReceiver};
use crate::persistence::Workspace;
use crate::registry::ShaderRegistry;
use crate::renderer::context::GpuContext;
use crate::screen_capture::ScreenCaptureManager;
use crate::surface::SurfaceManager;

use crate::engine::{CommandEnvelope, CommandResult, EngineState, ErrorCode};

// ── Domain sub-structs ──────────────────────────────────────────

/// Control inputs: OSC, MIDI, keyboard shortcuts, clock, timecode, and the
/// global actions a control surface asked for.
pub(crate) struct Inputs {
    pub osc_receiver: Option<OscReceiver>,
    pub osc_feedback: Option<OscFeedbackSender>,
    pub osc_config: OscConfig,
    pub midi_devices: Option<midi::MidiDeviceManager>,
    pub midi_mappings: midi::MidiMappingStore,
    pub controller_led_mgr: midi::ControllerLedManager,
    pub auto_map_engine: midi::AutoMapEngine,
    pub keymap: KeymapStore,
    pub clock_manager: crate::clock::ClockManager,
    /// Absolute position from an external master. The clock handles tempo.
    pub timecode: crate::timecode::TimecodeManager,
    /// The PCM tap LTC is decoded from, while one is patched.
    pub ltc_tap: Option<LtcTap>,
    /// Undo, redo and save from a control surface, not yet dispatched.
    pub pending_actions: inputs::PendingGlobalActions,
}

/// A raw PCM subscription to the audio input carrying LTC, on the same tee
/// as the recording path.
pub(crate) struct LtcTap {
    pub source_id: crate::audio::AudioSourceId,
    pub token: crate::audio::PcmToken,
    pub receiver: crossbeam_channel::Receiver<crate::audio::PcmChunk>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Output windows, headless outputs, the surface layout, and the dome.
// The group is named for its `outputs` field.
#[allow(clippy::struct_field_names)]
pub(crate) struct Outputs {
    pub outputs: Vec<crate::output::Output>,
    /// Every output sink type. See `sources.rs`.
    pub sinks: crate::output::SinkRegistry,
    /// The last sink type listing, reused by snapshots.
    pub sink_type_cache: sources::TypeCache,
    pub surface_manager: SurfaceManager,
    pub calibration_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    pub domemaster: Option<crate::renderer::dome::DomemasterRenderer>,
    /// Size the domemaster renderer is built at. Kept outside the renderer
    /// because it is restored before the renderer exists.
    pub domemaster_resolution: crate::renderer::dome::DomemasterResolution,
    /// Dome projection the domemaster is rendered for.
    pub dome: crate::engine::value::dome::DomeConfig,
}

/// Frame timing and system monitoring.
pub(crate) struct FrameStats {
    pub last_frame_instant: std::time::Instant,
    pub fps_history: std::collections::VecDeque<f32>,
    pub fps_smoothed: f32,
    pub frame_count: u64,
    pub system_monitor: crate::sysmon::SystemMonitor,
    /// Command buffer commits in the previous frame, sampled in
    /// `update_frame_timing`. See `renderer::submit_stats`.
    pub last_frame_submits: u32,
}

/// Session persistence: workspace, presets, undo/redo, notifications.
pub(crate) struct Session {
    pub workspace: Workspace,
    pub preset_library: crate::persistence::presets::PresetLibrary,
    pub history: history::HistoryManager,
    pub notifications: NotificationSystem,
    /// Stage editor prefs last sent by the GUI or loaded from `stage.json`,
    /// so API and headless saves keep them.
    pub editor_prefs: crate::engine::value::editor::EditorPrefs,
    /// Copied configs. Not persisted.
    pub clipboard: Option<state::clipboard::ClipboardPayload>,
}

/// The transport, automation recording, and cue navigation.
pub(crate) struct Show {
    /// Absolute show position. Tempo is `Inputs::clock_manager`.
    pub transport: crate::transport::Transport,
    /// Automation record arm state and open takes.
    pub recorder: state::recorder::Recorder,
    /// Where the last cue jump left the playhead, so repeated arrow presses
    /// step through cues during playback. Cleared by any other locate.
    pub cue_anchor: Option<f64>,
    /// When timecode went silent, so silence is reported once.
    pub chase_silent_since: Option<std::time::Instant>,
    pub chase_silence_reported: bool,
    /// Last position sent over OSC, so each timecode frame is sent once.
    pub published_timecode: Option<String>,
    /// Edge detection for the arrangement blackout notice.
    pub blackout_reported: bool,
}

/// Where decks come from: the shader and analyzer registries, every deck
/// source type, the device managers they share, and the background loader.
pub(crate) struct DeckSources {
    pub registry: ShaderRegistry,
    pub analyzer_registry: crate::analyzer::AnalyzerRegistry,
    /// Decks being built off the render thread. See `deck_loads`.
    pub deck_loader: deck_loads::DeckLoader,
    /// Every deck source type. See `sources.rs`.
    pub providers: crate::source::SourceRegistry,
    /// Device managers shared with the rest of the engine; outputs use the
    /// NDI, Syphon and Spout runtimes here.
    pub services: crate::source::Services,
    /// Camera held open for surface detection, if any. See `AcquireDetectionCamera`.
    pub detection_camera: Option<crate::camera::CameraId>,
    /// The last source type listing, reused by snapshots. See
    /// [`DeckSources::type_snapshots`].
    pub type_cache: sources::TypeCache,
}

/// What the mixer renders into: the GPU, the render size, and the frame rate.
pub(crate) struct RenderTarget {
    pub context: GpuContext,
    /// The GPU adapter, read once.
    pub gpu_info: crate::engine::types::GpuInfoSnapshot,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
}

/// Audio input and the textures shaders read it from.
pub(crate) struct Audio {
    pub manager: AudioManager,
    pub textures: crate::audio::AudioTextures,
}

/// Cross-thread message passing.
pub(crate) struct MessageBus {
    pub command_rx: tokio::sync::mpsc::UnboundedReceiver<CommandEnvelope>,
    pub command_tx: tokio::sync::mpsc::UnboundedSender<CommandEnvelope>,
    pub publication: std::sync::Arc<publish::StatePublication>,
}

// ── Main application struct ─────────────────────────────────────

/// The engine. Owns all subsystems except window/egui, grouped by use, and
/// processes `EngineCommand`s from every consumer. Work within a group is that
/// group's method; work across groups is a method here.
pub struct VardaApp {
    mixer: Mixer,
    render: RenderTarget,
    audio: Audio,
    sources: DeckSources,
    output: Outputs,
    input: Inputs,
    show: Show,
    session: Session,
    /// Interactive HTML window state (feature `html`).
    #[cfg(feature = "html")]
    interactive: interactive::InteractiveHtmlState,
    frame_stats: FrameStats,
    bus: MessageBus,

    // Channels rendered for off-air preview (`SetPreviewChannels`), by UUID.
    // `preview_channels` holds their positions, resolved each frame. Not persisted.
    preview_channel_uuids: Vec<String>,
    preview_channels: Vec<usize>,

    shutdown_requested: bool,
}

impl VardaApp {
    /// Create a `VardaApp` with a default two-channel mixer.
    ///
    /// # Errors
    /// Returns an error if the mixer's GPU targets cannot be allocated, or a
    /// subsystem `config` requires (MIDI, OSC, HTTP API) fails to start.
    pub fn new(gpu: GpuContext, config: &AppConfig) -> anyhow::Result<Self> {
        // Font scanning is slow; start it now, off the render thread.
        crate::fonts::warm();
        log::info!("[STARTUP]   Audio init...");
        let audio_manager = AudioManager::new();

        log::info!("[STARTUP]   Workspace init...");
        let workspace = Workspace::new(config.effective_workspace_root());

        // Library paths; later ones override earlier ones by shader name.
        // 1. Bundled shaders (exe-relative, for packaged .app / AppImage)
        // 2. CWD shaders/ (dev builds / cargo run)
        // 3. Workspace .varda/shaders/ (per-show user shaders)
        // 4. Platform user dir (global user shader collection)
        // 5. Any --shader-dir flags (added last, override built-ins by name)
        let mut registry = ShaderRegistry::new();
        if let Some(bundled) = crate::registry::get_bundled_shader_path()
            && let Err(e) = registry.add_library_path(&bundled)
        {
            log::warn!("Failed to add bundled shaders path: {e}");
        }
        if let Err(e) = registry.add_library_path("shaders") {
            log::warn!("Failed to add shaders path: {e}");
        }
        let ws_shaders = workspace.shaders_dir();
        if ws_shaders.is_dir()
            && let Err(e) = registry.add_library_path(&ws_shaders)
        {
            log::warn!("Failed to add workspace shaders path: {e}");
        }
        for path in crate::registry::get_default_library_paths() {
            if path.is_dir()
                && let Err(e) = registry.add_library_path(&path)
            {
                log::warn!(
                    "Failed to add user shader library path {}: {}",
                    path.display(),
                    e
                );
            }
        }
        for path in &config.shader_dirs {
            if let Err(e) = registry.add_library_path(path) {
                log::warn!("Failed to add --shader-dir {}: {}", path.display(), e);
            }
        }
        match registry.scan() {
            Ok(count) => log::info!("Loaded {count} shaders"),
            Err(e) => log::error!("Failed to scan shaders: {e}"),
        }
        if let Err(e) = registry.start_watching() {
            log::warn!("Failed to start shader hot-reload: {e}");
        }

        let mut osc_config = if workspace.has_osc() {
            OscConfig::load(workspace.osc_path()).unwrap_or_else(|e| {
                log::warn!("Failed to load OSC config: {e}, using defaults");
                OscConfig::default()
            })
        } else {
            OscConfig::default()
        };

        if config.osc_disabled {
            osc_config.enabled = false;
        }
        if let Some(port) = config.osc_port {
            osc_config.in_port = port;
        }
        for target in &config.osc_targets {
            if !osc_config.feedback_targets.contains(target) {
                osc_config.feedback_targets.push(target.clone());
            }
        }

        log::info!("[STARTUP]   OSC init...");
        let osc_receiver = if osc_config.enabled {
            match OscReceiver::new(osc_config.in_port) {
                Ok(osc) => {
                    log::info!("OSC receiver started on port {}", osc_config.in_port);
                    Some(osc)
                }
                Err(e) => {
                    log::warn!(
                        "Failed to start OSC receiver on port {}: {}",
                        osc_config.in_port,
                        e
                    );
                    None
                }
            }
        } else {
            log::info!("OSC input disabled by config");
            None
        };

        let osc_feedback = match OscFeedbackSender::new() {
            Ok(mut sender) => {
                for target in &osc_config.feedback_targets {
                    if let Err(e) = sender.add_target(target) {
                        log::warn!("Failed to add OSC feedback target '{target}': {e}");
                    }
                }
                Some(sender)
            }
            Err(e) => {
                log::warn!("Failed to create OSC feedback sender: {e}");
                None
            }
        };

        log::info!("[STARTUP]   MIDI init...");
        let mut controller_led_mgr = midi::ControllerLedManager::new();
        let mut auto_map_engine = midi::AutoMapEngine::new();
        let midi_devices = match midi::MidiDeviceManager::new() {
            Ok(mut mgr) => {
                mgr.load_user_profiles(&workspace.controller_profiles_dir());
                if workspace.controller_profiles_dir().is_dir() {
                    let _ = mgr.scan_devices();
                }
                log::info!("MIDI initialized: {} device(s)", mgr.devices.len());
                controller_led_mgr.sync_devices(&mgr);
                auto_map_engine.sync_devices(&mgr);
                Some(mgr)
            }
            Err(e) => {
                log::warn!("Failed to initialize MIDI: {e}");
                None
            }
        };

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let publication = std::sync::Arc::new(publish::StatePublication::default());

        log::info!("[STARTUP]   GPU resources (textures, mixer)...");
        let audio_textures = crate::audio::AudioTextures::new(&gpu.device);
        let calibration_textures =
            crate::renderer::context::create_calibration_textures(&gpu.device, &gpu.queue, 8);
        let mixer = Mixer::new(&gpu, DEFAULT_RENDER_WIDTH, DEFAULT_RENDER_HEIGHT)?;
        // Shipped presets live beside the built-in shaders: the bundled folder
        // in an installed build, `./shaders` in the repo.
        let built_in_presets: Vec<std::path::PathBuf> = crate::registry::get_bundled_shader_path()
            .into_iter()
            .chain(std::iter::once(std::path::PathBuf::from("shaders")))
            .map(|dir| crate::persistence::presets::built_in_deck_presets_dir(&dir))
            .filter(|dir| dir.is_dir())
            .collect();
        let preset_library = crate::persistence::presets::PresetLibrary::load_with_built_in(
            &workspace,
            built_in_presets,
        );

        Ok(Self {
            mixer,
            render: RenderTarget {
                gpu_info: {
                    let info = gpu.adapter.get_info();
                    crate::engine::types::GpuInfoSnapshot {
                        name: info.name,
                        backend: format!("{:?}", info.backend),
                        driver: info.driver,
                        driver_info: info.driver_info,
                        device_type: format!("{:?}", info.device_type),
                    }
                },
                context: gpu,
                width: DEFAULT_RENDER_WIDTH,
                height: DEFAULT_RENDER_HEIGHT,
                target_fps: config.target_fps,
            },
            audio: Audio {
                manager: audio_manager,
                textures: audio_textures,
            },
            sources: DeckSources {
                registry,
                analyzer_registry: crate::deck::analyzer_registry(),
                deck_loader: deck_loads::DeckLoader::new(),
                providers: sources::source_providers(),
                services: sources::source_services(config),
                detection_camera: None,
                type_cache: sources::TypeCache::default(),
            },
            output: Outputs {
                outputs: Vec::new(),
                sinks: sources::output_sinks(),
                sink_type_cache: sources::TypeCache::default(),
                surface_manager: SurfaceManager::new(),
                calibration_textures,
                domemaster: None,
                domemaster_resolution: crate::renderer::dome::DomemasterResolution::default(),
                dome: crate::engine::value::dome::DomeConfig::default(),
            },
            input: Inputs {
                osc_receiver,
                osc_feedback,
                osc_config,
                midi_devices,
                keymap: KeymapStore::with_defaults(),
                midi_mappings: midi::MidiMappingStore::new(),
                controller_led_mgr,
                auto_map_engine,
                clock_manager: crate::clock::ClockManager::new(),
                timecode: crate::timecode::TimecodeManager::new(),
                ltc_tap: None,
                pending_actions: inputs::PendingGlobalActions::default(),
            },
            show: Show {
                transport: crate::transport::Transport::new(),
                recorder: state::recorder::Recorder::default(),
                cue_anchor: None,
                chase_silent_since: None,
                chase_silence_reported: false,
                published_timecode: None,
                blackout_reported: false,
            },
            session: Session {
                workspace,
                preset_library,
                history: history::HistoryManager::new(),
                notifications: NotificationSystem::new(),
                editor_prefs: crate::engine::value::editor::EditorPrefs::default(),
                clipboard: None,
            },
            #[cfg(feature = "html")]
            interactive: interactive::InteractiveHtmlState::default(),
            frame_stats: FrameStats {
                last_frame_instant: std::time::Instant::now(),
                fps_history: std::collections::VecDeque::with_capacity(60),
                fps_smoothed: 0.0,
                frame_count: 0,
                system_monitor: crate::sysmon::SystemMonitor::new(),
                last_frame_submits: 0,
            },
            bus: MessageBus {
                command_rx,
                command_tx,
                publication,
            },
            preview_channel_uuids: Vec::new(),
            preview_channels: Vec::new(),
            shutdown_requested: false,
        })
    }

    /// A command sender for other threads (HTTP API, CLI).
    pub fn command_sender(&self) -> tokio::sync::mpsc::UnboundedSender<CommandEnvelope> {
        self.bus.command_tx.clone()
    }

    /// Where other threads read the published engine snapshot.
    pub fn state_reader(&self) -> std::sync::Arc<publish::StatePublication> {
        self.bus.publication.clone()
    }

    /// Process all queued cross-thread commands. Called once per frame.
    pub fn process_commands(&mut self) {
        self.attach_finished_deck_loads();
        while let Ok((cmd, reply_tx)) = self.bus.command_rx.try_recv() {
            // Record undo state for bus commands (the GUI records its own). A
            // recording pass already has its entry. Kept only on success, so a
            // rejected command doesn't clear redo.
            let before =
                (self.is_undoable(&cmd) && !self.is_recording()).then(|| self.history_snapshot());
            let result = self.execute_command(cmd);
            if let Some(snapshot) = before
                && !matches!(result, CommandResult::Err { .. })
            {
                self.session.history.push(snapshot);
            }
            if let Some(tx) = reply_tx {
                let _ = tx.send(result);
            }
        }
    }

    /// Apply `f` to a deck's auto-transition, creating it if needed.
    fn exec_auto_transition(
        &mut self,
        deck_uuid: &str,
        f: impl FnOnce(&mut crate::channel::DeckAutoTransition),
    ) -> CommandResult {
        let (ch_idx, deck_idx) = match self.mixer.resolve_deck(deck_uuid) {
            Ok(loc) => loc,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                };
            }
        };
        let slot = &mut self.mixer.channels_mut()[ch_idx].decks[deck_idx];
        f(slot
            .auto_transition
            .get_or_insert_with(crate::channel::DeckAutoTransition::new));
        CommandResult::Ok
    }

    /// Update a modulation source by UUID.
    fn exec_modulation_update(
        &mut self,
        uuid: &str,
        f: impl FnOnce(&mut crate::modulation::ModulationSource),
    ) -> CommandResult {
        if let Some(source) = self.mixer.modulation_mut().source_mut(uuid) {
            f(source);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Modulation source {uuid} not found"),
            }
        }
    }

    /// Build a framework-free engine state snapshot.
    pub fn build_engine_state(&self) -> EngineState {
        snapshot::build_engine_state(self)
    }

    /// Build and publish a snapshot. Called every 10th frame.
    pub fn publish_state(&self) {
        self.publish(self.build_engine_state());
    }

    /// Publish a snapshot already built this frame, such as the GUI's.
    pub fn publish(&self, state: EngineState) {
        self.bus.publication.publish(state);
    }

    // ── Public accessors ─────────────────────────────────────────────

    /// Read-only access to the GPU context.
    pub fn gpu_context(&self) -> &GpuContext {
        &self.render.context
    }

    /// Command buffer commits issued during the previous frame.
    pub fn last_frame_submits(&self) -> u32 {
        self.frame_stats.last_frame_submits
    }

    /// Read-only access to the mixer.
    pub fn mixer_ref(&self) -> &crate::mixer::Mixer {
        &self.mixer
    }

    /// Read-only access to the camera manager.
    pub fn camera_manager(&self) -> &CameraManager {
        self.sources.service::<CameraManager>()
    }

    /// Open a camera, returning its resolution.
    ///
    /// # Errors
    /// Returns an error if the camera is missing, access is refused, or its
    /// GPU textures cannot be allocated.
    pub(crate) fn open_camera(
        &mut self,
        id: crate::camera::CameraId,
    ) -> anyhow::Result<(u32, u32)> {
        self.sources
            .service_mut::<CameraManager>()
            .open_camera(id, &self.render.context.device)
    }

    /// Read-only access to the screen-capture manager.
    pub fn screen_capture_manager(&self) -> &ScreenCaptureManager {
        self.sources.service::<ScreenCaptureManager>()
    }

    /// Read-only access to the depth-sensor manager.
    pub fn depth_manager(&self) -> &DepthSensorManager {
        self.sources.service::<DepthSensorManager>()
    }

    /// Every registered deck source type.
    pub fn source_providers(&self) -> &crate::source::SourceRegistry {
        &self.sources.providers
    }

    /// Read-only access to the outputs.
    pub fn outputs_ref(&self) -> &[crate::output::Output] {
        &self.output.outputs
    }

    /// Read-only access to the domemaster renderer output view (if enabled).
    pub fn domemaster_view(&self) -> Option<&wgpu::TextureView> {
        self.output
            .domemaster
            .as_ref()
            .map(super::internal::renderer::dome::DomemasterRenderer::output_view)
    }

    /// Edge length of the live domemaster texture, or `None` without a
    /// renderer. Can differ from [`Self::domemaster_resolution`] after a failed rebuild.
    pub fn domemaster_output_size(&self) -> Option<u32> {
        self.output
            .domemaster
            .as_ref()
            .map(super::internal::renderer::dome::DomemasterRenderer::output_size)
    }

    /// Create the domemaster renderer if needed, and enable it.
    pub fn ensure_domemaster(&mut self) {
        if let Some(dome) = &mut self.output.domemaster {
            dome.enabled = true;
        } else {
            let config = crate::renderer::dome::DomemasterConfig {
                resolution: self.output.domemaster_resolution,
                ..crate::renderer::dome::DomemasterConfig::default()
            };
            match crate::renderer::dome::DomemasterRenderer::new(
                &self.render.context.device,
                self.render.context.compositing_format,
                config,
            ) {
                Ok(mut dome) => {
                    dome.enabled = true;
                    self.output.domemaster = Some(dome);
                    log::info!("Domemaster renderer created and enabled");
                }
                Err(e) => {
                    log::error!("Failed to create domemaster renderer: {e}");
                }
            }
        }
    }

    /// Size the domemaster is rendered at.
    pub fn domemaster_resolution(&self) -> crate::renderer::dome::DomemasterResolution {
        self.output.domemaster_resolution
    }

    /// Set the domemaster output size, rebuilding a live renderer. A failed
    /// rebuild leaves no renderer, so the dome goes black rather than keeping
    /// the old size.
    pub fn set_domemaster_resolution(
        &mut self,
        resolution: crate::renderer::dome::DomemasterResolution,
    ) {
        if resolution == self.output.domemaster_resolution {
            return;
        }
        log::info!(
            "Domemaster resolution: {} → {}",
            self.output.domemaster_resolution,
            resolution
        );
        self.output.domemaster_resolution = resolution;

        let Some(existing) = self.output.domemaster.take() else {
            return;
        };
        let config = crate::renderer::dome::DomemasterConfig {
            resolution,
            ..existing.config.clone()
        };
        let was_enabled = existing.enabled;
        let content_rotation = existing.content_rotation;
        drop(existing);

        match crate::renderer::dome::DomemasterRenderer::new(
            &self.render.context.device,
            self.render.context.compositing_format,
            config,
        ) {
            Ok(mut dome) => {
                dome.enabled = was_enabled;
                dome.content_rotation = content_rotation;
                self.output.domemaster = Some(dome);
            }
            Err(e) => {
                log::error!("Failed to rebuild domemaster renderer at {resolution}: {e}");
            }
        }
    }

    /// Dome projection the domemaster is rendered for.
    pub fn dome_config(&self) -> crate::engine::value::dome::DomeConfig {
        self.output.dome
    }

    /// The camera held open for surface detection, if any.
    pub fn detection_camera(&self) -> Option<crate::camera::CameraId> {
        self.sources.detection_camera
    }

    /// Resolve the cued channel UUIDs to their current positions, reusing the
    /// buffer the render gate reads.
    fn resolve_preview_channels(&mut self) {
        self.preview_channels.clear();
        for uuid in &self.preview_channel_uuids {
            if let Some(idx) = self.mixer.find_channel_by_uuid(uuid) {
                self.preview_channels.push(idx);
            }
        }
    }

    /// Number of loaded shaders.
    pub fn shader_count(&self) -> usize {
        self.sources.registry.count()
    }

    /// Tick notification expiry timers.
    pub fn update_notifications(&mut self) {
        self.session.notifications.update();
    }

    /// Current render width.
    pub fn render_width(&self) -> u32 {
        self.render.width
    }

    /// Current render height.
    pub fn render_height(&self) -> u32 {
        self.render.height
    }

    /// Current target FPS (0 = uncapped).
    pub fn target_fps(&self) -> u32 {
        self.render.target_fps
    }

    /// Where the workspace keeps its LUT files.
    pub fn luts_dir(&self) -> std::path::PathBuf {
        self.session.workspace.luts_dir()
    }

    /// Whether the API or a signal asked the engine to shut down.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Largest render width or height the GPU can allocate, the only limit.
    pub fn max_render_dimension(&self) -> u32 {
        self.render.context.device.limits().max_texture_dimension_2d
    }

    /// Change the master render resolution. Resizes all textures in the pipeline.
    ///
    /// Zero is rejected; oversize requests are clamped to the GPU limit.
    pub fn set_render_resolution(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            log::warn!("Ignoring zero render resolution {width}×{height}");
            return;
        }
        let max_dim = self.max_render_dimension();
        let (width, height) = clamp_resolution_to_gpu(width, height, max_dim);
        if width == self.render.width && height == self.render.height {
            return;
        }
        log::info!(
            "Changing render resolution: {}×{} → {}×{}",
            self.render.width,
            self.render.height,
            width,
            height
        );
        self.render.width = width;
        self.render.height = height;
        self.mixer.resize(&self.render.context, width, height);
        // Textures were recreated.
        self.mixer.clear_sub_mix_cache();
        let stopped = self.resize_headless_outputs(width, height);
        self.session
            .notifications
            .info(format!("📐 Resolution changed to {width}×{height}"));
        if !stopped.is_empty() {
            self.session.notifications.warn(format!(
                "⏹ Stopped {} to resize: {}",
                if stopped.len() == 1 {
                    "an output".to_string()
                } else {
                    format!("{} outputs", stopped.len())
                },
                stopped.join(", ")
            ));
        }
    }

    /// Set the target FPS. 0 = uncapped.
    pub fn set_target_fps(&mut self, fps: u32) {
        if fps == self.render.target_fps {
            return;
        }
        log::info!("Target FPS: {} → {}", self.render.target_fps, fps);
        self.render.target_fps = fps;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Parse `AppConfig` from a simulated CLI invocation.
    fn parse_args(args: &[&str]) -> AppConfig {
        AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
    }

    #[test]
    fn app_config_defaults() {
        let config = parse_args(&[]);
        assert!(!config.headless);
        assert_eq!(config.api_port, 8080);
        assert_eq!(config.target_fps, 60);
        assert!(config.workspace_root.is_none());
        assert!(config.scene_path.is_none());
        assert!(config.stage_path.is_none());
        assert!(config.osc_port.is_none());
        assert_eq!(config.osc_targets.len(), 0);
        assert!(!config.osc_disabled);
        assert!(!config.ndi_disabled);
        assert!(!config.syphon_disabled);
        assert_eq!(config.shader_dirs.len(), 0);
    }

    #[test]
    fn app_config_shader_dirs_repeatable() {
        let config = parse_args(&[
            "--shader-dir",
            "/srv/shaders/show-a",
            "--shader-dir",
            "/media/usb/shaders",
        ]);
        assert_eq!(
            config.shader_dirs,
            vec![
                std::path::PathBuf::from("/srv/shaders/show-a"),
                std::path::PathBuf::from("/media/usb/shaders"),
            ]
        );
    }

    #[test]
    fn app_config_headless_with_port() {
        let config = parse_args(&["--headless", "--port", "3030", "--fps", "30"]);
        assert!(config.headless);
        assert_eq!(config.api_port, 3030);
        assert_eq!(config.target_fps, 30);
    }

    #[test]
    fn app_config_osc_flags() {
        let config = parse_args(&[
            "--osc-port",
            "7000",
            "--osc-out",
            "192.168.1.1:8000",
            "--osc-out",
            "10.0.0.1:9000",
        ]);
        assert_eq!(config.osc_port, Some(7000));
        assert_eq!(
            config.osc_targets,
            vec!["192.168.1.1:8000", "10.0.0.1:9000"]
        );
        assert!(!config.osc_disabled);
    }

    #[test]
    fn app_config_disable_flags() {
        let config = parse_args(&["--no-osc", "--no-ndi", "--no-syphon"]);
        assert!(config.osc_disabled);
        assert!(config.ndi_disabled);
        assert!(config.syphon_disabled);
    }

    #[test]
    fn workspace_resolution_explicit_flag_wins() {
        let explicit = std::path::PathBuf::from("/tmp/show");
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join(".varda")).unwrap();
        let home = tempfile::tempdir().unwrap();

        let result = AppConfig::resolve_workspace_root(
            Some(explicit.as_path()),
            Some(cwd.path()),
            Some(home.path()),
        );
        assert_eq!(result, explicit);
    }

    #[test]
    fn workspace_resolution_cwd_with_varda_dir() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join(".varda")).unwrap();
        let home = tempfile::tempdir().unwrap();

        let result = AppConfig::resolve_workspace_root(None, Some(cwd.path()), Some(home.path()));
        assert_eq!(result, cwd.path());
    }

    #[test]
    fn workspace_resolution_falls_back_to_home() {
        let cwd = tempfile::tempdir().unwrap(); // no .varda/ dir
        let home = tempfile::tempdir().unwrap();

        let result = AppConfig::resolve_workspace_root(None, Some(cwd.path()), Some(home.path()));
        assert_eq!(result, home.path());
    }

    #[test]
    fn workspace_resolution_no_cwd_no_home() {
        let result = AppConfig::resolve_workspace_root(None, None, None);
        assert_eq!(result, std::path::PathBuf::from("."));
    }

    #[test]
    fn workspace_resolution_explicit_overrides_cwd_with_varda() {
        let explicit = std::path::PathBuf::from("/tmp/custom");
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join(".varda")).unwrap();

        let result =
            AppConfig::resolve_workspace_root(Some(explicit.as_path()), Some(cwd.path()), None);
        assert_eq!(result, explicit);
    }

    #[test]
    fn app_config_clone() {
        let config = parse_args(&["--headless", "--port", "3030", "--no-ndi"]);
        let cloned = config.clone();
        assert!(cloned.headless);
        assert_eq!(cloned.api_port, 3030);
        assert!(cloned.ndi_disabled);
    }

    // ── Engine smoke tests ──────────────────────────────────────

    fn headless_app() -> Option<VardaApp> {
        crate::testing::headless_app()
    }

    fn channel_uuid(app: &VardaApp, idx: usize) -> String {
        app.build_engine_state().mixer.channels[idx].uuid.clone()
    }

    #[test]
    fn smoke_engine_starts_with_two_channels() {
        let Some(app) = headless_app() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let state = app.build_engine_state();
        assert_eq!(
            state.mixer.channels.len(),
            2,
            "default mixer has 2 channels"
        );
        assert_eq!(state.mixer.crossfader, 0.0, "crossfader starts at A");
    }

    #[test]
    fn smoke_add_channel_via_command() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((crate::engine::EngineCommand::AddChannel, None))
            .unwrap();
        app.process_commands();
        let state = app.build_engine_state();
        assert_eq!(state.mixer.channels.len(), 3);
    }

    #[test]
    fn smoke_set_crossfader_via_command() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((crate::engine::EngineCommand::SetCrossfader(0.75), None))
            .unwrap();
        app.process_commands();
        let state = app.build_engine_state();
        assert!((state.mixer.crossfader - 0.75).abs() < 1e-5);
    }

    #[test]
    fn smoke_render_frame_no_crash() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Render several frames — verify no panics and FPS stabilizes
        for _ in 0..5 {
            app.update_frame_timing();
            app.render_mixer_frame();
        }
        let state = app.build_engine_state();
        assert!(state.fps >= 0.0, "FPS should be non-negative");
    }

    #[test]
    fn smoke_add_solid_color_deck() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let before = app.build_engine_state().mixer.channels[0].decks.len();
        let tx = app.command_sender();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send((
            crate::engine::EngineCommand::AddDeck {
                channel_uuid: ch0,
                source: crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            },
            Some(reply_tx),
        ))
        .unwrap();
        app.process_commands();
        let result = reply_rx.blocking_recv().unwrap();
        // Deck-creating commands report the new deck's UUID.
        assert!(
            matches!(result, crate::engine::CommandResult::OkWithId { .. }),
            "command should succeed: {result:?}"
        );
        let after = app.build_engine_state().mixer.channels[0].decks.len();
        assert_eq!(after, before + 1, "should have one more deck");
    }

    #[test]
    fn smoke_set_channel_opacity() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let tx = app.command_sender();
        tx.send((
            crate::engine::EngineCommand::SetChannelOpacity {
                channel_uuid: ch0,
                opacity: 0.5,
            },
            None,
        ))
        .unwrap();
        app.process_commands();
        let state = app.build_engine_state();
        assert!((state.mixer.channels[0].opacity - 0.5).abs() < 1e-5);
    }

    /// Bus commands record undo state, and bus `Undo`/`Redo` restore it.
    #[test]
    fn api_command_undo_redo_roundtrip() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let tx = app.command_sender();

        // Baseline: no history yet, nothing to undo.
        assert!(!app.history_can_undo(), "fresh app has empty undo stack");
        let original = app.build_engine_state().mixer.channels[0].opacity;

        // An undoable, bus-driven edit records a pre-mutation snapshot.
        let new_opacity = if (original - 0.5).abs() < 1e-3 {
            0.25
        } else {
            0.5
        };
        tx.send((
            crate::engine::EngineCommand::SetChannelOpacity {
                channel_uuid: ch0,
                opacity: new_opacity,
            },
            None,
        ))
        .unwrap();
        app.process_commands();
        assert!(
            app.history_can_undo(),
            "undoable API command should record history"
        );
        assert!((app.build_engine_state().mixer.channels[0].opacity - new_opacity).abs() < 1e-5);

        // Undo over the bus restores the pre-edit opacity.
        tx.send((crate::engine::EngineCommand::Undo, None)).unwrap();
        app.process_commands();
        assert!(
            (app.build_engine_state().mixer.channels[0].opacity - original).abs() < 1e-5,
            "API undo must restore the pre-command state"
        );
        assert!(app.history_can_redo(), "after undo, redo is available");

        // Redo re-applies the edit.
        tx.send((crate::engine::EngineCommand::Redo, None)).unwrap();
        app.process_commands();
        assert!(
            (app.build_engine_state().mixer.channels[0].opacity - new_opacity).abs() < 1e-5,
            "API redo must re-apply the command"
        );
    }

    /// Live-control commands (crossfader) are not recorded for undo.
    #[test]
    fn live_control_commands_do_not_record_history() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((crate::engine::EngineCommand::SetCrossfader(0.5), None))
            .unwrap();
        app.process_commands();
        assert!(
            !app.history_can_undo(),
            "crossfader is a live control and must not record history"
        );
    }

    #[test]
    fn stage_diff_restores_surface_geometry() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((
            crate::engine::EngineCommand::AddSurface {
                name: "S".into(),
                source: crate::engine::OutputSource::Master,
            },
            None,
        ))
        .unwrap();
        app.process_commands();

        // Capture the added surface's uuid + original geometry.
        let (uuid, orig) = {
            let s = app
                .output
                .surface_manager
                .surfaces
                .last()
                .expect("surface added");
            (s.uuid.clone(), s.vertices.clone())
        };

        let snap = app.history_snapshot();

        tx.send((
            crate::engine::EngineCommand::MoveSurface {
                uuid: uuid.clone(),
                dx: 0.2,
                dy: 0.1,
            },
            None,
        ))
        .unwrap();
        app.process_commands();
        let moved = app
            .output
            .surface_manager
            .surfaces
            .last()
            .unwrap()
            .vertices
            .clone();
        assert_ne!(orig, moved, "surface should have moved");

        // Restore the stage snapshot — geometry returns to pre-move state.
        app.apply_stage_diff(&snap.stage);
        let restored = app
            .output
            .surface_manager
            .surfaces
            .last()
            .unwrap()
            .vertices
            .clone();
        assert_eq!(
            orig, restored,
            "apply_stage_diff should restore pre-move surface geometry"
        );
    }

    #[test]
    fn smoke_add_lfo_modulation() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send((
            crate::engine::EngineCommand::AddLfo {
                waveform: crate::modulation::LFOWaveform::Sine,
                frequency: 2.0,
            },
            Some(reply_tx),
        ))
        .unwrap();
        app.process_commands();
        let result = reply_rx.blocking_recv().unwrap();
        assert!(
            matches!(result, crate::engine::CommandResult::Ok),
            "AddLfo failed: {result:?}"
        );
        let state = app.build_engine_state();
        assert!(!state.modulation.sources.is_empty());
    }

    #[test]
    fn smoke_remove_channel() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((crate::engine::EngineCommand::AddChannel, None))
            .unwrap();
        app.process_commands();
        assert_eq!(app.build_engine_state().mixer.channels.len(), 3);
        let ch2 = channel_uuid(&app, 2);
        tx.send((
            crate::engine::EngineCommand::RemoveChannel { channel_uuid: ch2 },
            None,
        ))
        .unwrap();
        app.process_commands();
        assert_eq!(app.build_engine_state().mixer.channels.len(), 2);
    }

    #[test]
    fn smoke_set_deck_blend_mode() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let added = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddDeck {
                channel_uuid: ch0,
                source: crate::solid_color::SolidColor::config_for([0.0, 1.0, 0.0, 1.0]),
            },
        );
        let crate::engine::CommandResult::OkWithId { uuid } = added else {
            panic!("expected OkWithId, got {added:?}");
        };
        fire(
            &mut app,
            crate::engine::EngineCommand::SetDeckBlendMode {
                deck_uuid: uuid,
                mode: crate::engine::BlendMode::Add,
            },
        );
        let state = app.build_engine_state();
        assert_eq!(
            state.mixer.channels[0].decks[0].blend_mode,
            crate::engine::BlendMode::Add
        );
    }

    #[test]
    fn clamp_resolution_leaves_in_range_untouched() {
        // Within the GPU limit: returned unchanged.
        assert_eq!(clamp_resolution_to_gpu(1920, 1080, 16384), (1920, 1080));
        assert_eq!(clamp_resolution_to_gpu(16384, 16384, 16384), (16384, 16384));
    }

    #[test]
    fn clamp_resolution_caps_each_dimension_to_gpu_max() {
        // Beyond the GPU limit: each dimension clamped independently.
        assert_eq!(clamp_resolution_to_gpu(20000, 12000, 16384), (16384, 12000));
        assert_eq!(clamp_resolution_to_gpu(30000, 30000, 16384), (16384, 16384));
        assert_eq!(clamp_resolution_to_gpu(1024, 99999, 8192), (1024, 8192));
    }

    #[test]
    fn smoke_set_render_resolution() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_render_resolution(1280, 720);
        assert_eq!(app.render_width(), 1280);
        assert_eq!(app.render_height(), 720);
    }

    #[test]
    fn smoke_set_render_resolution_zero_ignored() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_render_resolution(0, 0);
        // Previous resolution kept.
        assert!(app.render_width() > 0);
        assert!(app.render_height() > 0);
    }

    /// A headless output follows render resolution changes made after creation.
    #[test]
    fn headless_outputs_follow_the_render_resolution() {
        let Some(mut app) = headless_app() else {
            return;
        };
        fire(
            &mut app,
            crate::engine::EngineCommand::CreateOutput {
                sink: crate::output::SinkConfig::new("recording").with("path", "unused.mp4"),
            },
        );

        let dimensions = |app: &VardaApp| {
            app.output
                .outputs
                .iter()
                .map(crate::output::Output::size)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            dimensions(&app),
            vec![(app.render_width(), app.render_height())],
            "a new output should start at the current render resolution"
        );

        app.set_render_resolution(1080, 1920);
        assert_eq!(
            dimensions(&app),
            vec![(1080, 1920)],
            "the output should have followed the resolution change"
        );
    }

    #[test]
    fn smoke_publish_and_read_state() {
        let Some(app) = headless_app() else {
            return;
        };
        let reader = app.state_reader();
        app.publish_state();
        let state = reader.latest().expect("state should be published");
        assert_eq!(state.mixer.channels.len(), 2);
    }

    #[test]
    fn smoke_notifications() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.session.notifications.info("Test notification");
        app.update_notifications();
        // Verify no crash
    }

    // ── Extended smoke tests ───────────────────────────────────────

    /// Send a command, process it, and return the result.
    fn send_cmd(
        app: &mut VardaApp,
        cmd: crate::engine::EngineCommand,
    ) -> crate::engine::CommandResult {
        let tx = app.command_sender();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send((cmd, Some(reply_tx))).unwrap();
        app.process_commands();
        reply_rx.blocking_recv().unwrap()
    }

    /// Send a command without waiting for a result.
    fn fire(app: &mut VardaApp, cmd: crate::engine::EngineCommand) {
        app.command_sender().send((cmd, None)).unwrap();
        app.process_commands();
    }

    #[test]
    fn smoke_auto_crossfade_lifecycle() {
        let Some(mut app) = headless_app() else {
            return;
        };
        fire(
            &mut app,
            crate::engine::EngineCommand::AutoCrossfade {
                target: 1.0,
                duration_secs: 0.05,
                easing: crate::mixer::CrossfadeEasing::Linear,
            },
        );
        // Tick enough frames for it to complete
        for _ in 0..60 {
            app.update_frame_timing();
            app.render_mixer_frame();
        }
        // Headless timing is unpredictable; only check for no panic.
    }

    #[test]
    fn smoke_add_video_deck_no_crash() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        // Bad path: should not panic
        let _ = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddDeck {
                channel_uuid: ch0,
                source: crate::source::SourceConfig::new("Video")
                    .with("path", "/nonexistent/video.mp4"),
            },
        );
    }

    #[test]
    fn source_param_a_source_does_not_declare_is_invalid_input() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let added = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddDeck {
                channel_uuid: ch0,
                source: crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            },
        );
        let crate::engine::CommandResult::OkWithId { uuid } = added else {
            panic!("expected OkWithId, got {added:?}");
        };

        // A solid color has no transport, so a video control is refused.
        let result = send_cmd(
            &mut app,
            crate::engine::EngineCommand::SetSourceParam {
                deck_uuid: uuid.clone(),
                name: "speed".into(),
                value: crate::source::ControlValue::Float(0.75),
            },
        );
        assert!(matches!(
            result,
            crate::engine::CommandResult::Err {
                code: crate::engine::ErrorCode::InvalidInput,
                ..
            }
        ));
        let result = send_cmd(
            &mut app,
            crate::engine::EngineCommand::TriggerSourceAction {
                deck_uuid: uuid,
                action: "clear".into(),
            },
        );
        assert!(matches!(
            result,
            crate::engine::CommandResult::Err {
                code: crate::engine::ErrorCode::InvalidInput,
                ..
            }
        ));
    }

    #[test]
    fn source_param_writes_reach_the_source() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let added = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddDeck {
                channel_uuid: ch0,
                source: crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            },
        );
        let crate::engine::CommandResult::OkWithId { uuid } = added else {
            panic!("expected OkWithId, got {added:?}");
        };
        let green = crate::source::ControlValue::Color([0.0, 1.0, 0.0, 1.0]);
        let result = send_cmd(
            &mut app,
            crate::engine::EngineCommand::SetSourceParam {
                deck_uuid: uuid.clone(),
                name: "color".into(),
                value: green.clone(),
            },
        );
        assert!(matches!(result, crate::engine::CommandResult::Ok));
        let state = app.build_engine_state();
        let deck = &state.mixer.channels[0]
            .decks
            .iter()
            .find(|d| d.uuid == uuid)
            .expect("deck in snapshot");
        assert_eq!(deck.source.source_type, "SolidColor");
        assert_eq!(deck.source.status.params.get("color"), Some(&green));
    }

    #[test]
    fn smoke_create_sequence() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let r = send_cmd(&mut app, crate::engine::EngineCommand::CreateSequence);
        let crate::engine::CommandResult::OkWithId { uuid } = r else {
            panic!("expected OkWithId, got {r:?}");
        };
        let state = app.build_engine_state();
        assert_eq!(state.mixer.sequences.len(), 1);
        assert_eq!(state.mixer.sequences[0].uuid, uuid);
    }

    #[test]
    fn smoke_sequence_add_steps() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let ch1 = channel_uuid(&app, 1);
        let created = send_cmd(&mut app, crate::engine::EngineCommand::CreateSequence);
        let crate::engine::CommandResult::OkWithId { uuid: seq } = created else {
            panic!("expected OkWithId, got {created:?}");
        };
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddFadeStep {
                sequence_uuid: seq.clone(),
                from_channel_uuid: ch0,
                to_channel_uuid: ch1,
            },
        );
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddWaitStep { sequence_uuid: seq },
        );
        let state = app.build_engine_state();
        assert_eq!(state.mixer.sequences[0].steps.len(), 2);
    }

    #[test]
    fn smoke_effect_chain_operations() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let added = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddDeck {
                channel_uuid: ch0,
                source: crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            },
        );
        let crate::engine::CommandResult::OkWithId { uuid: deck } = added else {
            panic!("expected OkWithId, got {added:?}");
        };
        let r = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddEffect {
                target: crate::engine::EffectTarget::Deck(deck),
                shader_name: "invert".into(),
            },
        );
        let crate::engine::CommandResult::OkWithId { uuid: effect } = r else {
            panic!("expected OkWithId, got {r:?}");
        };
        let toggled = send_cmd(
            &mut app,
            crate::engine::EngineCommand::ToggleEffect {
                effect_uuid: effect.clone(),
            },
        );
        assert!(matches!(toggled, crate::engine::CommandResult::Ok));
        let removed = send_cmd(
            &mut app,
            crate::engine::EngineCommand::RemoveEffect {
                effect_uuid: effect,
            },
        );
        assert!(matches!(removed, crate::engine::CommandResult::Ok));
    }

    #[test]
    fn smoke_multiple_modulation_sources() {
        let Some(mut app) = headless_app() else {
            return;
        };
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddLfo {
                waveform: crate::modulation::LFOWaveform::Sine,
                frequency: 1.0,
            },
        );
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddStepSequencer {
                num_steps: 8,
                rate: 2.0,
            },
        );
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddAdsr {
                attack: 0.1,
                decay: 0.2,
                sustain: 0.7,
                release: 0.3,
            },
        );
        let state = app.build_engine_state();
        assert_eq!(state.modulation.sources.len(), 3);
    }

    #[test]
    fn smoke_adsr_trigger_release() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let r = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddAdsr {
                attack: 0.01,
                decay: 0.01,
                sustain: 0.5,
                release: 0.01,
            },
        );
        assert!(matches!(r, crate::engine::CommandResult::Ok));
        let state = app.build_engine_state();
        let uuid = state.modulation.sources[0].uuid.clone();
        // Trigger
        fire(
            &mut app,
            crate::engine::EngineCommand::TriggerAdsr { uuid: uuid.clone() },
        );
        for _ in 0..5 {
            app.update_frame_timing();
            app.render_mixer_frame();
        }
        // Release
        fire(&mut app, crate::engine::EngineCommand::ReleaseAdsr { uuid });
        for _ in 0..5 {
            app.update_frame_timing();
            app.render_mixer_frame();
        }
    }

    #[test]
    fn smoke_set_channel_blend_mode() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        fire(
            &mut app,
            crate::engine::EngineCommand::SetChannelBlendMode {
                channel_uuid: ch0,
                mode: crate::engine::BlendMode::Add,
            },
        );
        let state = app.build_engine_state();
        assert_eq!(
            state.mixer.channels[0].blend_mode,
            crate::engine::BlendMode::Add
        );
    }
}
