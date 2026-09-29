//! Servo-backed HTML rendering (feature `html`).
//!
//! The only module that references `servo`. One "html-servo" thread creates and
//! holds the [`Servo`] instance and all [`WebView`]s and
//! [`SoftwareRenderingContext`]s, because those handles are `!Send`. The render
//! thread sends [`HtmlCommand`]s and reads RGBA frames from per-instance
//! [`FrameSlot`]s.
//!
//! Frames are rasterized with software GL and read back to the CPU
//! ([`RenderingContext::read_to_image`]), which works on every platform.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle, Thread};
use std::time::Duration;

use anyhow::{Result, anyhow};
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

/// Park time while any `WebView` is loading, animating or has a new frame.
const ACTIVE_PARK: Duration = Duration::from_millis(8);
/// Idle park time; the waker unparks the thread on Servo events.
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

/// Commands from the render thread to the servo thread.
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

/// Handle to the shared servo thread.
pub struct ServoEngine {
    sender: Sender<HtmlCommand>,
    thread: Option<JoinHandle<()>>,
}

impl ServoEngine {
    /// Spawns the servo thread. Servo is constructed on that thread.
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("html-servo".into())
            .spawn(move || run_servo_thread(&receiver))
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

    /// Forwards an interactive-mode input event to `WebView` `id`.
    pub fn send_input(&self, id: HtmlId, event: HtmlInputEvent) {
        let _ = self.sender.send(HtmlCommand::Input { id, event });
        self.unpark();
    }

    pub fn stop(&self, id: HtmlId) {
        let _ = self.sender.send(HtmlCommand::Stop { id });
        self.unpark();
    }

    /// Wakes the servo thread so a queued command applies promptly.
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

/// One offscreen `WebView` on the servo thread, with its own software-GL
/// surface and [`FrameSlot`].
struct Entry {
    webview: WebView,
    rendering_context: Rc<SoftwareRenderingContext>,
    slot: FrameSlot,
    width: u32,
    height: u32,
    ready: Rc<Cell<bool>>,
}

impl Entry {
    /// Paints the `WebView` and reads the frame back as RGBA. Returns `None`
    /// until a frame of the right size is available.
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

/// Creates a `WebView` and software surface for `url` on the servo thread.
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
    // Start ready so the first frame paints without waiting for a notification.
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

/// A `WebView` point in device pixels.
fn device_point(x: f32, y: f32) -> WebViewPoint {
    WebViewPoint::Device(DevicePoint::new(x, y))
}

/// Converts an [`HtmlInputEvent`] to a Servo input, applies it to `entry`'s
/// `WebView` and flags it for repaint.
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
            let event = MouseButtonEvent::new(
                action,
                MouseButton::from(u64::from(button)),
                device_point(x, y),
            );
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

/// Servo thread loop: applies commands, pumps Servo and publishes the latest
/// frame of each `WebView` to its slot.
fn run_servo_thread(rx: &Receiver<HtmlCommand>) {
    let waker = UnparkWaker(thread::current());
    // Transparent viewport instead of opaque white, so transparent pages read
    // back with alpha 0.
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
        // Drain queued commands.
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
                Ok(HtmlCommand::Shutdown) | Err(TryRecvError::Disconnected) => break 'main,
                Err(TryRecvError::Empty) => break,
            }
        }

        // Pump Servo once, then paint the WebViews that need it.
        servo.spin_event_loop();
        let mut active = false;
        for entry in entries.values() {
            let loading = entry.webview.load_status() != LoadStatus::Complete;
            let animating = entry.webview.clone().animating();
            let frame_ready = entry.ready.replace(false);
            if loading || animating || frame_ready {
                active = true;
                if let Some(frame) = entry.paint_and_read()
                    && let Ok(mut guard) = entry.slot.lock()
                {
                    *guard = Some(frame);
                }
            }
        }

        // The waker unparks early on Servo events.
        thread::park_timeout(if active { ACTIVE_PARK } else { IDLE_PARK });
    }

    // Drop WebViews, then spin so pending teardown flushes before Servo drops.
    entries.clear();
    for _ in 0..10 {
        servo.spin_event_loop();
    }
}

/// Renders a page with a real `ServoEngine` on the "html-servo" thread and
/// polls the frame slot from the test thread.
///
/// Ignored because it starts Servo (several seconds). Run with:
///   cargo test --features html `servo_renders_on_background_thread` -- --ignored --test-threads=1
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
