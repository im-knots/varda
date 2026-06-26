//! Application layer — concrete engine implementation.
//!
//! VardaApp owns all engine subsystems (Mixer, Audio, Cameras, MIDI, OSC,
//! ShaderRegistry, SurfaceManager) and implements the engine traits.
//!
//! The main.rs `App` struct owns window/egui state and holds a `VardaApp`.

mod actions;
mod engine_impl;
pub(crate) mod history;
mod inputs;
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

use crate::engine::{CommandEnvelope, CommandResult, EngineCommand, EngineState, ErrorCode};

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

        // Build shader registry with all library paths (order = priority for hot-reload):
        // 1. Bundled shaders (exe-relative, for packaged .app / AppImage)
        // 2. CWD shaders/ (dev builds / cargo run)
        // 3. Workspace .varda/shaders/ (per-show user shaders)
        // 4. Platform user dir (global user shader collection)
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
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
            target_fps: config.target_fps,
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

    /// Execute a single command and return the result.
    fn execute_command(&mut self, cmd: EngineCommand) -> CommandResult {
        use crate::engine::traits::*;
        use crate::modulation::ModulationSource;
        match cmd {
            // ── Mixer ────────────────────────────────────────
            EngineCommand::SetCrossfader(pos) => {
                self.set_crossfader(pos);
                CommandResult::Ok
            }
            EngineCommand::SetTonemapMode(mode) => {
                self.set_tonemap_mode(mode);
                CommandResult::Ok
            }
            EngineCommand::LoadLut { filename } => match self.load_lut(&filename) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::UnloadLut => {
                self.unload_lut();
                CommandResult::Ok
            }
            EngineCommand::AutoCrossfade {
                target,
                duration_secs,
                easing,
            } => {
                self.start_auto_crossfade(target, duration_secs, easing);
                CommandResult::Ok
            }
            EngineCommand::BeatCrossfade { target, beats } => {
                self.start_beat_crossfade(target, beats);
                CommandResult::Ok
            }
            EngineCommand::AddDeck {
                channel_idx,
                shader_name,
            } => match self.add_deck(channel_idx, &shader_name) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::AddImageDeck { channel_idx, path } => {
                match self.add_image_deck(channel_idx, &path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddVideoDeck { channel_idx, path } => {
                match self.add_video_deck(channel_idx, &path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddSolidColorDeck { channel_idx, color } => {
                match self.add_solid_color_deck(channel_idx, color) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddCameraDeck {
                channel_idx,
                camera_id,
            } => match self.add_camera_deck(channel_idx, camera_id) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::RemoveDeck {
                channel_idx,
                deck_idx,
            } => match self.remove_deck(channel_idx, deck_idx) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                },
            },
            EngineCommand::MoveDeck {
                src_ch,
                src_deck,
                dst_ch,
            } => match self.move_deck(src_ch, src_deck, dst_ch) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::ReorderDeck {
                ch,
                from_idx,
                to_idx,
            } => {
                self.reorder_deck(ch, from_idx, to_idx);
                CommandResult::Ok
            }
            EngineCommand::SetDeckOpacity {
                channel_idx,
                deck_idx,
                opacity,
            } => {
                self.set_deck_opacity(channel_idx, deck_idx, opacity);
                CommandResult::Ok
            }
            EngineCommand::SetDeckBlendMode {
                channel_idx,
                deck_idx,
                mode,
            } => {
                self.set_deck_blend_mode(channel_idx, deck_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::SetDeckSolo {
                channel_idx,
                deck_idx,
                solo,
            } => {
                self.set_deck_solo(channel_idx, deck_idx, solo);
                CommandResult::Ok
            }
            EngineCommand::SetDeckMute {
                channel_idx,
                deck_idx,
                mute,
            } => {
                self.set_deck_mute(channel_idx, deck_idx, mute);
                CommandResult::Ok
            }
            EngineCommand::SetDeckRenderFps {
                channel_idx,
                deck_idx,
                render_fps,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if let Some(slot) = ch.decks.get_mut(deck_idx) {
                        slot.render_fps = render_fps;
                        CommandResult::Ok
                    } else {
                        CommandResult::Err {
                            code: ErrorCode::NotFound,
                            message: format!("Deck {} not found", deck_idx),
                        }
                    }
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("Channel {} not found", channel_idx),
                    }
                }
            }
            EngineCommand::SetDeckScalingMode {
                channel_idx,
                deck_idx,
                mode,
            } => {
                self.set_deck_scaling_mode(channel_idx, deck_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::SetChannelOpacity {
                channel_idx,
                opacity,
            } => {
                self.set_channel_opacity(channel_idx, opacity);
                CommandResult::Ok
            }
            EngineCommand::SetChannelBlendMode { channel_idx, mode } => {
                self.set_channel_blend_mode(channel_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::AddChannel => match self.add_channel() {
                Ok(_idx) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::RemoveChannel { channel_idx } => {
                match self.remove_channel(channel_idx) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddEffect {
                target,
                shader_name,
            } => match self.add_effect(target, &shader_name) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::RemoveEffect { target, effect_idx } => {
                self.remove_effect(target, effect_idx);
                CommandResult::Ok
            }
            EngineCommand::ToggleEffect { target, effect_idx } => {
                self.toggle_effect(target, effect_idx);
                CommandResult::Ok
            }
            EngineCommand::MoveEffect {
                target,
                from_idx,
                to_idx,
            } => {
                self.move_effect(target, from_idx, to_idx);
                CommandResult::Ok
            }
            EngineCommand::SetTransition { shader_name } => {
                match self.set_transition(shader_name.as_deref()) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::SetParam { path, value } => {
                self.set_param(&path, value);
                CommandResult::Ok
            }

            // ── Audio ────────────────────────────────────────
            EngineCommand::OpenAudioSource { source_id } => {
                match self.open_audio_source(source_id) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::CloseAudioSource { source_id } => {
                self.close_audio_source(source_id);
                CommandResult::Ok
            }
            EngineCommand::ScanAudioDevices => {
                self.scan_audio_devices();
                CommandResult::Ok
            }

            // ── Modulation ───────────────────────────────────
            EngineCommand::AddLfo {
                waveform,
                frequency,
            } => {
                self.add_lfo(waveform, frequency);
                CommandResult::Ok
            }
            EngineCommand::AddAudioBand { preset, source_id } => {
                self.add_audio_band(preset, source_id);
                CommandResult::Ok
            }
            EngineCommand::AddAdsr {
                attack,
                decay,
                sustain,
                release,
            } => {
                self.add_adsr(attack, decay, sustain, release);
                CommandResult::Ok
            }
            EngineCommand::AddStepSequencer { num_steps, rate } => {
                self.add_step_sequencer(num_steps, rate);
                CommandResult::Ok
            }
            EngineCommand::RemoveModulationSource { uuid } => {
                self.remove_modulation_source(&uuid);
                CommandResult::Ok
            }
            EngineCommand::AssignModulation {
                target,
                source_id,
                amount,
            } => {
                self.assign_modulation(&target, &source_id, amount);
                CommandResult::Ok
            }
            EngineCommand::ClearModulation { target } => {
                self.clear_modulation(&target);
                CommandResult::Ok
            }

            // ── Output ───────────────────────────────────────
            EngineCommand::CreateOutput => {
                self.request_create_output();
                CommandResult::Ok
            }
            EngineCommand::CloseOutput { idx } => {
                self.close_output(idx);
                CommandResult::Ok
            }
            EngineCommand::SetOutputDisplay { idx, monitor_name } => {
                self.set_output_display(idx, &monitor_name);
                CommandResult::Ok
            }
            EngineCommand::SetOutputTarget { idx, target } => {
                self.cmd_set_output_target(idx, target)
            }

            // ── Surfaces ────────────────────────────────────
            EngineCommand::AddSurface { name, source } => {
                self.add_surface(&name, source);
                CommandResult::Ok
            }
            EngineCommand::AddPolygonSurface {
                name,
                vertices,
                source,
            } => {
                self.add_polygon_surface(&name, &vertices, source);
                CommandResult::Ok
            }
            EngineCommand::AddCircleSurface {
                name,
                center,
                radius,
                sides,
                aspect_ratio,
                source,
            } => {
                self.add_circle_surface(&name, center, radius, sides, aspect_ratio, source);
                CommandResult::Ok
            }
            EngineCommand::RemoveSurface { uuid } => self.cmd_remove_surface(&uuid),
            EngineCommand::SetSurfaceSource { uuid, source } => {
                self.set_surface_source(&uuid, source);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::SetSurfaceOutputType { uuid, output_type } => {
                self.set_surface_output_type(&uuid, output_type);
                CommandResult::Ok
            }
            EngineCommand::SetSurfaceContentMapping { uuid, mapping } => {
                self.set_surface_content_mapping(&uuid, mapping);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::RenameSurface { uuid, name } => {
                self.rename_surface(&uuid, &name);
                CommandResult::Ok
            }
            EngineCommand::UpdateSurfaceVertices { uuid, vertices } => {
                self.cmd_update_surface_vertices(&uuid, vertices)
            }
            EngineCommand::DuplicateSurface { uuid } => self.cmd_duplicate_surface(&uuid),
            EngineCommand::FlipSurfaceHorizontal { uuid } => {
                self.cmd_flip_surface_horizontal(&uuid)
            }
            EngineCommand::FlipSurfaceVertical { uuid } => self.cmd_flip_surface_vertical(&uuid),
            EngineCommand::InsertSurfaceVertex {
                uuid,
                after_vert_idx,
                position,
            } => self.cmd_insert_surface_vertex(&uuid, after_vert_idx, position),
            EngineCommand::SetCircleRadius { uuid, radius } => {
                self.cmd_set_circle_radius(&uuid, radius)
            }
            EngineCommand::SetCircleSides { uuid, sides } => {
                self.cmd_set_circle_sides(&uuid, sides)
            }
            EngineCommand::ConvertSurfaceToPolygon { uuid } => {
                self.cmd_convert_surface_to_polygon(&uuid)
            }
            EngineCommand::CombineSurfaces { uuids } => self.cmd_combine_surfaces(&uuids),
            EngineCommand::MoveSurface { uuid, dx, dy } => self.cmd_move_surface(&uuid, dx, dy),
            EngineCommand::UpdateSurfaceContourVertices {
                uuid,
                contour,
                vertices,
            } => self.cmd_update_surface_contour_vertices(&uuid, contour, vertices),
            EngineCommand::AssignSurfaceToOutput {
                output_uuid,
                surface_uuid,
            } => {
                self.assign_surface_to_output(&output_uuid, &surface_uuid);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::UnassignSurfaceFromOutput {
                output_uuid,
                assignment_idx,
            } => {
                self.unassign_surface_from_output(&output_uuid, assignment_idx);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::AssignSurfaceToOutputByIdx {
                output_idx,
                surface_uuid,
            } => self.cmd_assign_surface_to_output_by_idx(output_idx, &surface_uuid),
            EngineCommand::UnassignSurfaceFromOutputByIdx {
                output_idx,
                assignment_idx,
            } => self.cmd_unassign_surface_from_output_by_idx(output_idx, assignment_idx),

            // ── Surface Auto-Detection ────────────────────────
            EngineCommand::DetectFromImage { image_data, params } => {
                match self.detect_from_image(&image_data, &params) {
                    Ok(result) => CommandResult::OkWithData {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    },
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::DetectFromSvg { svg_data } => match self.detect_from_svg(&svg_data) {
                Ok(result) => CommandResult::OkWithData {
                    data: serde_json::to_value(&result).unwrap_or_default(),
                },
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::DetectFromDxf { dxf_data } => match self.detect_from_dxf(&dxf_data) {
                Ok(result) => CommandResult::OkWithData {
                    data: serde_json::to_value(&result).unwrap_or_default(),
                },
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::ConfirmDetectedContours { contours } => {
                let uuids = self.confirm_detected_contours(&contours);
                CommandResult::OkWithData {
                    data: serde_json::json!({ "surface_uuids": uuids }),
                }
            }
            EngineCommand::DetectFromCamera { camera_id, params } => {
                match self.detect_from_camera(camera_id, &params) {
                    Ok(result) => {
                        let uuids = self.confirm_detected_contours(&result.contours);
                        CommandResult::OkWithData {
                            data: serde_json::json!({ "surface_uuids": uuids, "contours_found": result.contours.len() }),
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }

            // ── Video Playback ────────────────────────────────
            EngineCommand::VideoTogglePlay {
                channel_idx,
                deck_idx,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() && ch.decks[deck_idx].deck.video_toggle_play() {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSeek {
                channel_idx,
                deck_idx,
                position_secs,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_seek(position_secs)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found".into(),
                }
            }
            EngineCommand::VideoSetSpeed {
                channel_idx,
                deck_idx,
                speed,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() && ch.decks[deck_idx].deck.video_set_speed(speed) {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSetLoopMode {
                channel_idx,
                deck_idx,
                mode,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_set_loop_mode(mode)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSetInPoint {
                channel_idx,
                deck_idx,
                secs,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() && ch.decks[deck_idx].deck.video_set_in_point(secs)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSetOutPoint {
                channel_idx,
                deck_idx,
                secs,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_set_out_point(secs)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoClearInOutPoints {
                channel_idx,
                deck_idx,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_clear_in_out_points()
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }

            // ── Deck Auto-Transitions ─────────────────────────
            EngineCommand::SetAutoTransitionEnabled {
                channel_idx,
                deck_idx,
                enabled,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.enabled = enabled;
                if !enabled {
                    at.phase = crate::channel::DeckTransitionPhase::Inactive;
                }
            }),
            EngineCommand::SetAutoTransitionTrigger {
                channel_idx,
                deck_idx,
                clip_end,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.trigger = if clip_end {
                    crate::channel::TransitionTrigger::ClipEnd
                } else {
                    crate::channel::TransitionTrigger::Timer
                };
            }),
            EngineCommand::SetAutoTransitionPlayDuration {
                channel_idx,
                deck_idx,
                value,
                unit,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.play_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
            }),
            EngineCommand::SetAutoTransitionDuration {
                channel_idx,
                deck_idx,
                value,
                unit,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.transition_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
            }),
            EngineCommand::SetAutoTransitionShader {
                channel_idx,
                deck_idx,
                shader_name,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        let slot = &mut ch.decks[deck_idx];
                        if slot.auto_transition.is_none() {
                            slot.auto_transition = Some(crate::channel::DeckAutoTransition::new());
                        }
                        if let Some(at) = slot.auto_transition.as_mut() {
                            at.transition_shader_name = shader_name.clone();
                        }
                        if let Some(shader_name) = &shader_name {
                            if let Some(shader) = self
                                .registry
                                .transitions()
                                .iter()
                                .find(|s| s.name() == *shader_name)
                            {
                                let _ =
                                    slot.set_transition_shader(&self.context, (*shader).clone());
                            }
                        } else {
                            slot.transition_effect = None;
                        }
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found".into(),
                }
            }
            EngineCommand::ToggleAutoTransitionPlayDurationUnit {
                channel_idx,
                deck_idx,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                let next_unit = at.play_duration.unit().next();
                at.play_duration = crate::channel::DurationSpec::from_value_unit(
                    at.play_duration.value(),
                    next_unit,
                );
            }),
            EngineCommand::ToggleAutoTransitionDurationUnit {
                channel_idx,
                deck_idx,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                let next_unit = at.transition_duration.unit().next();
                at.transition_duration = crate::channel::DurationSpec::from_value_unit(
                    at.transition_duration.value(),
                    next_unit,
                );
            }),
            EngineCommand::SetAutoTransitionPlayDurationValue {
                channel_idx,
                deck_idx,
                value,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.play_duration.set_value(value);
            }),
            EngineCommand::SetAutoTransitionDurationValue {
                channel_idx,
                deck_idx,
                value,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.transition_duration.set_value(value);
            }),

            // ── External I/O Deck Sources ─────────────────────
            EngineCommand::AddNdiDeck {
                channel_idx,
                source_name,
            } => self.cmd_add_ndi_deck(channel_idx, source_name),
            EngineCommand::AddSyphonDeck {
                channel_idx,
                server_name,
            } => self.cmd_add_syphon_deck(channel_idx, server_name),
            EngineCommand::AddSrtDeck {
                channel_idx,
                url,
                mode,
            } => self.cmd_add_srt_deck(channel_idx, url, mode),
            EngineCommand::AddHlsDeck { channel_idx, url } => {
                self.cmd_add_hls_deck(channel_idx, url)
            }
            EngineCommand::AddDashDeck { channel_idx, url } => {
                self.cmd_add_dash_deck(channel_idx, url)
            }
            EngineCommand::AddRtmpDeck {
                channel_idx,
                url,
                mode,
            } => self.cmd_add_rtmp_deck(channel_idx, url, mode),
            EngineCommand::AddHtmlDeck { channel_idx, url } => {
                self.cmd_add_html_deck(channel_idx, url)
            }

            // ── Transition Sequences ──────────────────────────
            EngineCommand::CreateSequence => self.cmd_create_sequence(),
            EngineCommand::DeleteSequence { idx } => self.cmd_delete_sequence(idx),
            EngineCommand::PlaySequence { idx } => self.cmd_play_sequence(idx),
            EngineCommand::StopSequence { idx } => self.cmd_stop_sequence(idx),
            EngineCommand::ToggleSequence { idx } => self.cmd_toggle_sequence(idx),
            EngineCommand::AddFadeStep {
                seq_idx,
                from_ch,
                to_ch,
            } => self.cmd_add_fade_step(seq_idx, from_ch, to_ch),
            EngineCommand::AddWaitStep { seq_idx } => self.cmd_add_wait_step(seq_idx),
            EngineCommand::AddGoToStep {
                seq_idx,
                step_index,
            } => self.cmd_add_goto_step(seq_idx, step_index),
            EngineCommand::RemoveStep { seq_idx, step_idx } => {
                self.cmd_remove_step(seq_idx, step_idx)
            }
            EngineCommand::SetStepDuration {
                seq_idx,
                step_idx,
                value,
                unit,
            } => self.cmd_set_step_duration(seq_idx, step_idx, value, unit),
            EngineCommand::SetStepEasing {
                seq_idx,
                step_idx,
                easing,
            } => self.cmd_set_step_easing(seq_idx, step_idx, easing),
            EngineCommand::SetStepTransitionShader {
                seq_idx,
                step_idx,
                shader_name,
            } => self.cmd_set_step_transition_shader(seq_idx, step_idx, shader_name),
            EngineCommand::MoveStep { seq_idx, from, to } => self.cmd_move_step(seq_idx, from, to),
            EngineCommand::SetStepDurationUnit {
                seq_idx,
                step_idx,
                unit,
            } => self.cmd_set_step_duration_unit(seq_idx, step_idx, unit),
            EngineCommand::ToggleStepDurationUnit { seq_idx, step_idx } => {
                self.cmd_toggle_step_duration_unit(seq_idx, step_idx)
            }
            EngineCommand::SetStepDurationValue {
                seq_idx,
                step_idx,
                value,
            } => self.cmd_set_step_duration_value(seq_idx, step_idx, value),
            EngineCommand::SetStepFromCh {
                seq_idx,
                step_idx,
                ch,
            } => self.cmd_set_step_from_ch(seq_idx, step_idx, ch),
            EngineCommand::SetStepToCh {
                seq_idx,
                step_idx,
                ch,
            } => self.cmd_set_step_to_ch(seq_idx, step_idx, ch),
            EngineCommand::SetGoToTarget {
                seq_idx,
                step_idx,
                target,
            } => self.cmd_set_goto_target(seq_idx, step_idx, target),
            EngineCommand::SetStepTargetAmount {
                seq_idx,
                step_idx,
                amount,
            } => self.cmd_set_step_target_amount(seq_idx, step_idx, amount),

            // ── Stream Library ─────────────────────────────────
            EngineCommand::AddStreamLibraryEntry { url, mode } => {
                self.cmd_add_stream_library_entry(url, mode)
            }
            EngineCommand::RemoveStreamLibraryEntry { url } => {
                self.cmd_remove_stream_library_entry(url)
            }
            EngineCommand::AddHlsLibraryEntry { url } => self.cmd_add_hls_library_entry(url),
            EngineCommand::RemoveHlsLibraryEntry { url } => self.cmd_remove_hls_library_entry(url),
            EngineCommand::AddDashLibraryEntry { url } => self.cmd_add_dash_library_entry(url),
            EngineCommand::RemoveDashLibraryEntry { url } => {
                self.cmd_remove_dash_library_entry(url)
            }
            EngineCommand::AddRtmpLibraryEntry { url, mode } => {
                self.cmd_add_rtmp_library_entry(url, mode)
            }
            EngineCommand::RemoveRtmpLibraryEntry { url } => {
                self.cmd_remove_rtmp_library_entry(url)
            }

            // ── Output Management ─────────────────────────────────
            EngineCommand::CreateHeadlessOutput { target } => {
                self.cmd_create_headless_output(target)
            }
            EngineCommand::StartOutput { idx } => self.cmd_start_output(idx),
            EngineCommand::StopOutput { idx } => self.cmd_stop_output(idx),
            EngineCommand::ToggleCalibration { idx } => self.cmd_toggle_calibration(idx),
            EngineCommand::SetWarpCorner {
                output_idx,
                assignment_idx,
                corner_idx,
                position,
            } => self.cmd_set_warp_corner(output_idx, assignment_idx, corner_idx, position),
            EngineCommand::ResetWarp {
                output_idx,
                assignment_idx,
            } => self.cmd_reset_warp(output_idx, assignment_idx),
            EngineCommand::SetEdgeBlend { output_idx, config } => {
                self.cmd_set_edge_blend(output_idx, config)
            }
            EngineCommand::SetEdgeBlendMode { output_idx, mode } => {
                self.cmd_set_edge_blend_mode(output_idx, mode)
            }
            EngineCommand::SetOutputRotation { idx, rotation } => {
                self.cmd_set_output_rotation(idx, rotation)
            }

            // ── Modulation Updates ────────────────────────────────
            EngineCommand::UpdateLfoFrequency { uuid, frequency } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        frequency: ref mut f,
                        ..
                    } = s
                    {
                        *f = frequency;
                    }
                })
            }
            EngineCommand::UpdateLfoWaveform { uuid, waveform } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        waveform: ref mut w,
                        ..
                    } = s
                    {
                        *w = waveform;
                    }
                })
            }
            EngineCommand::UpdateLfoPhase { uuid, phase } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        phase: ref mut p, ..
                    } = s
                    {
                        *p = phase;
                    }
                })
            }
            EngineCommand::UpdateLfoAmplitude { uuid, amplitude } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        amplitude: ref mut a,
                        ..
                    } = s
                    {
                        *a = amplitude;
                    }
                })
            }
            EngineCommand::UpdateLfoBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        bipolar: ref mut b, ..
                    } = s
                    {
                        *b = bipolar;
                    }
                })
            }
            EngineCommand::UpdateAudioSmoothing { uuid, smoothing } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        smoothing: ref mut sm,
                        ..
                    } = s
                    {
                        *sm = smoothing;
                    }
                })
            }
            EngineCommand::UpdateAudioFreqRange {
                uuid,
                freq_low,
                freq_high,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::AudioBand {
                    freq_low: ref mut fl,
                    freq_high: ref mut fh,
                    ..
                } = s
                {
                    *fl = freq_low;
                    *fh = freq_high;
                }
            }),
            EngineCommand::UpdateAudioGain { uuid, gain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        gain: ref mut g, ..
                    } = s
                    {
                        *g = gain;
                    }
                })
            }
            EngineCommand::UpdateAudioPreset { uuid, preset } => {
                let (lo, hi) = preset.freq_range();
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_low: ref mut fl,
                        freq_high: ref mut fh,
                        ..
                    } = s
                    {
                        *fl = lo;
                        *fh = hi;
                    }
                })
            }
            EngineCommand::UpdateAudioMode { uuid, mode } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        mode: ref mut m, ..
                    } = s
                    {
                        *m = mode;
                    }
                })
            }
            EngineCommand::UpdateAdsrAttack { uuid, attack } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        attack: ref mut a, ..
                    } = s
                    {
                        *a = attack;
                    }
                })
            }
            EngineCommand::UpdateAdsrDecay { uuid, decay } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        decay: ref mut d, ..
                    } = s
                    {
                        *d = decay;
                    }
                })
            }
            EngineCommand::UpdateAdsrSustain { uuid, sustain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        sustain: ref mut su,
                        ..
                    } = s
                    {
                        *su = sustain;
                    }
                })
            }
            EngineCommand::UpdateAdsrRelease { uuid, release } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        release: ref mut r, ..
                    } = s
                    {
                        *r = release;
                    }
                })
            }
            EngineCommand::TriggerAdsr { uuid } => {
                self.mixer.modulation_mut().trigger_adsr(&uuid);
                CommandResult::Ok
            }
            EngineCommand::ReleaseAdsr { uuid } => {
                self.mixer.modulation_mut().release_adsr(&uuid);
                CommandResult::Ok
            }
            EngineCommand::UpdateStepSeqSteps { uuid, steps } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer {
                        steps: ref mut st, ..
                    } = s
                    {
                        *st = steps;
                    }
                })
            }
            EngineCommand::UpdateStepSeqRate { uuid, rate } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer {
                        rate: ref mut r, ..
                    } = s
                    {
                        *r = rate;
                    }
                })
            }
            EngineCommand::UpdateStepSeqInterpolation {
                uuid,
                interpolation,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::StepSequencer {
                    interpolation: ref mut i,
                    ..
                } = s
                {
                    *i = interpolation;
                }
            }),
            EngineCommand::UpdateStepSeqBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer {
                        bipolar: ref mut b, ..
                    } = s
                    {
                        *b = bipolar;
                    }
                })
            }
            EngineCommand::SetStepSeqCount { uuid, count } => {
                let count = count.clamp(2, 64);
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { steps, .. } = s {
                        steps.resize(count, 0.0);
                    }
                })
            }
            EngineCommand::UpdateStepSeqValue {
                uuid,
                step_idx,
                value,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::StepSequencer { steps, .. } = s {
                    if step_idx < steps.len() {
                        steps[step_idx] = value;
                    }
                }
            }),
            EngineCommand::UpdateAudioFreqLow { uuid, freq_low } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_low: ref mut fl,
                        ..
                    } = s
                    {
                        *fl = freq_low;
                    }
                })
            }
            EngineCommand::UpdateAudioFreqHigh { uuid, freq_high } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_high: ref mut fh,
                        ..
                    } = s
                    {
                        *fh = freq_high;
                    }
                })
            }
            EngineCommand::UpdateAudioSource { uuid, source_id } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        source_id: ref mut sid,
                        ..
                    } = s
                    {
                        *sid = source_id;
                    }
                })
            }
            EngineCommand::UpdateAudioNoiseGate { uuid, noise_gate } => self
                .exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        noise_gate: ref mut ng,
                        ..
                    } = s
                    {
                        *ng = noise_gate;
                    }
                }),
            EngineCommand::AssignModOnMod {
                target_source_id,
                param_name,
                modulator_id,
                amount,
            } => {
                self.mixer.modulation_mut().assign_mod_on_mod(
                    &target_source_id,
                    &param_name,
                    &modulator_id,
                    amount,
                );
                CommandResult::Ok
            }
            EngineCommand::RemoveModOnMod {
                target_source_id,
                param_name,
            } => {
                self.mixer
                    .modulation_mut()
                    .clear_mod_on_mod(&target_source_id, &param_name);
                CommandResult::Ok
            }

            // ── Analyzers ────────────────────────────────────────
            EngineCommand::RequestAnalyzer {
                deck_id,
                analyzer_type,
                options,
            } => match self.request_analyzer(&deck_id, &analyzer_type, &options) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::ReleaseAnalyzer {
                deck_id,
                analyzer_type,
            } => {
                self.release_analyzer(&deck_id, &analyzer_type);
                CommandResult::Ok
            }
            EngineCommand::AddAnalyzerModSource {
                deck_id,
                analyzer_type,
                output_name,
            } => {
                let source = crate::modulation::ModulationSource::Analyzer {
                    deck_id,
                    analyzer_type,
                    output_name,
                    smoothing: 0.3,
                };
                let uuid = self.mixer.modulation_mut().add_source(source);
                CommandResult::OkWithId { uuid }
            }
            EngineCommand::UpdateAnalyzerSmoothing { uuid, smoothing } => {
                if let Some(src) = self.mixer.modulation_mut().source_mut(&uuid) {
                    if let crate::modulation::ModulationSource::Analyzer { smoothing: s, .. } = src
                    {
                        *s = smoothing.clamp(0.0, 0.99);
                        CommandResult::Ok
                    } else {
                        CommandResult::Err {
                            code: ErrorCode::InvalidInput,
                            message: "Source is not an analyzer".into(),
                        }
                    }
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("Modulation source '{uuid}' not found"),
                    }
                }
            }

            // ── Device Scanning ───────────────────────────────────
            EngineCommand::RescanNdi => {
                self.external_io.ndi_manager.discover();
                CommandResult::Ok
            }
            EngineCommand::RescanSyphon => {
                #[cfg(target_os = "macos")]
                self.external_io.syphon_manager.discover();
                CommandResult::Ok
            }
            EngineCommand::RescanCameras => {
                self.camera_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::RescanMidi => {
                if let Some(ref mut midi) = self.input.midi_devices {
                    midi.load_user_profiles(&self.session.workspace.controller_profiles_dir());
                    if let Err(e) = midi.scan_devices() {
                        return CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: e.to_string(),
                        };
                    }
                    self.input.controller_led_mgr.sync_devices(midi);
                    self.input.auto_map_engine.sync_devices(midi);
                }
                CommandResult::Ok
            }
            EngineCommand::RescanAudio => {
                self.audio_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::ToggleAudioSource { source_id, enabled } => {
                if enabled {
                    if let Err(e) = self.audio_manager.open_source(source_id) {
                        log::warn!("Failed to open audio source {}: {}", source_id, e);
                        return CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: format!("Failed to open audio source: {}", e),
                        };
                    }
                } else {
                    self.audio_manager.close_source(source_id);
                }
                CommandResult::Ok
            }
            EngineCommand::SetMidiDeviceEnabled { device_id, enabled } => {
                if let Some(ref mut midi) = self.input.midi_devices {
                    midi.set_device_enabled(device_id, enabled);
                }
                CommandResult::Ok
            }

            // ── MIDI Mappings ─────────────────────────────────────
            EngineCommand::ClearMidiMappings => {
                self.input.midi_mappings.clear_all();
                CommandResult::Ok
            }
            EngineCommand::RemoveMidiMapping { key } => {
                self.input.midi_mappings.remove(&key);
                CommandResult::Ok
            }

            // ── Clock ─────────────────────────────────────────────
            EngineCommand::SetClockPreference { preference } => {
                self.input.clock_manager.set_preference(preference);
                CommandResult::Ok
            }
            EngineCommand::SetManualBpm { bpm } => {
                self.input
                    .clock_manager
                    .set_preference(crate::clock::ClockPreference::ForceManual { bpm });
                CommandResult::Ok
            }

            // ── Parameters (index-based) ────────────────────────────
            EngineCommand::SetGeneratorParam {
                channel_idx,
                deck_idx,
                name,
                value,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        ch.decks[deck_idx].deck.generator_params.set(&name, value);
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetEffectParam {
                channel_idx,
                deck_idx,
                effect_idx,
                name,
                value,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        let deck = &mut ch.decks[deck_idx].deck;
                        if effect_idx < deck.effects.len() {
                            deck.effects[effect_idx].params.set(&name, value);
                        }
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetChannelEffectParam {
                channel_idx,
                effect_idx,
                name,
                value,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if effect_idx < ch.effects.len() {
                        ch.effects[effect_idx].params.set(&name, value);
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetMasterEffectParam {
                effect_idx,
                name,
                value,
            } => {
                if effect_idx < self.mixer.master_effects().len() {
                    self.mixer.master_effects_mut()[effect_idx]
                        .params
                        .set(&name, value);
                }
                CommandResult::Ok
            }
            EngineCommand::ResetGeneratorParamsToDefaults {
                channel_idx,
                deck_idx,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        ch.decks[deck_idx].deck.generator_params.reset_to_defaults();
                    }
                }
                CommandResult::Ok
            }

            // ── Resolution ────────────────────────────────────────
            EngineCommand::SetRenderResolution { width, height } => {
                self.set_render_resolution(width, height);
                CommandResult::Ok
            }

            EngineCommand::SetTargetFps { fps } => {
                self.set_target_fps(fps);
                CommandResult::Ok
            }

            EngineCommand::StartPerfProfile { frames } => {
                self.mixer.start_perf_profile(frames);
                CommandResult::Ok
            }

            // ── Persistence ───────────────────────────────────────
            EngineCommand::SaveWorkspace => {
                let layout = crate::usecases::ui::UILayoutState::default();
                self.save_workspace(&layout);
                CommandResult::Ok
            }
            EngineCommand::LoadWorkspace => {
                let _ = self.load_workspace();
                CommandResult::Ok
            }

            // ── History ───────────────────────────────────────────
            EngineCommand::Undo => {
                let current = crate::persistence::snapshot_scene(
                    &self.mixer,
                    self.render_width,
                    self.render_height,
                );
                if let Some(config) = self.session.history.undo(current) {
                    let rw = self.render_width;
                    let rh = self.render_height;
                    let (warnings, _) = self.apply_scene_diff(&config, rw, rh);
                    self.mixer.clear_sub_mix_cache();
                    for w in &warnings {
                        log::warn!("Undo warning: {}", w);
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: "Nothing to undo".into(),
                    }
                }
            }
            EngineCommand::Redo => {
                let current = crate::persistence::snapshot_scene(
                    &self.mixer,
                    self.render_width,
                    self.render_height,
                );
                if let Some(config) = self.session.history.redo(current) {
                    let rw = self.render_width;
                    let rh = self.render_height;
                    let (warnings, _) = self.apply_scene_diff(&config, rw, rh);
                    self.mixer.clear_sub_mix_cache();
                    for w in &warnings {
                        log::warn!("Redo warning: {}", w);
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: "Nothing to redo".into(),
                    }
                }
            }

            // ── System ────────────────────────────────────────────
            EngineCommand::Shutdown => {
                self.shutdown_requested = true;
                CommandResult::Ok
            }
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

    /// Collect all data needed by the UI into a read-only snapshot.
    /// `layout` is UI-consumer-owned selection/layout state.
    /// `deck_preview_textures` and `main_output_texture` are egui-owned state passed in.
    pub fn collect_ui_data(
        &self,
        layout: &crate::usecases::ui::UILayoutState,
        deck_preview_textures: &std::collections::HashMap<(usize, usize), egui::TextureId>,
        channel_preview_textures: &std::collections::HashMap<usize, egui::TextureId>,
        output_preview_textures: &std::collections::HashMap<usize, egui::TextureId>,
        main_output_texture: Option<egui::TextureId>,
    ) -> crate::usecases::ui::UIData {
        snapshot::build_ui_data(
            self,
            layout,
            deck_preview_textures,
            channel_preview_textures,
            output_preview_textures,
            main_output_texture,
        )
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

    /// Change the master render resolution. Resizes all textures in the pipeline.
    pub fn set_render_resolution(&mut self, width: u32, height: u32) {
        if width == self.render_width && height == self.render_height {
            return;
        }
        if width == 0 || height == 0 {
            log::warn!("Ignoring zero render resolution {}×{}", width, height);
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
        assert!(
            matches!(result, crate::engine::CommandResult::Ok),
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

    #[test]
    fn smoke_undo_redo_roundtrip() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        // Set crossfader to 0.5 (will trigger history push)
        tx.send((crate::engine::EngineCommand::SetCrossfader(0.5), None))
            .unwrap();
        app.process_commands();
        // Undo
        tx.send((crate::engine::EngineCommand::Undo, None)).unwrap();
        app.process_commands();
        // Redo
        tx.send((crate::engine::EngineCommand::Redo, None)).unwrap();
        app.process_commands();
        // Just verify no crash — undo/redo correctness is tested in history.rs
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
