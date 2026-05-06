//! Application layer — concrete engine implementation.
//!
//! VardaApp owns all engine subsystems (Mixer, Audio, Cameras, MIDI, OSC,
//! ShaderRegistry, SurfaceManager) and implements the engine traits.
//!
//! The main.rs `App` struct owns window/egui state and holds a `VardaApp`.

mod actions;
mod engine_impl;
mod inputs;
mod outputs;
mod render;
mod snapshot;
pub(crate) mod state;
mod surfaces;
mod workspace;

/// Fixed render resolution for all decks and stage output (Full HD 1080p)
pub const RENDER_WIDTH: u32 = 1920;
/// Fixed render resolution for all decks and stage output (Full HD 1080p)
pub const RENDER_HEIGHT: u32 = 1080;

use crate::audio::AudioManager;
use crate::camera::CameraManager;
use crate::midi;
use crate::mixer::Mixer;
use crate::osc::OscReceiver;
use crate::persistence::Workspace;
use crate::registry::ShaderRegistry;
use crate::renderer::context::{OutputWindow, GpuContext};
use crate::surface::SurfaceManager;
use crate::usecases::ui::notifications::NotificationSystem;

use std::sync::mpsc;

use crate::engine::{EngineCommand, EngineState};

/// Core engine application. Owns all subsystems except window/egui.
///
/// Implements all engine traits (MixerCommands, AudioCommands, etc.)
/// for direct same-thread access. Also processes EngineCommands from
/// cross-thread consumers via mpsc channel.
pub struct VardaApp {
    // ── Engine subsystems ──────────────────────────────────────
    pub mixer: Option<Mixer>,
    pub audio_manager: AudioManager,
    pub camera_manager: CameraManager,
    pub registry: ShaderRegistry,
    pub context: Option<GpuContext>,

    // ── Control subsystems ─────────────────────────────────────
    pub osc_receiver: Option<OscReceiver>,
    pub midi_devices: Option<midi::MidiDeviceManager>,
    pub midi_mappings: midi::MidiMappingStore,
    pub controller_led_mgr: midi::ControllerLedManager,

    // ── Output & surfaces ──────────────────────────────────────
    pub output_windows: Vec<OutputWindow>,
    pub surface_manager: SurfaceManager,
    pub calibration_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,

    // ── Notifications ──────────────────────────────────────────
    pub notifications: NotificationSystem,

    // ── Persistence ────────────────────────────────────────────
    pub workspace: Workspace,

    // ── UI state (owned here for trait access, not egui-specific) ──
    pub selected_deck: Option<(usize, usize)>,
    pub selected_channel: Option<usize>,
    pub selected_master: bool,
    pub stage_editor_open: bool,
    pub stage_editor_grid_size: f32,
    pub stage_editor_snap: bool,
    pub library_panel_open: bool,

    // ── Pending actions (deferred to event loop) ───────────────
    pub pending_output_creates: Vec<()>,
    pub cached_monitors: Vec<(String, winit::monitor::MonitorHandle)>,

    // ── Audio textures (GPU resource, owned here) ──────────────
    pub audio_textures: Option<crate::audio::AudioTextures>,

    // ── Message passing (cross-thread consumers) ───────────────
    command_rx: mpsc::Receiver<EngineCommand>,
    command_tx: mpsc::Sender<EngineCommand>,

    // ── State distribution ─────────────────────────────────────
    state_tx: std::sync::Arc<std::sync::RwLock<Option<EngineState>>>,

    // ── Frame timing ───────────────────────────────────────────
    pub last_frame_instant: std::time::Instant,
    pub fps_history: Vec<f32>,
    pub fps_smoothed: f32,
    pub frame_count: u64,
}

