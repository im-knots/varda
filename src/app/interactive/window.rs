//! The interactive HTML window: a lightweight winit/wgpu window that blits a
//! deck's live HTML texture and carries the input state needed to forward
//! mouse/keyboard/scroll/IME into the offscreen Servo `WebView`.
//!
//! It is fixed 1:1 to the deck's WebView size and non-resizable (see
//! `/spec/html-source.md` §4), so the surface and the WebView share a device
//! pixel grid and cursor mapping is the identity (plus clamping). Modeled on
//! `OutputWindow` but display-only — it never joins `output.outputs`.

use anyhow::{Context, Result};
use winit::event::WindowEvent;
use winit::keyboard::ModifiersState;
use winit::window::Window;

use super::{input, InteractiveTarget};
use crate::html::HtmlInputEvent;
use crate::renderer::blit::BlitPipeline;
use crate::renderer::context::GpuContext;

/// One interactive window driving a single HTML instance.
pub(crate) struct InteractiveWindow {
    window: &'static Window,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    blit_pipeline: BlitPipeline,
    /// Deck/instance this window drives.
    pub(crate) target: InteractiveTarget,
    /// Last known cursor position in physical (== WebView device) pixels.
    pub(crate) cursor: (f64, f64),
    /// Currently-held modifier keys.
    pub(crate) modifiers: ModifiersState,
    /// True while an IME composition session is in progress.
    pub(crate) composing: bool,
}

impl InteractiveWindow {
    /// Create the window's surface + blit pipeline, sharing the engine device.
    pub(crate) fn new(
        context: &GpuContext,
        window: &'static Window,
        target: InteractiveTarget,
    ) -> Result<Self> {
        let size = window.inner_size();
        let surface = context
            .instance
            .create_surface(window)
            .context("Failed to create interactive surface")?;
        let caps = surface.get_capabilities(&context.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&context.device, &surface_config);
        let blit_pipeline = BlitPipeline::new(&context.device, surface_config.format)?;
        Ok(Self {
            window,
            surface,
            surface_config,
            blit_pipeline,
            target,
            cursor: (0.0, 0.0),
            modifiers: ModifiersState::empty(),
            composing: false,
        })
    }

    /// This window's winit id, for event routing.
    pub(crate) fn id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    /// WebView size in device pixels (for coordinate clamping).
    pub(crate) fn webview_size(&self) -> (u32, u32) {
        (self.target.width, self.target.height)
    }

    /// Translate a winit window event into input events for the WebView,
    /// updating tracked cursor/modifier/composition state. Returns the events to
    /// forward to the servo thread (usually 0 or 1). `CloseRequested` is handled
    /// by the caller; unhandled events return empty.
    pub(crate) fn process_event(&mut self, event: &WindowEvent) -> Vec<HtmlInputEvent> {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                let p = input::to_device_point(self.cursor, 1.0, self.webview_size());
                vec![input::mouse_move(p)]
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let p = input::to_device_point(self.cursor, 1.0, self.webview_size());
                vec![input::mouse_button(p, *button, *state)]
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let p = input::to_device_point(self.cursor, 1.0, self.webview_size());
                vec![input::wheel(p, *delta)]
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                vec![]
            }
            WindowEvent::KeyboardInput { event, .. } => {
                vec![input::keyboard(event, self.modifiers)]
            }
            WindowEvent::Ime(ime) => self.process_ime(ime),
            WindowEvent::Focused(focused) => vec![HtmlInputEvent::Focus(*focused)],
            _ => vec![],
        }
    }

    /// IME → composition events, tracking the start/update/end session lifecycle.
    fn process_ime(&mut self, ime: &winit::event::Ime) -> Vec<HtmlInputEvent> {
        use winit::event::Ime;
        match ime {
            Ime::Enabled => vec![],
            Ime::Preedit(text, _) if text.is_empty() => {
                if std::mem::take(&mut self.composing) {
                    vec![HtmlInputEvent::ImeDismissed]
                } else {
                    vec![]
                }
            }
            Ime::Preedit(text, _) => {
                let start = !self.composing;
                self.composing = true;
                vec![input::ime_preedit(text.clone(), start)]
            }
            Ime::Commit(text) => {
                self.composing = false;
                vec![input::ime_commit(text.clone())]
            }
            Ime::Disabled => {
                if std::mem::take(&mut self.composing) {
                    vec![HtmlInputEvent::ImeDismissed]
                } else {
                    vec![]
                }
            }
        }
    }

    /// Blit the current HTML texture into the window surface (1:1 fullscreen).
    pub(crate) fn render(&self, context: &GpuContext, html_view: &wgpu::TextureView) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface
                    .configure(&context.device, &self.surface_config);
                return;
            }
            other => {
                log::debug!("Interactive surface unavailable: {other:?}");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self
            .blit_pipeline
            .create_bind_group(&context.device, html_view);
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Interactive Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Interactive Blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.blit_pipeline.render(&mut pass, &bind_group);
        }
        context.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.window.request_redraw();
    }

    /// Close the OS window and reclaim the leaked `Box<Window>` (mirrors
    /// `OutputWindow::destroy`). Sole owner after removal from app state.
    pub(crate) fn destroy(self) {
        let window_ptr = self.window as *const Window as *mut Window;
        drop(self.surface);
        unsafe {
            let _ = Box::from_raw(window_ptr);
        }
    }
}
