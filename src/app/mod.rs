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

    /// Target render FPS in headless mode (ignored in windowed)
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
}

impl AppConfig {
    /// Resolve the effective workspace root (CLI flag or cwd).
    pub fn effective_workspace_root(&self) -> std::path::PathBuf {
        self.workspace_root.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
    }
}

use crate::audio::AudioManager;
use crate::camera::CameraManager;
use crate::keymap::KeymapStore;
use crate::midi;
use crate::mixer::Mixer;
use crate::osc::{OscConfig, OscFeedbackSender, OscReceiver};
use crate::persistence::Workspace;
use crate::registry::ShaderRegistry;
use crate::renderer::context::{GpuContext, UnifiedOutput};
use crate::surface::SurfaceManager;
use crate::notifications::NotificationSystem;

use crate::engine::{CommandEnvelope, CommandResult, ErrorCode, EngineCommand, EngineState};

/// Core engine application. Owns all subsystems except window/egui.
///
/// Implements all engine traits (MixerCommands, AudioCommands, etc.)
/// for direct same-thread access. Also processes EngineCommands from
/// cross-thread consumers via mpsc channel.
pub struct VardaApp {
    // ── Engine subsystems (always present after construction) ──
    mixer: Mixer,
    audio_manager: AudioManager,
    camera_manager: CameraManager,
    registry: ShaderRegistry,
    context: GpuContext,

    // ── Keyboard shortcuts ────────────────────────────────────
    keymap: KeymapStore,

    // ── Control subsystems ─────────────────────────────────────
    osc_receiver: Option<OscReceiver>,
    osc_feedback: Option<OscFeedbackSender>,
    osc_config: OscConfig,
    midi_devices: Option<midi::MidiDeviceManager>,
    midi_mappings: midi::MidiMappingStore,
    controller_led_mgr: midi::ControllerLedManager,
    auto_map_engine: midi::AutoMapEngine,
    clock_manager: crate::clock::ClockManager,

    // ── Output & surfaces ──────────────────────────────────────
    outputs: Vec<UnifiedOutput>,
    surface_manager: SurfaceManager,
    calibration_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,

    // ── Notifications ──────────────────────────────────────────
    notifications: NotificationSystem,

    // ── Persistence ────────────────────────────────────────────
    workspace: Workspace,

    // ── Pending actions (deferred to event loop) ───────────────
    pending_output_creates: Vec<crate::scene::OutputConfig>,
    cached_monitors: Vec<(String, winit::monitor::MonitorHandle)>,

    // ── Audio textures (GPU resource, owned here) ──────────────
    audio_textures: crate::audio::AudioTextures,

    // ── Message passing (cross-thread consumers) ───────────────
    command_rx: tokio::sync::mpsc::UnboundedReceiver<CommandEnvelope>,
    command_tx: tokio::sync::mpsc::UnboundedSender<CommandEnvelope>,

    // ── State distribution ─────────────────────────────────────
    state_tx: std::sync::Arc<std::sync::RwLock<Option<EngineState>>>,

    // ── Frame timing ───────────────────────────────────────────
    last_frame_instant: std::time::Instant,
    fps_history: Vec<f32>,
    fps_smoothed: f32,
    frame_count: u64,

    // ── System monitoring (CPU / RAM) ────────────────────────
    system_monitor: crate::sysmon::SystemMonitor,

    // ── Render resolution (configurable, scene-level) ───────
    render_width: u32,
    render_height: u32,

    // ── External I/O (input/discovery only — output is per-UnifiedOutput) ──
    ndi_manager: crate::ndi::NdiManager,
    #[cfg(target_os = "macos")]
    syphon_manager: crate::syphon::SyphonManager,
    stream_manager: crate::stream::StreamManager,
    /// Configured SRT input sources in the library (url, mode).
    stream_library: Vec<(String, crate::stream::SrtMode)>,
    /// Configured HLS input sources in the library (urls).
    hls_library: Vec<String>,
    /// Configured DASH input sources in the library (urls).
    dash_library: Vec<String>,

    // ── Presets ─────────────────────────────────────────────────
    preset_library: crate::persistence::presets::PresetLibrary,

    // ── History (undo/redo) ─────────────────────────────────────
    pub(crate) history: history::HistoryManager,

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
        let mut registry = ShaderRegistry::new();
        if let Err(e) = registry.add_library_path("shaders") {
            log::warn!("Failed to add shaders path: {}", e);
        }
        match registry.scan() {
            Ok(count) => log::info!("Loaded {} shaders", count),
            Err(e) => log::error!("Failed to scan shaders: {}", e),
        }
        if let Err(e) = registry.start_watching() {
            log::warn!("Failed to start shader hot-reload: {}", e);
        }

        let audio_manager = AudioManager::new();

        let workspace = Workspace::new(config.effective_workspace_root());

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

        let osc_receiver = if osc_config.enabled {
            match OscReceiver::new(osc_config.in_port) {
                Ok(osc) => { log::info!("OSC receiver started on port {}", osc_config.in_port); Some(osc) }
                Err(e) => { log::warn!("Failed to start OSC receiver on port {}: {}", osc_config.in_port, e); None }
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
            Err(e) => { log::warn!("Failed to create OSC feedback sender: {}", e); None }
        };

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
            Err(e) => { log::warn!("Failed to initialize MIDI: {}", e); None }
        };

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let state_tx = std::sync::Arc::new(std::sync::RwLock::new(None));

