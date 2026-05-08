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

use crate::audio::AudioManager;
use crate::camera::CameraManager;
use crate::midi;
use crate::mixer::Mixer;
use crate::osc::OscReceiver;
use crate::persistence::Workspace;
use crate::registry::ShaderRegistry;
use crate::renderer::context::{GpuContext, UnifiedOutput};
use crate::surface::SurfaceManager;
use crate::notifications::NotificationSystem;

use std::sync::mpsc;

use crate::engine::{EngineCommand, EngineState};

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

    // ── Control subsystems ─────────────────────────────────────
    osc_receiver: Option<OscReceiver>,
    midi_devices: Option<midi::MidiDeviceManager>,
    midi_mappings: midi::MidiMappingStore,
    controller_led_mgr: midi::ControllerLedManager,
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
    command_rx: mpsc::Receiver<EngineCommand>,
    command_tx: mpsc::Sender<EngineCommand>,

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
    srt_manager: crate::srt::SrtManager,
    /// Configured SRT input sources in the library (url, mode).
    /// These appear in the library panel for drag-and-drop to channels.
    srt_library: Vec<(String, crate::srt::SrtMode)>,
}

impl VardaApp {
    /// Create a new VardaApp with all subsystems initialized.
    ///
    /// Requires a fully initialized `GpuContext` — the engine cannot exist
    /// without a GPU. A default two-channel mixer is always created.
    pub fn new(gpu: GpuContext) -> anyhow::Result<Self> {
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

        // Always create GPU-dependent resources up front
        let audio_textures = crate::audio::AudioTextures::new(&gpu.device);
        let calibration_textures =
            crate::renderer::context::create_calibration_textures(&gpu.device, &gpu.queue, 8);
        let mixer = Mixer::new(&gpu, DEFAULT_RENDER_WIDTH, DEFAULT_RENDER_HEIGHT)?;

        Ok(Self {
            mixer,
            audio_manager,
            camera_manager: CameraManager::new(),
            registry,
            context: gpu,
            osc_receiver,
            midi_devices,
            midi_mappings: midi::MidiMappingStore::new(),
            controller_led_mgr,
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
            ndi_manager: crate::ndi::NdiManager::new(),
            #[cfg(target_os = "macos")]
            syphon_manager: crate::syphon::SyphonManager::new(),
            srt_manager: crate::srt::SrtManager::new(),
            srt_library: Vec::new(),
        })
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
    ///
    /// Exhaustive match — the compiler enforces that every EngineCommand variant
    /// is handled. Adding a new variant requires wiring it here.
    pub fn process_commands(&mut self) {
        use crate::engine::traits::*;
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                // ── Mixer ────────────────────────────────────────
                EngineCommand::SetCrossfader(pos) => self.set_crossfader(pos),
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
                EngineCommand::AddImageDeck { channel_idx, path } => {
                    if let Err(e) = self.add_image_deck(channel_idx, &path) {
                        log::error!("Command AddImageDeck failed: {}", e);
                    }
                }
                EngineCommand::AddVideoDeck { channel_idx, path } => {
                    if let Err(e) = self.add_video_deck(channel_idx, &path) {
                        log::error!("Command AddVideoDeck failed: {}", e);
                    }
                }
                EngineCommand::AddSolidColorDeck { channel_idx, color } => {
                    if let Err(e) = self.add_solid_color_deck(channel_idx, color) {
                        log::error!("Command AddSolidColorDeck failed: {}", e);
                    }
                }
                EngineCommand::AddCameraDeck { channel_idx, camera_id } => {
                    if let Err(e) = self.add_camera_deck(channel_idx, camera_id) {
                        log::error!("Command AddCameraDeck failed: {}", e);
                    }
                }
                EngineCommand::RemoveDeck { channel_idx, deck_idx } => {
                    if let Err(e) = self.remove_deck(channel_idx, deck_idx) {
                        log::error!("Command RemoveDeck failed: {}", e);
                    }
                }
                EngineCommand::MoveDeck { src_ch, src_deck, dst_ch } => {
                    if let Err(e) = self.move_deck(src_ch, src_deck, dst_ch) {
                        log::error!("Command MoveDeck failed: {}", e);
                    }
                }
                EngineCommand::SetDeckOpacity { channel_idx, deck_idx, opacity } => {
                    self.set_deck_opacity(channel_idx, deck_idx, opacity);
                }
                EngineCommand::SetDeckBlendMode { channel_idx, deck_idx, mode } => {
                    self.set_deck_blend_mode(channel_idx, deck_idx, mode);
                }
                EngineCommand::SetDeckSolo { channel_idx, deck_idx, solo } => {
                    self.set_deck_solo(channel_idx, deck_idx, solo);
                }
                EngineCommand::SetDeckMute { channel_idx, deck_idx, mute } => {
                    self.set_deck_mute(channel_idx, deck_idx, mute);
                }
                EngineCommand::SetDeckScalingMode { channel_idx, deck_idx, mode } => {
                    self.set_deck_scaling_mode(channel_idx, deck_idx, mode);
                }
                EngineCommand::SetChannelOpacity { channel_idx, opacity } => {
                    self.set_channel_opacity(channel_idx, opacity);
                }
                EngineCommand::SetChannelBlendMode { channel_idx, mode } => {
                    self.set_channel_blend_mode(channel_idx, mode);
                }
                EngineCommand::AddChannel => {
                    let _ = self.add_channel();
                }
                EngineCommand::RemoveChannel { channel_idx } => {
                    let _ = self.remove_channel(channel_idx);
                }
                EngineCommand::AddEffect { target, shader_name } => {
                    if let Err(e) = self.add_effect(target, &shader_name) {
                        log::error!("Command AddEffect failed: {}", e);
                    }
                }
                EngineCommand::RemoveEffect { target, effect_idx } => {
                    self.remove_effect(target, effect_idx);
                }
                EngineCommand::ToggleEffect { target, effect_idx } => {
                    self.toggle_effect(target, effect_idx);
                }
                EngineCommand::MoveEffect { target, from_idx, to_idx } => {
                    self.move_effect(target, from_idx, to_idx);
                }
                EngineCommand::SetTransition { shader_name } => {
                    if let Err(e) = self.set_transition(shader_name.as_deref()) {
                        log::error!("Command SetTransition failed: {}", e);
                    }
                }
                EngineCommand::SetParam { path, value } => {
                    self.set_param(&path, value);
                }

                // ── Audio ────────────────────────────────────────
                EngineCommand::OpenAudioSource { source_id } => {
                    if let Err(e) = self.open_audio_source(source_id) {
                        log::error!("Command OpenAudioSource failed: {}", e);
                    }
                }
                EngineCommand::CloseAudioSource { source_id } => {
                    self.close_audio_source(source_id);
                }
                EngineCommand::ScanAudioDevices => {
                    self.scan_audio_devices();
                }

                // ── Modulation ───────────────────────────────────
                EngineCommand::AddLfo { waveform, frequency } => {
                    self.add_lfo(waveform, frequency);
                }
                EngineCommand::AddAudioBand { preset, source_id } => {
                    self.add_audio_band(preset, source_id);
                }
                EngineCommand::AddAdsr { attack, decay, sustain, release } => {
                    self.add_adsr(attack, decay, sustain, release);
                }
                EngineCommand::AddStepSequencer { num_steps, rate } => {
                    self.add_step_sequencer(num_steps, rate);
                }
                EngineCommand::RemoveModulationSource { idx } => {
                    self.remove_modulation_source(idx);
                }
                EngineCommand::AssignModulation { target, source_idx, amount } => {
                    self.assign_modulation(&target, source_idx, amount);
                }
                EngineCommand::ClearModulation { target } => {
                    self.clear_modulation(&target);
                }

                // ── Output ───────────────────────────────────────
                EngineCommand::CreateOutput => {
                    self.request_create_output();
                }
                EngineCommand::CloseOutput { idx } => {
                    self.close_output(idx);
                }
                EngineCommand::SetOutputDisplay { idx, monitor_name } => {
                    self.set_output_display(idx, &monitor_name);
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
    /// `layout` is UI-consumer-owned selection/layout state.
    /// `deck_preview_textures` and `main_output_texture` are egui-owned state passed in.
    pub fn collect_ui_data(
        &self,
        layout: &crate::usecases::ui::UILayoutState,
        deck_preview_textures: &std::collections::HashMap<(usize, usize), egui::TextureId>,
        main_output_texture: Option<egui::TextureId>,
    ) -> crate::usecases::ui::UIData {
        snapshot::build_ui_data(self, layout, deck_preview_textures, main_output_texture)
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
