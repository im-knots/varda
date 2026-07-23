//! Application layer — concrete engine implementation.
//!
//! VardaApp owns all engine subsystems (Mixer, Audio, Cameras, MIDI, OSC,
//! ShaderRegistry, SurfaceManager) and implements the engine traits.
//!
//! The main.rs `App` struct owns window/egui state and holds a `VardaApp`.

mod actions;
mod commands;
mod engine_impl;
pub(crate) mod history;
mod inputs;
/// Interactive mode for HTML decks (feature `html`). See /spec/html-source.md §4.
#[cfg(feature = "html")]
pub(crate) mod interactive;
mod outputs;
pub(crate) mod render;
mod snapshot;
pub(crate) mod state;
mod surfaces;
mod workspace;

/// Default render resolution for all decks and stage output (Full HD 1080p)
pub const DEFAULT_RENDER_WIDTH: u32 = 1920;
/// Default render resolution for all decks and stage output (Full HD 1080p)
pub const DEFAULT_RENDER_HEIGHT: u32 = 1080;

/// Clamp a requested render resolution to what the GPU can allocate.
///
/// Varda imposes no artificial resolution cap; the only bound is the GPU's
/// `max_texture_dimension_2d` (spec/resolution-and-scaling.md, "No Maximum
/// Resolution"). Each dimension is independently clamped to `max_dim`. Zero is
/// left untouched — callers reject it separately so this stays a pure clamp.
pub(crate) fn clamp_resolution_to_gpu(width: u32, height: u32, max_dim: u32) -> (u32, u32) {
    (width.min(max_dim), height.min(max_dim))
}

/// Session configuration derived from CLI flags + workspace defaults.
/// CLI flags override persisted config for the session without modifying files.
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

    /// Disable HTML deck sources (skips Servo rendering)
    #[arg(long = "no-html")]
    pub html_disabled: bool,

    /// Additional shader library directory (repeatable). Scanned and
    /// hot-reloaded alongside the built-in locations. Added last, so a
    /// shader here overrides a built-in shader of the same name — useful
    /// for pointing at show- or rig-specific shader folders in the field.
    #[arg(long = "shader-dir")]
    pub shader_dirs: Vec<std::path::PathBuf>,
}

impl AppConfig {
    /// Resolve workspace root with three-tier fallback:
    /// 1. Explicit `--workspace` flag (highest priority)
    /// 2. CWD if it contains a `.varda/` directory (project-local workspace)
    /// 3. Home directory as root → uses `~/.varda/` (default workspace)
    pub fn effective_workspace_root(&self) -> std::path::PathBuf {
        Self::resolve_workspace_root(
            self.workspace_root.as_deref(),
            std::env::current_dir().ok().as_deref(),
            dirs::home_dir().as_deref(),
        )
    }

    /// Pure workspace resolution — testable without touching environment.
    fn resolve_workspace_root(
        explicit: Option<&std::path::Path>,
        cwd: Option<&std::path::Path>,
        home: Option<&std::path::Path>,
    ) -> std::path::PathBuf {
        // Tier 1: explicit --workspace flag
        if let Some(ws) = explicit {
            return ws.to_path_buf();
        }
        // Tier 2: CWD has .varda/
        if let Some(cwd) = cwd {
            if cwd.join(".varda").is_dir() {
                return cwd.to_path_buf();
            }
        }
        // Tier 3: home directory (workspace data lives at ~/.varda/)
        if let Some(home) = home {
            return home.to_path_buf();
        }
        // Ultimate fallback
        std::path::PathBuf::from(".")
    }
}

use crate::audio::AudioManager;
use crate::camera::CameraManager;
use crate::keymap::KeymapStore;
use crate::midi;
use crate::mixer::Mixer;
use crate::notifications::NotificationSystem;
use crate::osc::{OscConfig, OscFeedbackSender, OscReceiver};
use crate::persistence::Workspace;
use crate::registry::ShaderRegistry;
use crate::renderer::context::{GpuContext, UnifiedOutput};
use crate::surface::SurfaceManager;

