//! winit event-loop wiring: the `ApplicationHandler` callbacks and the
//! [`WindowHost`] abstraction over the parts of `ActiveEventLoop` the render
//! paths use.
//!
//! The only place that handles raw winit events. Decisions are delegated to
//! `UIRunner`, so per-frame logic is testable without an event loop.

use super::UIRunner;
use crate::app::VardaApp;
use crate::renderer::context::GpuContext;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

/// The parts of the winit event loop the per-frame render paths use.
///
/// `render_headless` and `render` touch `ActiveEventLoop` only to exit and to
/// let `VardaApp` reconcile output windows and monitors, which must happen on
/// the event-loop thread. The trait lets tests drive the render paths without
/// a real event loop, which cannot be constructed in a test.
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
        // `resumed` can be called more than once on some platforms.
        if self.varda.is_some() {
            return;
        }

        let startup_t0 = std::time::Instant::now();
        log::info!("[STARTUP] resumed() entered — beginning initialization");

        if self.config.headless {
            // Headless: GPU without a window surface, no egui.
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
            // Windowed: create the main window, then run GPU init on a background thread
            // and return from resumed() immediately. On macOS (especially under Rosetta or
            // on Intel), blocking the main thread during Metal device creation deadlocks
            // the GCD dispatch queue, because Metal dispatches work back to the main
            // queue. about_to_wait() polls the thread handle and finishes init.
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

            // Request a redraw so macOS marks the window live, which Metal/CALayer
            // requires (wgpu#5722).
            window_static.request_redraw();

            // Create the wgpu instance and surface on the main thread (macOS requires
            // NSView/CAMetalLayer access there), then create the adapter and device on a
            // background thread.
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

            // about_to_wait() completes initialization. Poll to keep the loop responsive.
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
                    if let Err(e) = varda.save_workspace() {
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
                    // No request_redraw() here; about_to_wait() schedules frames via WaitUntil.
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
        // Deferred GPU init: poll the background thread.
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
                // GPU init still running; keep the event loop alive.
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                return;
            }
        }

        // target_fps can change at runtime via UI or API.
        let target_fps = self
            .varda
            .as_ref()
            .map_or(self.config.target_fps, crate::app::VardaApp::target_fps);

        if self.config.headless {
            // Headless has no window, so no OS events; the loop schedules its own wake-up.
            if headless_frame_due(target_fps, self.cadence_anchor, std::time::Instant::now()) {
                self.render_headless(event_loop);
                self.advance_cadence_anchor(target_fps);
            }
            event_loop.set_control_flow(headless_wake(target_fps, self.cadence_anchor));
        } else {
            // Windowed: request_redraw only when the cadence anchor says a frame is due.
            // Between frames WaitUntil sleeps, avoiding CPU burn and burst-pause patterns.
            if target_fps > 0 {
                let now = std::time::Instant::now();
                let deadline = self.cadence_anchor.unwrap_or(now);

                if deadline > now {
                    // Not due yet: sleep until the deadline. Don't request_redraw; winit calls
                    // about_to_wait again when the timer fires.
                    event_loop
                        .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
                } else {
                    // At or past the deadline: render now.
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(now));
                    if let Some(w) = self.window {
                        w.request_redraw();
                    }
                }
            } else {
                // Uncapped: poll continuously.
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                if let Some(w) = self.window {
                    w.request_redraw();
                }
            }
        }
    }
}

/// Whether a headless frame is due: always when uncapped, otherwise once the
/// next frame's start time has passed.
fn headless_frame_due(
    target_fps: u32,
    next_frame: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    target_fps == 0 || next_frame.is_none_or(|due| due <= now)
}

/// When the headless loop wakes next. With no window, no OS event wakes it, so
/// it always names a time instead of `Wait`.
fn headless_wake(
    target_fps: u32,
    next_frame: Option<std::time::Instant>,
) -> winit::event_loop::ControlFlow {
    use winit::event_loop::ControlFlow;
    match next_frame {
        Some(due) if target_fps > 0 => ControlFlow::WaitUntil(due),
        _ => ControlFlow::Poll,
    }
}

#[cfg(test)]
mod tests {
    use super::{headless_frame_due, headless_wake};
    use std::time::{Duration, Instant};
    use winit::event_loop::ControlFlow;

    #[test]
    fn a_headless_frame_is_due_at_its_start_time() {
        let now = Instant::now();
        let later = now + Duration::from_millis(10);
        assert!(headless_frame_due(60, None, now));
        assert!(headless_frame_due(60, Some(now), now));
        assert!(!headless_frame_due(60, Some(later), now));
        assert!(
            headless_frame_due(0, Some(later), now),
            "uncapped runs every pass"
        );
    }

    #[test]
    fn the_headless_loop_always_names_its_next_wake() {
        let due = Instant::now() + Duration::from_millis(16);
        assert_eq!(headless_wake(60, Some(due)), ControlFlow::WaitUntil(due));
        assert_eq!(headless_wake(0, Some(due)), ControlFlow::Poll);
        assert_eq!(headless_wake(60, None), ControlFlow::Poll);
    }
}
