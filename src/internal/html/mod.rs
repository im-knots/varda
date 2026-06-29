//! HTML deck source — offscreen web rendering via Servo → wgpu texture.
//!
//! See `/spec/html-source.md` for the full design.
//!
//! The integration surface (`HtmlManager` and its public API) always compiles
//! so the deck/persistence/UI/API plumbing is testable without pulling in the
//! heavy Servo dependency. The Servo-backed rendering is gated behind the
//! `html` cargo feature (`servo_backend`); when the feature is disabled the
//! manager still allocates a texture per instance but produces a blank frame.

#[cfg(feature = "html")]
mod servo_backend;

use std::sync::{Arc, Mutex};

/// Default render resolution for an HTML instance when none is supplied.
const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;

/// Stable opaque identifier for an HTML render instance. Addresses the owning
/// servo thread's WebViews independently of render-side `Vec` ordering.
type HtmlId = u64;

/// A finished RGBA frame (`width*height*4`) published from the servo thread.
struct HtmlFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

/// Latest-frame slot shared between the servo thread (writer) and the render
/// thread (reader), mirroring the NDI/stream `Arc<Mutex<Option<…>>>` pattern.
type FrameSlot = Arc<Mutex<Option<HtmlFrame>>>;

/// A single interactive-mode input event forwarded from an interactive window to
/// the owning servo thread (feature `html`). Carries only `Send` data plus
/// `keyboard_types` values; the winit→here translation lives in the app layer so
/// winit never reaches this module, preserving the "only `Send` data crosses the
/// thread boundary" contract. Positions are WebView **device** pixels. See
/// `/spec/html-source.md` §4.
#[cfg(feature = "html")]
#[derive(Debug, Clone)]
pub enum HtmlInputEvent {
    /// Pointer moved to a WebView device-pixel position.
    MouseMove { x: f32, y: f32 },
    /// Mouse button pressed/released. `button`: 0=left, 1=middle, 2=right.
    MouseButton {
        x: f32,
        y: f32,
        button: u16,
        pressed: bool,
    },
    /// Wheel delta (DOM `wheel` event) in device pixels.
    Wheel { x: f32, y: f32, dx: f64, dy: f64 },
    /// Scroll the page by a device-pixel delta at a position.
    Scroll { x: f32, y: f32, dx: f64, dy: f64 },
    /// A translated keyboard event.
    Key(keyboard_types::KeyboardEvent),
    /// An IME composition update (preedit start/update or commit end).
    Ime(keyboard_types::CompositionEvent),
    /// IME composition dismissed/cancelled.
    ImeDismissed,
    /// Window focus gained/lost → `WebView::focus()`/`blur()`.
    Focus(bool),
}

/// Manages offscreen HTML render instances and their destination GPU textures.
///
/// Mirrors the shape of `NdiManager` / `StreamManager`: the render loop calls
/// [`HtmlManager::update`] each frame and looks up [`HtmlManager::texture_view`]
/// for compositing.
pub struct HtmlManager {
    instances: Vec<HtmlInstance>,
    disabled: bool,
    next_id: HtmlId,
    /// The single shared servo pump thread, spawned lazily on first render.
    #[cfg(feature = "html")]
    engine: Option<servo_backend::ServoEngine>,
}

/// A single HTML render target: the wgpu texture frames are uploaded into, plus
/// the shared slot the servo thread publishes finished frames to.
struct HtmlInstance {
    url: String,
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// True once at least one frame (or the placeholder) has been written.
    initialized: bool,
    /// Stable id addressing this instance on the servo thread.
    #[cfg_attr(not(feature = "html"), allow(dead_code))]
    id: HtmlId,
    /// Latest frame published by the servo thread (never filled when the `html`
    /// feature is off → placeholder is shown).
    frame: FrameSlot,
}

impl Default for HtmlManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlManager {
    /// Create an active manager. Servo itself is initialized lazily per instance.
    pub fn new() -> Self {
        Self {
            instances: Vec::new(),
            disabled: false,
            next_id: 0,
            #[cfg(feature = "html")]
            engine: None,
        }
    }

    /// Create a no-op manager (for the `--no-html` CLI flag). `start_render`
    /// always returns `None`.
    pub fn new_disabled() -> Self {
        log::info!("HTML source manager disabled");
        Self {
            instances: Vec::new(),
            disabled: true,
            next_id: 0,
            #[cfg(feature = "html")]
            engine: None,
        }
    }

    /// Whether HTML rendering is available (feature enabled and not disabled).
    pub fn is_available(&self) -> bool {
        !self.disabled && cfg!(feature = "html")
    }