use crate::engine::{CommandEnvelope, CommandResult, EngineState, ErrorCode};

// ── Domain sub-structs ──────────────────────────────────────────

/// Input subsystem: OSC, MIDI, keyboard shortcuts, clock.
pub(crate) struct InputSubsystem {
    pub osc_receiver: Option<OscReceiver>,
    pub osc_feedback: Option<OscFeedbackSender>,
    pub osc_config: OscConfig,
    pub midi_devices: Option<midi::MidiDeviceManager>,
    pub midi_mappings: midi::MidiMappingStore,
    pub controller_led_mgr: midi::ControllerLedManager,
    pub auto_map_engine: midi::AutoMapEngine,
    pub keymap: KeymapStore,
    pub clock_manager: crate::clock::ClockManager,
}

/// Output and surface subsystem: windows, headless outputs, surface layout, dome.
pub(crate) struct OutputSubsystem {
    pub outputs: Vec<UnifiedOutput>,
    pub surface_manager: SurfaceManager,
    pub calibration_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    pub domemaster: Option<crate::renderer::dome::DomemasterRenderer>,
    pub pending_output_creates: Vec<crate::scene::OutputConfig>,
    pub cached_monitors: Vec<(String, winit::monitor::MonitorHandle)>,
}

/// External I/O managers: NDI, Syphon, SRT/HLS/DASH/RTMP streams.
pub(crate) struct ExternalIO {
    pub ndi_manager: crate::ndi::NdiManager,
    #[cfg(target_os = "macos")]
    pub syphon_manager: crate::syphon::SyphonManager,
    /// Syphon decks restored from the workspace whose server is not yet
    /// published. The render thread auto-binds them once the server appears
    /// (`VardaApp::reconcile_syphon`). See `persistence::PendingSyphonDeck`.
    #[cfg(target_os = "macos")]
    pub pending_syphon: Vec<crate::persistence::PendingSyphonDeck>,
    /// Throttle for periodic Syphon re-discovery on the render thread.
    #[cfg(target_os = "macos")]
    pub last_syphon_scan: std::time::Instant,
    pub stream_manager: crate::stream::StreamManager,
    pub stream_library: Vec<(String, crate::stream::SrtMode)>,
    pub hls_library: Vec<String>,
    pub dash_library: Vec<String>,
    pub rtmp_library: Vec<(String, crate::stream::RtmpMode)>,
    pub html_manager: crate::html::HtmlManager,
    pub html_library: Vec<String>,
}

/// Frame timing and system monitoring.
pub(crate) struct FrameStats {
    pub last_frame_instant: std::time::Instant,
    pub fps_history: std::collections::VecDeque<f32>,
    pub fps_smoothed: f32,
    pub frame_count: u64,
    pub system_monitor: crate::sysmon::SystemMonitor,
}

/// Session persistence: workspace, presets, undo/redo, notifications.
pub(crate) struct SessionState {
    pub workspace: Workspace,
    pub preset_library: crate::persistence::presets::PresetLibrary,
    pub history: history::HistoryManager,
    pub notifications: NotificationSystem,
}

/// Cross-thread message passing.
pub(crate) struct MessageBus {
    pub command_rx: tokio::sync::mpsc::UnboundedReceiver<CommandEnvelope>,
    pub command_tx: tokio::sync::mpsc::UnboundedSender<CommandEnvelope>,
    pub state_tx: std::sync::Arc<std::sync::RwLock<Option<EngineState>>>,
}

// ── Main application struct ─────────────────────────────────────

/// Core engine application. Owns all subsystems except window/egui.
///
/// Implements all engine traits (MixerCommands, AudioCommands, etc.)
/// for direct same-thread access. Also processes EngineCommands from
/// cross-thread consumers via mpsc channel.
pub struct VardaApp {
    // ── Engine core ──────────────────────────────────────────────
    mixer: Mixer,
    audio_manager: AudioManager,
    camera_manager: CameraManager,
    registry: ShaderRegistry,
    analyzer_registry: crate::analyzer::AnalyzerRegistry,
    context: GpuContext,

