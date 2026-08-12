//! `UIRunner` — windowed delivery layer for the Varda engine.
//!
//! Owns the window, egui state, blit pipeline, texture registrations, and `WindowSurface`.
//! The engine (`VardaApp`) is owned here and driven each frame.
//! For headless operation (HTTP API, CLI), this module is simply not used.

use crate::app::render::{DeckLoadResult, FileDialogKind, FileDialogResult};
use crate::app::{AppConfig, VardaApp};
use crate::renderer::blit::BlitPipeline;
use crate::renderer::context::{GpuContext, WindowSurface};
use crate::usecases::ui;

use winit::{
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

mod camera_detect;
mod deck_load;
mod detect;
mod event_loop;
mod preview;

pub(crate) use event_loop::WindowHost;

use preview::PreviewEncoder;

use deck_load::{apply_deck_texture_outcomes, register_deck_preview_texture, DeckLoadTargets};
use detect::{spawn_detect_thread, DetectRequest, DetectResponse};

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
    preview_encoder: Option<PreviewEncoder>,
    deck_preview_textures: std::collections::HashMap<String, egui::TextureId>,
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
    deck_load_targets: DeckLoadTargets,

    // ── Engine (created after GPU init in resumed()) ─────────────────
    varda: Option<VardaApp>,

    // ── Deferred GPU init (avoids Metal dispatch-queue deadlock on Rosetta/Intel) ──
    gpu_init_handle: Option<std::thread::JoinHandle<anyhow::Result<(GpuContext, WindowSurface)>>>,
    startup_t0: Option<std::time::Instant>,

    // ── Undo/redo ─────────────────────────────────────────────────────
    // The undo/redo timeline itself lives on `VardaApp` (shared with the
    // HTTP/headless command bus). The runner only tracks gesture edges so a
    // continuous stage/warp drag collapses into one undo step.
    /// Previous frame's `gesture_active` flag, for detecting drag start vs.
    /// continuation so a continuous stage/warp drag collapses into one undo step.
    prev_gesture_active: bool,

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
            preview_encoder: None,
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
            deck_load_targets: DeckLoadTargets::default(),
            varda: None,
            gpu_init_handle: None,
            startup_t0: None,
            prev_gesture_active: false,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the winit event loop cannot be created (no display
    /// server, or one already exists on this thread), or if the event loop
    /// itself terminates with an error.
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
            .map_err(|e| anyhow::anyhow!("Event loop error: {e:?}"))?;
        Ok(())
    }
}

/// Whether this frame's mutations deserve their own undo entry.
///
/// A held drag reports a mutation on every frame it moves, so only the frame
/// that *starts* the gesture takes a snapshot. Without this a single region drag
/// would fill the fifty-deep history by itself and undo would rewind the show a
/// pixel at a time.
fn wants_history_snapshot(
    prev_gesture_active: &mut bool,
    dirty: bool,
    gesture_active: bool,
) -> bool {
    let continuation = gesture_active && *prev_gesture_active;
    *prev_gesture_active = gesture_active;
    dirty && !continuation
}

impl UIRunner {
    /// Complete initialization once GPU context is available.
    /// Called from `resumed()` for headless or from `about_to_wait()` for windowed.
    fn finish_init(
        &mut self,
        gpu: GpuContext,
        win_surface: Option<WindowSurface>,
        startup_t0: std::time::Instant,
        event_loop: &ActiveEventLoop,
    ) {
        // Set up egui + blit pipeline for windowed mode
        if let (Some(window_static), Some(ws)) = (self.window, &win_surface) {
            self.cached_screen_size = window_static.inner_size();
            self.cached_scale_factor = window_static.scale_factor() as f32;
            self.egui_start_time = std::time::Instant::now();
            self.blit_pipeline = BlitPipeline::new(&gpu.device, ws.surface_config.format).ok();
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
                ws.surface_config.format,
                egui_wgpu::RendererOptions::default(),
            ));

