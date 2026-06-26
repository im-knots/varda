//! Servo-backed HTML rendering (feature `html`).
//!
//! See `/spec/html-source.md`. This module is the ONLY place the `servo`
//! dependency is referenced. It uses Servo's built-in [`SoftwareRenderingContext`]
//! and the CPU readback path ([`RenderingContext::read_to_image`]), which works on
//! all platforms without any native GPU-interop crate. The tradeoff is software-GL
//! rasterization (slower than GPU); zero-copy GPU import is a follow-up (see spec
//! Open Questions).
//!
//! NOTE: Servo is heavy to build. This backend MUST be validated with a real macOS
//! build (`cargo build --features html`) on both Apple Silicon and Intel before
//! release, per the project's universal-DMG rule.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use euclid::Box2D;
use servo::{
    RenderingContext, Servo, ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder,
};
use url::Url;
use winit::dpi::PhysicalSize;

/// A no-op event loop waker. We pump Servo synchronously each frame from the
/// render loop, so we do not need to wake an external event loop.
#[derive(Clone)]
struct NoopWaker;

impl servo::EventLoopWaker for NoopWaker {
    fn wake(&self) {}
    fn clone_box(&self) -> Box<dyn servo::EventLoopWaker> {
        Box::new(self.clone())
    }
}

/// One offscreen Servo WebView rendering into a software-GL surface.
pub struct ServoInstance {
    servo: Servo,
    webview: WebView,
    rendering_context: Rc<SoftwareRenderingContext>,
    size: PhysicalSize<u32>,
    /// Reusable RGBA scratch buffer to avoid per-frame allocation.
    scratch: RefCell<Vec<u8>>,
}

impl ServoInstance {
    /// Create an offscreen Servo instance loading `url` at `width`×`height`.
    pub fn new(url: &str, width: u32, height: u32) -> Result<Self> {
        let size = PhysicalSize::new(width, height);

        let rendering_context = Rc::new(
            SoftwareRenderingContext::new(size)
                .map_err(|e| anyhow!("SoftwareRenderingContext::new failed: {e:?}"))?,
        );
        rendering_context
            .make_current()
            .map_err(|e| anyhow!("make_current failed: {e:?}"))?;

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(NoopWaker))
            .build();

        let parsed = Url::parse(url).map_err(|e| anyhow!("invalid URL '{url}': {e}"))?;
        let webview = WebViewBuilder::new(&servo, rendering_context.clone())
            .url(parsed)
            .build();
        webview.focus();

        Ok(Self {
            servo,
            webview,
            rendering_context,
            size,
            scratch: RefCell::new(Vec::new()),
        })
    }

    /// Pump the Servo event loop once, paint the WebView, and read back the
    /// latest frame as RGBA bytes (`width*height*4`). Returns `None` until a
    /// frame is ready (e.g. before the first layout/paint completes).
    pub fn pump_and_read(&mut self) -> Option<Vec<u8>> {
        self.servo.spin_event_loop();
        self.webview.paint();

        // read_to_image reads the back buffer, so the just-painted frame is
        // returned without needing present().
        let rect = Box2D::from_size(self.rendering_context.size2d().to_i32());
        let image = self.rendering_context.read_to_image(rect)?;
        let (w, h) = (image.width(), image.height());
        if w != self.size.width || h != self.size.height {
            log::debug!(
                "HTML frame {}x{} != target {}x{}; skipping",
                w,
                h,
                self.size.width,
                self.size.height
            );
            return None;
        }

        let mut scratch = self.scratch.borrow_mut();
        scratch.clear();
        scratch.extend_from_slice(image.as_raw());
        Some(scratch.clone())
    }

    /// Navigate this WebView to a new URL.
    pub fn navigate(&mut self, url: &str) {
        match Url::parse(url) {
            Ok(parsed) => self.webview.load(parsed),
            Err(e) => log::error!("HTML navigate: invalid URL '{url}': {e}"),
        }
    }
}

impl Drop for ServoInstance {
    fn drop(&mut self) {
        // Servo 0.1.1 exposes no explicit shutdown; dropping the WebView handle
        // signals closure. Spin a bounded number of times so pending teardown
        // work flushes before the handles drop.
        for _ in 0..10 {
            self.servo.spin_event_loop();
        }
    }
}