    // ── Domain sub-structs ───────────────────────────────────────
    pub(crate) input: InputSubsystem,
    pub(crate) output: OutputSubsystem,
    pub(crate) external_io: ExternalIO,
    /// Interactive HTML window state (feature `html`). See /spec/html-source.md §4.
    #[cfg(feature = "html")]
    pub(crate) interactive: interactive::InteractiveHtmlState,
    pub(crate) frame_stats: FrameStats,
    pub(crate) session: SessionState,
    pub(crate) bus: MessageBus,

    // ── Audio textures (GPU resource, owned here) ──────────────
    audio_textures: crate::audio::AudioTextures,

    // ── Render resolution (configurable, scene-level) ───────
    render_width: u32,
    render_height: u32,

    // ── Frame pacing (global, runtime-mutable) ──────────
    target_fps: u32,

    // ── Channel preview / cue (ephemeral, published by UI each frame) ──
    // Channels force-rendered for off-air preview. Never persisted; affects the
    // render gate only, never the compositor. See /spec/channel-preview.md.
    preview_channels: Vec<usize>,

    // ── Pending MIDI-triggered actions (consumed by runner) ──
    pub(crate) midi_pending_undo: bool,
    pub(crate) midi_pending_redo: bool,
    pub(crate) midi_pending_save: bool,

    // ── Shutdown request flag ──────────────────────────────────
    pub(crate) shutdown_requested: bool,
}

