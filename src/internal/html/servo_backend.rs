//! Servo-backed HTML rendering (feature `html`).
//!
//! See `/spec/html-source.md` ("Off-Thread Servo Rendering", AGREED). This module
//! is the ONLY place the `servo` dependency is referenced. A single dedicated
//! "html-servo" thread constructs and owns one [`Servo`] instance plus all its
//! [`WebView`]s and [`SoftwareRenderingContext`]s — the `!Send` Servo handles
//! never cross a thread boundary. The render thread talks to it only via `Send`
//! data: [`HtmlCommand`]s in and finished RGBA frames out through per-instance
//! [`FrameSlot`]s, mirroring the NDI/stream off-thread pattern.
//!
//! Rendering is software-GL rasterization + CPU readback ([`RenderingContext::read_to_image`]),
//! which works on all platforms without a native GPU-interop crate. Zero-copy GPU
//! import is a follow-up (see spec Open Questions).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle, Thread};
use std::time::Duration;

use anyhow::{anyhow, Result};
use euclid::Box2D;
use servo::{
    DevicePoint, DeviceVector2D, ImeEvent, InputEvent, KeyboardEvent, LoadStatus, MouseButton,
    MouseButtonAction, MouseButtonEvent, MouseMoveEvent, Preferences, RenderingContext, Scroll,
    Servo, ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
    WebViewPoint, WebViewVector, WheelDelta, WheelEvent, WheelMode,
};
use url::Url;
use winit::dpi::PhysicalSize;

use super::{FrameSlot, HtmlFrame, HtmlId, HtmlInputEvent};

/// Cadence when at least one WebView is loading/animating/has a new frame.
const ACTIVE_PARK: Duration = Duration::from_millis(8);
/// Idle cadence (safety net); the waker unparks the thread on Servo events.
const IDLE_PARK: Duration = Duration::from_millis(100);

/// Wakes the pump thread from Servo's event loop by unparking it.
struct UnparkWaker(Thread);

impl servo::EventLoopWaker for UnparkWaker {
    fn wake(&self) {
        self.0.unpark();
    }
    fn clone_box(&self) -> Box<dyn servo::EventLoopWaker> {
        Box::new(UnparkWaker(self.0.clone()))
    }
}

/// Per-WebView delegate that flags when Servo has a fresh frame to paint.
struct FrameReadyDelegate {
    ready: Rc<Cell<bool>>,
}

impl WebViewDelegate for FrameReadyDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.ready.set(true);
    }
}

/// Commands from the render thread to the owning servo thread.
enum HtmlCommand {
    Start {
        id: HtmlId,
        url: String,
        width: u32,
        height: u32,
        slot: FrameSlot,
    },
    Navigate {
        id: HtmlId,
        url: String,
    },
    Reload {
        id: HtmlId,
    },
    Input {
        id: HtmlId,
        event: HtmlInputEvent,
    },
    Stop {
        id: HtmlId,
    },
    Shutdown,
}

/// Handle to the single shared servo pump thread.
pub struct ServoEngine {
    sender: Sender<HtmlCommand>,
    thread: Option<JoinHandle<()>>,
}

impl ServoEngine {
    /// Spawn the owning servo thread. Servo is constructed lazily on the thread.
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("html-servo".into())
            .spawn(move || run_servo_thread(receiver))
            .ok();
        Self { sender, thread }
    }

    pub fn start(&self, id: HtmlId, url: &str, width: u32, height: u32, slot: FrameSlot) {
        let _ = self.sender.send(HtmlCommand::Start {
            id,
            url: url.to_string(),
            width,
            height,
            slot,
        });
        self.unpark();
    }

    pub fn navigate(&self, id: HtmlId, url: &str) {
        let _ = self.sender.send(HtmlCommand::Navigate {
            id,
            url: url.to_string(),
        });
        self.unpark();
    }

    pub fn reload(&self, id: HtmlId) {
        let _ = self.sender.send(HtmlCommand::Reload { id });
        self.unpark();
    }

    /// Forward an interactive-mode input event to the WebView `id`.
    pub fn send_input(&self, id: HtmlId, event: HtmlInputEvent) {
        let _ = self.sender.send(HtmlCommand::Input { id, event });
        self.unpark();
    }

    pub fn stop(&self, id: HtmlId) {
        let _ = self.sender.send(HtmlCommand::Stop { id });
        self.unpark();
    }

    /// Wake the pump thread so a freshly queued command is applied promptly.
    fn unpark(&self) {
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }
}