    /// Start rendering `url` at `width`×`height`. Returns the instance index, or
    /// reuses an existing instance for the same URL. Returns `None` when the
    /// manager is disabled.
    pub fn start_render(
        &mut self,
        url: &str,
        width: u32,
        height: u32,
        device: &wgpu::Device,
    ) -> Option<usize> {
        if self.disabled {
            log::warn!("HTML manager disabled; cannot render '{}'", url);
            return None;
        }

        if let Some(idx) = self.instances.iter().position(|i| i.url == url) {
            log::info!("Reusing existing HTML instance {} for '{}'", idx, url);
            return Some(idx);
        }

        let width = if width == 0 { DEFAULT_WIDTH } else { width };
        let height = if height == 0 { DEFAULT_HEIGHT } else { height };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("html-{}", url)),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            // COPY_SRC lets the rendered frame be read back to CPU (deck smoke
            // tests; future thumbnail/snapshot of HTML decks).
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let id = self.next_id;
        self.next_id += 1;
        let frame: FrameSlot = Arc::new(Mutex::new(None));

        // Spawn the shared servo thread on first use, then start this instance.
        #[cfg(feature = "html")]
        {
            let engine = self
                .engine
                .get_or_insert_with(servo_backend::ServoEngine::new);
            engine.start(id, url, width, height, frame.clone());
        }

        self.instances.push(HtmlInstance {
            url: url.to_string(),
            width,
            height,
            texture,
            view,
            initialized: false,
            id,
            frame,
        });
        log::info!(
            "HTML instance {} started for '{}' ({}x{})",
            self.instances.len() - 1,
            url,
            width,
            height
        );
        Some(self.instances.len() - 1)
    }

    /// Per-frame: pump each Servo instance and upload its latest frame. When the
    /// `html` feature is off this writes a one-time placeholder so the deck is
    /// not invisible.
    pub fn update(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue) {
        for instance in &mut self.instances {
            // Non-blocking poll of the latest frame published by the servo thread
            // (latest-wins, identical to the NDI/stream sources).
            let frame = instance.frame.try_lock().ok().and_then(|mut g| g.take());

            if let Some(frame) = frame {
                if frame.width == instance.width && frame.height == instance.height {
                    Self::upload(queue, instance, &frame.data);
                    instance.initialized = true;
                }
            } else if !instance.initialized {
                let placeholder = placeholder_frame(instance.width, instance.height);
                Self::upload(queue, instance, &placeholder);
                instance.initialized = true;
            }
        }
    }

    /// Upload an RGBA byte buffer (`width*height*4`) into the instance texture.
    fn upload(queue: &wgpu::Queue, instance: &HtmlInstance, rgba: &[u8]) {
        let expected = (instance.width * instance.height * 4) as usize;
        if rgba.len() < expected {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &instance.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba[..expected],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(instance.width * 4),
                rows_per_image: Some(instance.height),
            },
            wgpu::Extent3d {
                width: instance.width,
                height: instance.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Texture view for compositing the instance at `idx`.
    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.instances.get(idx).map(|i| &i.view)
    }

    /// Render dimensions of the instance at `idx`.
    pub fn instance_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.instances.get(idx).map(|i| (i.width, i.height))
    }

    /// URL of the instance at `idx`.
    pub fn instance_url(&self, idx: usize) -> Option<&str> {
        self.instances.get(idx).map(|i| i.url.as_str())
    }

    /// Number of active instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Navigate an existing instance to a new URL.
    pub fn navigate(&mut self, idx: usize, url: &str) {
        if let Some(instance) = self.instances.get_mut(idx) {
            instance.url = url.to_string();
            instance.initialized = false;
            #[cfg(feature = "html")]
            if let Some(engine) = self.engine.as_ref() {
                engine.navigate(instance.id, url);
            }
        }
    }

    /// Reload an existing instance, re-fetching its current URL.
    pub fn reload(&mut self, idx: usize) {
        if let Some(instance) = self.instances.get_mut(idx) {
            instance.initialized = false;
            #[cfg(feature = "html")]
            if let Some(engine) = self.engine.as_ref() {
                engine.reload(instance.id);
            }
        }
    }

    /// Forward an interactive-mode input event to the instance at `idx`.
    /// No-op for an unknown index or when no engine has been spawned.
    #[cfg(feature = "html")]
    pub fn send_input(&self, idx: usize, event: HtmlInputEvent) {
        if let (Some(instance), Some(engine)) = (self.instances.get(idx), self.engine.as_ref()) {
            engine.send_input(instance.id, event);
        }
    }

    /// Stop and drop the instance at `idx`. Note: indices of later instances are
    /// not preserved — callers that hold indices should treat this as teardown.
    pub fn stop_render(&mut self, idx: usize) {
        if idx < self.instances.len() {
            #[cfg(feature = "html")]
            {
                let id = self.instances[idx].id;
                if let Some(engine) = self.engine.as_ref() {
                    engine.stop(id);
                }
            }
            self.instances.remove(idx);
        }
    }
}