impl VardaApp {
    /// Create a new VardaApp with all subsystems initialized.
    ///
    /// Requires a fully initialized `GpuContext` — the engine cannot exist
    /// without a GPU. A default two-channel mixer is always created.
    /// `config` provides session settings from CLI flags + workspace defaults.
    pub fn new(gpu: GpuContext, config: &AppConfig) -> anyhow::Result<Self> {
        log::info!("[STARTUP]   Audio init...");
        let audio_manager = AudioManager::new();

        log::info!("[STARTUP]   Workspace init...");
        let workspace = Workspace::new(config.effective_workspace_root());

        // Build shader registry with all library paths. Order is precedence:
        // later paths override earlier ones by shader name, and that ordering is
        // held across the whole session (hot-reload and removal re-resolve the
        // winner), not just at the initial scan.
        // 1. Bundled shaders (exe-relative, for packaged .app / AppImage)
        // 2. CWD shaders/ (dev builds / cargo run)
        // 3. Workspace .varda/shaders/ (per-show user shaders)
        // 4. Platform user dir (global user shader collection)
        // 5. Any --shader-dir flags (added last, override built-ins by name)
        let mut registry = ShaderRegistry::new();
        if let Some(bundled) = crate::registry::get_bundled_shader_path() {
            if let Err(e) = registry.add_library_path(&bundled) {
                log::warn!("Failed to add bundled shaders path: {}", e);
            }
        }
        if let Err(e) = registry.add_library_path("shaders") {
            log::warn!("Failed to add shaders path: {}", e);
        }
        let ws_shaders = workspace.shaders_dir();
        if ws_shaders.is_dir() {
            if let Err(e) = registry.add_library_path(&ws_shaders) {
                log::warn!("Failed to add workspace shaders path: {}", e);
            }
        }
        for path in crate::registry::get_default_library_paths() {
            if path.is_dir() {
                if let Err(e) = registry.add_library_path(&path) {
                    log::warn!(
                        "Failed to add user shader library path {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
        for path in &config.shader_dirs {
            if let Err(e) = registry.add_library_path(path) {
                log::warn!("Failed to add --shader-dir {}: {}", path.display(), e);
            }
        }
        match registry.scan() {
            Ok(count) => log::info!("Loaded {} shaders", count),
            Err(e) => log::error!("Failed to scan shaders: {}", e),
        }
        if let Err(e) = registry.start_watching() {
            log::warn!("Failed to start shader hot-reload: {}", e);
        }

        // Load OSC config from workspace (or use defaults), then apply CLI overrides
        let mut osc_config = if workspace.has_osc() {
            OscConfig::load(workspace.osc_path()).unwrap_or_else(|e| {
                log::warn!("Failed to load OSC config: {}, using defaults", e);
                OscConfig::default()
            })
        } else {
            OscConfig::default()
        };

        // CLI overrides for OSC
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
                        log::warn!("Failed to add OSC feedback target '{}': {}", target, e);
                    }
                }
                Some(sender)
            }
            Err(e) => {
                log::warn!("Failed to create OSC feedback sender: {}", e);
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
                log::warn!("Failed to initialize MIDI: {}", e);
                None
            }
        };

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let state_tx = std::sync::Arc::new(std::sync::RwLock::new(None));

        // Always create GPU-dependent resources up front
        log::info!("[STARTUP]   GPU resources (textures, mixer)...");
        let audio_textures = crate::audio::AudioTextures::new(&gpu.device);
        let calibration_textures =
            crate::renderer::context::create_calibration_textures(&gpu.device, &gpu.queue, 8);
        let mixer = Mixer::new(&gpu, DEFAULT_RENDER_WIDTH, DEFAULT_RENDER_HEIGHT)?;
        let preset_library = crate::persistence::presets::PresetLibrary::load(&workspace);

        Ok(Self {
            mixer,
            audio_manager,
            camera_manager: CameraManager::new(),
            registry,
            analyzer_registry: crate::analyzer::default_registry(),
            context: gpu,
            input: InputSubsystem {
                osc_receiver,
                osc_feedback,
                osc_config,
                midi_devices,
                keymap: KeymapStore::with_defaults(),
                midi_mappings: midi::MidiMappingStore::new(),
                controller_led_mgr,
                auto_map_engine,
                clock_manager: crate::clock::ClockManager::new(),
            },
            output: OutputSubsystem {
                outputs: Vec::new(),
                surface_manager: SurfaceManager::new(),
                calibration_textures,
                domemaster: None,
                pending_output_creates: Vec::new(),
                cached_monitors: Vec::new(),
            },
            external_io: ExternalIO {
                ndi_manager: if config.ndi_disabled {
                    log::info!("NDI disabled by CLI flag");
                    crate::ndi::NdiManager::new_disabled()
                } else {
                    crate::ndi::NdiManager::new()
                },
                #[cfg(target_os = "macos")]
                syphon_manager: if config.syphon_disabled {
                    log::info!("Syphon disabled by CLI flag");
                    crate::syphon::SyphonManager::new_disabled()
                } else {
                    crate::syphon::SyphonManager::new()
                },
                #[cfg(target_os = "macos")]
                pending_syphon: Vec::new(),
                #[cfg(target_os = "macos")]
                last_syphon_scan: std::time::Instant::now(),
                stream_manager: crate::stream::StreamManager::new(),
                stream_library: Vec::new(),
                hls_library: Vec::new(),
                dash_library: Vec::new(),
                rtmp_library: Vec::new(),
                html_manager: if config.html_disabled {
                    crate::html::HtmlManager::new_disabled()
                } else {
                    crate::html::HtmlManager::new()
                },
                html_library: Vec::new(),
            },
            frame_stats: FrameStats {
                last_frame_instant: std::time::Instant::now(),
                fps_history: std::collections::VecDeque::with_capacity(60),
                fps_smoothed: 0.0,
                frame_count: 0,
                system_monitor: crate::sysmon::SystemMonitor::new(),
            },
            session: SessionState {
                workspace,
                preset_library,
                history: history::HistoryManager::new(),
                notifications: NotificationSystem::new(),
            },
            bus: MessageBus {
                command_rx,
                command_tx,
                state_tx,
            },
            audio_textures,
            #[cfg(feature = "html")]
            interactive: interactive::InteractiveHtmlState::default(),
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
            target_fps: config.target_fps,
            preview_channels: Vec::new(),
            midi_pending_undo: false,
            midi_pending_redo: false,
            midi_pending_save: false,
            shutdown_requested: false,
        })
    }