impl Drop for ServoEngine {
    fn drop(&mut self) {
        let _ = self.sender.send(HtmlCommand::Shutdown);
        self.unpark();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One offscreen WebView owned by the servo thread, rendering into its own
/// software-GL surface and publishing finished frames into a shared [`FrameSlot`].
struct Entry {
    webview: WebView,
    rendering_context: Rc<SoftwareRenderingContext>,
    slot: FrameSlot,
    width: u32,
    height: u32,
    ready: Rc<Cell<bool>>,
}

impl Entry {
    /// Paint the WebView and read back the latest frame as RGBA bytes. Returns
    /// `None` until a correctly-sized frame is available.
    fn paint_and_read(&self) -> Option<HtmlFrame> {
        if let Err(e) = self.rendering_context.make_current() {
            log::debug!("HTML make_current failed: {e:?}");
            return None;
        }
        self.webview.paint();

        let rect = Box2D::from_size(self.rendering_context.size2d().to_i32());
        let image = self.rendering_context.read_to_image(rect)?;
        let (w, h) = (image.width(), image.height());
        if w != self.width || h != self.height {
            return None;
        }
        Some(HtmlFrame {
            data: image.as_raw().clone(),
            width: w,
            height: h,
        })
    }
}

/// Construct a WebView + software surface for `url` on the servo thread.
fn create_entry(
    servo: &Servo,
    url: &str,
    width: u32,
    height: u32,
    slot: FrameSlot,
) -> Result<Entry> {
    let width = width.max(1);
    let height = height.max(1);
    let size = PhysicalSize::new(width, height);

    let rendering_context = Rc::new(
        SoftwareRenderingContext::new(size)
            .map_err(|e| anyhow!("SoftwareRenderingContext::new failed: {e:?}"))?,
    );
    rendering_context
        .make_current()
        .map_err(|e| anyhow!("make_current failed: {e:?}"))?;

    let parsed = Url::parse(url).map_err(|e| anyhow!("invalid URL '{url}': {e}"))?;
    // Start "ready" so the first frame paints before any frame-ready notification.
    let ready = Rc::new(Cell::new(true));
    let delegate: Rc<dyn WebViewDelegate> = Rc::new(FrameReadyDelegate {
        ready: ready.clone(),
    });
    let webview = WebViewBuilder::new(servo, rendering_context.clone())
        .delegate(delegate)
        .url(parsed)
        .build();
    webview.focus();

    Ok(Entry {
        webview,
        rendering_context,
        slot,
        width,
        height,
        ready,
    })
}

/// A WebView point in device pixels.
fn device_point(x: f32, y: f32) -> WebViewPoint {
    WebViewPoint::Device(DevicePoint::new(x, y))
}

/// Translate a `Send` [`HtmlInputEvent`] into the corresponding Servo input and
/// apply it to `entry`'s WebView, flagging it for a fresh paint. The WebView is
/// only ever touched here, on its owning thread.
fn apply_input(entry: &Entry, event: HtmlInputEvent) {
    let wv = &entry.webview;
    match event {
        HtmlInputEvent::MouseMove { x, y } => {
            let _ = wv.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                device_point(x, y),
            )));
        }
        HtmlInputEvent::MouseButton {
            x,
            y,
            button,
            pressed,
        } => {
            let action = if pressed {
                MouseButtonAction::Down
            } else {
                MouseButtonAction::Up
            };
            let event =
                MouseButtonEvent::new(action, MouseButton::from(button as u64), device_point(x, y));
            let _ = wv.notify_input_event(InputEvent::MouseButton(event));
        }
        HtmlInputEvent::Wheel { x, y, dx, dy } => {
            let delta = WheelDelta {
                x: dx,
                y: dy,
                z: 0.0,
                mode: WheelMode::DeltaPixel,
            };
            let _ = wv.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                delta,
                device_point(x, y),
            )));
        }
        HtmlInputEvent::Scroll { x, y, dx, dy } => {
            let vector = WebViewVector::Device(DeviceVector2D::new(dx as f32, dy as f32));
            wv.notify_scroll_event(Scroll::Delta(vector), device_point(x, y));
        }
        HtmlInputEvent::Key(kt) => {
            let _ = wv.notify_input_event(InputEvent::Keyboard(KeyboardEvent::new(kt)));
        }
        HtmlInputEvent::Ime(comp) => {
            let _ = wv.notify_input_event(InputEvent::Ime(ImeEvent::Composition(comp)));
        }
        HtmlInputEvent::ImeDismissed => {
            let _ = wv.notify_input_event(InputEvent::Ime(ImeEvent::Dismissed));
        }
        HtmlInputEvent::Focus(true) => wv.focus(),
        HtmlInputEvent::Focus(false) => wv.blur(),
    }
    entry.ready.set(true);
}

