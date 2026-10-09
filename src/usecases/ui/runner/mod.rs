//! `UIRunner`: the windowed delivery layer for the Varda engine.
//!
//! Owns the window, egui state, blit pipeline, texture registrations,
//! `WindowSurface`, and the engine (`VardaApp`), which it drives each frame.
//! Headless runs (HTTP API, CLI) don't use it.

use crate::app::render::FileDialogResult;
use crate::app::{AppConfig, VardaApp};
use crate::engine::EngineCommand;
use crate::engine::value::editor::EditorPrefs;
use crate::renderer::blit::BlitPipeline;
use crate::renderer::context::{GpuContext, WindowSurface};
use crate::usecases::ui;

use winit::{
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

mod camera_detect;
mod detect;
mod event_loop;
pub(crate) mod preview;

pub(crate) use event_loop::WindowHost;

use preview::PreviewEncoder;

use detect::{DetectRequest, DetectResponse, spawn_detect_thread};

/// How long quitting waits for running shader builds before exiting anyway, so
/// a stuck build cannot hang quitting.
const SHADER_BUILD_EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct UIRunner {
    // Session config: CLI flags and workspace defaults.
    config: AppConfig,

    // Window and egui state.
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
    lut_catalog: crate::usecases::ui::LutCatalog,
    dome_preview_renderer: Option<crate::renderer::dome_preview::DomePreviewRenderer>,
    dome_preview_texture: Option<egui::TextureId>,
    // Camera detection mode state
    camera_detect_texture: Option<egui::TextureId>,
    /// Detection camera last requested from the engine.
    camera_detect_camera_id: Option<crate::camera::CameraId>,
    /// Commands raised outside the UI pass (camera detection), sent with this
    /// frame's UI commands.
    queued_commands: Vec<EngineCommand>,
    /// Cued preview channels last sent to the engine.
    sent_preview_channels: Vec<String>,
    camera_detect_contours: Vec<crate::surface::detect::DetectedContour>,
    // Background detection thread channels.
    detect_req_tx: std::sync::mpsc::Sender<DetectRequest>,
    detect_res_rx: std::sync::mpsc::Receiver<DetectResponse>,
    detect_in_flight: bool,
    main_window_id: Option<WindowId>,

    // UI-owned layout and selection state.
    layout: super::UILayoutState,

    // File dialog results (async).
    file_dialog_tx: std::sync::mpsc::Sender<FileDialogResult>,
    file_dialog_rx: std::sync::mpsc::Receiver<FileDialogResult>,

    // Engine, created after GPU init.
    varda: Option<VardaApp>,

    // Deferred GPU init (avoids a Metal dispatch-queue deadlock on Rosetta/Intel).
    gpu_init_handle: Option<std::thread::JoinHandle<anyhow::Result<(GpuContext, WindowSurface)>>>,
    startup_t0: Option<std::time::Instant>,

    /// Previous frame's `gesture_active`, to tell drag start from continuation so
    /// a continuous drag is one undo step. The undo history lives on `VardaApp`.
    prev_gesture_active: bool,
    /// Editor prefs last sent to the engine; `None` until the first frame sends them.
    sent_editor_prefs: Option<EditorPrefs>,

    // Gates publish_state to reduce snapshot overhead.
    publish_counter: u32,

    // HTTP API server (background thread).
    api_handle: Option<crate::usecases::api::runner::ApiServerHandle>,

    /// Ideal start time of the next frame. Advances by `frame_budget` each frame;
    /// after an overshoot it snaps to `now + budget` to avoid catch-up bursts.
    cadence_anchor: Option<std::time::Instant>,

    // Set by SIGINT/SIGTERM.
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,

    // Cached window geometry. winit 0.30's Window::inner_size() on X11 makes a
    // blocking XGetGeometry request, and egui_winit's take_egui_input() calls it
    // every frame. The size is cached from Resized/ScaleFactorChanged events and
    // take_egui_input() is bypassed.
    egui_start_time: std::time::Instant,
    cached_screen_size: winit::dpi::PhysicalSize<u32>,
    cached_scale_factor: f32,
    frame_loop_counter: u32,
}

impl UIRunner {
    pub fn new(config: AppConfig) -> Self {
        let (file_dialog_tx, file_dialog_rx) = std::sync::mpsc::channel();
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
            lut_catalog: crate::usecases::ui::LutCatalog::default(),
            dome_preview_renderer: None,
            dome_preview_texture: None,
            camera_detect_texture: None,
            camera_detect_camera_id: None,
            queued_commands: Vec::new(),
            sent_preview_channels: Vec::new(),
            camera_detect_contours: Vec::new(),
            detect_req_tx,
            detect_res_rx,
            detect_in_flight: false,
            main_window_id: None,
            layout: super::UILayoutState::default(),
            file_dialog_tx,
            file_dialog_rx,
            varda: None,
            gpu_init_handle: None,
            startup_t0: None,
            prev_gesture_active: false,
            sent_editor_prefs: None,
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
    /// server, or one already exists on this thread) or exits with an error.
    pub fn run(mut self) -> anyhow::Result<()> {
        // Ctrl-C handler for graceful shutdown, mainly for headless.
        let flag = self.shutdown_flag.clone();
        let _ = ctrlc::set_handler(move || {
            log::info!("Received interrupt signal, shutting down...");
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let event_loop = EventLoop::new()?;
        let result = event_loop
            .run_app(&mut self)
            .map_err(|e| anyhow::anyhow!("Event loop error: {e:?}"));
        // A shader compile still running when the process exits crashes it.
        if !crate::renderer::builds::shutdown(SHADER_BUILD_EXIT_WAIT) {
            log::warn!("Exiting with shader builds still running after {SHADER_BUILD_EXIT_WAIT:?}");
        }
        result
    }
}

/// Whether this frame's mutations get their own undo entry.
///
/// A held drag mutates every frame it moves, so only the frame that starts the
/// gesture takes a snapshot; otherwise one drag would fill the history.
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
    /// Finish initialization once the GPU context is ready. Called from
    /// `resumed()` when headless, or from `about_to_wait()` when windowed.
    fn finish_init(
        &mut self,
        gpu: GpuContext,
        win_surface: Option<WindowSurface>,
        startup_t0: std::time::Instant,
        event_loop: &ActiveEventLoop,
    ) {
        // egui and blit pipeline, windowed only.
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

            // Application icon on egui's viewport (dock/taskbar icon).
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

        // Load the workspace; may replace the default mixer with a saved scene.
        log::info!("[STARTUP] Loading workspace...");
        let loaded = varda.load_workspace();
        if let Some(prefs) = loaded.editor_prefs {
            self.layout.apply_editor_prefs(prefs);
        }
        // `load_workspace` clears the engine-owned undo/redo timeline.
        log::info!("[STARTUP] Workspace loaded ({:.0?})", startup_t0.elapsed());

        // HTTP API server on a background thread.
        if self.api_handle.is_none() {
            self.api_handle = crate::usecases::api::runner::start(
                self.config.api_port,
                varda.command_sender(),
                varda.state_reader(),
            );
        }

        self.varda = Some(varda);

        // Register preview textures with egui (windowed only).
        if !self.config.headless {
            self.register_preview_textures();
        }
        log::info!(
            "[STARTUP] Initialization complete ({:.0?})",
            startup_t0.elapsed()
        );
    }

    /// Advance the cadence anchor after a frame (rendered or headless).
    ///
    /// The anchor is the ideal start of the next frame and advances by one budget
    /// per call. If the frame overshot (the anchor is already past), it snaps to
    /// `now + budget` instead of catching up, which would fire a burst of short
    /// frames.
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
                    // Finished before the deadline; cadence kept.
                    ideal_next
                } else {
                    // Overshot; restart the cadence from now.
                    now + budget
                }
            }
            None => now + budget,
        });
    }

    /// Headless render loop: engine processing without egui.
    fn render_headless(&mut self, host: &dyn WindowHost) {
        let Some(varda) = self.varda.as_mut() else {
            return;
        };

        // Shutdown requested by the API or SIGINT/SIGTERM.
        if varda.shutdown_requested()
            || self
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            log::info!("Shutdown requested, saving workspace and exiting...");
            if let Err(e) = varda.save_workspace() {
                log::error!("{e}");
            }
            if let Some(api) = self.api_handle.take() {
                api.shutdown();
            }
            host.exit();
            return;
        }

        varda.begin_frame();
        varda.run_pending_global_actions();

        // Create pending output windows (API-driven when headless).
        host.create_pending_outputs(varda);
        host.refresh_monitors(varda);
        #[cfg(feature = "html")]
        host.create_pending_interactive(varda);

        varda.render_frame();
        self.publish_counter += 1;
        if self.publish_counter.is_multiple_of(10) {
            varda.publish_state();
        }
    }

    /// Main render loop; the logic lives in `VardaApp`.
    fn render(&mut self, event_loop: &ActiveEventLoop) {
        // 1. Frame timing, notifications, inputs.
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            varda.begin_frame();
        }

        // 2. Sync egui texture registrations.
        self.refresh_textures();

        let Some(window) = self.window else { return };

        // 3. Create pending output windows and refresh monitors.
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            varda.create_pending_outputs(event_loop);
            varda.refresh_monitors(event_loop);
            #[cfg(feature = "html")]
            varda.create_pending_interactive(event_loop);
        }

        // 3b. Render the dome preview when the preview or dome mode is open.
        if (self.layout.dome_preview_open || self.layout.dome_mode_active)
            && let (Some(renderer), Some(varda)) = (&mut self.dome_preview_renderer, &self.varda)
        {
            let context = varda.gpu_context();
            let dome = varda.dome_config();

            // Slice overlays in dome mode.
            if self.layout.dome_mode_active {
                let setup = dome.preset.to_setup_with_geometry(dome.geometry);
                renderer.set_slice_overlays(&context.device, &setup);
            } else {
                renderer.clear_slice_overlays();
            }

            // Domemaster output if available, else the mixer composite.
            let source_view = varda
                .domemaster_view()
                .unwrap_or_else(|| varda.mixer_ref().composite_view());
            let (c_az, c_el, c_roll) = dome.geometry.content_rotation_radians();
            renderer.render(context, source_view, c_az, c_el, c_roll);
        }

        self.sync_camera_detect_capture();

        // 4. UI data snapshot: engine state plus UI-owned layout state.
        let Some(varda_ref) = self.varda.as_ref() else {
            return;
        };
        // One snapshot per frame: the GUI view borrows it, and every tenth frame the
        // API gets the same build.
        let engine = varda_ref.build_engine_state();
        let mut ui_data = crate::usecases::ui::build_ui_data(
            &engine,
            &self.layout,
            &crate::usecases::ui::PreviewTextures {
                deck: &self.deck_preview_textures,
                channel: &self.channel_preview_textures,
                output: &self.output_preview_textures,
                main_output: self.main_output_texture,
            },
            self.lut_catalog
                .files(&varda_ref.luts_dir(), std::time::Instant::now()),
        );
        self.publish_counter += 1;
        if self.publish_counter.is_multiple_of(10) {
            varda_ref.publish(engine);
        }
        ui_data.can_undo = varda_ref.history_can_undo();
        ui_data.can_redo = varda_ref.history_can_redo();
        ui_data.dome_preview_open = self.layout.dome_preview_open;
        ui_data.dome_preview_texture = self.dome_preview_texture;
        ui_data.camera_detect_texture = self.camera_detect_texture;
        ui_data.camera_detect_mode = self.layout.camera_detect_mode.clone();

        // Poll background detection results.
        while let Ok(response) = self.detect_res_rx.try_recv() {
            self.detect_in_flight = false;
            if response.is_capture {
                // Capture complete: switch to Preview mode.
                let n = response.contours.len();
                self.camera_detect_contours = response.contours.clone();
                self.layout.camera_detect_mode = ui::CameraDetectMode::Preview {
                    camera_id: response.camera_id,
                    contours: response.contours,
                    selected: vec![true; n],
                };
                // Refresh UIData's mode after the change above.
                ui_data.camera_detect_mode = self.layout.camera_detect_mode.clone();
            } else {
                // Live overlay update
                self.camera_detect_contours = response.contours;
            }
        }

        // In Live mode with no work in flight, submit new detection work.
        if let ui::CameraDetectMode::Live {
            camera_id,
            ref params,
        } = self.layout.camera_detect_mode
            && !self.detect_in_flight
            && let Some(frame) = varda_ref.camera_manager().snapshot_frame(camera_id)
        {
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

        ui_data
            .camera_detect_contours
            .clone_from(&self.camera_detect_contours);

        // 5. Run the egui frame.
        let t_egui = std::time::Instant::now();
        // Build the input by hand instead of take_egui_input(), using the cached size,
        // to avoid a blocking X11 round-trip every frame.
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

        // 6. Apply UI actions.
        // 6a. UI-owned selection and layout state.
        self.layout.apply_selections(&ui_actions);

        // 6a2. Dome camera actions go to the renderer, not layout state.
        {
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
                    ui::DomeAction::SetMode(_) => {} // handled by layout.apply_selections
                }
            }
        }

        self.apply_camera_detect_actions(&mut ui_actions);

        // 6b. Engine actions, via VardaApp.
        {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };

            ui_actions.commands.append(&mut self.queued_commands);

            // Files picked in a dialog become deck-adds on this frame's command stream.
            // The dialog carries a channel UUID, since the UI stayed live while it was open.
            while let Ok(result) = self.file_dialog_rx.try_recv() {
                ui_actions.commands.extend(result.commands());
            }

            // Undo: a snapshot is taken when the frame has an undoable mutation and is
            // not the continuation of a held drag, so a continuous gesture is one undo
            // step. `command_is_undoable` (via `batch_has_undoable`) decides
            // undoability; the engine takes the snapshot inside the drain.
            let dirty = varda.batch_has_undoable(&ui_actions.commands);
            // A recording pass is one gesture: its entry was pushed when the first
            // parameter was touched.
            let starts_undo_step = wants_history_snapshot(
                &mut self.prev_gesture_active,
                dirty,
                ui_actions.session.gesture_active || varda.is_recording(),
            );

            // Editor prefs, undo/redo, and save go on the same command stream, after this
            // frame's edits, in that order. Control-surface requests are merged in.
            let pending = varda.take_pending_global_actions();
            // Compared in place so an unchanged frame allocates nothing.
            let cued = self.layout.preview_channels();
            let channels = varda.mixer_ref().channels();
            let unchanged = cued.len() == self.sent_preview_channels.len()
                && cued
                    .iter()
                    .zip(&self.sent_preview_channels)
                    .all(|(&idx, sent)| channels.get(idx).is_some_and(|ch| ch.uuid() == sent));
            if !unchanged {
                let channel_uuids: Vec<String> = cued
                    .iter()
                    .filter_map(|&idx| channels.get(idx))
                    .map(|ch| ch.uuid().to_string())
                    .collect();
                self.sent_preview_channels.clone_from(&channel_uuids);
                ui_actions
                    .commands
                    .push(EngineCommand::SetPreviewChannels { channel_uuids });
            }
            let prefs = self.layout.editor_prefs();
            if self.sent_editor_prefs != Some(prefs) {
                ui_actions
                    .commands
                    .push(EngineCommand::SetEditorPrefs { prefs });
                self.sent_editor_prefs = Some(prefs);
            }
            if ui_actions.session.undo_requested || pending.undo {
                ui_actions.commands.push(EngineCommand::Undo);
            } else if ui_actions.session.redo_requested || pending.redo {
                ui_actions.commands.push(EngineCommand::Redo);
            }
            if ui_actions.session.save_requested || pending.save {
                ui_actions.commands.push(EngineCommand::SaveWorkspace);
            }

            // Current position of each channel this frame removes, so selection can
            // follow once the engine confirms the removal.
            let mut removals: Vec<(usize, String)> = ui_actions
                .commands
                .iter()
                .filter_map(|cmd| match cmd {
                    EngineCommand::RemoveChannel { channel_uuid } => varda
                        .mixer_ref()
                        .find_channel_by_uuid(channel_uuid)
                        .map(|idx| (idx, channel_uuid.clone())),
                    _ => None,
                })
                .collect();

            varda.apply_engine_actions(std::mem::take(&mut ui_actions.commands), starts_undo_step);

            // Highest position first, so each fixup sees the indices it expects.
            removals.sort_unstable_by_key(|r| std::cmp::Reverse(r.0));
            for (ch_idx, uuid) in removals {
                if varda.mixer_ref().find_channel_by_uuid(&uuid).is_none() {
                    self.layout.fixup_channel_removal(ch_idx);
                }
            }

            // File dialogs run on background threads.
            if let Some(request) = ui_actions.session.open_file_dialog.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, request);
            }
        }

        let egui_us = t_egui.elapsed().as_micros();

        // 7. Drain the previous frame's GPU work before submitting new work, so the
        // queue doesn't build up and block get_current_texture()/present().
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

        // 8-9. Engine render: the mixer, then output windows before the UI, so
        // latency-critical projectors and displays don't wait on the UI surface's
        // get_current_texture()/present().
        let times = {
            let Some(varda) = self.varda.as_mut() else {
                return;
            };
            varda.render_frame()
        };
        let mixer_us = times.mixer.as_micros();
        let outputs_us = times.outputs.as_micros();

        // 9b. Gamma-encode previews after the mixer (8) and output windows (9),
        // since window previews read their intermediate texture, and before egui
        // paints (10), so thumbnails show this frame. Only the ones this frame's
        // UI drew are encoded.
        self.encode_previews(&preview::textures_drawn(&full_output.shapes));

        // 10. UI surface last: blit, egui overlay and present are latency-tolerant.
        let t_submit = std::time::Instant::now();
        self.submit_frame(
            window,
            full_output.shapes,
            full_output.pixels_per_point,
            &full_output.textures_delta,
        );
        let submit_us = t_submit.elapsed().as_micros();

        let target_fps = self
            .varda
            .as_ref()
            .map_or(self.config.target_fps, crate::app::VardaApp::target_fps);
        self.advance_cadence_anchor(target_fps);

        // Frame loop timing, logged every 120 frames.
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

    /// Blit the mixer output to screen, overlay egui, and present.
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

        // Apply texture updates even when the surface is unavailable (e.g. Occluded
        // at startup), so the egui renderer stays in sync.
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        for (id, deltas) in &textures_delta.set {
            for delta in deltas {
                egui_renderer.update_texture(&context.device, &context.queue, *id, delta);
            }
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
        context.queue.present(output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // Frame pacing. `UIRunner::new` needs no window, GPU, or event loop, so the
    // pacing logic can be tested directly.

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

    /// A frame inside its budget advances the anchor by exactly one budget instead
    /// of resetting to `now`, so frame times don't drift.
    #[test]
    fn cadence_anchor_advances_by_one_budget_when_on_time() {
        let mut runner = runner();
        let budget = std::time::Duration::from_secs_f64(1.0 / 60.0);
        // Far enough ahead that `now` is well before the deadline.
        let anchor = std::time::Instant::now() + budget * 10;
        runner.cadence_anchor = Some(anchor);
        runner.advance_cadence_anchor(60);
        assert_eq!(
            runner.cadence_anchor,
            Some(anchor + budget),
            "on-time frame advances the anchor by exactly one budget"
        );
    }

    /// A frame past its deadline restarts the cadence from `now` instead of
    /// chasing the stale anchor with zero-budget catch-up frames.
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

    /// The worker returns `is_capture` and `camera_id` unchanged; the runner uses
    /// them to tell a capture from a live overlay refresh.
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

    /// A blank frame yields no contours and the worker stays available.
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

    // render_headless. A real `ActiveEventLoop` cannot be constructed in a test,
    // so `WindowHost` stands in for it; the `GpuContext` and `VardaApp` are real.

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

    /// A runner with a real headless engine, or `None` without a GPU adapter.
    fn headless_runner() -> Option<UIRunner> {
        let gpu = crate::testing::headless_gpu()?;
        // One config for both, so the engine and runner use the same scratch
        // workspace. `render_headless` saves on shutdown, so one is required.
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

    /// State is published every tenth frame, observed through the shared reader
    /// the HTTP API uses.
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
        let generation = || reader.latest().map(|p| p.generation);
        let before = generation();

        let host = FakeHost::default();
        for frame in 1..10 {
            runner.render_headless(&host);
            assert_eq!(generation(), before, "frame {frame} must not publish");
        }

        runner.render_headless(&host);
        assert_eq!(runner.publish_counter, 10);
        assert_ne!(
            generation(),
            before,
            "the tenth frame publishes engine state"
        );
    }

    /// A flagged shutdown exits and skips the frame's work.
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

        // This test writes a default scene to disk; check it goes to the scratch
        // workspace.
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

    /// A timeline drag mutates every frame it moves; exactly one frame becomes an
    /// undo entry.
    #[test]
    fn a_held_drag_is_one_undo_entry() {
        let mut prev = false;
        let frames: Vec<bool> = (0..40)
            .map(|_| wants_history_snapshot(&mut prev, true, true))
            .collect();
        assert_eq!(frames.iter().filter(|pushed| **pushed).count(), 1);
        assert!(frames[0], "the entry belongs to the frame that started it");
    }

    /// Releasing and grabbing again is a second undo step.
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

    /// Every discrete edit is its own undo step.
    #[test]
    fn clicks_are_not_coalesced() {
        let mut prev = false;
        let pushes = (0..5)
            .filter(|_| wants_history_snapshot(&mut prev, true, false))
            .count();
        assert_eq!(pushes, 5);
    }

    /// A frame with no changes never takes a snapshot.
    #[test]
    fn a_quiet_frame_records_nothing() {
        let mut prev = false;
        assert!(!wants_history_snapshot(&mut prev, false, true));
        assert!(!wants_history_snapshot(&mut prev, false, false));
    }
}