    /// Get a command sender for cross-thread consumers (HTTP API, CLI).
    pub fn command_sender(&self) -> tokio::sync::mpsc::UnboundedSender<CommandEnvelope> {
        self.bus.command_tx.clone()
    }

    /// Get a shared reference to the latest engine state (for cross-thread consumers).
    pub fn state_reader(&self) -> std::sync::Arc<std::sync::RwLock<Option<EngineState>>> {
        self.bus.state_tx.clone()
    }

    /// Process all queued cross-thread commands. Called once per frame.
    ///
    /// Exhaustive match — the compiler enforces that every EngineCommand variant
    /// is handled. Adding a new variant requires wiring it here.
    pub fn process_commands(&mut self) {
        while let Ok((cmd, reply_tx)) = self.bus.command_rx.try_recv() {
            // Record pre-mutation state so bus-driven (HTTP API / WebSocket /
            // CLI / MIDI-issued) edits are undoable on the same timeline the
            // windowed UI uses. In-process UI mutations do NOT flow through the
            // bus — the windowed runner records those itself — so there is no
            // double-record here. See commands::command_is_undoable.
            if commands::command_is_undoable(&cmd) {
                let snapshot = self.history_snapshot_default();
                self.push_history(snapshot);
            }
            let result = self.execute_command(cmd);
            if let Some(tx) = reply_tx {
                let _ = tx.send(result);
            }
        }
    }