impl VardaApp {
    /// Create a new VardaApp with all subsystems initialized.
    pub fn new() -> Self {
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

        let osc_receiver = match OscReceiver::new(9000) {
            Ok(osc) => { log::info!("OSC receiver started on port 9000"); Some(osc) }
            Err(e) => { log::warn!("Failed to start OSC receiver: {}", e); None }
        };

        let workspace = Workspace::from_cwd()
            .unwrap_or_else(|_| Workspace::new(std::path::PathBuf::from(".")));

        let mut controller_led_mgr = midi::ControllerLedManager::new();
        let midi_devices = match midi::MidiDeviceManager::new() {
            Ok(mut mgr) => {
                mgr.load_user_profiles(&workspace.controllers_dir());
                if workspace.controllers_dir().is_dir() {
                    let _ = mgr.scan_devices();
                }
                log::info!("MIDI initialized: {} device(s)", mgr.devices.len());
                controller_led_mgr.sync_devices(&mgr);
                Some(mgr)
            }
            Err(e) => { log::warn!("Failed to initialize MIDI: {}", e); None }
        };

        let (command_tx, command_rx) = mpsc::channel();
        let state_tx = std::sync::Arc::new(std::sync::RwLock::new(None));

        Self {
            mixer: None,
            audio_manager,
            camera_manager: CameraManager::new(),
            registry,
            context: None,
            osc_receiver,
            midi_devices,
            midi_mappings: midi::MidiMappingStore::new(),
            controller_led_mgr,
            output_windows: Vec::new(),
            surface_manager: SurfaceManager::new(),
            calibration_textures: Vec::new(),
            notifications: NotificationSystem::new(),
            workspace,
            selected_deck: None,
            selected_channel: None,
            selected_master: false,
            stage_editor_open: false,
            stage_editor_grid_size: 0.05,
            stage_editor_snap: true,
            library_panel_open: true,
            pending_output_creates: Vec::new(),
            cached_monitors: Vec::new(),
            audio_textures: None,
            command_rx,
            command_tx,
            state_tx,
            last_frame_instant: std::time::Instant::now(),
            fps_history: Vec::with_capacity(60),
            fps_smoothed: 0.0,
            frame_count: 0,
        }
    }

    /// Get a command sender for cross-thread consumers (HTTP API, CLI).
    pub fn command_sender(&self) -> mpsc::Sender<EngineCommand> {
        self.command_tx.clone()
    }

    /// Get a shared reference to the latest engine state (for cross-thread consumers).
    pub fn state_reader(&self) -> std::sync::Arc<std::sync::RwLock<Option<EngineState>>> {
        self.state_tx.clone()
    }

    /// Process all queued cross-thread commands. Called once per frame.
    pub fn process_commands(&mut self) {
        use crate::engine::traits::*;
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                EngineCommand::SetCrossfader(pos) => self.set_crossfader(pos),
                EngineCommand::SnapCrossfader(pos) => self.snap_crossfader(pos),
                EngineCommand::AutoCrossfade { target, duration_secs, easing } => {
                    self.start_auto_crossfade(target, duration_secs, easing);
                }
                EngineCommand::BeatCrossfade { target, beats } => {
                    self.start_beat_crossfade(target, beats);
                }
                EngineCommand::AddDeck { channel_idx, shader_name } => {
                    if let Err(e) = self.add_deck(channel_idx, &shader_name) {
                        log::error!("Command AddDeck failed: {}", e);
                    }
                }
                EngineCommand::RemoveDeck { channel_idx, deck_idx } => {
                    if let Err(e) = self.remove_deck(channel_idx, deck_idx) {
                        log::error!("Command RemoveDeck failed: {}", e);
                    }
                }
                EngineCommand::SetDeckOpacity { channel_idx, deck_idx, opacity } => {
                    self.set_deck_opacity(channel_idx, deck_idx, opacity);
                }
                EngineCommand::SetChannelOpacity { channel_idx, opacity } => {
                    self.set_channel_opacity(channel_idx, opacity);
                }
                EngineCommand::AddChannel => {
                    let _ = self.add_channel();
                }
                EngineCommand::RemoveChannel { channel_idx } => {
                    let _ = self.remove_channel(channel_idx);
                }
                EngineCommand::OpenAudioSource { source_id } => {
                    let _ = self.open_audio_source(source_id);
                }
                EngineCommand::CloseAudioSource { source_id } => {
                    self.close_audio_source(source_id);
                }
                EngineCommand::ScanAudioDevices => {
                    self.scan_audio_devices();
                }
                EngineCommand::CreateOutput => {
                    self.request_create_output();
                }
                // Remaining commands routed through traits
                _ => {
                    log::debug!("Unhandled engine command: {:?}", cmd);
                }
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
        if let Ok(mut guard) = self.state_tx.write() {
            *guard = Some(state);
        }
    }

    /// Collect all data needed by the UI into a read-only snapshot.
    /// `deck_preview_textures` and `main_output_texture` are egui-owned state passed in.
    pub fn collect_ui_data(
        &self,
        deck_preview_textures: &std::collections::HashMap<(usize, usize), egui::TextureId>,
        main_output_texture: Option<egui::TextureId>,
    ) -> crate::usecases::ui::UIData {
        snapshot::build_ui_data(self, deck_preview_textures, main_output_texture)
    }
}
