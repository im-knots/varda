//! UIRunner — windowed delivery layer for the Varda engine.
//!
//! Owns the window, egui state, blit pipeline, texture registrations, and WindowSurface.
//! The engine (`VardaApp`) is owned here and driven each frame.
//! For headless operation (HTTP API, CLI), this module is simply not used.

use crate::app::history::HistoryManager;
use crate::app::render::{DeckLoadResult, FileDialogKind, FileDialogResult};
use crate::app::{AppConfig, VardaApp};
use crate::renderer::blit::BlitPipeline;
use crate::renderer::context::{GpuContext, WindowSurface};
use crate::usecases::ui;

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

/// Work item sent to the background detection thread.
struct DetectRequest {
    rgba: Vec<u8>,
    w: u32,
    h: u32,
    params: crate::surface::detect::DetectionParams,
    /// When true, this is a capture (freeze-frame) request — the response
    /// triggers a transition to Preview mode rather than just updating overlays.
    is_capture: bool,
    camera_id: crate::camera::CameraId,
}

/// Result returned from the background detection thread.
struct DetectResponse {
    contours: Vec<crate::surface::detect::DetectedContour>,
    is_capture: bool,
    camera_id: crate::camera::CameraId,
}

/// Spawn a long-lived detection worker thread. It reads requests from `rx`,
/// runs detection (which is wrapped in `catch_unwind` inside `detect_from_rgba`),
/// and sends results back on the returned receiver.
fn spawn_detect_thread(
    rx: std::sync::mpsc::Receiver<DetectRequest>,
) -> std::sync::mpsc::Receiver<DetectResponse> {
    let (tx, result_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("varda-detect".into())
        .spawn(move || {
            let mut consecutive_errors: u32 = 0;
            while let Ok(req) = rx.recv() {
                let contours = match crate::surface::import::detect_from_rgba(
                    &req.rgba,
                    req.w,
                    req.h,
                    &req.params,
                ) {
                    Ok(result) => {
                        consecutive_errors = 0;
                        result.contours
                    }
                    Err(e) => {
                        // Rate-limit error logging: log first, then every 60th
                        if !matches!(e, crate::surface::import::ImportError::NoContours) {
                            consecutive_errors += 1;
                            if consecutive_errors == 1 || consecutive_errors % 60 == 0 {
                                log::warn!("Detection error (count={}): {}", consecutive_errors, e);
                            }
                        }
                        Vec::new()
                    }
                };
                if tx
                    .send(DetectResponse {
                        contours,
                        is_capture: req.is_capture,
                        camera_id: req.camera_id,
                    })
                    .is_err()
                {
                    break; // main thread dropped the receiver — exit
                }
            }
            log::info!("Detection worker thread exiting");
        })
        .expect("Failed to spawn detection thread");
    result_rx
}

pub struct UIRunner {
    // ── Session config (CLI flags + workspace defaults) ──────────────
    config: AppConfig,

    // ── Window / egui state (delivery layer) ────────────────────────
    window: Option<&'static Window>,
    window_surface: Option<WindowSurface>,
    blit_pipeline: Option<BlitPipeline>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    deck_preview_textures: std::collections::HashMap<(usize, usize), egui::TextureId>,
    channel_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    output_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    main_output_texture: Option<egui::TextureId>,
    dome_preview_renderer: Option<crate::renderer::dome_preview::DomePreviewRenderer>,
    dome_preview_texture: Option<egui::TextureId>,
    // Camera detection mode state
    camera_detect_texture: Option<egui::TextureId>,
    camera_detect_camera_id: Option<crate::camera::CameraId>,
    camera_detect_contours: Vec<crate::surface::detect::DetectedContour>,
    // Background detection thread channels
    detect_req_tx: std::sync::mpsc::Sender<DetectRequest>,
    detect_res_rx: std::sync::mpsc::Receiver<DetectResponse>,
    detect_in_flight: bool,
    main_window_id: Option<WindowId>,

    // ── UI-consumer-owned layout/selection state ─────────────────────
    layout: super::UILayoutState,

    // ── File dialog channel (async, non-blocking) ─────────────────────
    file_dialog_tx: std::sync::mpsc::Sender<FileDialogResult>,
    file_dialog_rx: std::sync::mpsc::Receiver<FileDialogResult>,

    // ── Background deck loading channel (async, non-blocking) ────────
    deck_load_tx: std::sync::mpsc::Sender<DeckLoadResult>,
    deck_load_rx: std::sync::mpsc::Receiver<DeckLoadResult>,
    /// Number of deck loads currently in-flight on background threads
    pending_deck_loads: std::sync::Arc<std::sync::atomic::AtomicUsize>,

    // ── Engine (created after GPU init in resumed()) ─────────────────
    varda: Option<VardaApp>,

    // ── Undo/redo history ─────────────────────────────────────────────
    history: HistoryManager,

    // ── Performance: gate publish_state to reduce snapshot overhead ──
    publish_counter: u32,

    // ── HTTP API server (background thread) ──────────────────────────
    api_handle: Option<crate::usecases::api::runner::ApiServerHandle>,

    // ── Adaptive frame pacing (windowed + headless) ────────────────
    /// The ideal start time for the next frame. Advances by `frame_budget`
    /// each frame to maintain a steady cadence. When a frame overshoots its
    /// budget, the anchor snaps forward to `now + budget` to avoid catch-up
    /// bursts.
    cadence_anchor: Option<std::time::Instant>,

    // ── Signal-driven shutdown (SIGINT/SIGTERM) ─────────────────────
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,

    // ── Cached window geometry (avoids XGetGeometry round-trip per frame) ──
    // winit 0.30's Window::inner_size() on X11 issues a synchronous XGetGeometry
    // request every call. egui_winit::State::take_egui_input() calls inner_size()
    // unconditionally, causing a blocking X11 round-trip each frame. We cache
    // the size here, updated from Resized/ScaleFactorChanged events, and bypass
    // take_egui_input() to avoid the stall.
    egui_start_time: std::time::Instant,
    cached_screen_size: winit::dpi::PhysicalSize<u32>,
    cached_scale_factor: f32,
    frame_loop_counter: u32,
}

impl UIRunner {
    pub fn new(config: AppConfig) -> Self {
        let (file_dialog_tx, file_dialog_rx) = std::sync::mpsc::channel();
        let (deck_load_tx, deck_load_rx) = std::sync::mpsc::channel();
        let (detect_req_tx, detect_req_rx) = std::sync::mpsc::channel();
        let detect_res_rx = spawn_detect_thread(detect_req_rx);
        Self {
            config,
            window: None,
            window_surface: None,
            blit_pipeline: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            deck_preview_textures: std::collections::HashMap::new(),
            channel_preview_textures: std::collections::HashMap::new(),
            output_preview_textures: std::collections::HashMap::new(),
            main_output_texture: None,
            dome_preview_renderer: None,
            dome_preview_texture: None,
            camera_detect_texture: None,
            camera_detect_camera_id: None,
            camera_detect_contours: Vec::new(),
            detect_req_tx,
            detect_res_rx,
            detect_in_flight: false,
            main_window_id: None,
            layout: super::UILayoutState::default(),
            file_dialog_tx,
            file_dialog_rx,
            deck_load_tx,
            deck_load_rx,
            pending_deck_loads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            varda: None,
            history: HistoryManager::new(),
            publish_counter: 0,
            api_handle: None,
            cadence_anchor: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            egui_start_time: std::time::Instant::now(),
            cached_screen_size: winit::dpi::PhysicalSize::new(0, 0),
            cached_scale_factor: 1.0,
            frame_loop_counter: 0,
        }
    }

    /// Run the UI event loop. Blocks until the window is closed.
    pub fn run(mut self) -> anyhow::Result<()> {
        // Install Ctrl-C handler for graceful shutdown (especially useful in headless)
        let flag = self.shutdown_flag.clone();
        let _ = ctrlc::set_handler(move || {
            log::info!("Received interrupt signal, shutting down...");
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let event_loop = EventLoop::new()?;
        event_loop
            .run_app(&mut self)
            .map_err(|e| anyhow::anyhow!("Event loop error: {:?}", e))?;
        Ok(())
    }
}

impl ApplicationHandler for UIRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Guard re-entry (resumed can be called multiple times on some platforms)
        if self.varda.is_some() {
            return;
        }

        let gpu = if self.config.headless {
            // Headless: no main window, no egui — GPU without window surface
            log::info!("Headless mode: skipping main window creation");
            match GpuContext::new_headless() {
                Ok(gpu) => gpu,
                Err(e) => {
                    log::error!("Failed to create headless GPU context: {}", e);
                    event_loop.exit();
                    return;
                }
            }
        } else {
            // Windowed: create main UI window + egui
            let window_icon = {
                static ICON_BYTES: &[u8] = include_bytes!("../../../assets/icon.png");
                image::load_from_memory(ICON_BYTES)
                    .ok()
                    .map(|img| {
                        let rgba = img.into_rgba8();
                        let (w, h) = (rgba.width(), rgba.height());
                        winit::window::Icon::from_rgba(rgba.into_raw(), w, h).ok()
                    })
                    .flatten()
            };
            let mut window_attrs = Window::default_attributes()
                .with_title("Varda VJ Software")
                .with_inner_size(winit::dpi::LogicalSize::new(1920, 1080));
            if let Some(icon) = window_icon {
                window_attrs = window_attrs.with_window_icon(Some(icon));
            }

            let window_static: &'static Window = match event_loop.create_window(window_attrs) {
                Ok(w) => {
                    log::info!("Window created");
                    Box::leak(Box::new(w))
                }
                Err(e) => {
                    log::error!("Failed to create window: {}", e);
                    event_loop.exit();
                    return;
                }
            };
            self.main_window_id = Some(window_static.id());
            self.window = Some(window_static);

            let (gpu, win_surface) =
                match pollster::block_on(GpuContext::new_for_window(window_static)) {
                    Ok(pair) => pair,
                    Err(e) => {
                        log::error!("Failed to create render context: {}", e);
                        event_loop.exit();
                        return;
                    }
                };

            self.cached_screen_size = window_static.inner_size();
            self.cached_scale_factor = window_static.scale_factor() as f32;
            self.egui_start_time = std::time::Instant::now(); // reset to window-creation epoch
            self.blit_pipeline =
                BlitPipeline::new(&gpu.device, win_surface.surface_config.format).ok();
            self.egui_state = Some(egui_winit::State::new(
                self.egui_ctx.clone(),
                egui::ViewportId::ROOT,
                window_static,
                Some(window_static.scale_factor() as f32),
                None,
                Some(2 * 1024),
            ));
            self.egui_renderer = Some(egui_wgpu::Renderer::new(
                &gpu.device,
                win_surface.surface_config.format,
                egui_wgpu::RendererOptions::default(),
            ));

            // Set the application icon on egui's viewport (controls dock/taskbar icon)
            {
                static ICON_BYTES: &[u8] = include_bytes!("../../../assets/icon.png");
                if let Ok(img) = image::load_from_memory(ICON_BYTES) {
                    let rgba = img.into_rgba8();
                    let icon_data = egui::IconData {
                        rgba: rgba.as_raw().to_vec(),
                        width: rgba.width(),
                        height: rgba.height(),
                    };
                    self.egui_ctx
                        .send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(
                            icon_data,
                        ))));
                }
            }
            self.window_surface = Some(win_surface);
            gpu
        };

        // Create engine now that GPU is ready
        let mut varda = match VardaApp::new(gpu, &self.config) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Failed to initialize engine: {}", e);
                event_loop.exit();
                return;
            }
        };
        log::info!("Varda initialized: {} shaders", varda.shader_count());

        // Load workspace (may replace default mixer with saved scene)
        if let Some(loaded_layout) = varda.load_workspace() {
            self.layout = loaded_layout;
        }
        self.history.clear();

        // Start HTTP API server on background thread
        if self.api_handle.is_none() {
            self.api_handle = crate::usecases::api::runner::start(
                self.config.api_port,
                varda.command_sender(),
                varda.state_reader(),
            );
        }

        self.varda = Some(varda);

        // Register GPU textures with egui for previews (windowed only)
        if !self.config.headless {
            self.register_preview_textures();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(varda) = self.varda.as_mut() else {
            return;
        };
        if self.main_window_id == Some(window_id) {
            if let (Some(window), Some(egui_state)) = (self.window, &mut self.egui_state) {
                if egui_state.on_window_event(window, &event).consumed {
                    return;
                }
            }
            match event {
                WindowEvent::CloseRequested => {
                    log::info!("Close requested, saving workspace and exiting...");
                    varda.save_workspace(&self.layout);
                    if let Some(api) = self.api_handle.take() {
                        api.shutdown();
                    }
                    event_loop.exit();
                }
                WindowEvent::Resized(new_size) => {
                    self.cached_screen_size = new_size;
                    let device = &varda.gpu_context().device;
                    if let Some(ws) = &mut self.window_surface {
                        ws.resize(device, new_size);
                    }
                }
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    self.cached_scale_factor = scale_factor as f32;
                }
                WindowEvent::RedrawRequested => {
                    self.render(event_loop);
                    // Frame pacing: don't request_redraw() here.
                    // about_to_wait() schedules the next frame via WaitUntil.
                }
                _ => {}
            }
        } else {
            match event {
                WindowEvent::CloseRequested => {
                    if let Some(name) = varda.close_output_window_by_id(window_id) {
                        log::info!("Output window '{}' closed", name);
                    }
                }
                WindowEvent::Resized(new_size) => {
                    varda.resize_output_window_by_id(window_id, new_size);
                }
                _ => {}
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Read target_fps from engine state (runtime-mutable via UI/API).
        let target_fps = self
            .varda
            .as_ref()
            .map(|v| v.target_fps())
            .unwrap_or(self.config.target_fps);

        if self.config.headless {
            // Headless: adaptive sleep-based pacing
            if target_fps > 0 {
                if let Some(deadline) = self.cadence_anchor {
                    let now = std::time::Instant::now();
                    if deadline > now {
                        std::thread::sleep(deadline - now);
                    }
                }
            }
            self.render_headless(event_loop);
            self.advance_cadence_anchor(target_fps);
        } else {
            // Windowed: adaptive cadence pacing.
            // Only request_redraw when the cadence anchor says it's time.
            // Between frames, let WaitUntil handle OS-level sleeping so we
            // don't burn CPU or produce burst-pause patterns.
            if target_fps > 0 {
                let now = std::time::Instant::now();
                let deadline = self.cadence_anchor.unwrap_or(now);

                if deadline > now {
                    // Not time yet — let the OS sleep until the deadline.
                    // Do NOT request_redraw; winit will call about_to_wait
                    // again when the timer fires.
                    event_loop
                        .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
                } else {
                    // At or past deadline — render now.
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(now));
                    if let Some(w) = self.window {
                        w.request_redraw();
                    }
                }
            } else {
                // Uncapped: poll continuously
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                if let Some(w) = self.window {
                    w.request_redraw();
                }
            }
        }
    }
}

