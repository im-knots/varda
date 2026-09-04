//! winit event-loop wiring: the `ApplicationHandler` callbacks and the
//! [`WindowHost`] abstraction over the parts of `ActiveEventLoop` the render
//! paths need.
//!
//! This is the only place that reacts to raw winit events. Everything it decides
//! is delegated straight back to `UIRunner`, so the per-frame logic stays
//! testable without an event loop — see `/spec/app-presentation-boundary.md`.

use super::UIRunner;
use crate::app::VardaApp;
use crate::renderer::context::GpuContext;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

/// The slice of the winit event loop that the per-frame render paths actually use.
///
/// `render_headless` and `render` are long functions, but they touch
/// `ActiveEventLoop` in only a handful of places: to exit, and to let `VardaApp`
/// reconcile output windows and monitors (which must happen on the event-loop
/// thread). Naming that dependency as a trait lets the render paths be driven
/// without a real event loop, which is otherwise impossible to construct in a
/// test. See `/spec/app-presentation-boundary.md` for why winit access is
/// confined to this boundary.
pub(crate) trait WindowHost {
    /// Ask the event loop to terminate.
    fn exit(&self);
    /// Create any output windows the engine has queued.
    fn create_pending_outputs(&self, varda: &mut VardaApp);
    /// Refresh the engine's cached monitor list.
    fn refresh_monitors(&self, varda: &mut VardaApp);
    /// Create any queued interactive (HTML) surfaces.
    #[cfg(feature = "html")]
    fn create_pending_interactive(&self, varda: &mut VardaApp);
}

impl WindowHost for ActiveEventLoop {
    fn exit(&self) {
        ActiveEventLoop::exit(self);
    }

    fn create_pending_outputs(&self, varda: &mut VardaApp) {
        varda.create_pending_outputs(self);
    }

    fn refresh_monitors(&self, varda: &mut VardaApp) {
        varda.refresh_monitors(self);
    }

    #[cfg(feature = "html")]
    fn create_pending_interactive(&self, varda: &mut VardaApp) {
        varda.create_pending_interactive(self);
    }
}

impl ApplicationHandler for UIRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Guard re-entry (resumed can be called multiple times on some platforms)
        if self.varda.is_some() {
            return;
        }

        let startup_t0 = std::time::Instant::now();
        log::info!("[STARTUP] resumed() entered — beginning initialization");

        if self.config.headless {
            // Headless: no main window, no egui — GPU without window surface
            log::info!("[STARTUP] Headless mode: skipping main window creation");
            let gpu = match GpuContext::new_headless() {
                Ok(gpu) => gpu,
                Err(e) => {
                    log::error!("Failed to create headless GPU context: {e}");
                    event_loop.exit();
                    return;
                }
            };
            self.finish_init(gpu, None, startup_t0, event_loop);
        } else {
            // Windowed: create main UI window, then spawn GPU init on a background
            // thread so we return from resumed() immediately.  On macOS (especially
            // under Rosetta / Intel), blocking the main thread during Metal device
            // creation causes a GCD dispatch-queue deadlock because Metal needs to
            // dispatch work back to the main queue.  By returning to the event loop
            // we keep that queue alive; about_to_wait() polls the thread handle and
            // finishes initialization once the GPU is ready.
            let window_icon = {
                static ICON_BYTES: &[u8] = include_bytes!("../../../../assets/icon.png");
                image::load_from_memory(ICON_BYTES).ok().and_then(|img| {
                    let rgba = img.into_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    winit::window::Icon::from_rgba(rgba.into_raw(), w, h).ok()
                })
            };
            let mut window_attrs = Window::default_attributes()
                .with_title("Varda VJ Software")
                .with_inner_size(winit::dpi::LogicalSize::new(1920, 1080));
            if let Some(icon) = window_icon {
                window_attrs = window_attrs.with_window_icon(Some(icon));
            }

            let window_static: &'static Window = match event_loop.create_window(window_attrs) {
                Ok(w) => {
                    log::info!("[STARTUP] Window created ({:.0?})", startup_t0.elapsed());
                    Box::leak(Box::new(w))
                }
                Err(e) => {
                    log::error!("Failed to create window: {e}");
                    event_loop.exit();
                    return;
                }
            };
            self.main_window_id = Some(window_static.id());
            self.window = Some(window_static);

            // Kick a redraw so macOS marks the window as "live" — required for
            // Metal/CALayer to function correctly (see wgpu#5722).
            window_static.request_redraw();

            // Create wgpu instance + surface on the main thread (macOS requires
            // NSView/CAMetalLayer access from the main thread), then hand off
            // adapter/device creation to a background thread.
            log::info!("[STARTUP] Creating surface on main thread...");
            let (instance, surface, size) =
                match GpuContext::create_surface_for_window(window_static) {
                    Ok(triple) => triple,
                    Err(e) => {
                        log::error!("Failed to create surface: {e}");
                        event_loop.exit();
                        return;
                    }
                };

            log::info!("[STARTUP] Spawning GPU adapter/device init on background thread...");
            self.startup_t0 = Some(startup_t0);
            self.gpu_init_handle = Some(std::thread::spawn(move || {
                pollster::block_on(GpuContext::new_with_surface(instance, surface, size))
            }));

            // Return immediately — about_to_wait() will complete initialization.
            // Keep polling so the event loop stays responsive.
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
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
            if let (Some(window), Some(egui_state)) = (self.window, &mut self.egui_state)
                && egui_state.on_window_event(window, &event).consumed
            {
                return;
            }
            match event {
                WindowEvent::CloseRequested => {
                    log::info!("Close requested, saving workspace and exiting...");
                    if let Err(e) = varda.save_workspace(&self.layout) {
                        log::error!("{e}");
                    }
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
            // Interactive HTML window: forward input to the WebView and consume.
            #[cfg(feature = "html")]
            if varda.handle_interactive_event(window_id, &event) {
                return;
            }
            match event {
                WindowEvent::CloseRequested => {
                    if let Some(name) = varda.close_output_window_by_id(window_id) {
                        log::info!("Output window '{name}' closed");
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
        // ── Phase 2 of deferred GPU init: poll the background thread ────
        if let Some(handle) = self.gpu_init_handle.as_ref() {
            if handle.is_finished() {
                let handle = self.gpu_init_handle.take().unwrap();
                let startup_t0 = self.startup_t0.take().unwrap();
                match handle.join().expect("GPU init thread panicked") {
                    Ok((gpu, win_surface)) => {
                        log::info!("[STARTUP] GPU context ready ({:.0?})", startup_t0.elapsed());
                        self.finish_init(gpu, Some(win_surface), startup_t0, event_loop);
                    }
                    Err(e) => {
                        log::error!("Failed to create render context: {e}");
                        event_loop.exit();
                        return;
                    }
                }
            } else {
                // GPU init still in progress — keep the event loop alive.
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                return;
            }
        }

        // Read target_fps from engine state (runtime-mutable via UI/API).
        let target_fps = self
            .varda
            .as_ref()
            .map_or(self.config.target_fps, crate::app::VardaApp::target_fps);

        if self.config.headless {
            // Headless: adaptive sleep-based pacing
            if target_fps > 0
                && let Some(deadline) = self.cadence_anchor
            {
                let now = std::time::Instant::now();
                if deadline > now {
                    std::thread::sleep(deadline - now);
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