/// A fully transparent placeholder used when no frame is available. Transparent
/// (rather than opaque dark-gray) so that on a deck flagged `transparent` the
/// pre-first-frame window shows through; an unflagged deck composites it over the
/// opaque black base clear, yielding black (see /spec/html-source.md §2).
fn placeholder_frame(width: u32, height: u32) -> Vec<u8> {
    vec![0u8; (width * height * 4) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_manager_new_no_crash() {
        let mgr = HtmlManager::new();
        assert_eq!(mgr.instance_count(), 0);
    }

    #[test]
    fn html_manager_disabled_not_available() {
        let mgr = HtmlManager::new_disabled();
        assert!(!mgr.is_available());
        assert_eq!(mgr.instance_count(), 0);
    }

    #[test]
    fn html_manager_lookup_out_of_bounds() {
        let mgr = HtmlManager::new();
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.instance_dimensions(0).is_none());
        assert!(mgr.instance_url(0).is_none());
        assert!(mgr.instance_url(999).is_none());
    }

    #[test]
    fn reload_out_of_range_is_noop() {
        // Reloading a non-existent instance must not panic (no engine spawned).
        let mut mgr = HtmlManager::new();
        mgr.reload(0);
        mgr.reload(999);
        let mut disabled = HtmlManager::new_disabled();
        disabled.reload(0);
    }

    #[cfg(feature = "html")]
    #[test]
    fn send_input_out_of_range_is_noop() {
        // Forwarding input to a non-existent instance must not panic (no engine).
        let mgr = HtmlManager::new();
        mgr.send_input(0, HtmlInputEvent::MouseMove { x: 1.0, y: 2.0 });
        mgr.send_input(999, HtmlInputEvent::Focus(true));
        let disabled = HtmlManager::new_disabled();
        disabled.send_input(0, HtmlInputEvent::Focus(false));
    }

    #[test]
    fn placeholder_frame_is_transparent_and_sized() {
        let buf = placeholder_frame(4, 2);
        assert_eq!(buf.len(), 4 * 2 * 4);
        // Fully transparent (all channels 0) so a `transparent` deck shows through
        // before its first Servo frame; see /spec/html-source.md §2.
        assert!(buf.iter().all(|&b| b == 0));
    }
}

/// True end-to-end smoke tests for the HTML deck path (feature `html`).
///
/// These drive a real Servo instance through `HtmlManager` (the deck source),
/// render into the GPU texture, then read the pixels back to verify content.
/// They are `#[ignore]` because each starts a full Servo engine (heavy, several
/// seconds). Run explicitly with:
///   cargo test html_deck_smoke -- --ignored --test-threads=1
#[cfg(all(test, feature = "html"))]
mod smoke_tests {
    use super::*;
    use base64::Engine as _;
    use std::time::{Duration, Instant};

    use crate::renderer::context::GpuContext;

    const W: u32 = 320; // 320*4 = 1280 bytes/row, already 256-aligned (no padding)
    const H: u32 = 240;
    const ROW_BYTES: u32 = W * 4;

    /// Wrap an HTML document in a base64 `data:` URL (avoids percent-encoding).
    fn data_url(html: &str) -> String {
        let b64 = base64::engine::general_purpose::STANDARD.encode(html.as_bytes());
        format!("data:text/html;base64,{b64}")
    }