    /// Helper: access auto-transition on a deck, creating if needed, then apply mutation.
    fn exec_auto_transition(
        &mut self,
        channel_idx: usize,
        deck_idx: usize,
        f: impl FnOnce(&mut crate::channel::DeckAutoTransition),
    ) -> CommandResult {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            if deck_idx < ch.decks.len() {
                let slot = &mut ch.decks[deck_idx];
                if slot.auto_transition.is_none() {
                    slot.auto_transition = Some(crate::channel::DeckAutoTransition::new());
                }
                if let Some(at) = slot.auto_transition.as_mut() {
                    f(at);
                }
                return CommandResult::Ok;
            }
        }
        CommandResult::Err {
            code: ErrorCode::NotFound,
            message: "Deck not found".into(),
        }
    }

    /// Helper: update a modulation source by UUID.
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
                message: format!("Modulation source {} not found", uuid),
            }
        }
    }

    /// Build a domain-neutral engine state snapshot for cross-thread consumers.
    pub fn build_engine_state(&self) -> EngineState {
        snapshot::build_engine_state(self)
    }

    /// Publish the latest engine state for cross-thread consumers.
    pub fn publish_state(&self) {
        let state = self.build_engine_state();
        if let Ok(mut guard) = self.bus.state_tx.write() {
            *guard = Some(state);
        }
    }

    // ── Public accessors (controlled access for delivery layers) ─────

    /// Read-only access to the GPU context.
    pub fn gpu_context(&self) -> &GpuContext {
        &self.context
    }

    /// Read-only access to the mixer.
    pub fn mixer_ref(&self) -> &crate::mixer::Mixer {
        &self.mixer
    }

    /// Read-only access to the camera manager.
    pub fn camera_manager(&self) -> &CameraManager {
        &self.camera_manager
    }

    /// Mutable access to the camera manager (open/release cameras).
    pub fn camera_manager_mut(&mut self) -> &mut CameraManager {
        &mut self.camera_manager
    }

    /// Open a camera, returning resolution on success. Avoids split-borrow issues
    /// by accessing both `camera_manager` and `context` internally.
    pub fn open_camera(&mut self, id: crate::camera::CameraId) -> anyhow::Result<(u32, u32)> {
        self.camera_manager.open_camera(id, &self.context.device)
    }

    /// Read-only access to the outputs.
    pub fn outputs_ref(&self) -> &[crate::renderer::context::UnifiedOutput] {
        &self.output.outputs
    }

    /// Mutable access to the mixer (for deck insertion from background loads).
    pub fn mixer_mut(&mut self) -> &mut crate::mixer::Mixer {
        &mut self.mixer
    }

    /// Read-only access to the domemaster renderer output view (if enabled).
    pub fn domemaster_view(&self) -> Option<&wgpu::TextureView> {
        self.output.domemaster.as_ref().map(|d| d.output_view())
    }

    /// Ensure the domemaster renderer exists and is enabled.
    /// Creates it lazily on first call; subsequent calls just ensure `enabled = true`.
    pub fn ensure_domemaster(&mut self) {
        if let Some(dome) = &mut self.output.domemaster {
            dome.enabled = true;
        } else {
            let config = crate::renderer::dome::DomemasterConfig::default();
            match crate::renderer::dome::DomemasterRenderer::new(
                &self.context.device,
                self.context.compositing_format,
                config,
            ) {
                Ok(mut dome) => {
                    dome.enabled = true;
                    self.output.domemaster = Some(dome);
                    log::info!("Domemaster renderer created and enabled");
                }
                Err(e) => {
                    log::error!("Failed to create domemaster renderer: {}", e);
                }
            }
        }
    }

    /// Set domemaster content rotation (azimuth, elevation, roll) in radians.
    /// Called each frame from the UI layer so content rotation is applied
    /// in real-time by the domemaster shader, not baked into warp meshes.
    pub fn set_domemaster_content_rotation(&mut self, az: f32, el: f32, roll: f32) {
        if let Some(dome) = &mut self.output.domemaster {
            dome.set_content_rotation(az, el, roll);
        }
    }

    /// Publish the set of channels to force-render for off-air preview.
    /// Called by the runner each frame from the UI selection. Ephemeral — never
    /// persisted; affects the render gate only. See /spec/channel-preview.md.
    pub fn set_preview_channels(&mut self, channels: Vec<usize>) {
        self.preview_channels = channels;
    }

    /// Number of loaded shaders.
    pub fn shader_count(&self) -> usize {
        self.registry.count()
    }

    /// Resolve a generator index to a cloned ISFShader.
    /// Returns None if the index is out of bounds.
    pub fn resolve_generator(&self, gen_idx: usize) -> Option<crate::isf::ISFShader> {
        self.registry
            .generators()
            .get(gen_idx)
            .map(|s| (*s).clone())
    }

    /// Tick notification expiry timers.
    pub fn update_notifications(&mut self) {
        self.session.notifications.update();
    }

    /// Push an info-level notification.
    pub fn notify_info(&mut self, message: impl Into<String>) {
        self.session.notifications.info(message);
    }

    /// Close an output window by its winit WindowId. Returns the name if found.
    pub fn close_output_window_by_id(
        &mut self,
        window_id: winit::window::WindowId,
    ) -> Option<String> {
        if let Some(idx) = self.output.outputs.iter().position(|o| {
            if let UnifiedOutput::Window(w) = o {
                w.window.id() == window_id
            } else {
                false
            }
        }) {
            let name = self.output.outputs[idx].name().to_string();
            if let UnifiedOutput::Window(w) = self.output.outputs.remove(idx) {
                w.destroy();
            }
            Some(name)
        } else {
            None
        }
    }

    /// Resize an output window by its winit WindowId.
    pub fn resize_output_window_by_id(
        &mut self,
        window_id: winit::window::WindowId,
        new_size: winit::dpi::PhysicalSize<u32>,
    ) {
        for o in &mut self.output.outputs {
            if let UnifiedOutput::Window(w) = o {
                if w.window.id() == window_id {
                    w.resize(&self.context.device, new_size);
                    return;
                }
            }
        }
    }

    /// Current render width.
    pub fn render_width(&self) -> u32 {
        self.render_width
    }

    /// Current render height.
    pub fn render_height(&self) -> u32 {
        self.render_height
    }

    /// Maximum render dimension (width or height) the GPU can allocate a
    /// texture for. Varda imposes no artificial cap — this hardware limit is
    /// the only bound on render resolution (see spec/resolution-and-scaling.md).
    pub fn max_render_dimension(&self) -> u32 {
        self.context.device.limits().max_texture_dimension_2d
    }

    /// Change the master render resolution. Resizes all textures in the pipeline.
    ///
    /// Zero dimensions are rejected. Larger-than-hardware requests are clamped to
    /// the GPU's `max_texture_dimension_2d` so both the UI and the HTTP API share
    /// the same bound and neither can trigger a GPU allocation failure.
    pub fn set_render_resolution(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            log::warn!("Ignoring zero render resolution {}×{}", width, height);
            return;
        }
        let max_dim = self.max_render_dimension();
        let (width, height) = clamp_resolution_to_gpu(width, height, max_dim);
        if width == self.render_width && height == self.render_height {
            return;
        }
        log::info!(
            "Changing render resolution: {}×{} → {}×{}",
            self.render_width,
            self.render_height,
            width,
            height
        );
        self.render_width = width;
        self.render_height = height;
        self.mixer.resize(&self.context, width, height);
        // Clear sub-mix cache since textures were recreated
        self.mixer.clear_sub_mix_cache();
        self.session
            .notifications
            .info(format!("📐 Resolution changed to {}×{}", width, height));
    }

    /// Current target FPS (0 = uncapped).
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }

    /// Set the target FPS. 0 = uncapped.
    pub fn set_target_fps(&mut self, fps: u32) {
        if fps == self.target_fps {
            return;
        }
        log::info!("Target FPS: {} → {}", self.target_fps, fps);
        self.target_fps = fps;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Parse AppConfig from a simulated CLI invocation.
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
        assert!(config.osc_targets.is_empty());
        assert!(!config.osc_disabled);
        assert!(!config.ndi_disabled);
        assert!(!config.syphon_disabled);
        assert!(config.shader_dirs.is_empty());
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
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
        VardaApp::new(gpu, &config).ok()
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
        let before = app.build_engine_state().mixer.channels[0].decks.len();
        let tx = app.command_sender();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send((
            crate::engine::EngineCommand::AddSolidColorDeck {
                channel_idx: 0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
            Some(reply_tx),
        ))
        .unwrap();
        app.process_commands();
        let result = reply_rx.blocking_recv().unwrap();
        // Deck-creating commands report the new deck's UUID (ui-engine-boundary.md WS1).
        assert!(
            matches!(result, crate::engine::CommandResult::OkWithId { .. }),
            "command should succeed: {:?}",
            result
        );
        let after = app.build_engine_state().mixer.channels[0].decks.len();
        assert_eq!(after, before + 1, "should have one more deck");
    }

    #[test]
    fn smoke_set_channel_opacity() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((
            crate::engine::EngineCommand::SetChannelOpacity {
                channel_idx: 0,
                opacity: 0.5,
            },
            None,
        ))
        .unwrap();
        app.process_commands();
        let state = app.build_engine_state();
        assert!((state.mixer.channels[0].opacity - 0.5).abs() < 1e-5);
    }

    /// Regression test for the previously-dead API undo/redo path: bus-driven
    /// (HTTP API / headless) commands must record onto the shared timeline so
    /// `Undo`/`Redo` sent over the same bus actually restore state.
    #[test]
    fn api_command_undo_redo_roundtrip() {
        let Some(mut app) = headless_app() else {
            return;
        };
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
                channel_idx: 0,
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

    /// Live-control commands (crossfader) must NOT pollute the undo timeline.
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

        // Snapshot the stage state before mutating.
        let snap = app.history_snapshot_default();

        // Move the surface.
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
            "AddLfo failed: {:?}",
            result
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
        // Add a third channel first
        tx.send((crate::engine::EngineCommand::AddChannel, None))
            .unwrap();
        app.process_commands();
        assert_eq!(app.build_engine_state().mixer.channels.len(), 3);
        // Remove the third channel
        tx.send((
            crate::engine::EngineCommand::RemoveChannel { channel_idx: 2 },
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
        let tx = app.command_sender();
        // Add a solid color deck first
        tx.send((
            crate::engine::EngineCommand::AddSolidColorDeck {
                channel_idx: 0,
                color: [0.0, 1.0, 0.0, 1.0],
            },
            None,
        ))
        .unwrap();
        app.process_commands();
        // Set its blend mode
        tx.send((
            crate::engine::EngineCommand::SetDeckBlendMode {
                channel_idx: 0,
                deck_idx: 1,
                mode: crate::engine::BlendMode::Add,
            },
            None,
        ))
        .unwrap();
        app.process_commands();
        // Verify no crash — blend mode is GPU-level and verified through state
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
        // Should keep previous resolution
        assert!(app.render_width() > 0);
        assert!(app.render_height() > 0);
    }

    #[test]
    fn smoke_publish_and_read_state() {
        let Some(app) = headless_app() else {
            return;
        };
        let reader = app.state_reader();
        app.publish_state();
        let guard = reader.read().unwrap();
        let state = guard.as_ref().expect("state should be published");
        assert_eq!(state.mixer.channels.len(), 2);
    }

    #[test]
    fn smoke_notifications() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.notify_info("Test notification");
        app.update_notifications();
        // Verify no crash
    }

    // ── Extended smoke tests ───────────────────────────────────────

    /// Helper: send command with reply, process, return result.
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

    /// Helper: fire-and-forget command.
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
        // Verify the auto crossfade was started and no crash occurred.
        // Timing in headless mode is unpredictable, so just verify no panic.
    }

    #[test]
    fn smoke_add_video_deck_no_crash() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Bad path — should not panic
        let _ = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddVideoDeck {
                channel_idx: 0,
                path: std::path::PathBuf::from("/nonexistent/video.mp4"),
            },
        );
    }

    #[test]
    fn smoke_video_toggle_play() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Toggle play on a non-video deck — should handle gracefully
        let _ = send_cmd(
            &mut app,
            crate::engine::EngineCommand::VideoTogglePlay {
                channel_idx: 0,
                deck_idx: 0,
            },
        );
    }

    #[test]
    fn smoke_video_set_speed() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let _ = send_cmd(
            &mut app,
            crate::engine::EngineCommand::VideoSetSpeed {
                channel_idx: 0,
                deck_idx: 0,
                speed: 2.0,
            },
        );
    }

    #[test]
    fn smoke_create_sequence() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let r = send_cmd(&mut app, crate::engine::EngineCommand::CreateSequence);
        assert!(matches!(r, crate::engine::CommandResult::Ok));
        let state = app.build_engine_state();
        assert_eq!(state.mixer.sequences.len(), 1);
    }

    #[test]
    fn smoke_sequence_add_steps() {
        let Some(mut app) = headless_app() else {
            return;
        };
        send_cmd(&mut app, crate::engine::EngineCommand::CreateSequence);
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddFadeStep {
                seq_idx: 0,
                from_ch: 0,
                to_ch: 1,
            },
        );
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddWaitStep { seq_idx: 0 },
        );
        let state = app.build_engine_state();
        assert_eq!(state.mixer.sequences[0].steps.len(), 2);
    }

    #[test]
    fn smoke_effect_chain_operations() {
        let Some(mut app) = headless_app() else {
            return;
        };
        send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddSolidColorDeck {
                channel_idx: 0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
        );
        let target = crate::engine::EffectTarget::Deck(0, 1);
        let r = send_cmd(
            &mut app,
            crate::engine::EngineCommand::AddEffect {
                target: target.clone(),
                shader_name: "Invert".into(),
            },
        );
        if matches!(r, crate::engine::CommandResult::Ok) {
            // Toggle
            let _ = send_cmd(
                &mut app,
                crate::engine::EngineCommand::ToggleEffect {
                    target: target.clone(),
                    effect_idx: 0,
                },
            );
            // Remove
            let _ = send_cmd(
                &mut app,
                crate::engine::EngineCommand::RemoveEffect {
                    target,
                    effect_idx: 0,
                },
            );
        }
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
        fire(
            &mut app,
            crate::engine::EngineCommand::SetChannelBlendMode {
                channel_idx: 0,
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