            // Set the application icon on egui's viewport (controls dock/taskbar icon)
            {
                static ICON_BYTES: &[u8] = include_bytes!("../../../../assets/icon.png");
                if let Ok(img) = image::load_from_memory(ICON_BYTES) {
                    let rgba = img.into_rgba8();
                    let icon_data = egui::IconData {
                        rgba: rgba.as_raw().clone(),
                        width: rgba.width(),
                        height: rgba.height(),
                    };
                    self.egui_ctx
                        .send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(
                            icon_data,
                        ))));
                }
            }
        }
        if let Some(ws) = win_surface {
            self.window_surface = Some(ws);
        }

        // Create engine now that GPU is ready
        log::info!("[STARTUP] Creating engine (audio, MIDI, shaders, mixer)...");
        let mut varda = match VardaApp::new(gpu, &self.config) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Failed to initialize engine: {e}");
                event_loop.exit();
                return;
            }
        };
        log::info!(
            "[STARTUP] Engine ready: {} shaders ({:.0?})",
            varda.shader_count(),
            startup_t0.elapsed()
        );

        // Load workspace (may replace default mixer with saved scene)
        log::info!("[STARTUP] Loading workspace...");
        if let Some(loaded_layout) = varda.load_workspace() {
            self.layout = loaded_layout;
        }
        // `load_workspace` clears the engine-owned undo/redo timeline.
        log::info!("[STARTUP] Workspace loaded ({:.0?})", startup_t0.elapsed());

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
        log::info!(
            "[STARTUP] Initialization complete ({:.0?})",
            startup_t0.elapsed()
        );
    }

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
        let budget = std::time::Duration::from_secs_f64(1.0 / f64::from(target_fps));
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

    /// Headless render loop — engine processing without UI/egui.
    fn render_headless(&mut self, host: &dyn WindowHost) {
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
            host.exit();
            return;
        }

        varda.update_frame_timing();
        varda.update_notifications();
        varda.process_commands();
        varda.process_inputs();

        // Create pending output windows (API-driven in headless)
        host.create_pending_outputs(varda);
        host.refresh_monitors(varda);
        #[cfg(feature = "html")]
        host.create_pending_interactive(varda);

        // Publish the cued channel(s) so off-air previews render live (issue #72).
        varda.set_preview_channels(self.layout.preview_channels());

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
        #[cfg(feature = "html")]
        varda.render_interactive();
        self.publish_counter += 1;
        if self.publish_counter.is_multiple_of(10) {
            varda.publish_state();
        }
    }

    /// Main render loop — delegates all logic to `VardaApp`.
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
            #[cfg(feature = "html")]
            varda.create_pending_interactive(event_loop);
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
                renderer.render(context, source_view, c_az, c_el, c_roll);
            }
        }

        self.sync_camera_detect_capture();

        // 4. Collect UI data snapshot (engine → UI, with UI-owned layout state)
        let Some(varda_ref) = self.varda.as_ref() else {
            return;
        };
        let mut ui_data = crate::usecases::ui::build_ui_data(
            varda_ref,
            &self.layout,
            &self.deck_preview_textures,
            &self.channel_preview_textures,
            &self.output_preview_textures,
            self.main_output_texture,
        );
        ui_data.can_undo = varda_ref.history_can_undo();
        ui_data.can_redo = varda_ref.history_can_redo();
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

        ui_data
            .camera_detect_contours
            .clone_from(&self.camera_detect_contours);

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
            for action in &ui_actions.session.dome_actions {
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

        self.apply_camera_detect_actions(&mut ui_actions);

        // 6b. Engine actions (delegated to VardaApp)
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            let Some(egui_renderer) = self.egui_renderer.as_mut() else {
                return;
            };

            // ── Undo/redo: snapshot before undoable mutations ──
            // Unified scene+stage timeline with general gesture coalescing
            // (ui-engine-boundary.md WS3). A snapshot is pushed when the frame
            // carries any undoable mutation AND it is not the *continuation* of a
            // held drag — so a continuous gesture of any kind (warp drag, param
            // slider) collapses into a single undo step (snapshot on the first
            // frame). Undoability is decided by the single, compiler-checked
            // `command_is_undoable` predicate (via `batch_has_undoable`) for
            // migrated domains, plus the residual `has_undoable_*` gates for
            // fields not yet migrated to `commands`.
            let dirty = ui_actions.has_undoable_action()
                || ui_actions.has_undoable_stage_action()
                || varda.batch_has_undoable(&ui_actions.commands);
            // A recording pass is one long gesture: it pushed its own entry
            // when the first parameter was touched, and every write until it
            // ends belongs to that entry.
            if wants_history_snapshot(
                &mut self.prev_gesture_active,
                dirty,
                ui_actions.session.gesture_active || varda.is_recording(),
            ) {
                let snapshot = varda.history_snapshot(&self.layout);
                varda.push_history(snapshot);
            }

            // Intercept shader_to_add: resolve and route to background loading
            if let Some((channel_uuid, gen_idx)) = ui_actions.session.shader_to_add.take() {
                if let Some(shader) = varda.resolve_generator(gen_idx) {
                    let token = self.deck_load_targets.record(channel_uuid);
                    let context = varda.gpu_context();
                    VardaApp::spawn_deck_loads(
                        &self.deck_load_tx,
                        context,
                        &self.pending_deck_loads,
                        varda.render_width(),
                        varda.render_height(),
                        Vec::new(),
                        Vec::new(),
                        vec![(token, shader)],
                    );
                }
            }

            let engine_outcome = varda.apply_engine_actions(&mut ui_actions);
            apply_deck_texture_outcomes(
                &engine_outcome.texture_outcomes,
                egui_renderer,
                varda.gpu_context(),
                varda.mixer_ref(),
                &mut self.deck_preview_textures,
            );

            // ── Drain MIDI-triggered global actions ──
            if std::mem::take(&mut varda.midi_pending_undo) {
                ui_actions.session.undo_requested = true;
            }
            if std::mem::take(&mut varda.midi_pending_redo) {
                ui_actions.session.redo_requested = true;
            }
            if std::mem::take(&mut varda.midi_pending_save) {
                ui_actions.session.save_requested = true;
            }

            // ── Undo/redo: restore via the engine's shared timeline ──
            // The restore (scene + stage diff-apply) lives on `VardaApp` so the
            // windowed and headless/API consumers behave identically; the
            // runner only layers on UI-specific refresh (dome layout flags +
            // GPU preview texture re-registration).
            if ui_actions.session.undo_requested || ui_actions.session.redo_requested {
                let undo = ui_actions.session.undo_requested;
                let outcome = varda.history_gui(&self.layout, undo);
                if let crate::engine::CommandOutcome::HistoryRestored {
                    structural_changed,
                    dome_layout,
                } = outcome
                {
                    // Dome layout flags live in UI layout, not engine state.
                    self.layout.dome_mode_active = dome_layout.dome_mode_active;
                    self.layout.dome_preset = dome_layout.dome_preset;
                    self.layout.dome_geometry = dome_layout.dome_geometry;

                    let label = if undo { "↩ Undo" } else { "↪ Redo" };
                    varda.notify_info(label);

                    if structural_changed {
                        // Structural change: re-register all deck + channel preview textures
                        self.deck_preview_textures.clear();
                        self.channel_preview_textures.clear();
                        let context = varda.gpu_context();
                        let mixer = varda.mixer_ref();
                        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
                            for slot in &ch.decks {
                                let tex_id = egui_renderer.register_native_texture(
                                    &context.device,
                                    &slot.deck.texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.deck_preview_textures
                                    .insert(slot.deck.uuid().to_string(), tex_id);
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
                                varda.mixer_ref().composite_view(),
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
            let resolution_changed = engine_outcome.resolution_changed;
            varda.update_controller_leds();

            // After resolution change, all GPU textures were recreated —
            // re-register them with egui so previews point to the new views.
            if resolution_changed {
                let context = varda.gpu_context();
                let mixer = varda.mixer_ref();
                for (ch_idx, ch) in mixer.channels().iter().enumerate() {
                    for slot in &ch.decks {
                        if let Some(&tex_id) = self.deck_preview_textures.get(slot.deck.uuid()) {
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
                        mixer.composite_view(),
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
            if let Some(ch_idx) = engine_outcome.removed_channel {
                self.layout.fixup_channel_removal(ch_idx);
            }

            if ui_actions.session.save_requested {
                varda.save_workspace(&self.layout);
                varda.notify_info("💾 Workspace saved");
            }

            // Spawn file dialogs on background threads (non-blocking)
            if let Some(uuid) = ui_actions.session.open_image_dialog_for_channel.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, FileDialogKind::Image, uuid);
            }
            if let Some(uuid) = ui_actions.session.open_video_dialog_for_channel.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, FileDialogKind::Video, uuid);
            }

            // Poll completed file dialog results → spawn background deck loads.
            // The dialog carries a channel UUID rather than an index: the user
            // may have spent minutes browsing while the UI stayed live.
            while let Ok(result) = self.file_dialog_rx.try_recv() {
                if varda
                    .mixer_ref()
                    .find_channel_by_uuid(&result.channel_uuid)
                    .is_none()
                {
                    log::warn!(
                        "Dropping {} file dialog result(s): channel {} no longer exists",
                        result.paths.len(),
                        result.channel_uuid
                    );
                    continue;
                }
                let mut images = Vec::new();
                let mut videos = Vec::new();
                for path in result.paths {
                    let token = self.deck_load_targets.record(result.channel_uuid.clone());
                    match result.kind {
                        FileDialogKind::Image => images.push((token, path)),
                        FileDialogKind::Video => videos.push((token, path)),
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
                let target = self.deck_load_targets.claim(result.token);
                match result.deck {
                    Ok(deck) => {
                        // Resolve the target here, not at spawn: the channel list
                        // can change while a decode or shader compile runs, and an
                        // index captured back then would now name a different
                        // channel. See `/spec/api-addressing.md`.
                        let Some(channel_uuid) = target else {
                            log::warn!(
                                "Dropping background load '{}': no target channel was recorded",
                                result.name
                            );
                            continue;
                        };
                        let Some(ch_idx) = varda.mixer_ref().find_channel_by_uuid(&channel_uuid)
                        else {
                            log::warn!(
                                "Dropping background load '{}': channel {} no longer exists",
                                result.name,
                                channel_uuid
                            );
                            continue;
                        };
                        // Same post-construction wiring the synchronous
                        // `add_deck` command performs — analyzer startup and
                        // required-device acquisition. Without it a shader
                        // dropped from the Library renders against blank
                        // preprocessor textures with no error shown.
                        let mut deck = deck;
                        if let Err(e) = varda.finalize_new_deck(&mut deck) {
                            log::error!("Failed to add deck: {e}");
                            varda.notify_error(format!("Failed to add deck: {e}"));
                            continue;
                        }
                        let deck_uuid = deck.uuid().to_string();
                        if let Some(ch) = varda.mixer_mut().channel_mut(ch_idx) {
                            let idx = ch.add_deck(deck);
                            log::info!(
                                "Background load complete: deck {} to channel {}: {}",
                                idx,
                                channel_uuid,
                                result.name
                            );
                        }
                        // Re-borrow for texture registration (separate from mixer borrow)
                        register_deck_preview_texture(
                            egui_renderer,
                            varda.gpu_context(),
                            varda.mixer_ref(),
                            &deck_uuid,
                            &mut self.deck_preview_textures,
                        );
                    }
                    Err(e) => {
                        log::error!("Background deck load failed for '{}': {}", result.name, e);
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
            // Publish the cued channel(s) so off-air previews render live (issue #72).
            varda.set_preview_channels(self.layout.preview_channels());
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
            #[cfg(feature = "html")]
            varda.render_interactive();
            self.publish_counter += 1;
            if self.publish_counter.is_multiple_of(10) {
                varda.publish_state();
            }
        }
        let outputs_us = t_outputs.elapsed().as_micros();

        // 9b. Gamma-encode previews. After the mixer render (8) and output
        // windows (9) — window previews source their intermediate texture — and
        // before egui paints (10), so thumbnails show this frame and match what
        // the output window displays.
        self.encode_previews();

        // 10. UI surface last — operator control surface, latency-tolerant.
        // The UI blit + egui overlay + present can safely happen after outputs.
        let t_submit = std::time::Instant::now();
        self.submit_frame(
            window,
            full_output.shapes,
            full_output.pixels_per_point,
            &full_output.textures_delta,
        );
        let submit_us = t_submit.elapsed().as_micros();

        // Advance the cadence anchor for adaptive frame pacing.
        let target_fps = self
            .varda
            .as_ref()
            .map_or(self.config.target_fps, crate::app::VardaApp::target_fps);
        self.advance_cadence_anchor(target_fps);

        // Frame loop timing (log every 120 frames)
        self.frame_loop_counter += 1;
        if self.frame_loop_counter.is_multiple_of(120) {
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
        textures_delta: &egui::TexturesDelta,
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
                        log::error!("Failed to get surface texture after reconfigure: {other:?}");
                        return;
                    }
                }
            }
            other => {
                log::debug!("UI surface unavailable: {other:?}");
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
            Some(blit.create_bind_group(&context.device, mixer.composite_view()))
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

        context.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // ── Frame pacing ────────────────────────────────────────────────
    //
    // `UIRunner::new` needs no window, GPU, or event loop — every such field is
    // `None` — so the pacing logic is directly exercisable.

    fn runner() -> UIRunner {
        UIRunner::new(AppConfig::parse_from(["varda", "--headless"]))
    }

    #[test]
    fn cadence_anchor_is_cleared_when_fps_is_uncapped() {
        let mut runner = runner();
        runner.advance_cadence_anchor(60);
        assert!(runner.cadence_anchor.is_some());
        runner.advance_cadence_anchor(0);
        assert!(
            runner.cadence_anchor.is_none(),
            "target_fps 0 means uncapped — no pacing deadline"
        );
    }

    #[test]
    fn cadence_anchor_starts_one_budget_ahead() {
        let mut runner = runner();
        let before = std::time::Instant::now();
        runner.advance_cadence_anchor(60);
        let anchor = runner.cadence_anchor.expect("anchor set");
        let budget = std::time::Duration::from_secs_f64(1.0 / 60.0);
        assert!(anchor >= before + budget, "at least one budget out");
        assert!(
            anchor <= std::time::Instant::now() + budget,
            "not more than one budget out"
        );
    }

    /// A frame that finishes inside its budget keeps the existing cadence: the
    /// anchor advances by exactly one budget rather than resetting to `now`.
    /// This is what stops frame times from drifting.
    #[test]
    fn cadence_anchor_advances_by_one_budget_when_on_time() {
        let mut runner = runner();
        let budget = std::time::Duration::from_secs_f64(1.0 / 60.0);
        // Far enough ahead that `now` is comfortably before the deadline.
        let anchor = std::time::Instant::now() + budget * 10;
        runner.cadence_anchor = Some(anchor);
        runner.advance_cadence_anchor(60);
        assert_eq!(
            runner.cadence_anchor,
            Some(anchor + budget),
            "on-time frame advances the anchor by exactly one budget"
        );
    }

    /// A frame that overshot its deadline restarts the cadence from `now` rather
    /// than chasing the stale anchor, which would emit a burst of zero-budget
    /// catch-up frames.
    #[test]
    fn cadence_anchor_snaps_forward_after_an_overshoot() {
        let mut runner = runner();
        let budget = std::time::Duration::from_secs_f64(1.0 / 60.0);
        let stale = std::time::Instant::now()
            .checked_sub(budget * 10)
            .expect("clock is far enough past boot to subtract 10 frames");
        runner.cadence_anchor = Some(stale);
        let before = std::time::Instant::now();
        runner.advance_cadence_anchor(60);
        let anchor = runner.cadence_anchor.expect("anchor set");
        assert!(
            anchor >= before + budget,
            "overshoot restarts from now, not from the stale anchor"
        );
        assert_ne!(anchor, stale + budget, "must not chase the stale deadline");
    }

    #[test]
    fn cadence_budget_scales_with_target_fps() {
        let mut fast = runner();
        fast.advance_cadence_anchor(120);
        let fast_anchor = fast.cadence_anchor.expect("anchor");
        let mut slow = runner();
        slow.advance_cadence_anchor(30);
        let slow_anchor = slow.cadence_anchor.expect("anchor");
        assert!(
            fast_anchor < slow_anchor,
            "a higher target fps yields a nearer deadline"
        );
    }

    // ── Detection worker thread ─────────────────────────────────────

    /// The worker echoes `is_capture` and `camera_id` back untouched, which is how
    /// the runner tells a freeze-frame capture from a live overlay refresh.
    #[test]
    fn detect_worker_round_trips_request_metadata() {
        let (req_tx, req_rx) = std::sync::mpsc::channel();
        let res_rx = spawn_detect_thread(req_rx);

        req_tx
            .send(DetectRequest {
                rgba: vec![0; 4 * 8 * 8],
                w: 8,
                h: 8,
                params: crate::surface::detect::DetectionParams::default(),
                is_capture: true,
                camera_id: 7,
            })
            .expect("send request");

        let res = res_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("worker replied");
        assert!(res.is_capture, "capture flag echoed back");
        assert_eq!(res.camera_id, 7);
    }

    /// A blank frame yields no contours rather than letting the error escape the
    /// thread — the worker must survive undetectable input and stay available.
    #[test]
    fn detect_worker_survives_undetectable_input_and_keeps_serving() {
        let (req_tx, req_rx) = std::sync::mpsc::channel();
        let res_rx = spawn_detect_thread(req_rx);

        for _ in 0..2 {
            req_tx
                .send(DetectRequest {
                    rgba: vec![0; 4 * 8 * 8],
                    w: 8,
                    h: 8,
                    params: crate::surface::detect::DetectionParams::default(),
                    is_capture: false,
                    camera_id: 1,
                })
                .expect("send request");
            let res = res_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("worker replied");
            assert!(res.contours.is_empty(), "blank frame has no contours");
        }
    }

    // ── render_headless ─────────────────────────────────────────────
    //
    // A real `ActiveEventLoop` cannot be constructed in a test, so `WindowHost`
    // stands in for it. Everything else is real: a headless `GpuContext` and a
    // genuine `VardaApp`, driven frame by frame.

    #[derive(Default)]
    struct FakeHost {
        exits: std::cell::Cell<u32>,
        outputs_created: std::cell::Cell<u32>,
        monitors_refreshed: std::cell::Cell<u32>,
    }

    impl WindowHost for FakeHost {
        fn exit(&self) {
            self.exits.set(self.exits.get() + 1);
        }

        fn create_pending_outputs(&self, _varda: &mut VardaApp) {
            self.outputs_created.set(self.outputs_created.get() + 1);
        }

        fn refresh_monitors(&self, _varda: &mut VardaApp) {
            self.monitors_refreshed
                .set(self.monitors_refreshed.get() + 1);
        }

        #[cfg(feature = "html")]
        fn create_pending_interactive(&self, _varda: &mut VardaApp) {}
    }

    /// A runner with a real headless engine attached, or `None` when the machine
    /// has no GPU adapter — matching the skip-without-adapter pattern used by the
    /// other GPU tests.
    fn headless_runner() -> Option<UIRunner> {
        let gpu = GpuContext::new_headless().ok()?;
        // One config for both halves, so the engine and the runner agree on
        // which scratch workspace a shutdown save writes to. The scratch
        // workspace is not optional: `render_headless` saves on shutdown.
        let config = crate::testing::headless_config();
        let varda = VardaApp::new(gpu, &config).expect("VardaApp::new");
        let mut runner = UIRunner::new(config);
        runner.varda = Some(varda);
        Some(runner)
    }

    #[test]
    fn render_headless_drives_a_frame_with_no_window() {
        let Some(mut runner) = headless_runner() else {
            return;
        };
        let host = FakeHost::default();
        runner.render_headless(&host);

        assert_eq!(host.exits.get(), 0, "a normal frame must not exit");
        assert_eq!(
            host.outputs_created.get(),
            1,
            "each frame reconciles pending output windows"
        );
        assert_eq!(
            host.monitors_refreshed.get(),
            1,
            "each frame refreshes the monitor list"
        );
        assert_eq!(runner.publish_counter, 1);
    }

    /// State publishing is gated to every tenth frame to keep snapshot cost off
    /// the per-frame path. Observed through the shared reader the HTTP API uses.
    #[test]
    fn render_headless_publishes_state_every_tenth_frame() {
        let Some(mut runner) = headless_runner() else {
            return;
        };
        let reader = runner
            .varda
            .as_ref()
            .expect("engine attached")
            .state_reader();
        // Start from a known-empty slot regardless of construction-time publishing.
        reader.write().expect("state lock").take();

        let host = FakeHost::default();
        for frame in 1..10 {
            runner.render_headless(&host);
            assert!(
                reader.read().expect("state lock").is_none(),
                "frame {frame} must not publish"
            );
        }

        runner.render_headless(&host);
        assert_eq!(runner.publish_counter, 10);
        assert!(
            reader.read().expect("state lock").is_some(),
            "the tenth frame publishes engine state"
        );
    }

    /// A flagged shutdown exits and skips the frame's work entirely, so nothing
    /// is rendered or reconciled after the signal.
    #[test]
    fn render_headless_exits_and_skips_work_when_shutdown_flagged() {
        let Some(mut runner) = headless_runner() else {
            return;
        };
        let workspace = runner
            .config
            .workspace_root
            .clone()
            .expect("headless_config supplies a scratch workspace");
        runner
            .shutdown_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let host = FakeHost::default();
        runner.render_headless(&host);

        // This test writes a default scene to disk. Pin where, so the isolation
        // is enforced rather than merely conventional.
        assert!(
            workspace.join(".varda").join("scene.json").is_file(),
            "the shutdown save must land in the scratch workspace"
        );
        assert_eq!(host.exits.get(), 1, "shutdown exits the event loop");
        assert_eq!(
            host.outputs_created.get(),
            0,
            "shutdown returns before reconciling outputs"
        );
        assert_eq!(
            runner.publish_counter, 0,
            "shutdown returns before the publish gate"
        );
    }

    /// A timeline drag pushes a mutation every frame it moves. Exactly one of
    /// them may become an undo entry, or a two-second drag would bury the rest
    /// of the session's history.
    #[test]
    fn a_held_drag_is_one_undo_entry() {
        let mut prev = false;
        let frames: Vec<bool> = (0..40)
            .map(|_| wants_history_snapshot(&mut prev, true, true))
            .collect();
        assert_eq!(frames.iter().filter(|pushed| **pushed).count(), 1);
        assert!(frames[0], "the entry belongs to the frame that started it");
    }

    /// Releasing and grabbing again is a second edit, and undo has to be able to
    /// step between them.
    #[test]
    fn a_second_drag_gets_its_own_entry() {
        let mut prev = false;
        let mut pushes = 0;
        for gesture in [true, true, false, true, true] {
            if wants_history_snapshot(&mut prev, true, gesture) {
                pushes += 1;
            }
        }
        assert_eq!(pushes, 3, "two drags and the release between them");
    }

    /// Discrete edits are unaffected: every click is still its own step.
    #[test]
    fn clicks_are_not_coalesced() {
        let mut prev = false;
        let pushes = (0..5)
            .filter(|_| wants_history_snapshot(&mut prev, true, false))
            .count();
        assert_eq!(pushes, 5);
    }

    /// A frame that changed nothing never takes a snapshot, gesture or not.
    #[test]
    fn a_quiet_frame_records_nothing() {
        let mut prev = false;
        assert!(!wants_history_snapshot(&mut prev, false, true));
        assert!(!wants_history_snapshot(&mut prev, false, false));
    }
}