    /// Read the center pixel of an instance's texture back to CPU.
    fn center_pixel(gpu: &GpuContext, mgr: &HtmlManager, idx: usize) -> [u8; 4] {
        let texture = &mgr.instances[idx].texture;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("html-smoke-readback"),
            size: (ROW_BYTES * H) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));

        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();

        let data = buffer.slice(..).get_mapped_range();
        let off = ((H / 2) * ROW_BYTES + (W / 2) * 4) as usize;
        let px = [data[off], data[off + 1], data[off + 2], data[off + 3]];
        drop(data);
        buffer.unmap();
        px
    }

    /// True when `px` is a solid version of `want` (each channel near 0 or 255).
    fn is_color(px: [u8; 4], want: [u8; 3]) -> bool {
        px[3] > 200
            && (0..3).all(|i| {
                if want[i] >= 200 {
                    px[i] >= 200
                } else {
                    px[i] <= 60
                }
            })
    }

    /// Pump the deck each frame until its center pixel matches `want` (or timeout).
    fn pump_until(
        gpu: &GpuContext,
        mgr: &mut HtmlManager,
        idx: usize,
        want: [u8; 3],
        timeout: Duration,
    ) -> [u8; 4] {
        let start = Instant::now();
        let mut last = [0u8; 4];
        while start.elapsed() < timeout {
            mgr.update(&gpu.device, &gpu.queue);
            last = center_pixel(gpu, mgr, idx);
            if is_color(last, want) {
                return last;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        last
    }

    #[test]
    #[ignore = "heavy: starts a real Servo engine; run with --ignored --test-threads=1"]
    fn html_deck_smoke_renders_plain_and_css_js() {
        let Ok(gpu) = GpuContext::new_headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut mgr = HtmlManager::new();

        // 1) Plain HTML: the body background propagates to the viewport.
        let red = data_url("<!doctype html><html><body bgcolor=\"red\"></body></html>");
        let idx = mgr
            .start_render(&red, W, H, &gpu.device)
            .expect("start_render returned None with the html feature enabled");
        let px = pump_until(&gpu, &mut mgr, idx, [255, 0, 0], Duration::from_secs(30));
        assert!(
            is_color(px, [255, 0, 0]),
            "plain HTML deck did not render red; got {px:?}"
        );

        // 2) HTML + CSS + JS: CSS paints black, then JS overrides to blue.
        //    Asserting blue proves both CSS parsing and JS execution in the deck.
        let css_js = "<!doctype html><html><head><style>html,body{height:100%;margin:0}body{background:#000}</style></head><body><script>document.body.style.background='rgb(0,0,255)';</script></body></html>";
        mgr.navigate(idx, &data_url(css_js));
        let px2 = pump_until(&gpu, &mut mgr, idx, [0, 0, 255], Duration::from_secs(30));
        assert!(
            is_color(px2, [0, 0, 255]),
            "HTML+CSS+JS deck did not render blue (CSS/JS not applied); got {px2:?}"
        );
    }

    /// Idle-repaint correctness: a `setInterval` timer mutates the DOM *after*
    /// the initial load completes — driving neither `requestAnimationFrame` nor a
    /// CSS animation, so `animating()` stays false. This is the gating caveat
    /// noted in `/spec/html-source.md`: the off-thread engine must still repaint
    /// via the `frame_ready` path (`notify_new_frame_ready` + the event-loop
    /// waker unparking the pump thread). Reaching blue proves a timer-driven
    /// update on a settled page is not dropped.
    #[test]
    #[ignore = "heavy: starts a real Servo engine; run with --ignored --test-threads=1"]
    fn html_deck_setinterval_idle_repaint() {
        let Ok(gpu) = GpuContext::new_headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut mgr = HtmlManager::new();

        // Paints black at load, then a setInterval timer flips to blue on its
        // first tick (~120ms after load) — no rAF, no CSS animation/transition.
        let idle = "<!doctype html><html><head><style>html,body{height:100%;margin:0}\
body{background:#000}</style></head><body><script>var done=false;\
setInterval(function(){if(!done){document.body.style.background='rgb(0,0,255)';\
done=true;}},120);</script></body></html>";
        let idx = mgr
            .start_render(&data_url(idle), W, H, &gpu.device)
            .expect("start_render returned None with the html feature enabled");
        let px = pump_until(&gpu, &mut mgr, idx, [0, 0, 255], Duration::from_secs(30));
        assert!(
            is_color(px, [0, 0, 255]),
            "setInterval idle DOM update was not repainted; got {px:?}"
        );
    }

    /// Reload correctness: after a page has rendered, `reload()` re-fetches the
    /// current URL and re-executes its JS, repainting the same content without
    /// error. Proves the `HtmlCommand::Reload` → `WebView::reload()` path drives a
    /// fresh paint on a live, settled instance.
    #[test]
    #[ignore = "heavy: starts a real Servo engine; run with --ignored --test-threads=1"]
    fn html_deck_reload_repaints() {
        let Ok(gpu) = GpuContext::new_headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut mgr = HtmlManager::new();

        // CSS paints black, JS overrides to blue — reaching blue proves JS ran.
        let css_js = "<!doctype html><html><head><style>html,body{height:100%;margin:0}body{background:#000}</style></head><body><script>document.body.style.background='rgb(0,0,255)';</script></body></html>";
        let idx = mgr
            .start_render(&data_url(css_js), W, H, &gpu.device)
            .expect("start_render returned None with the html feature enabled");
        let px = pump_until(&gpu, &mut mgr, idx, [0, 0, 255], Duration::from_secs(30));
        assert!(
            is_color(px, [0, 0, 255]),
            "deck did not render blue; got {px:?}"
        );

        // Reload re-fetches the same URL and re-runs the JS; it must paint blue again.
        mgr.reload(idx);
        let px2 = pump_until(&gpu, &mut mgr, idx, [0, 0, 255], Duration::from_secs(30));
        assert!(
            is_color(px2, [0, 0, 255]),
            "deck did not repaint blue after reload; got {px2:?}"
        );
    }

    /// Interactive input correctness: a forwarded mouse click reaches the DOM.
    /// The page paints red and registers a `click` handler that flips it blue;
    /// after `send_input` delivers MouseMove + button down/up at the center, the
    /// deck must repaint blue. Proves the `HtmlInputEvent` → `HtmlCommand::Input`
    /// → `WebView::notify_input_event` path drives a real DOM event + repaint.
    #[test]
    #[ignore = "heavy: starts a real Servo engine; run with --ignored --test-threads=1"]
    fn html_deck_click_input_repaints() {
        let Ok(gpu) = GpuContext::new_headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut mgr = HtmlManager::new();

        // Paints red; a document-level click handler flips the background to blue.
        let page = "<!doctype html><html><head><style>html,body{height:100%;margin:0}\
body{background:red}</style></head><body><script>\
document.addEventListener('click',function(){\
document.body.style.background='rgb(0,0,255)';});</script></body></html>";
        let idx = mgr
            .start_render(&data_url(page), W, H, &gpu.device)
            .expect("start_render returned None with the html feature enabled");

        // Wait for the page's red paint (distinct from the gray placeholder) so
        // the document is loaded and interactive before we click.
        let red = pump_until(&gpu, &mut mgr, idx, [255, 0, 0], Duration::from_secs(30));
        assert!(
            is_color(red, [255, 0, 0]),
            "page did not render red; got {red:?}"
        );

        // Forward a click at the viewport center: move, press, release.
        let (cx, cy) = ((W / 2) as f32, (H / 2) as f32);
        mgr.send_input(idx, HtmlInputEvent::MouseMove { x: cx, y: cy });
        mgr.send_input(
            idx,
            HtmlInputEvent::MouseButton {
                x: cx,
                y: cy,
                button: 0,
                pressed: true,
            },
        );
        mgr.send_input(
            idx,
            HtmlInputEvent::MouseButton {
                x: cx,
                y: cy,
                button: 0,
                pressed: false,
            },
        );

        let px = pump_until(&gpu, &mut mgr, idx, [0, 0, 255], Duration::from_secs(30));
        assert!(
            is_color(px, [0, 0, 255]),
            "forwarded click did not repaint blue; got {px:?}"
        );
    }

    /// Transparency correctness (Blocker 1): with `shell_background_color_rgba`
    /// set to `[0,0,0,0]`, a page whose html/body are transparent must read back
    /// with alpha below 255 — proving the Servo viewport clears transparent rather
    /// than the default opaque white. See /spec/html-source.md §2.
    #[test]
    #[ignore = "heavy: starts a real Servo engine; run with --ignored --test-threads=1"]
    fn html_deck_transparent_background_has_alpha() {
        let Ok(gpu) = GpuContext::new_headless() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut mgr = HtmlManager::new();

        // Explicitly transparent html/body; no opaque content anywhere.
        let page = "<!doctype html><html><head><style>html,body{height:100%;margin:0;\
background:transparent}</style></head><body></body></html>";
        let idx = mgr
            .start_render(&data_url(page), W, H, &gpu.device)
            .expect("start_render returned None with the html feature enabled");

        // Pump until the center pixel reads transparent (alpha < 255), or time out.
        let start = Instant::now();
        let mut px = [255u8; 4];
        while start.elapsed() < Duration::from_secs(30) {
            mgr.update(&gpu.device, &gpu.queue);
            px = center_pixel(&gpu, &mgr, idx);
            if px[3] < 255 {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        assert!(
            px[3] < 255,
            "transparent page did not yield alpha<255 (viewport still opaque); got {px:?}"
        );
    }
}