/// The owning servo thread: builds Servo, applies commands, and pumps WebViews,
/// publishing the latest frame for each into its shared slot (latest-wins).
fn run_servo_thread(rx: Receiver<HtmlCommand>) {
    let waker = UnparkWaker(thread::current());
    // Clear the Servo viewport to fully transparent instead of the default opaque
    // white, so pages with a transparent html/body yield alpha=0 pixels in
    // read_to_image. This is Blocker 1 for per-deck transparency (/spec/html-source.md §2).
    let preferences = Preferences {
        shell_background_color_rgba: [0.0, 0.0, 0.0, 0.0],
        ..Preferences::default()
    };
    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(waker))
        .preferences(preferences)
        .build();
    let mut entries: HashMap<HtmlId, Entry> = HashMap::new();

    'main: loop {
        // 1) Drain queued commands.
        loop {
            match rx.try_recv() {
                Ok(HtmlCommand::Start {
                    id,
                    url,
                    width,
                    height,
                    slot,
                }) => match create_entry(&servo, &url, width, height, slot) {
                    Ok(entry) => {
                        log::info!("HTML instance {id} started for '{url}' ({width}x{height})");
                        entries.insert(id, entry);
                    }
                    Err(e) => log::error!("Servo init failed for '{url}': {e}"),
                },
                Ok(HtmlCommand::Navigate { id, url }) => {
                    if let Some(entry) = entries.get_mut(&id) {
                        match Url::parse(&url) {
                            Ok(parsed) => {
                                entry.webview.load(parsed);
                                entry.ready.set(true);
                            }
                            Err(e) => log::error!("HTML navigate: invalid URL '{url}': {e}"),
                        }
                    }
                }
                Ok(HtmlCommand::Reload { id }) => {
                    if let Some(entry) = entries.get_mut(&id) {
                        entry.webview.reload();
                        entry.ready.set(true);
                    }
                }
                Ok(HtmlCommand::Input { id, event }) => {
                    if let Some(entry) = entries.get(&id) {
                        apply_input(entry, event);
                    }
                }
                Ok(HtmlCommand::Stop { id }) => {
                    entries.remove(&id);
                }
                Ok(HtmlCommand::Shutdown) => break 'main,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break 'main,
            }
        }

        // 2) Pump Servo once, then paint/publish the WebViews that need it.
        servo.spin_event_loop();
        let mut active = false;
        for entry in entries.values() {
            let loading = entry.webview.load_status() != LoadStatus::Complete;
            let animating = entry.webview.clone().animating();
            let frame_ready = entry.ready.replace(false);
            if loading || animating || frame_ready {
                active = true;
                if let Some(frame) = entry.paint_and_read() {
                    if let Ok(mut guard) = entry.slot.lock() {
                        *guard = Some(frame);
                    }
                }
            }
        }

        // 3) Sleep to cadence; the waker unparks early on Servo events.
        thread::park_timeout(if active { ACTIVE_PARK } else { IDLE_PARK });
    }

    // Drop WebViews, then spin so pending teardown flushes before Servo drops.
    entries.clear();
    for _ in 0..10 {
        servo.spin_event_loop();
    }
}

/// Off-thread rendering regression test (promoted from the threading spike, see
/// `/spec/html-source.md`). Drives a real `ServoEngine` — Servo lives entirely on
/// the spawned "html-servo" thread — and confirms a page renders by polling the
/// shared frame slot from the test (render-thread) side.
///
/// `#[ignore]` (starts a real Servo engine, several seconds). Run with:
///   cargo test --features html servo_renders_on_background_thread -- --ignored --test-threads=1
#[cfg(test)]
mod offthread_spike {
    use super::*;
    use base64::Engine as _;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    const W: u32 = 320;
    const H: u32 = 240;

    fn data_url(html: &str) -> String {
        let b64 = base64::engine::general_purpose::STANDARD.encode(html.as_bytes());
        format!("data:text/html;base64,{b64}")
    }

    #[test]
    #[ignore = "spike: starts a real Servo engine on a background thread; run with --ignored --test-threads=1"]
    fn servo_renders_on_background_thread() {
        let url = data_url("<!doctype html><html><body bgcolor=\"red\"></body></html>");
        let engine = ServoEngine::new();
        let slot: FrameSlot = Arc::new(Mutex::new(None));
        engine.start(1, &url, W, H, slot.clone());

        let start = Instant::now();
        let mut found = None;
        while start.elapsed() < Duration::from_secs(30) {
            if let Some(frame) = slot.lock().unwrap().take() {
                let off = ((H / 2) * W * 4 + (W / 2) * 4) as usize;
                let px = [
                    frame.data[off],
                    frame.data[off + 1],
                    frame.data[off + 2],
                    frame.data[off + 3],
                ];
                if px[0] > 200 && px[1] < 60 && px[2] < 60 && px[3] > 200 {
                    found = Some(px);
                    break;
                }
            }
            thread::sleep(Duration::from_millis(16));
        }

        let px = found.expect("off-thread Servo did not render red within timeout");
        assert!(
            px[0] > 200 && px[1] < 60 && px[2] < 60 && px[3] > 200,
            "off-thread Servo did not render red; got {px:?}"
        );
    }
}