        // Always create GPU-dependent resources up front
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
            context: gpu,
            osc_receiver,
            osc_feedback,
            osc_config,
            midi_devices,
            keymap: KeymapStore::with_defaults(),
            midi_mappings: midi::MidiMappingStore::new(),
            controller_led_mgr,
            auto_map_engine,
            clock_manager: crate::clock::ClockManager::new(),
            outputs: Vec::new(),
            surface_manager: SurfaceManager::new(),
            calibration_textures,
            notifications: NotificationSystem::new(),
            workspace,
            pending_output_creates: Vec::new(),
            cached_monitors: Vec::new(),
            audio_textures,
            command_rx,
            command_tx,
            state_tx,
            last_frame_instant: std::time::Instant::now(),
            fps_history: Vec::with_capacity(60),
            fps_smoothed: 0.0,
            frame_count: 0,
            system_monitor: crate::sysmon::SystemMonitor::new(),
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
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
            preset_library,
            history: history::HistoryManager::new(),
            midi_pending_undo: false,
            midi_pending_redo: false,
            midi_pending_save: false,
            shutdown_requested: false,
        })
    }

    /// Get a command sender for cross-thread consumers (HTTP API, CLI).
    pub fn command_sender(&self) -> tokio::sync::mpsc::UnboundedSender<CommandEnvelope> {
        self.command_tx.clone()
    }

    /// Get a shared reference to the latest engine state (for cross-thread consumers).
    pub fn state_reader(&self) -> std::sync::Arc<std::sync::RwLock<Option<EngineState>>> {
        self.state_tx.clone()
    }

    /// Process all queued cross-thread commands. Called once per frame.
    ///
    /// Exhaustive match — the compiler enforces that every EngineCommand variant
    /// is handled. Adding a new variant requires wiring it here.
    pub fn process_commands(&mut self) {
        while let Ok((cmd, reply_tx)) = self.command_rx.try_recv() {
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
        CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found".into() }
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
            EngineCommand::AutoCrossfade { target, duration_secs, easing } => {
                self.start_auto_crossfade(target, duration_secs, easing);
                CommandResult::Ok
            }
            EngineCommand::BeatCrossfade { target, beats } => {
                self.start_beat_crossfade(target, beats);
                CommandResult::Ok
            }
            EngineCommand::AddDeck { channel_idx, shader_name } => {
                match self.add_deck(channel_idx, &shader_name) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::AddImageDeck { channel_idx, path } => {
                match self.add_image_deck(channel_idx, &path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::AddVideoDeck { channel_idx, path } => {
                match self.add_video_deck(channel_idx, &path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::AddSolidColorDeck { channel_idx, color } => {
                match self.add_solid_color_deck(channel_idx, color) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::AddCameraDeck { channel_idx, camera_id } => {
                match self.add_camera_deck(channel_idx, camera_id) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::RemoveDeck { channel_idx, deck_idx } => {
                match self.remove_deck(channel_idx, deck_idx) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::NotFound, message: e.to_string() },
                }
            }
            EngineCommand::MoveDeck { src_ch, src_deck, dst_ch } => {
                match self.move_deck(src_ch, src_deck, dst_ch) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::SetDeckOpacity { channel_idx, deck_idx, opacity } => {
                self.set_deck_opacity(channel_idx, deck_idx, opacity);
                CommandResult::Ok
            }
            EngineCommand::SetDeckBlendMode { channel_idx, deck_idx, mode } => {
                self.set_deck_blend_mode(channel_idx, deck_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::SetDeckSolo { channel_idx, deck_idx, solo } => {
                self.set_deck_solo(channel_idx, deck_idx, solo);
                CommandResult::Ok
            }
            EngineCommand::SetDeckMute { channel_idx, deck_idx, mute } => {
                self.set_deck_mute(channel_idx, deck_idx, mute);
                CommandResult::Ok
            }
            EngineCommand::SetDeckScalingMode { channel_idx, deck_idx, mode } => {
                self.set_deck_scaling_mode(channel_idx, deck_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::SetChannelOpacity { channel_idx, opacity } => {
                self.set_channel_opacity(channel_idx, opacity);
                CommandResult::Ok
            }
            EngineCommand::SetChannelBlendMode { channel_idx, mode } => {
                self.set_channel_blend_mode(channel_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::AddChannel => {
                match self.add_channel() {
                    Ok(_idx) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                }
            }
            EngineCommand::RemoveChannel { channel_idx } => {
                match self.remove_channel(channel_idx) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::NotFound, message: e.to_string() },
                }
            }
            EngineCommand::AddEffect { target, shader_name } => {
                match self.add_effect(target, &shader_name) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
                }
            }
            EngineCommand::RemoveEffect { target, effect_idx } => {
                self.remove_effect(target, effect_idx);
                CommandResult::Ok
            }
            EngineCommand::ToggleEffect { target, effect_idx } => {
                self.toggle_effect(target, effect_idx);
                CommandResult::Ok
            }
            EngineCommand::MoveEffect { target, from_idx, to_idx } => {
                self.move_effect(target, from_idx, to_idx);
                CommandResult::Ok
            }
            EngineCommand::SetTransition { shader_name } => {
                match self.set_transition(shader_name.as_deref()) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
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
                    Err(e) => CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() },
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
            EngineCommand::AddLfo { waveform, frequency } => {
                self.add_lfo(waveform, frequency);
                CommandResult::Ok
            }
            EngineCommand::AddAudioBand { preset, source_id } => {
                self.add_audio_band(preset, source_id);
                CommandResult::Ok
            }
            EngineCommand::AddAdsr { attack, decay, sustain, release } => {
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
            EngineCommand::AssignModulation { target, source_id, amount } => {
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
                use crate::renderer::context::{UnifiedOutput, OutputTarget};
                if let Some(output) = self.outputs.get_mut(idx) {
                    match output {
                        UnifiedOutput::Window(w) => {
                            if target.is_windowed() {
                                let monitor = match &target {
                                    OutputTarget::Display { monitor_index, .. } => {
                                        self.cached_monitors.get(*monitor_index).map(|(_, h)| h.clone())
                                    }
                                    _ => None,
                                };
                                w.set_target(target, monitor);
                            }
                            CommandResult::Ok
                        }
                        UnifiedOutput::Headless(h) => {
                            if target.is_headless() {
                                if h.active {
                                    if let Some(mut sub) = h.subprocess.take() { sub.stop(); }
                                    h.active = false;
                                    h.started_at = None;
                                }
                                h.target = target;
                            }
                            CommandResult::Ok
                        }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }

            // ── Surfaces ────────────────────────────────────
            EngineCommand::AddSurface { name, source } => {
                self.add_surface(&name, source);
                CommandResult::Ok
            }
            EngineCommand::AddPolygonSurface { name, vertices, source } => {
                self.add_polygon_surface(&name, &vertices, source);
                CommandResult::Ok
            }
            EngineCommand::AddCircleSurface { name, center, radius, sides, aspect_ratio, source } => {
                self.add_circle_surface(&name, center, radius, sides, aspect_ratio, source);
                CommandResult::Ok
            }
            EngineCommand::RemoveSurface { uuid } => {
                self.remove_surface(&uuid);
                CommandResult::Ok
            }
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
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    surface.vertices = vertices;
                    self.recompute_auto_edge_blend();
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::DuplicateSurface { uuid } => {
                if let Some(new_uuid) = self.surface_manager.duplicate_surface(&uuid) {
                    CommandResult::OkWithId { uuid: new_uuid }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::FlipSurfaceHorizontal { uuid } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    for v in &mut surface.vertices {
                        v[0] = 1.0 - v[0];
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::FlipSurfaceVertical { uuid } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    for v in &mut surface.vertices {
                        v[1] = 1.0 - v[1];
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::InsertSurfaceVertex { uuid, after_vert_idx, position } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    surface.convert_to_polygon();
                    if after_vert_idx < surface.vertices.len() {
                        surface.vertices.insert(after_vert_idx + 1, position);
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::SetCircleRadius { uuid, radius } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    if let Some(ref mut hint) = surface.circle_hint {
                        hint.radius = radius;
                        surface.vertices = hint.generate_vertices();
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::SetCircleSides { uuid, sides } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    if let Some(ref mut hint) = surface.circle_hint {
                        hint.sides = sides;
                        surface.vertices = hint.generate_vertices();
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::ConvertSurfaceToPolygon { uuid } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    surface.convert_to_polygon();
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::CombineSurfaces { uuids } => {
                if let Some(new_uuid) = self.surface_manager.combine_surfaces(&uuids) {
                    CommandResult::OkWithId { uuid: new_uuid }
                } else {
                    CommandResult::Err { code: ErrorCode::InvalidInput, message: "Failed to combine surfaces".into() }
                }
            }
            EngineCommand::MoveSurface { uuid, dx, dy } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    surface.translate(dx, dy);
                    if let Some(ref mut hint) = surface.circle_hint {
                        let n = surface.vertices.len().max(1) as f32;
                        let sum = surface.vertices.iter().fold([0.0f32, 0.0], |acc, v| {
                            [acc[0] + v[0], acc[1] + v[1]]
                        });
                        hint.center = [sum[0] / n, sum[1] / n];
                    }
                    self.recompute_auto_edge_blend();
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::UpdateSurfaceContourVertices { uuid, contour, vertices } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(&uuid) {
                    if contour == 0 {
                        if let Some(ref mut hint) = surface.circle_hint {
                            let n = vertices.len().max(1) as f32;
                            let sum = vertices.iter().fold([0.0f32, 0.0], |acc, v| {
                                [acc[0] + v[0], acc[1] + v[1]]
                            });
                            hint.center = [sum[0] / n, sum[1] / n];
                        }
                        surface.vertices = vertices;
                    } else if let Some(c) = surface.extra_contours.get_mut(contour - 1) {
                        *c = vertices;
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: format!("Surface {} not found", uuid) }
                }
            }
            EngineCommand::AssignSurfaceToOutput { output_uuid, surface_uuid } => {
                self.assign_surface_to_output(&output_uuid, &surface_uuid);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::UnassignSurfaceFromOutput { output_uuid, assignment_idx } => {
                self.unassign_surface_from_output(&output_uuid, assignment_idx);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::AssignSurfaceToOutputByIdx { output_idx, surface_uuid } => {
                if let Some(output) = self.outputs.get_mut(output_idx) {
                    let assignments = output.surface_assignments_mut();
                    if !assignments.iter().any(|a| a.surface_uuid == surface_uuid) {
                        if let Some((_, surface)) = self.surface_manager.find_by_uuid(&surface_uuid) {
                            let bb = surface.bounding_box();
                            let assignment = crate::renderer::context::SurfaceAssignment {
                                surface_uuid,
                                warp_corners: [
                                    [bb.x, bb.y],
                                    [bb.x + bb.width, bb.y],
                                    [bb.x + bb.width, bb.y + bb.height],
                                    [bb.x, bb.y + bb.height],
                                ],
                                enabled: true,
                                overlap_zones: Default::default(),
                            };
                            assignments.push(assignment);
                        }
                    }
                    self.recompute_auto_edge_blend();
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }
            EngineCommand::UnassignSurfaceFromOutputByIdx { output_idx, assignment_idx } => {
                if let Some(output) = self.outputs.get_mut(output_idx) {
                    let assignments = output.surface_assignments_mut();
                    if assignment_idx < assignments.len() {
                        assignments.remove(assignment_idx);
                    }
                    self.recompute_auto_edge_blend();
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }

            // ── Video Playback ────────────────────────────────
            EngineCommand::VideoTogglePlay { channel_idx, deck_idx } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Some(ps) = ch.decks[deck_idx].deck.playback_state_mut() {
                            ps.playing = !ps.playing;
                            return CommandResult::Ok;
                        }
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found or not a video".into() }
            }
            EngineCommand::VideoSeek { channel_idx, deck_idx, position_secs } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Err(e) = ch.decks[deck_idx].deck.video_seek(position_secs) {
                            return CommandResult::Err { code: ErrorCode::InvalidInput, message: e.to_string() };
                        }
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found".into() }
            }
            EngineCommand::VideoSetSpeed { channel_idx, deck_idx, speed } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Some(ps) = ch.decks[deck_idx].deck.playback_state_mut() {
                            ps.speed = speed;
                            return CommandResult::Ok;
                        }
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found or not a video".into() }
            }
            EngineCommand::VideoSetLoopMode { channel_idx, deck_idx, mode } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Some(ps) = ch.decks[deck_idx].deck.playback_state_mut() {
                            ps.loop_mode = mode;
                            return CommandResult::Ok;
                        }
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found or not a video".into() }
            }
            EngineCommand::VideoSetInPoint { channel_idx, deck_idx, secs } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Some(ps) = ch.decks[deck_idx].deck.playback_state_mut() {
                            ps.in_point = secs;
                            return CommandResult::Ok;
                        }
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found or not a video".into() }
            }
            EngineCommand::VideoSetOutPoint { channel_idx, deck_idx, secs } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Some(ps) = ch.decks[deck_idx].deck.playback_state_mut() {
                            ps.out_point = secs;
                            return CommandResult::Ok;
                        }
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found or not a video".into() }
            }
            EngineCommand::VideoClearInOutPoints { channel_idx, deck_idx } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        if let Some(ps) = ch.decks[deck_idx].deck.playback_state_mut() {
                            ps.in_point = 0.0;
                            ps.out_point = 0.0;
                            return CommandResult::Ok;
                        }
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found or not a video".into() }
            }

            // ── Deck Auto-Transitions ─────────────────────────
            EngineCommand::SetAutoTransitionEnabled { channel_idx, deck_idx, enabled } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    at.enabled = enabled;
                    if !enabled { at.phase = crate::channel::DeckTransitionPhase::Inactive; }
                })
            }
            EngineCommand::SetAutoTransitionTrigger { channel_idx, deck_idx, clip_end } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    at.trigger = if clip_end { crate::channel::TransitionTrigger::ClipEnd } else { crate::channel::TransitionTrigger::Timer };
                })
            }
            EngineCommand::SetAutoTransitionPlayDuration { channel_idx, deck_idx, value, unit } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    at.play_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
                })
            }
            EngineCommand::SetAutoTransitionDuration { channel_idx, deck_idx, value, unit } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    at.transition_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
                })
            }
            EngineCommand::SetAutoTransitionShader { channel_idx, deck_idx, shader_name } => {
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
                            if let Some(shader) = self.registry.transitions().iter()
                                .find(|s| s.name() == *shader_name) {
                                let _ = slot.set_transition_shader(&self.context, (*shader).clone());
                            }
                        } else {
                            slot.transition_effect = None;
                        }
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err { code: ErrorCode::NotFound, message: "Deck not found".into() }
            }
            EngineCommand::ToggleAutoTransitionPlayDurationUnit { channel_idx, deck_idx } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    let next_unit = at.play_duration.unit().next();
                    at.play_duration = crate::channel::DurationSpec::from_value_unit(at.play_duration.value(), next_unit);
                })
            }
            EngineCommand::ToggleAutoTransitionDurationUnit { channel_idx, deck_idx } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    let next_unit = at.transition_duration.unit().next();
                    at.transition_duration = crate::channel::DurationSpec::from_value_unit(at.transition_duration.value(), next_unit);
                })
            }
            EngineCommand::SetAutoTransitionPlayDurationValue { channel_idx, deck_idx, value } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    at.play_duration.set_value(value);
                })
            }
            EngineCommand::SetAutoTransitionDurationValue { channel_idx, deck_idx, value } => {
                self.exec_auto_transition(channel_idx, deck_idx, |at| {
                    at.transition_duration.set_value(value);
                })
            }

            // ── External I/O Deck Sources ─────────────────────
            EngineCommand::AddNdiDeck { channel_idx, source_name } => {
                match self.ndi_manager.start_receive(&source_name, &self.context.device) {
                    Some(receiver_idx) => {
                        let (src_w, src_h) = self.ndi_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                        match crate::deck::Deck::new_from_ndi(&self.context, receiver_idx, &source_name, src_w, src_h, self.render_width, self.render_height) {
                            Ok(deck) => {
                                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                    ch.add_deck(deck);
                                    CommandResult::Ok
                                } else {
                                    CommandResult::Err { code: ErrorCode::NotFound, message: "Channel not found".into() }
                                }
                            }
                            Err(e) => CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    None => CommandResult::Err { code: ErrorCode::InvalidInput, message: format!("Failed to start NDI receive for '{}'", source_name) },
                }
            }
            EngineCommand::AddSyphonDeck { channel_idx, server_name } => {
                #[cfg(target_os = "macos")]
                {
                    match self.syphon_manager.start_receive(&server_name, &self.context.device) {
                        Some(client_idx) => {
                            let (src_w, src_h) = self.syphon_manager.client_dimensions(client_idx).unwrap_or((1920, 1080));
                            match crate::deck::Deck::new_from_syphon(&self.context, client_idx, &server_name, src_w, src_h, self.render_width, self.render_height) {
                                Ok(deck) => {
                                    if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                        ch.add_deck(deck);
                                        CommandResult::Ok
                                    } else {
                                        CommandResult::Err { code: ErrorCode::NotFound, message: "Channel not found".into() }
                                    }
                                }
                                Err(e) => CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                            }
                        }
                        None => CommandResult::Err { code: ErrorCode::InvalidInput, message: format!("Failed to start Syphon receive for '{}'", server_name) },
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (channel_idx, server_name);
                    CommandResult::Err { code: ErrorCode::Unavailable, message: "Syphon is only available on macOS".into() }
                }
            }
            EngineCommand::AddSrtDeck { channel_idx, url, mode } => {
                match self.stream_manager.start_srt_receive(&url, mode, &self.context.device) {
                    Some(receiver_idx) => {
                        let (src_w, src_h) = self.stream_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                        match crate::deck::Deck::new_from_srt(&self.context, receiver_idx, &url, src_w, src_h, self.render_width, self.render_height) {
                            Ok(deck) => {
                                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                    ch.add_deck(deck);
                                    CommandResult::Ok
                                } else {
                                    CommandResult::Err { code: ErrorCode::NotFound, message: "Channel not found".into() }
                                }
                            }
                            Err(e) => CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    None => CommandResult::Err { code: ErrorCode::InvalidInput, message: format!("Failed to start SRT receive for '{}'", url) },
                }
            }

            EngineCommand::AddHlsDeck { channel_idx, url } => {
                match self.stream_manager.start_receive(&url, crate::stream::StreamProtocol::Hls, &self.context.device) {
                    Some(receiver_idx) => {
                        let (src_w, src_h) = self.stream_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                        match crate::deck::Deck::new_from_hls(&self.context, receiver_idx, &url, src_w, src_h, self.render_width, self.render_height) {
                            Ok(deck) => {
                                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                    ch.add_deck(deck);
                                    CommandResult::Ok
                                } else {
                                    CommandResult::Err { code: ErrorCode::NotFound, message: "Channel not found".into() }
                                }
                            }
                            Err(e) => CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    None => CommandResult::Err { code: ErrorCode::InvalidInput, message: format!("Failed to start HLS receive for '{}'", url) },
                }
            }
            EngineCommand::AddDashDeck { channel_idx, url } => {
                match self.stream_manager.start_receive(&url, crate::stream::StreamProtocol::Dash, &self.context.device) {
                    Some(receiver_idx) => {
                        let (src_w, src_h) = self.stream_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                        match crate::deck::Deck::new_from_dash(&self.context, receiver_idx, &url, src_w, src_h, self.render_width, self.render_height) {
                            Ok(deck) => {
                                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                    ch.add_deck(deck);
                                    CommandResult::Ok
                                } else {
                                    CommandResult::Err { code: ErrorCode::NotFound, message: "Channel not found".into() }
                                }
                            }
                            Err(e) => CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    None => CommandResult::Err { code: ErrorCode::InvalidInput, message: format!("Failed to start DASH receive for '{}'", url) },
                }
            }

            // ── Transition Sequences ──────────────────────────
            EngineCommand::CreateSequence => {
                let n = self.mixer.transition_sequences().len() + 1;
                self.mixer.transition_sequences_mut().push(
                    crate::mixer::TransitionSequence::new(format!("Sequence {}", n))
                );
                CommandResult::Ok
            }
            EngineCommand::DeleteSequence { idx } => {
                if idx < self.mixer.transition_sequences().len() {
                    self.mixer.transition_sequences_mut().remove(idx);
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::PlaySequence { idx } => {
                self.mixer.start_sequence(idx);
                CommandResult::Ok
            }
            EngineCommand::StopSequence { idx } => {
                self.mixer.stop_sequence(idx);
                CommandResult::Ok
            }
            EngineCommand::ToggleSequence { idx } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(idx) {
                    seq.enabled = !seq.enabled;
                    if !seq.enabled { seq.state.reset(); }
                }
                CommandResult::Ok
            }
            EngineCommand::AddFadeStep { seq_idx, from_ch, to_ch } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    seq.steps.push(crate::mixer::TransitionStep { kind: crate::mixer::StepKind::Fade {
                        from_ch, to_ch,
                        duration: crate::channel::DurationSpec::Seconds(2.0),
                        easing: crate::mixer::CrossfadeEasing::EaseInOut,
                        transition_shader: None,
                    }});
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::AddWaitStep { seq_idx } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    seq.steps.push(crate::mixer::TransitionStep { kind: crate::mixer::StepKind::Wait {
                        duration: crate::channel::DurationSpec::Seconds(2.0),
                    }});
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::AddGoToStep { seq_idx, step_index } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    seq.steps.push(crate::mixer::TransitionStep { kind: crate::mixer::StepKind::GoTo { step_index } });
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::RemoveStep { seq_idx, step_idx } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if step_idx < seq.steps.len() {
                        seq.steps.remove(step_idx);
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepDuration { seq_idx, step_idx, value, unit } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        match &mut step.kind {
                            crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                                *duration = crate::channel::DurationSpec::from_value_unit(value, unit);
                            }
                            _ => {}
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepEasing { seq_idx, step_idx, easing } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        if let crate::mixer::StepKind::Fade { easing: e, .. } = &mut step.kind {
                            *e = match easing.as_str() {
                                "Linear" => crate::mixer::CrossfadeEasing::Linear,
                                "EaseIn" => crate::mixer::CrossfadeEasing::EaseIn,
                                "EaseOut" => crate::mixer::CrossfadeEasing::EaseOut,
                                _ => crate::mixer::CrossfadeEasing::EaseInOut,
                            };
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepTransitionShader { seq_idx, step_idx, shader_name } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        if let crate::mixer::StepKind::Fade { transition_shader, .. } = &mut step.kind {
                            *transition_shader = shader_name;
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }

            EngineCommand::MoveStep { seq_idx, from, to } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if from < seq.steps.len() && to < seq.steps.len() && from != to {
                        let step = seq.steps.remove(from);
                        seq.steps.insert(to, step);
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepDurationUnit { seq_idx, step_idx, unit } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        match &mut step.kind {
                            crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                                *duration = crate::channel::DurationSpec::from_value_unit(duration.value(), unit);
                            }
                            _ => {}
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::ToggleStepDurationUnit { seq_idx, step_idx } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        match &mut step.kind {
                            crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                                let next_unit = duration.unit().next();
                                *duration = crate::channel::DurationSpec::from_value_unit(duration.value(), next_unit);
                            }
                            _ => {}
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepDurationValue { seq_idx, step_idx, value } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        match &mut step.kind {
                            crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                                duration.set_value(value);
                            }
                            _ => {}
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepFromCh { seq_idx, step_idx, ch } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        if let crate::mixer::StepKind::Fade { from_ch, .. } = &mut step.kind { *from_ch = ch; }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetStepToCh { seq_idx, step_idx, ch } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        if let crate::mixer::StepKind::Fade { to_ch, .. } = &mut step.kind { *to_ch = ch; }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }
            EngineCommand::SetGoToTarget { seq_idx, step_idx, target } => {
                if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
                    if let Some(step) = seq.steps.get_mut(step_idx) {
                        if let crate::mixer::StepKind::GoTo { step_index } = &mut step.kind { *step_index = target; }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
                }
            }

            // ── Stream Library ─────────────────────────────────
            EngineCommand::AddStreamLibraryEntry { url, mode } => {
                if !self.stream_library.iter().any(|(u, _)| u == &url) {
                    self.stream_library.push((url, mode));
                }
                CommandResult::Ok
            }
            EngineCommand::RemoveStreamLibraryEntry { url } => {
                self.stream_library.retain(|(u, _)| u != &url);
                CommandResult::Ok
            }
            EngineCommand::AddHlsLibraryEntry { url } => {
                if !self.hls_library.contains(&url) {
                    log::info!("Added HLS source to library via API: {}", url);
                    self.hls_library.push(url);
                }
                CommandResult::Ok
            }
            EngineCommand::RemoveHlsLibraryEntry { url } => {
                self.hls_library.retain(|u| u != &url);
                CommandResult::Ok
            }
            EngineCommand::AddDashLibraryEntry { url } => {
                if !self.dash_library.contains(&url) {
                    log::info!("Added DASH source to library via API: {}", url);
                    self.dash_library.push(url);
                }
                CommandResult::Ok
            }
            EngineCommand::RemoveDashLibraryEntry { url } => {
                self.dash_library.retain(|u| u != &url);
                CommandResult::Ok
            }

            // ── Output Management ─────────────────────────────────
            EngineCommand::CreateHeadlessOutput { target } => {
                use crate::renderer::context::{HeadlessOutput, UnifiedOutput, OutputSource};
                let idx = self.outputs.len() + 1;
                let name = format!("Output {}", idx);
                let headless = HeadlessOutput::new(
                    &self.context.device, name.clone(), OutputSource::Master,
                    target, self.render_width, self.render_height,
                );
                log::info!("Created headless output '{}'", name);
                self.outputs.push(UnifiedOutput::Headless(headless));
                CommandResult::Ok
            }
            EngineCommand::StartOutput { idx } => {
                use crate::renderer::context::{UnifiedOutput, OutputTarget};
                if let Some(UnifiedOutput::Headless(h)) = self.outputs.get_mut(idx) {
                    if !h.active {
                        match &h.target {
                            OutputTarget::SrtStream { url, codec } => {
                                match crate::renderer::FfmpegSubprocess::spawn_srt(url, codec, h.width, h.height, 30) {
                                    Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                                    Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                                }
                            }
                            OutputTarget::Recording { path, codec } => {
                                match crate::renderer::FfmpegSubprocess::spawn_recording(path, codec, h.width, h.height, 30) {
                                    Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                                    Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                                }
                            }
                            OutputTarget::HlsStream { name, codec, low_latency } => {
                                match crate::renderer::FfmpegSubprocess::spawn_hls(name, codec, h.width, h.height, 30, *low_latency) {
                                    Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                                    Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                                }
                            }
                            OutputTarget::DashStream { name, codec } => {
                                match crate::renderer::FfmpegSubprocess::spawn_dash(name, codec, h.width, h.height, 30) {
                                    Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                                    Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                                }
                            }
                            OutputTarget::NdiSend { .. } | OutputTarget::SyphonServer { .. } => {
                                h.active = true;
                                h.started_at = Some(std::time::Instant::now());
                            }
                            _ => return CommandResult::Err { code: ErrorCode::InvalidInput, message: "Cannot start windowed target".into() },
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Ok // already active
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found or not headless".into() }
                }
            }
            EngineCommand::StopOutput { idx } => {
                use crate::renderer::context::UnifiedOutput;
                if let Some(UnifiedOutput::Headless(h)) = self.outputs.get_mut(idx) {
                    if h.active {
                        if let Some(mut sub) = h.subprocess.take() { sub.stop(); }
                        h.active = false;
                        h.started_at = None;
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found or not headless".into() }
                }
            }
            EngineCommand::ToggleCalibration { idx } => {
                use crate::renderer::context::UnifiedOutput;
                if let Some(UnifiedOutput::Window(w)) = self.outputs.get_mut(idx) {
                    w.calibration_mode = !w.calibration_mode;
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found or not windowed".into() }
                }
            }
            EngineCommand::SetWarpCorner { output_idx, assignment_idx, corner_idx, position } => {
                use crate::renderer::context::UnifiedOutput;
                if let Some(UnifiedOutput::Window(w)) = self.outputs.get_mut(output_idx) {
                    if let Some(a) = w.surface_assignments.get_mut(assignment_idx) {
                        if corner_idx < 4 { a.warp_corners[corner_idx] = position; }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Assignment not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }
            EngineCommand::ResetWarp { output_idx, assignment_idx } => {
                if let Some(output) = self.outputs.get_mut(output_idx) {
                    let assignments = output.surface_assignments_mut();
                    if let Some(a) = assignments.get_mut(assignment_idx) {
                        if let Some((_, surface)) = self.surface_manager.find_by_uuid(&a.surface_uuid) {
                            let bb = surface.bounding_box();
                            a.warp_corners = [
                                [bb.x, bb.y],
                                [bb.x + bb.width, bb.y],
                                [bb.x + bb.width, bb.y + bb.height],
                                [bb.x, bb.y + bb.height],
                            ];
                        }
                        CommandResult::Ok
                    } else {
                        CommandResult::Err { code: ErrorCode::NotFound, message: "Assignment not found".into() }
                    }
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }

            EngineCommand::SetEdgeBlend { output_idx, config } => {
                use crate::renderer::context::UnifiedOutput;
                if let Some(output) = self.outputs.get_mut(output_idx) {
                    match output {
                        UnifiedOutput::Window(w) => { w.edge_blend = config; }
                        UnifiedOutput::Headless(h) => { h.edge_blend = config; }
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }
            EngineCommand::SetEdgeBlendMode { output_idx, mode } => {
                use crate::renderer::context::UnifiedOutput;
                use crate::renderer::edge_blend::EdgeBlendMode;
                if let Some(output) = self.outputs.get_mut(output_idx) {
                    match output {
                        UnifiedOutput::Window(w) => { w.edge_blend_mode = mode; }
                        UnifiedOutput::Headless(h) => { h.edge_blend_mode = mode; }
                    }
                    if mode == EdgeBlendMode::Auto {
                        self.recompute_auto_edge_blend();
                    }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
                }
            }

            // ── Modulation Updates ────────────────────────────────
            EngineCommand::UpdateLfoFrequency { uuid, frequency } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { frequency: ref mut f, .. } = s { *f = frequency; }
                })
            }
            EngineCommand::UpdateLfoWaveform { uuid, waveform } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { waveform: ref mut w, .. } = s { *w = waveform; }
                })
            }
            EngineCommand::UpdateLfoPhase { uuid, phase } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { phase: ref mut p, .. } = s { *p = phase; }
                })
            }
            EngineCommand::UpdateLfoAmplitude { uuid, amplitude } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { amplitude: ref mut a, .. } = s { *a = amplitude; }
                })
            }
            EngineCommand::UpdateLfoBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { bipolar: ref mut b, .. } = s { *b = bipolar; }
                })
            }
            EngineCommand::UpdateAudioSmoothing { uuid, smoothing } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { smoothing: ref mut sm, .. } = s { *sm = smoothing; }
                })
            }
            EngineCommand::UpdateAudioFreqRange { uuid, freq_low, freq_high } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, freq_high: ref mut fh, .. } = s { *fl = freq_low; *fh = freq_high; }
                })
            }
            EngineCommand::UpdateAudioGain { uuid, gain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { gain: ref mut g, .. } = s { *g = gain; }
                })
            }
            EngineCommand::UpdateAudioPreset { uuid, preset } => {
                let (lo, hi) = preset.freq_range();
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, freq_high: ref mut fh, .. } = s { *fl = lo; *fh = hi; }
                })
            }
            EngineCommand::UpdateAudioMode { uuid, mode } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { mode: ref mut m, .. } = s { *m = mode; }
                })
            }
            EngineCommand::UpdateAdsrAttack { uuid, attack } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { attack: ref mut a, .. } = s { *a = attack; }
                })
            }
            EngineCommand::UpdateAdsrDecay { uuid, decay } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { decay: ref mut d, .. } = s { *d = decay; }
                })
            }
            EngineCommand::UpdateAdsrSustain { uuid, sustain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { sustain: ref mut su, .. } = s { *su = sustain; }
                })
            }
            EngineCommand::UpdateAdsrRelease { uuid, release } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { release: ref mut r, .. } = s { *r = release; }
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
                    if let ModulationSource::StepSequencer { steps: ref mut st, .. } = s { *st = steps; }
                })
            }
            EngineCommand::UpdateStepSeqRate { uuid, rate } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { rate: ref mut r, .. } = s { *r = rate; }
                })
            }
            EngineCommand::UpdateStepSeqInterpolation { uuid, interpolation } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { interpolation: ref mut i, .. } = s { *i = interpolation; }
                })
            }
            EngineCommand::UpdateStepSeqBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { bipolar: ref mut b, .. } = s { *b = bipolar; }
                })
            }
            EngineCommand::SetStepSeqCount { uuid, count } => {
                let count = count.max(2).min(64);
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { steps, .. } = s { steps.resize(count, 0.0); }
                })
            }
            EngineCommand::UpdateStepSeqValue { uuid, step_idx, value } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { steps, .. } = s {
                        if step_idx < steps.len() { steps[step_idx] = value; }
                    }
                })
            }
            EngineCommand::UpdateAudioFreqLow { uuid, freq_low } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, .. } = s { *fl = freq_low; }
                })
            }
            EngineCommand::UpdateAudioFreqHigh { uuid, freq_high } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { freq_high: ref mut fh, .. } = s { *fh = freq_high; }
                })
            }
            EngineCommand::UpdateAudioSource { uuid, source_id } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { source_id: ref mut sid, .. } = s { *sid = source_id; }
                })
            }
            EngineCommand::UpdateAudioNoiseGate { uuid, noise_gate } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { noise_gate: ref mut ng, .. } = s { *ng = noise_gate; }
                })
            }
            EngineCommand::AssignModOnMod { target_source_id, param_name, modulator_id, amount } => {
                self.mixer.modulation_mut().assign_mod_on_mod(&target_source_id, &param_name, &modulator_id, amount);
                CommandResult::Ok
            }
            EngineCommand::RemoveModOnMod { target_source_id, param_name } => {
                self.mixer.modulation_mut().clear_mod_on_mod(&target_source_id, &param_name);
                CommandResult::Ok
            }

            // ── Device Scanning ───────────────────────────────────
            EngineCommand::RescanNdi => {
                self.ndi_manager.discover();
                CommandResult::Ok
            }
            EngineCommand::RescanSyphon => {
                #[cfg(target_os = "macos")]
                self.syphon_manager.discover();
                CommandResult::Ok
            }
            EngineCommand::RescanCameras => {
                self.camera_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::RescanMidi => {
                if let Some(ref mut midi) = self.midi_devices {
                    midi.load_user_profiles(&self.workspace.controller_profiles_dir());
                    if let Err(e) = midi.scan_devices() {
                        return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() };
                    }
                    self.controller_led_mgr.sync_devices(midi);
                    self.auto_map_engine.sync_devices(midi);
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
                        return CommandResult::Err { code: ErrorCode::InternalError, message: format!("Failed to open audio source: {}", e) };
                    }
                } else {
                    self.audio_manager.close_source(source_id);
                }
                CommandResult::Ok
            }
            EngineCommand::SetMidiDeviceEnabled { device_id, enabled } => {
                if let Some(ref mut midi) = self.midi_devices {
                    midi.set_device_enabled(device_id, enabled);
                }
                CommandResult::Ok
            }

            // ── MIDI Mappings ─────────────────────────────────────
            EngineCommand::ClearMidiMappings => {
                self.midi_mappings.clear_all();
                CommandResult::Ok
            }
            EngineCommand::RemoveMidiMapping { key } => {
                self.midi_mappings.remove(&key);
                CommandResult::Ok
            }

            // ── Clock ─────────────────────────────────────────────
            EngineCommand::SetClockPreference { preference } => {
                self.clock_manager.set_preference(preference);
                CommandResult::Ok
            }
            EngineCommand::SetManualBpm { bpm } => {
                self.clock_manager.set_preference(crate::clock::ClockPreference::ForceManual { bpm });
                CommandResult::Ok
            }

            // ── Parameters (index-based) ────────────────────────────
            EngineCommand::SetGeneratorParam { channel_idx, deck_idx, name, value } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        ch.decks[deck_idx].deck.generator_params.set(&name, value);
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetEffectParam { channel_idx, deck_idx, effect_idx, name, value } => {
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
            EngineCommand::SetChannelEffectParam { channel_idx, effect_idx, name, value } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if effect_idx < ch.effects.len() {
                        ch.effects[effect_idx].params.set(&name, value);
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetMasterEffectParam { effect_idx, name, value } => {
                if effect_idx < self.mixer.master_effects().len() {
                    self.mixer.master_effects_mut()[effect_idx].params.set(&name, value);
                }
                CommandResult::Ok
            }
            EngineCommand::ResetGeneratorParamsToDefaults { channel_idx, deck_idx } => {
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
                    &self.mixer, self.render_width, self.render_height,
                );
                if let Some(config) = self.history.undo(current) {
                    let rw = self.render_width;
                    let rh = self.render_height;
                    let (warnings, _) = self.apply_scene_diff(&config, rw, rh);
                    for w in &warnings { log::warn!("Undo warning: {}", w); }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::InvalidInput, message: "Nothing to undo".into() }
                }
            }
            EngineCommand::Redo => {
                let current = crate::persistence::snapshot_scene(
                    &self.mixer, self.render_width, self.render_height,
                );
                if let Some(config) = self.history.redo(current) {
                    let rw = self.render_width;
                    let rh = self.render_height;
                    let (warnings, _) = self.apply_scene_diff(&config, rw, rh);
                    for w in &warnings { log::warn!("Redo warning: {}", w); }
                    CommandResult::Ok
                } else {
                    CommandResult::Err { code: ErrorCode::InvalidInput, message: "Nothing to redo".into() }
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
            CommandResult::Err { code: ErrorCode::NotFound, message: format!("Modulation source {} not found", uuid) }
        }
    }

    /// Build a domain-neutral engine state snapshot for cross-thread consumers.
    pub fn build_engine_state(&self) -> EngineState {
        snapshot::build_engine_state(self)
    }

    /// Publish the latest engine state for cross-thread consumers.
    pub fn publish_state(&self) {
        let state = self.build_engine_state();
        if let Ok(mut guard) = self.state_tx.write() {
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
        snapshot::build_ui_data(self, layout, deck_preview_textures, channel_preview_textures, output_preview_textures, main_output_texture)
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

    /// Read-only access to the outputs.
    pub fn outputs_ref(&self) -> &[crate::renderer::context::UnifiedOutput] {
        &self.outputs
    }

    /// Mutable access to the mixer (for deck insertion from background loads).
    pub fn mixer_mut(&mut self) -> &mut crate::mixer::Mixer {
        &mut self.mixer
    }

    /// Number of loaded shaders.
    pub fn shader_count(&self) -> usize {
        self.registry.count()
    }

    /// Resolve a generator index to a cloned ISFShader.
    /// Returns None if the index is out of bounds.
    pub fn resolve_generator(&self, gen_idx: usize) -> Option<crate::isf::ISFShader> {
        self.registry.generators().get(gen_idx).map(|s| (*s).clone())
    }

    /// Tick notification expiry timers.
    pub fn update_notifications(&mut self) {
        self.notifications.update();
    }

    /// Push an info-level notification.
    pub fn notify_info(&mut self, message: impl Into<String>) {
        self.notifications.info(message);
    }

    /// Close an output window by its winit WindowId. Returns the name if found.
    pub fn close_output_window_by_id(&mut self, window_id: winit::window::WindowId) -> Option<String> {
        if let Some(idx) = self.outputs.iter().position(|o| {
            if let UnifiedOutput::Window(w) = o { w.window.id() == window_id } else { false }
        }) {
            let name = self.outputs[idx].name().to_string();
            if let UnifiedOutput::Window(w) = self.outputs.remove(idx) {
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
        for o in &mut self.outputs {
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
        log::info!("Changing render resolution: {}×{} → {}×{}", self.render_width, self.render_height, width, height);
        self.render_width = width;
        self.render_height = height;
        self.mixer.resize(&self.context, width, height);
        // Clear sub-mix cache since textures were recreated
        self.mixer.clear_sub_mix_cache();
        self.notifications.info(format!("📐 Resolution changed to {}×{}", width, height));
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
        let config = parse_args(&["--osc-port", "7000", "--osc-out", "192.168.1.1:8000", "--osc-out", "10.0.0.1:9000"]);
        assert_eq!(config.osc_port, Some(7000));
        assert_eq!(config.osc_targets, vec!["192.168.1.1:8000", "10.0.0.1:9000"]);
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
    fn app_config_workspace_root_resolution() {
        let config = parse_args(&["--workspace", "/tmp/show"]);
        assert_eq!(config.effective_workspace_root(), std::path::PathBuf::from("/tmp/show"));

        let config = parse_args(&[]);
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(config.effective_workspace_root(), cwd);
    }

    #[test]
    fn app_config_clone() {
        let config = parse_args(&["--headless", "--port", "3030", "--no-ndi"]);
        let cloned = config.clone();
        assert!(cloned.headless);
        assert_eq!(cloned.api_port, 3030);
        assert!(cloned.ndi_disabled);
    }
}