impl UIRunner {
    /// Advance the cadence anchor after a frame completes (render or headless).
    ///
    /// The anchor represents the ideal start time for the next frame. Each call
    /// advances it by one frame budget to maintain a steady cadence. If the
    /// frame overshot its budget (anchor is already in the past), the anchor
    /// snaps forward to `now + budget` instead of trying to catch up — this
    /// prevents the burst-pause pattern where multiple short frames fire in
    /// rapid succession after one long frame.
    fn advance_cadence_anchor(&mut self, target_fps: u32) {
        if target_fps == 0 {
            self.cadence_anchor = None;
            return;
        }
        let budget = std::time::Duration::from_secs_f64(1.0 / target_fps as f64);
        let now = std::time::Instant::now();
        self.cadence_anchor = Some(match self.cadence_anchor {
            Some(anchor) => {
                let ideal_next = anchor + budget;
                if ideal_next > now {
                    // Frame finished before deadline — cadence maintained
                    ideal_next
                } else {
                    // Frame overshot — restart cadence from now
                    now + budget
                }
            }
            None => now + budget,
        });
    }

    /// Register GPU textures with egui for deck/channel/output previews and main output.
    fn register_preview_textures(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        let context = varda.gpu_context();
        let mixer = varda.mixer_ref();

        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                let tid = egui_renderer.register_native_texture(
                    &context.device,
                    &slot.deck.texture_view,
                    wgpu::FilterMode::Linear,
                );
                self.deck_preview_textures.insert((ch_idx, deck_idx), tid);
            }
            // Channel composite preview
            let ch_tid = egui_renderer.register_native_texture(
                &context.device,
                &ch.composite_view,
                wgpu::FilterMode::Linear,
            );
            self.channel_preview_textures.insert(ch_idx, ch_tid);
        }
        self.main_output_texture = Some(egui_renderer.register_native_texture(
            &context.device,
            &mixer.composite_view(),
            wgpu::FilterMode::Linear,
        ));
        // Output preview textures — resolve source view for live preview
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            let view = Self::output_preview_view(output, mixer);
            let tid = egui_renderer.register_native_texture(
                &context.device,
                view,
                wgpu::FilterMode::Linear,
            );
            self.output_preview_textures.insert(out_idx, tid);
        }
        // Dome preview renderer + texture
        if self.dome_preview_renderer.is_none() {
            match crate::renderer::dome_preview::DomePreviewRenderer::new(
                &context.device,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ) {
                Ok(renderer) => {
                    let tid = egui_renderer.register_native_texture(
                        &context.device,
                        &renderer.output_view,
                        wgpu::FilterMode::Linear,
                    );
                    self.dome_preview_texture = Some(tid);
                    self.dome_preview_renderer = Some(renderer);
                }
                Err(e) => log::error!("Failed to create dome preview renderer: {}", e),
            }
        }
    }

    /// Resolve the texture view to use for an output preview.
    /// Windowed outputs use their intermediate render texture (shows surface geometry + warp).
    /// Headless outputs resolve their source.
    fn output_preview_view<'a>(
        output: &'a crate::renderer::context::UnifiedOutput,
        mixer: &'a crate::mixer::Mixer,
    ) -> &'a wgpu::TextureView {
        use crate::renderer::context::{OutputSource, UnifiedOutput};
        match output {
            UnifiedOutput::Window(w) => &w.preview_texture_view,
            UnifiedOutput::Headless(h) => match &h.source {
                OutputSource::Master => mixer.composite_view(),
                OutputSource::Channel(idx) => mixer
                    .channels()
                    .get(*idx)
                    .map(|c| &c.composite_view)
                    .unwrap_or_else(|| mixer.composite_view()),
                OutputSource::Deck(ch, dk) => mixer
                    .channels()
                    .get(*ch)
                    .and_then(|c| c.decks.get(*dk))
                    .map(|s| &s.deck.texture_view)
                    .unwrap_or_else(|| mixer.composite_view()),
                OutputSource::Channels(indices) => {
                    let mut sorted = indices.clone();
                    sorted.sort();
                    sorted.dedup();
                    mixer
                        .get_sub_mix_view(&sorted)
                        .unwrap_or_else(|| mixer.composite_view())
                }
                OutputSource::Domemaster => {
                    // Domemaster preview falls back to composite view;
                    // the actual domemaster texture is rendered in the output pipeline.
                    mixer.composite_view()
                }
            },
        }
    }

    /// Re-register GPU textures when deck/channel/output layout changes.
    fn refresh_textures(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        let context = varda.gpu_context();
        let mixer = varda.mixer_ref();

        if self.main_output_texture.is_none() {
            self.main_output_texture = Some(egui_renderer.register_native_texture(
                &context.device,
                &mixer.composite_view(),
                wgpu::FilterMode::Linear,
            ));
        }
        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                let key = (ch_idx, deck_idx);
                if !self.deck_preview_textures.contains_key(&key) {
                    let tid = egui_renderer.register_native_texture(
                        &context.device,
                        &slot.deck.texture_view,
                        wgpu::FilterMode::Linear,
                    );
                    self.deck_preview_textures.insert(key, tid);
                }
            }
            if !self.channel_preview_textures.contains_key(&ch_idx) {
                let tid = egui_renderer.register_native_texture(
                    &context.device,
                    &ch.composite_view,
                    wgpu::FilterMode::Linear,
                );
                self.channel_preview_textures.insert(ch_idx, tid);
            }
        }
        // Register any new output preview textures
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            if !self.output_preview_textures.contains_key(&out_idx) {
                let view = Self::output_preview_view(output, mixer);
                let tid = egui_renderer.register_native_texture(
                    &context.device,
                    view,
                    wgpu::FilterMode::Linear,
                );
                self.output_preview_textures.insert(out_idx, tid);
            }
        }
    }

    /// Headless render loop — engine processing without UI/egui.
    fn render_headless(&mut self, event_loop: &ActiveEventLoop) {
        let Some(varda) = self.varda.as_mut() else {
            return;
        };

        // Check for shutdown request (from API or SIGINT/SIGTERM)
        if varda.shutdown_requested
            || self
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            log::info!("Shutdown requested, saving workspace and exiting...");
            varda.save_workspace(&self.layout);
            if let Some(api) = self.api_handle.take() {
                api.shutdown();
            }
            event_loop.exit();
            return;
        }

        varda.update_frame_timing();
        varda.update_notifications();
        varda.process_commands();
        varda.process_inputs();

        // Create pending output windows (API-driven in headless)
        varda.create_pending_outputs(event_loop);
        varda.refresh_monitors(event_loop);

        // GPU render (mixer compositing)
        varda.render_mixer_frame();

        // Push content rotation to domemaster renderer (headless path)
        let c_az = self
            .layout
            .dome_geometry
            .content_azimuth_degrees
            .to_radians();
        let c_el = self
            .layout
            .dome_geometry
            .content_elevation_degrees
            .to_radians();
        let c_roll = self.layout.dome_geometry.content_roll_degrees.to_radians();
        varda.set_domemaster_content_rotation(c_az, c_el, c_roll);

        // Render output windows + publish state
        varda.render_outputs();
        self.publish_counter += 1;
        if self.publish_counter % 10 == 0 {
            varda.publish_state();
        }
    }

    /// Main render loop — delegates all logic to VardaApp.
    fn render(&mut self, event_loop: &ActiveEventLoop) {
        // 1. Frame timing + notifications + inputs
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            varda.update_frame_timing();
            varda.update_notifications();
            varda.process_commands();
            varda.process_inputs();
        }

        // 2. Sync egui texture registrations
        self.refresh_textures();

        let Some(window) = self.window else { return };

        // 3. Create pending output windows + refresh monitors
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            varda.create_pending_outputs(event_loop);
            varda.refresh_monitors(event_loop);
        }

        // 3b. Render dome preview if open (either dome_preview_open or dome_mode_active)
        if self.layout.dome_preview_open || self.layout.dome_mode_active {
            if let (Some(renderer), Some(varda)) = (&mut self.dome_preview_renderer, &self.varda) {
                let context = varda.gpu_context();

                // Update slice overlays when in dome mode
                if self.layout.dome_mode_active {
                    let setup = self
                        .layout
                        .dome_preset
                        .to_setup_with_geometry(self.layout.dome_geometry);
                    renderer.set_slice_overlays(&context.device, &setup);
                } else {
                    renderer.clear_slice_overlays();
                }

                // Use domemaster output if available, otherwise fall back to mixer composite
                let source_view = varda
                    .domemaster_view()
                    .unwrap_or_else(|| varda.mixer_ref().composite_view());
                let c_az = self
                    .layout
                    .dome_geometry
                    .content_azimuth_degrees
                    .to_radians();
                let c_el = self
                    .layout
                    .dome_geometry
                    .content_elevation_degrees
                    .to_radians();
                let c_roll = self.layout.dome_geometry.content_roll_degrees.to_radians();
                renderer.render(
                    &context.device,
                    &context.queue,
                    source_view,
                    c_az,
                    c_el,
                    c_roll,
                );
            }
        }

        // 3c. Camera detection mode — open/release camera as needed
        {
            let detect_camera_id = match &self.layout.camera_detect_mode {
                ui::CameraDetectMode::Live { camera_id, .. } => Some(*camera_id),
                ui::CameraDetectMode::Preview { camera_id, .. } => Some(*camera_id),
                ui::CameraDetectMode::Off => None,
            };

            if let (Some(cam_id), Some(varda)) = (detect_camera_id, self.varda.as_mut()) {
                if self.camera_detect_camera_id != Some(cam_id) {
                    // Release previous camera if switching
                    if let Some(prev_id) = self.camera_detect_camera_id.take() {
                        varda.camera_manager_mut().release_camera(prev_id);
                        if let (Some(tex_id), Some(egui_renderer)) = (
                            self.camera_detect_texture.take(),
                            self.egui_renderer.as_mut(),
                        ) {
                            egui_renderer.free_texture(&tex_id);
                        }
                    }
                    // Open new camera (uses convenience method to avoid split-borrow)
                    match varda.open_camera(cam_id) {
                        Ok(_res) => {
                            if let Some(tex_view) = varda.camera_manager().texture_view(cam_id) {
                                let context = varda.gpu_context();
                                if let Some(egui_renderer) = self.egui_renderer.as_mut() {
                                    let tid = egui_renderer.register_native_texture(
                                        &context.device,
                                        tex_view,
                                        wgpu::FilterMode::Linear,
                                    );
                                    self.camera_detect_texture = Some(tid);
                                }
                            }
                            self.camera_detect_camera_id = Some(cam_id);
                            log::info!("Camera detection: opened camera {}", cam_id);
                        }
                        Err(e) => {
                            log::error!(
                                "Camera detection: failed to open camera {}: {}",
                                cam_id,
                                e
                            );
                            self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
                        }
                    }
                }
            } else if detect_camera_id.is_none() && self.camera_detect_camera_id.is_some() {
                // Mode is Off — release camera
                if let Some(prev_id) = self.camera_detect_camera_id.take() {
                    if let Some(varda) = self.varda.as_mut() {
                        varda.camera_manager_mut().release_camera(prev_id);
                    }
                    if let (Some(tex_id), Some(egui_renderer)) = (
                        self.camera_detect_texture.take(),
                        self.egui_renderer.as_mut(),
                    ) {
                        egui_renderer.free_texture(&tex_id);
                    }
                }
                self.camera_detect_contours.clear();
            }
        }

        // 4. Collect UI data snapshot (engine → UI, with UI-owned layout state)
        let Some(varda_ref) = self.varda.as_ref() else {
            return;
        };
        let mut ui_data = varda_ref.collect_ui_data(
            &self.layout,
            &self.deck_preview_textures,
            &self.channel_preview_textures,
            &self.output_preview_textures,
            self.main_output_texture,
        );
        ui_data.can_undo = self.history.can_undo();
        ui_data.can_redo = self.history.can_redo();
        ui_data.pending_deck_loads = self
            .pending_deck_loads
            .load(std::sync::atomic::Ordering::Relaxed);
        ui_data.dome_preview_open = self.layout.dome_preview_open;
        ui_data.dome_preview_texture = self.dome_preview_texture;
        ui_data.camera_detect_texture = self.camera_detect_texture;
        ui_data.camera_detect_mode = self.layout.camera_detect_mode.clone();

        // Poll background detection results (non-blocking)
        while let Ok(response) = self.detect_res_rx.try_recv() {
            self.detect_in_flight = false;
            if response.is_capture {
                // Capture complete — transition to Preview mode
                let n = response.contours.len();
                self.camera_detect_contours = response.contours.clone();
                self.layout.camera_detect_mode = ui::CameraDetectMode::Preview {
                    camera_id: response.camera_id,
                    contours: response.contours,
                    selected: vec![true; n],
                };
                // Re-snapshot UIData mode since we just changed it
                ui_data.camera_detect_mode = self.layout.camera_detect_mode.clone();
            } else {
                // Live overlay update
                self.camera_detect_contours = response.contours;
            }
        }

        // Submit new detection work if in Live mode and no work in flight
        if let ui::CameraDetectMode::Live {
            camera_id,
            ref params,
        } = self.layout.camera_detect_mode
        {
            if !self.detect_in_flight {
                if let Some(frame) = varda_ref.camera_manager().snapshot_frame(camera_id) {
                    let _ = self.detect_req_tx.send(DetectRequest {
                        rgba: frame.0,
                        w: frame.1,
                        h: frame.2,
                        params: params.clone(),
                        is_capture: false,
                        camera_id,
                    });
                    self.detect_in_flight = true;
                }
            }
        }

        ui_data.camera_detect_contours = self.camera_detect_contours.clone();

        // 5. Run egui frame
        let t_egui = std::time::Instant::now();
        // Bypass take_egui_input() to avoid an XGetGeometry round-trip every frame.
        // winit 0.30's Window::inner_size() on X11 is a synchronous xcb request;
        // take_egui_input() calls it unconditionally. We replicate what it does
        // using cached values updated from Resized/ScaleFactorChanged events.
        let raw_input = {
            let Some(egui_state) = &mut self.egui_state else {
                return;
            };
            let display_scale = self.cached_scale_factor;
            let pixels_per_point = self.egui_ctx.zoom_factor() * display_scale;
            let w = self.cached_screen_size.width as f32 / pixels_per_point;
            let h = self.cached_screen_size.height as f32 / pixels_per_point;
            let input = egui_state.egui_input_mut();
            input.time = Some(self.egui_start_time.elapsed().as_secs_f64());
            if w > 0.0 && h > 0.0 {
                input.screen_rect = Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w, h),
                ));
            }
            input.viewport_id = egui::ViewportId::ROOT;
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(display_scale);
            input.take()
        };
        let mut ui_actions = ui::UIActions::new();
        let full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            ui_actions = ui::panels::render_ui(ui, &ui_data);
        });
        {
            let Some(egui_state) = &mut self.egui_state else {
                return;
            };
            egui_state.handle_platform_output(window, full_output.platform_output);
        }

        // 6. Apply all UI actions
        // 6a. UI-consumer-owned selection/layout state
        self.layout.apply_selections(&ui_actions);

        // 6a2. Dome camera actions — apply to renderer (not layout state)
        {
            let dome_resized = false;
            for action in &ui_actions.dome_actions {
                match action {
                    ui::DomeAction::RotateCamera { delta_x, delta_y } => {
                        if let Some(renderer) = &mut self.dome_preview_renderer {
                            renderer.camera.rotate(*delta_x, *delta_y);
                        }
                    }
                    ui::DomeAction::ZoomCamera { delta } => {
                        if let Some(renderer) = &mut self.dome_preview_renderer {
                            renderer.camera.zoom(*delta);
                        }
                    }
                    ui::DomeAction::ResetCamera => {
                        if let Some(renderer) = &mut self.dome_preview_renderer {
                            renderer.camera.reset();
                        }
                    }
                    _ => {} // Config actions handled by layout.apply_selections
                }
            }
            // Handle dome resize if needed
            if let (Some(renderer), Some(varda)) = (&mut self.dome_preview_renderer, &self.varda) {
                let context = varda.gpu_context();
                if let Some(egui_renderer) = &mut self.egui_renderer {
                    // Check if dome_preview_texture needs re-registration after resize
                    if dome_resized {
                        let _ = dome_resized; // suppress unused warning
                    }
                    let _ = (context, egui_renderer, renderer); // used below if resize
                }
            }
        }

        // 6a3. Camera detection actions
        {
            let actions: Vec<_> = ui_actions.camera_detect_actions.drain(..).collect();
            for action in actions {
                match action {
                    ui::CameraDetectAction::Enter { camera_id } => {
                        self.layout.camera_detect_mode = ui::CameraDetectMode::Live {
                            camera_id,
                            params: crate::surface::detect::DetectionParams::default(),
                        };
                    }
                    ui::CameraDetectAction::Exit => {
                        self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
                        // Camera release handled by lifecycle block on next frame
                    }
                    ui::CameraDetectAction::UpdateParams(params) => {
                        if let ui::CameraDetectMode::Live {
                            params: ref mut p, ..
                        } = self.layout.camera_detect_mode
                        {
                            *p = params.clone();
                            // Detection runs every frame in the lifecycle block — no need to run here
                        }
                    }
                    ui::CameraDetectAction::Capture => {
                        // Send a capture request to the background thread — the
                        // response (polled above) will transition to Preview mode.
                        if let ui::CameraDetectMode::Live {
                            camera_id,
                            ref params,
                        } = self.layout.camera_detect_mode
                        {
                            if let Some(varda) = &self.varda {
                                if let Some(frame) =
                                    varda.camera_manager().snapshot_frame(camera_id)
                                {
                                    let _ = self.detect_req_tx.send(DetectRequest {
                                        rgba: frame.0,
                                        w: frame.1,
                                        h: frame.2,
                                        params: params.clone(),
                                        is_capture: true,
                                        camera_id,
                                    });
                                    self.detect_in_flight = true;
                                }
                            }
                        }
                    }
                    ui::CameraDetectAction::ToggleContour(idx) => {
                        if let ui::CameraDetectMode::Preview {
                            ref mut selected, ..
                        } = self.layout.camera_detect_mode
                        {
                            if let Some(s) = selected.get_mut(idx) {
                                *s = !*s;
                            }
                        }
                    }
                    ui::CameraDetectAction::SelectAll(val) => {
                        if let ui::CameraDetectMode::Preview {
                            ref mut selected, ..
                        } = self.layout.camera_detect_mode
                        {
                            selected.iter_mut().for_each(|s| *s = val);
                        }
                    }
                    ui::CameraDetectAction::Accept => {
                        if let ui::CameraDetectMode::Preview {
                            ref contours,
                            ref selected,
                            ..
                        } = self.layout.camera_detect_mode
                        {
                            let chosen: Vec<_> = contours
                                .iter()
                                .zip(selected.iter())
                                .filter(|(_, &s)| s)
                                .map(|(c, _)| c.clone())
                                .collect();
                            if !chosen.is_empty() {
                                ui_actions.surface_actions.push(
                                    ui::SurfaceAction::ConfirmDetectedContours { contours: chosen },
                                );
                            }
                        }
                        self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
                    }
                }
            }
        }

        // 6b. Engine actions (delegated to VardaApp)
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            let Some(egui_renderer) = self.egui_renderer.as_mut() else {
                return;
            };

            // ── Undo/redo: snapshot before undoable mutations ──
            if ui_actions.has_undoable_action() {
                let snapshot = crate::persistence::snapshot_scene(
                    varda.mixer_ref(),
                    varda.render_width(),
                    varda.render_height(),
                );
                self.history.push(snapshot);
            }

            // Intercept shader_to_add: resolve and route to background loading
            if let Some((ch_idx, gen_idx)) = ui_actions.shader_to_add.take() {
                if let Some(shader) = varda.resolve_generator(gen_idx) {
                    let context = varda.gpu_context();
                    VardaApp::spawn_deck_loads(
                        &self.deck_load_tx,
                        context,
                        &self.pending_deck_loads,
                        varda.render_width(),
                        varda.render_height(),
                        Vec::new(),
                        Vec::new(),
                        vec![(ch_idx, shader)],
                    );
                }
            }

            let removed_ch = varda.apply_engine_actions(
                &mut ui_actions,
                egui_renderer,
                &mut self.deck_preview_textures,
            );

            // ── Drain MIDI-triggered global actions ──
            if std::mem::take(&mut varda.midi_pending_undo) {
                ui_actions.undo_requested = true;
            }
            if std::mem::take(&mut varda.midi_pending_redo) {
                ui_actions.redo_requested = true;
            }
            if std::mem::take(&mut varda.midi_pending_save) {
                ui_actions.save_requested = true;
            }

            // ── Undo/redo: diff-apply from history ──
            if ui_actions.undo_requested || ui_actions.redo_requested {
                let current = crate::persistence::snapshot_scene(
                    varda.mixer_ref(),
                    varda.render_width(),
                    varda.render_height(),
                );
                let target = if ui_actions.undo_requested {
                    self.history.undo(current)
                } else {
                    self.history.redo(current)
                };
                if let Some(config) = target {
                    let rw = varda.render_width();
                    let rh = varda.render_height();
                    let (warnings, structural_changed) = varda.apply_scene_diff(&config, rw, rh);
                    for w in &warnings {
                        log::warn!("Undo/redo restore warning: {}", w);
                    }
                    let label = if ui_actions.undo_requested {
                        "↩ Undo"
                    } else {
                        "↪ Redo"
                    };
                    varda.notify_info(label);

                    if structural_changed {
                        // Structural change: re-register all deck + channel preview textures
                        self.deck_preview_textures.clear();
                        self.channel_preview_textures.clear();
                        let context = varda.gpu_context();
                        let mixer = varda.mixer_ref();
                        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
                            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                                let tex_id = egui_renderer.register_native_texture(
                                    &context.device,
                                    &slot.deck.texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.deck_preview_textures
                                    .insert((ch_idx, deck_idx), tex_id);
                            }
                            let ch_tid = egui_renderer.register_native_texture(
                                &context.device,
                                &ch.composite_view,
                                wgpu::FilterMode::Linear,
                            );
                            self.channel_preview_textures.insert(ch_idx, ch_tid);
                        }
                        if let Some(main_id) = self.main_output_texture {
                            egui_renderer.update_egui_texture_from_wgpu_texture(
                                &context.device,
                                &varda.mixer_ref().composite_view(),
                                wgpu::FilterMode::Linear,
                                main_id,
                            );
                        }
                        // Re-register output preview textures
                        self.output_preview_textures.clear();
                        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
                            let view = Self::output_preview_view(output, mixer);
                            let tid = egui_renderer.register_native_texture(
                                &context.device,
                                view,
                                wgpu::FilterMode::Linear,
                            );
                            self.output_preview_textures.insert(out_idx, tid);
                        }
                    }
                }
            }

            varda.apply_ui_actions(&ui_actions);
            varda.apply_output_actions(&ui_actions);
            varda.apply_surface_actions(&ui_actions, self.layout.stage_editor_grid_size);
            varda.apply_device_actions(&ui_actions);
            // Recording/SRT now handled per-output in apply_output_actions()
            varda.apply_clock_actions(&ui_actions);
            let resolution_changed = varda.apply_resolution_change(&ui_actions);
            varda.apply_target_fps_change(&ui_actions);
            varda.update_controller_leds();

            // After resolution change, all GPU textures were recreated —
            // re-register them with egui so previews point to the new views.
            if resolution_changed {
                let context = varda.gpu_context();
                let mixer = varda.mixer_ref();
                for (ch_idx, ch) in mixer.channels().iter().enumerate() {
                    for (deck_idx, slot) in ch.decks.iter().enumerate() {
                        let key = (ch_idx, deck_idx);
                        if let Some(&tex_id) = self.deck_preview_textures.get(&key) {
                            egui_renderer.update_egui_texture_from_wgpu_texture(
                                &context.device,
                                &slot.deck.texture_view,
                                wgpu::FilterMode::Linear,
                                tex_id,
                            );
                        }
                    }
                    if let Some(&ch_tid) = self.channel_preview_textures.get(&ch_idx) {
                        egui_renderer.update_egui_texture_from_wgpu_texture(
                            &context.device,
                            &ch.composite_view,
                            wgpu::FilterMode::Linear,
                            ch_tid,
                        );
                    }
                }
                if let Some(main_id) = self.main_output_texture {
                    egui_renderer.update_egui_texture_from_wgpu_texture(
                        &context.device,
                        &mixer.composite_view(),
                        wgpu::FilterMode::Linear,
                        main_id,
                    );
                }
                // Update output preview textures after resolution change
                for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
                    if let Some(&tid) = self.output_preview_textures.get(&out_idx) {
                        let view = Self::output_preview_view(output, mixer);
                        egui_renderer.update_egui_texture_from_wgpu_texture(
                            &context.device,
                            view,
                            wgpu::FilterMode::Linear,
                            tid,
                        );
                    }
                }
            }

            // Fix up selection state after channel removal
            if let Some(ch_idx) = removed_ch {
                self.layout.fixup_channel_removal(ch_idx);
            }

            if ui_actions.save_requested {
                varda.save_workspace(&self.layout);
                varda.notify_info("💾 Workspace saved");
            }

            // Spawn file dialogs on background threads (non-blocking)
            if let Some(ch_idx) = ui_actions.open_image_dialog_for_channel.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, FileDialogKind::Image, ch_idx);
            }
            if let Some(ch_idx) = ui_actions.open_video_dialog_for_channel.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, FileDialogKind::Video, ch_idx);
            }

            // Poll completed file dialog results → spawn background deck loads
            while let Ok(result) = self.file_dialog_rx.try_recv() {
                let mut images = Vec::new();
                let mut videos = Vec::new();
                for path in result.paths {
                    match result.kind {
                        FileDialogKind::Image => images.push((result.ch_idx, path)),
                        FileDialogKind::Video => videos.push((result.ch_idx, path)),
                    }
                }
                if !images.is_empty() || !videos.is_empty() {
                    let context = varda.gpu_context();
                    VardaApp::spawn_deck_loads(
                        &self.deck_load_tx,
                        context,
                        &self.pending_deck_loads,
                        varda.render_width(),
                        varda.render_height(),
                        images,
                        videos,
                        Vec::new(),
                    );
                }
            }

            // Poll completed background deck loads (non-blocking)
            while let Ok(result) = self.deck_load_rx.try_recv() {
                match result.deck {
                    Ok(deck) => {
                        let ch_idx = result.ch_idx;
                        if let Some(ch) = varda.mixer_mut().channel_mut(ch_idx) {
                            let idx = ch.add_deck(deck);
                            log::info!(
                                "Background load complete: deck {} to channel {}: {}",
                                idx,
                                ch_idx,
                                result.name
                            );
                        }
                        // Re-borrow for texture registration (separate from mixer borrow)
                        if let Some(ch) = varda.mixer_ref().channels().get(ch_idx) {
                            let idx = ch.decks.len() - 1;
                            let texture_id = egui_renderer.register_native_texture(
                                &varda.gpu_context().device,
                                &ch.decks[idx].deck.texture_view,
                                wgpu::FilterMode::Linear,
                            );
                            self.deck_preview_textures.insert((ch_idx, idx), texture_id);
                        }
                    }
                    Err(e) => {
                        log::error!("Background deck load failed for '{}': {}", result.name, e)
                    }
                }
            }
        }

        let egui_us = t_egui.elapsed().as_micros();

        // 7. GPU sync: drain the previous frame's GPU work BEFORE submitting new work.
        // This prevents GPU queue buildup that causes get_current_texture()/present()
        // to block for multiple frames worth of GPU time.
        let t_poll = std::time::Instant::now();
        {
            let Some(varda) = self.varda.as_ref() else {
                return;
            };
            let _ = varda.gpu_context().device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_millis(100)),
            });
        }
        let poll_us = t_poll.elapsed().as_micros();

        // 8. GPU: render mixer compositing (offscreen — no surface involved)
        let t_mixer = std::time::Instant::now();
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            varda.render_mixer_frame();
        }
        let mixer_us = t_mixer.elapsed().as_micros();

        // 9. Output windows FIRST — projectors/displays are latency-critical.
        // Present outputs before the UI so they aren't gated behind the UI
        // surface's get_current_texture()/present() cycle.
        let t_outputs = std::time::Instant::now();
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            // Push content rotation to domemaster renderer each frame (real-time, MIDI-mappable)
            let c_az = self
                .layout
                .dome_geometry
                .content_azimuth_degrees
                .to_radians();
            let c_el = self
                .layout
                .dome_geometry
                .content_elevation_degrees
                .to_radians();
            let c_roll = self.layout.dome_geometry.content_roll_degrees.to_radians();
            varda.set_domemaster_content_rotation(c_az, c_el, c_roll);
            varda.render_outputs();
            self.publish_counter += 1;
            if self.publish_counter % 10 == 0 {
                varda.publish_state();
            }
        }
        let outputs_us = t_outputs.elapsed().as_micros();

        // 10. UI surface last — operator control surface, latency-tolerant.
        // The UI blit + egui overlay + present can safely happen after outputs.
        let t_submit = std::time::Instant::now();
        self.submit_frame(
            window,
            full_output.shapes,
            full_output.pixels_per_point,
            full_output.textures_delta,
        );
        let submit_us = t_submit.elapsed().as_micros();

        // Advance the cadence anchor for adaptive frame pacing.
        let target_fps = self
            .varda
            .as_ref()
            .map(|v| v.target_fps())
            .unwrap_or(self.config.target_fps);
        self.advance_cadence_anchor(target_fps);

        // Frame loop timing (log every 120 frames)
        self.frame_loop_counter += 1;
        if self.frame_loop_counter % 120 == 0 {
            let total_us = mixer_us + submit_us + outputs_us + poll_us;
            log::debug!(
                "[PERF] frame_loop | egui={}us mixer={}us outputs={}us submit_ui={}us poll={}us | total={}us ({:.1}ms)",
                egui_us,
                mixer_us,
                outputs_us,
                submit_us,
                poll_us,
                total_us,
                total_us as f64 / 1000.0,
            );
        }
    }

    /// Blit mixer output to screen, overlay egui, and present.
    fn submit_frame(
        &mut self,
        window: &Window,
        shapes: Vec<egui::epaint::ClippedShape>,
        pixels_per_point: f32,
        textures_delta: egui::TexturesDelta,
    ) {
        let Some(varda) = &self.varda else { return };
        let context = varda.gpu_context();
        let Some(win_surface) = &self.window_surface else {
            return;
        };

        let paint_jobs = self.egui_ctx.tessellate(shapes, pixels_per_point);

        // Always apply texture updates so the egui renderer stays in sync,
        // even when the surface is unavailable (e.g. Occluded at startup).
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        for (id, delta) in &textures_delta.set {
            egui_renderer.update_texture(&context.device, &context.queue, *id, delta);
        }

        let _ = context.device.poll(wgpu::PollType::Poll);
        let output = match win_surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(o) => o,
            wgpu::CurrentSurfaceTexture::Suboptimal(o) => {
                log::warn!("UI surface suboptimal, will reconfigure");
                o
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                log::warn!("UI surface outdated, reconfiguring");
                win_surface
                    .surface
                    .configure(&context.device, &win_surface.surface_config);
                match win_surface.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(o)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(o) => o,
                    other => {
                        log::error!(
                            "Failed to get surface texture after reconfigure: {:?}",
                            other
                        );
                        return;
                    }
                }
            }
            other => {
                log::debug!("UI surface unavailable: {:?}", other);
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Screen Encoder"),
            });

        let bind_group = if let Some(blit) = &self.blit_pipeline {
            let mixer = varda.mixer_ref();
            Some(blit.create_bind_group(&context.device, &mixer.composite_view()))
        } else {
            None
        };

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [win_surface.size.width, win_surface.size.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        egui_renderer.update_buffers(
            &context.device,
            &context.queue,
            &mut encoder,
            &paint_jobs,
            &screen_desc,
        );

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let (Some(bg), Some(blit)) = (&bind_group, &self.blit_pipeline) {
                blit.render(&mut rp, bg);
            }
            let mut rp_static = rp.forget_lifetime();
            egui_renderer.render(&mut rp_static, &paint_jobs, &screen_desc);
        }

        for id in &textures_delta.free {
            egui_renderer.free_texture(id);
        }

        context.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}
