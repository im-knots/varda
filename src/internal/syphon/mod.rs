//! Syphon — macOS inter-app GPU texture sharing.
//!
//! varda is the *client*: it discovers Syphon servers and receives their frames
//! via IOSurface. The receive is a CPU readback — `[SyphonMetalClient newFrameImage]`
//! returns an IOSurface-backed `MTLTexture`, we `getBytes` it into a buffer, and
//! `update()` uploads that to the wgpu texture. This needs no wgpu↔Metal device
//! bridge and reuses varda's existing texture plumbing; on Apple-silicon unified
//! memory the readback is a cheap same-memory copy. A zero-copy path (wrap the
//! IOSurface `MTLTexture` directly as a `wgpu::Texture`) is a possible follow-on.
//!
//! macOS only. Syphon.framework is loaded at runtime via `dlopen` (see
//! `framework_loaded`), not linked — a Mac without Syphon installed still builds
//! and runs, with Syphon features disabled.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread::JoinHandle;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{extern_class, msg_send, AnyThread, ClassType};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin,
    MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor,
    MTLTextureType, MTLTextureUsage,
};

// ---------------------------------------------------------------------------
// Syphon.framework FFI. These classes are vended by Syphon.framework (loaded at
// runtime via dlopen), declared here as opaque NSObject subclasses.
//
// SyphonServerDirectory.h:
//   + (SyphonServerDirectory *)sharedDirectory;
//   - (NSArray<NSDictionary *> *)serversMatchingName:(NSString *)name
//                                              appName:(NSString *)appName;
// SyphonMetalClient.h:
//   - initWithServerDescription:(NSDictionary *)desc device:(id<MTLDevice>)dev
//                       options:(NSDictionary *)opts
//               newFrameHandler:(void (^)(SyphonMetalClient *))handler;
//   - (id<MTLTexture>)newFrameImage;
//   - (void)stop;
// ---------------------------------------------------------------------------
extern_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[name = "SyphonServerDirectory"]
    pub struct SyphonServerDirectory;
);

extern_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[name = "SyphonMetalClient"]
    pub struct SyphonMetalClient;
);

// SyphonMetalServer.h (sender side):
//   - initWithName:(NSString *)name device:(id<MTLDevice>)device
//                  options:(NSDictionary *)options;
//   - (void)publishFrameTexture:(id<MTLTexture>)tex
//                  onCommandBuffer:(id<MTLCommandBuffer>)cb
//                      imageRegion:(NSRect)region flipped:(BOOL)isFlipped;
//   - (void)stop;
extern_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[name = "SyphonMetalServer"]
    pub struct SyphonMetalServer;
);

// Syphon server-description dictionary keys. The framework exports these as
// extern NSString constants; we reconstruct them from their documented values
// to avoid an extern-static binding.
const KEY_NAME: &str = "SyphonServerDescriptionNameKey";
const KEY_APP: &str = "SyphonServerDescriptionAppNameKey";

/// Discovered Syphon server.
#[derive(Debug, Clone)]
pub struct SyphonSource {
    /// Server name (matches the publisher's server name).
    pub name: String,
    /// Application name publishing the server.
    pub app_name: String,
}

/// Latest decoded frame handed from a receive thread to the render thread.
struct SyphonFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

/// Manages Syphon server discovery, client receivers, and server publishers.
pub struct SyphonManager {
    available: bool,
    sources: Vec<SyphonSource>,
    /// Server-description dicts kept alongside `sources` by name, needed to init
    /// a `SyphonMetalClient`. Looked up on the render thread in `start_receive`
    /// because `SyphonServerDirectory` observes notifications on that run loop.
    descriptions: Vec<(String, Retained<NSDictionary<NSString, AnyObject>>)>,
    receivers: Vec<SyphonReceiver>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// Our own Metal device, shared by client init and publishers. Lazily created.
    metal_device: Option<Retained<ProtocolObject<dyn MTLDevice>>>,
    /// wgpu's own `MTLDevice`, extracted via `as_hal` so the publish textures,
    /// the server, and the publish queue share one device — the basis for the
    /// zero-copy (no readback) sender path. Lazily extracted on first publish.
    wgpu_metal_device: Option<Retained<ProtocolObject<dyn MTLDevice>>>,
    /// Command queue for publishing (sender side), on wgpu's device. Lazy.
    publish_queue: Option<Retained<ProtocolObject<dyn MTLCommandQueue>>>,
    /// RGBA→BGRA GPU conversion pipeline (sender side), replacing the old CPU
    /// swizzle. Targets `Bgra8UnormSrgb`. Lazily built on first publish.
    convert_pipeline: Option<crate::renderer::blit::BlitPipeline>,
    /// Active Syphon publishers, keyed by server name (sender side). Created
    /// lazily on first `publish_frame_gpu`. Render-thread only.
    servers: HashMap<String, SyphonServerHandle>,
}

/// A background receive thread plus the render-thread-side wgpu texture it feeds.
/// Mirrors `NdiReceiver`: the Metal client lives entirely on the receive thread;
/// only RGBA bytes cross back via `frame_data`.
struct SyphonReceiver {
    #[allow(dead_code)]
    server_name: String,
    frame_data: Arc<Mutex<Option<SyphonFrame>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    width: u32,
    height: u32,
}

/// A Syphon publisher (sender side). The Metal server and its ring of publish
/// textures live on the render thread, mirroring `NdiSender`. The conversion
/// blit (RGBA→BGRA) renders into a ring slot via wgpu; Syphon then publishes
/// that slot's native `MTLTexture` directly — no CPU readback, no CPU swizzle.
struct SyphonServerHandle {
    server: Retained<SyphonMetalServer>,
    width: u32,
    height: u32,
    /// Ring of BGRA textures shared between wgpu (render) and Syphon (publish).
    slots: Vec<PublishSlot>,
    sched: PublishScheduler,
}

/// One publish texture, created on wgpu's `MTLDevice` and imported into wgpu so
/// the conversion blit can render into it while Syphon reads the same native
/// `MTLTexture`.
struct PublishSlot {
    /// Native handle handed to Syphon's `publishFrameTexture:`.
    mtl: Retained<ProtocolObject<dyn MTLTexture>>,
    /// The same texture imported into wgpu; kept alive so `view` stays valid.
    _wgpu_texture: wgpu::Texture,
    /// Render target for the conversion blit.
    view: wgpu::TextureView,
    /// Set true once this slot's blit submission completes on the GPU. Ensures
    /// Syphon never reads a half-rendered texture (the write→read hazard).
    write_done: Arc<AtomicBool>,
}

/// Number of publish textures per server. One is being rendered, one is pending
/// publish, and the extra gives Syphon's in-flight read margin before the slot
/// is reused (the softer read→overwrite hazard, mitigated by ring depth).
const PUBLISH_RING: usize = 3;

/// Round-robin scheduler decoupling GPU render (write) from Syphon publish.
/// Pure index logic — unit-tested without a GPU. A slot is published only after
/// its blit is GPU-complete, and never overwritten while pending publish.
struct PublishScheduler {
    n: usize,
    write_cursor: usize,
    /// Slot rendered-into but not yet published.
    pending: Option<usize>,
}

impl PublishScheduler {
    fn new(n: usize) -> Self {
        Self {
            n,
            write_cursor: 0,
            pending: None,
        }
    }

    /// If a pending slot's blit has signalled completion, return it to publish
    /// (clearing pending). `write_done[i]` reflects slot i's GPU completion.
    fn poll_publish(&mut self, write_done: &[bool]) -> Option<usize> {
        let p = self.pending?;
        if write_done.get(p).copied().unwrap_or(false) {
            self.pending = None;
            Some(p)
        } else {
            None
        }
    }

    /// Next slot to render into, or None if the previously rendered slot hasn't
    /// been published yet (backpressure — drop the frame rather than clobber it).
    fn poll_write(&mut self) -> Option<usize> {
        if self.pending.is_some() {
            return None;
        }
        let idx = self.write_cursor;
        self.write_cursor = (self.write_cursor + 1) % self.n;
        Some(idx)
    }

    fn mark_written(&mut self, idx: usize) {
        self.pending = Some(idx);
    }
}

// The manager lives on (and is only touched from) varda's render thread. The
// publisher Metal objects (`servers`, `wgpu_metal_device`, `publish_queue`) and
// the retained server descriptions are render-thread-only; receiver Metal
// clients live on their own threads and are never stored here. Nothing is shared
// across threads, so asserting Send is sound.
unsafe impl Send for SyphonManager {}

impl Default for SyphonManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SyphonManager {
    pub fn new() -> Self {
        let available = Self::check_framework();
        if available {
            log::info!("Syphon.framework found");
        } else {
            log::info!("Syphon.framework not found — Syphon features disabled");
        }
        Self {
            available,
            sources: Vec::new(),
            descriptions: Vec::new(),
            receivers: Vec::new(),
            textures: Vec::new(),
            metal_device: None,
            wgpu_metal_device: None,
            publish_queue: None,
            convert_pipeline: None,
            servers: HashMap::new(),
        }
    }

    /// Create a disabled Syphon manager (CLI `--no-syphon` flag).
    pub fn new_disabled() -> Self {
        let mut m = Self::new();
        m.available = false;
        m
    }

    pub fn is_available(&self) -> bool {
        self.available
    }
    pub fn sources(&self) -> &[SyphonSource] {
        &self.sources
    }

    /// Return discovered server names for UI display.
    pub fn discovered_sources(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name.clone()).collect()
    }

    fn metal_device(&mut self) -> Option<&Retained<ProtocolObject<dyn MTLDevice>>> {
        if self.metal_device.is_none() {
            self.metal_device = MTLCreateSystemDefaultDevice();
            if self.metal_device.is_none() {
                log::error!("Syphon: MTLCreateSystemDefaultDevice returned nil");
            }
        }
        self.metal_device.as_ref()
    }

    /// Scan for available Syphon servers via SyphonServerDirectory.
    pub fn discover(&mut self) {
        if !self.available {
            return;
        }
        self.sources.clear();
        self.descriptions.clear();

        unsafe {
            let dir: Retained<SyphonServerDirectory> =
                msg_send![SyphonServerDirectory::class(), sharedDirectory];
            // serversMatchingName:nil appName:nil → all servers.
            let nil_str: *const NSString = std::ptr::null();
            let servers: Retained<NSArray<NSDictionary<NSString, AnyObject>>> =
                msg_send![&dir, serversMatchingName: nil_str, appName: nil_str];

            let count: usize = msg_send![&servers, count];
            let name_key = NSString::from_str(KEY_NAME);
            let app_key = NSString::from_str(KEY_APP);
            for i in 0..count {
                let desc: Retained<NSDictionary<NSString, AnyObject>> =
                    msg_send![&servers, objectAtIndex: i];
                let name = nsstring_value(&desc, &name_key).unwrap_or_default();
                let app_name = nsstring_value(&desc, &app_key).unwrap_or_default();
                self.sources.push(SyphonSource {
                    name: name.clone(),
                    app_name,
                });
                self.descriptions.push((name, desc));
            }
        }
        log::debug!("Syphon discover: {} server(s)", self.sources.len());
    }

    /// Start receiving from a named Syphon server. Spawns a dedicated receive
    /// thread (`syphon-recv-{name}`) that owns the `SyphonMetalClient`, polls
    /// `newFrameImage`, performs the `getBytes` CPU readback off the render
    /// thread, and publishes RGBA into `frame_data`. Returns a client index used
    /// by `texture_view()` / `client_dimensions()`.
    pub fn start_receive(&mut self, server_name: &str, device: &wgpu::Device) -> Option<usize> {
        if !self.available {
            log::warn!("Cannot receive Syphon: framework not available");
            return None;
        }

        // The description must be resolved here (render thread): SyphonServerDirectory
        // observes distributed notifications on the render-thread run loop.
        let desc = self
            .descriptions
            .iter()
            .find(|(n, _)| n == server_name)
            .map(|(_, d)| d.clone());
        let Some(desc) = desc else {
            log::warn!("Syphon: no discovered server named '{}'", server_name);
            return None;
        };

        let metal = self.metal_device()?.clone();
        let client: Retained<SyphonMetalClient> = unsafe {
            let alloc = SyphonMetalClient::alloc();
            let nil_opts: *const NSDictionary<NSString, AnyObject> = std::ptr::null();
            // newFrameHandler: nil — we poll newFrameImage on the receive thread.
            let handler: *const AnyObject = std::ptr::null();
            msg_send![
                alloc,
                initWithServerDescription: &*desc,
                device: &*metal,
                options: nil_opts,
                newFrameHandler: handler,
            ]
        };

        // Placeholder size; corrected on the first frame in update().
        let (width, height) = (1920u32, 1080u32);
        let (texture, view) = make_texture(device, server_name, width, height);

        let frame_data: Arc<Mutex<Option<SyphonFrame>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));

        // Move the !Send Metal client onto the receive thread via a raw-pointer
        // ownership transfer, mirroring the NDI receiver's handle hand-off.
        let client_ptr = Retained::into_raw(client) as usize;
        let frame_clone = Arc::clone(&frame_data);
        let stop_clone = Arc::clone(&stop_flag);
        let connected_clone = Arc::clone(&connected);
        let name_log = server_name.to_string();

        let thread = std::thread::Builder::new()
            .name(format!("syphon-recv-{}", server_name))
            .spawn(move || {
                let client: Retained<SyphonMetalClient> =
                    match unsafe { Retained::from_raw(client_ptr as *mut SyphonMetalClient) } {
                        Some(c) => c,
                        None => {
                            log::error!("Syphon '{}': null client on receive thread", name_log);
                            return;
                        }
                    };
                syphon_receive_loop(
                    &client,
                    &frame_clone,
                    &stop_clone,
                    &connected_clone,
                    &name_log,
                );
                unsafe {
                    let _: () = msg_send![&client, stop];
                }
                log::info!("Syphon '{}': receive thread stopping", name_log);
            })
            .ok();

        let idx = self.receivers.len();
        self.receivers.push(SyphonReceiver {
            server_name: server_name.to_string(),
            frame_data,
            stop_flag,
            connected,
            thread,
            width,
            height,
        });
        self.textures.push((texture, view));
        log::info!("Syphon client connected to '{}'", server_name);
        Some(idx)
    }

    /// Pull the newest frame from each receive thread and upload it to its wgpu
    /// texture. Render-thread, upload-only: non-blocking `try_lock` + `take` +
    /// `write_texture`, recreating the texture when the server's frame size changes.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for i in 0..self.receivers.len() {
            let new_frame = {
                if let Ok(mut guard) = self.receivers[i].frame_data.try_lock() {
                    guard.take()
                } else {
                    None
                }
            };
            let Some(frame) = new_frame else { continue };
            let (w, h) = (frame.width, frame.height);
            if w == 0 || h == 0 {
                continue;
            }

            // Recreate the wgpu texture if the server's frame size changed.
            if w != self.receivers[i].width || h != self.receivers[i].height {
                let name = self.receivers[i].server_name.clone();
                let (t, v) = make_texture(device, &name, w, h);
                self.textures[i] = (t, v);
                self.receivers[i].width = w;
                self.receivers[i].height = h;
            }

            let expected = (w * h * 4) as usize;
            if frame.data.len() < expected {
                continue;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.textures[i].0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data[..expected],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.textures.get(idx).map(|(_, v)| v)
    }

    pub fn client_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.receivers.get(idx).map(|c| (c.width, c.height))
    }

    /// Whether the receive thread for `idx` is currently getting frames.
    pub fn is_connected(&self, idx: usize) -> bool {
        self.receivers
            .get(idx)
            .map(|r| r.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Publish a composited frame to a Syphon server, GPU-side (zero-copy).
    ///
    /// Render-thread. Unlike the old CPU path, this takes no readback bytes: it
    /// extracts wgpu's `MTLDevice` (`as_hal`), keeps a ring of BGRA `MTLTexture`s
    /// imported into wgpu, runs the RGBA→BGRA conversion as a GPU blit into a
    /// ring slot, and hands that slot's native texture to `SyphonMetalServer`.
    /// This removes the pipeline-stalling readback and the CPU swizzle.
    ///
    /// `src_view` is the output's rendered texture view (e.g. `h.texture_view`).
    /// Synchronization: a slot is published only after its blit submission
    /// completes (`on_submitted_work_done` → `write_done`), so Syphon always
    /// reads a fully-rendered texture; the ring depth covers Syphon's in-flight
    /// read before a slot is reused.
    pub fn publish_frame_gpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        server_name: &str,
        src_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        if !self.available || width == 0 || height == 0 {
            return;
        }

        // 1. wgpu's MTLDevice — unify so the imported textures, the server, and
        //    the publish queue all live on one device.
        if self.wgpu_metal_device.is_none() {
            let dev =
                unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.map(|d| d.raw_device().clone());
            match dev {
                Some(d) => self.wgpu_metal_device = Some(d),
                None => {
                    log::error!("Syphon publish: device.as_hal::<Metal> returned None");
                    return;
                }
            }
        }
        let mtl_dev = self.wgpu_metal_device.as_ref().unwrap().clone();

        // 2. Publish command queue on wgpu's device (wgpu's own queue handle is
        //    private in wgpu-hal, so the publish runs on a separate queue).
        if self.publish_queue.is_none() {
            self.publish_queue = mtl_dev.newCommandQueue();
            if self.publish_queue.is_none() {
                log::error!("Syphon publish: newCommandQueue returned nil");
                return;
            }
        }

        // 3. RGBA→BGRA conversion pipeline (GPU; replaces the old CPU swizzle).
        if self.convert_pipeline.is_none() {
            match crate::renderer::blit::BlitPipeline::new(
                device,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ) {
                Ok(p) => self.convert_pipeline = Some(p),
                Err(e) => {
                    log::error!("Syphon publish: failed to build convert pipeline: {e}");
                    return;
                }
            }
        }

        // 4. Server + slot ring (recreate on first use / size change).
        let need_new = match self.servers.get(server_name) {
            None => true,
            Some(h) => h.width != width || h.height != height,
        };
        if need_new {
            let name = NSString::from_str(server_name);
            let server: Option<Retained<SyphonMetalServer>> = unsafe {
                let alloc = SyphonMetalServer::alloc();
                let nil_opts: *const NSDictionary<NSString, AnyObject> = std::ptr::null();
                msg_send![alloc, initWithName: &*name, device: &*mtl_dev, options: nil_opts]
            };
            let Some(server) = server else {
                log::error!("Syphon publish: failed to create server '{}'", server_name);
                return;
            };
            let mut slots = Vec::with_capacity(PUBLISH_RING);
            for _ in 0..PUBLISH_RING {
                match Self::make_publish_slot(&mtl_dev, device, width, height) {
                    Some(s) => slots.push(s),
                    None => {
                        log::error!("Syphon publish: failed to create publish texture");
                        return;
                    }
                }
            }
            self.servers.insert(
                server_name.to_string(),
                SyphonServerHandle {
                    server,
                    width,
                    height,
                    slots,
                    sched: PublishScheduler::new(PUBLISH_RING),
                },
            );
            log::info!("Syphon server publishing as '{}'", server_name);
        }

        let pipeline = self.convert_pipeline.as_ref().unwrap();
        let pub_queue = self.publish_queue.as_ref().unwrap();
        let handle = self.servers.get_mut(server_name).unwrap();

        // 5a. Publish step (before write, so a written slot is never overwritten
        //     before it is published). Only a GPU-complete slot is published.
        let write_done: Vec<bool> = handle
            .slots
            .iter()
            .map(|s| s.write_done.load(Ordering::SeqCst))
            .collect();
        if let Some(p) = handle.sched.poll_publish(&write_done) {
            if let Some(cmd_buf) = pub_queue.commandBuffer() {
                let image_region = NSRect {
                    origin: NSPoint { x: 0.0, y: 0.0 },
                    size: NSSize {
                        width: width as f64,
                        height: height as f64,
                    },
                };
                unsafe {
                    let _: () = msg_send![
                        &handle.server,
                        publishFrameTexture: &*handle.slots[p].mtl,
                        onCommandBuffer: &*cmd_buf,
                        imageRegion: image_region,
                        flipped: false,
                    ];
                }
                cmd_buf.commit();
            } else {
                log::error!("Syphon publish: commandBuffer returned nil");
            }
        }

        // 5b. Write step: render the RGBA→BGRA conversion into a free slot and
        //     arm its completion flag so it can be published next frame.
        if let Some(w) = handle.sched.poll_write() {
            let bind = pipeline.create_bind_group(device, src_view);
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Syphon Convert Encoder"),
            });
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Syphon Convert Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &handle.slots[w].view,
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
                pipeline.render(&mut rp, &bind);
            }
            let flag = handle.slots[w].write_done.clone();
            flag.store(false, Ordering::SeqCst);
            queue.submit(std::iter::once(encoder.finish()));
            queue.on_submitted_work_done(move || flag.store(true, Ordering::SeqCst));
            handle.sched.mark_written(w);
        }
    }

    /// Create one publish texture on wgpu's `MTLDevice` and import it into wgpu
    /// so the conversion blit can target it while Syphon reads the same native
    /// texture (zero-copy share). `BGRA8Unorm_sRGB` matches Syphon's BGRA
    /// convention and the sRGB encoding the old CPU path produced.
    fn make_publish_slot(
        mtl_dev: &Retained<ProtocolObject<dyn MTLDevice>>,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Option<PublishSlot> {
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm_sRGB,
                width as usize,
                height as usize,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        desc.setStorageMode(MTLStorageMode::Private);
        let mtl = mtl_dev.newTextureWithDescriptor(&desc)?;

        // Import the same MTLTexture into wgpu; keep a retained clone for Syphon.
        let hal_texture = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(
                mtl.clone(),
                wgpu::TextureFormat::Bgra8UnormSrgb,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
            )
        };
        let wgpu_texture = unsafe {
            device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("Syphon Publish Texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some(PublishSlot {
            mtl,
            _wgpu_texture: wgpu_texture,
            view,
            write_done: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get_mut(idx) {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn check_framework() -> bool {
        framework_loaded()
    }
}

/// Receive loop body, run on a dedicated `syphon-recv-*` thread. Polls the
/// client for new frames, performs the CPU readback, and swaps RGBA into
/// `frame_data`. Tracks a `connected` flag: a producer that stops publishing is
/// marked disconnected after ~2s of nil frames.
fn syphon_receive_loop(
    client: &Retained<SyphonMetalClient>,
    frame_data: &Arc<Mutex<Option<SyphonFrame>>>,
    stop_flag: &Arc<AtomicBool>,
    connected: &Arc<AtomicBool>,
    name: &str,
) {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);
    // ~Nil polls before declaring the producer gone (~2s at 8ms).
    const DISCONNECT_AFTER: u32 = 250;
    let mut nil_count: u32 = 0;

    while !stop_flag.load(Ordering::SeqCst) {
        let tex: Option<Retained<ProtocolObject<dyn MTLTexture>>> =
            unsafe { msg_send![client, newFrameImage] };
        let Some(tex) = tex else {
            nil_count = nil_count.saturating_add(1);
            if nil_count == DISCONNECT_AFTER {
                connected.store(false, Ordering::SeqCst);
                log::debug!("Syphon '{}': no frames (~2s), marked disconnected", name);
            }
            std::thread::sleep(POLL_INTERVAL);
            continue;
        };
        nil_count = 0;

        let w = tex.width() as u32;
        let h = tex.height() as u32;
        if w == 0 || h == 0 {
            std::thread::sleep(POLL_INTERVAL);
            continue;
        }

        // CPU readback from the IOSurface-backed Metal texture (off the render
        // thread). Syphon surface textures are Shared on Apple silicon.
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: w as usize,
                height: h as usize,
                depth: 1,
            },
        };
        unsafe {
            tex.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                std::ptr::NonNull::new(buf.as_mut_ptr() as *mut std::ffi::c_void).unwrap(),
                (w * 4) as usize,
                region,
                0,
            );
        }

        connected.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = frame_data.lock() {
            *guard = Some(SyphonFrame {
                data: buf,
                width: w,
                height: h,
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// dlopen Syphon.framework once and keep it loaded for the process lifetime.
///
/// We load at runtime (via libloading, already a varda dep) rather than linking
/// the framework, so a Mac *without* Syphon installed still builds and runs —
/// `available` simply stays false and every Syphon call is guarded. Once loaded,
/// the Obj-C runtime can resolve `SyphonServerDirectory` / `SyphonMetalClient`
/// by name (`objc_getClass`), which is all `extern_class!` + `msg_send!` need.
fn framework_loaded() -> bool {
    static SYPHON_LIB: OnceLock<Option<libloading::Library>> = OnceLock::new();
    let lib = SYPHON_LIB.get_or_init(|| {
        let candidates = [
            "/Library/Frameworks/Syphon.framework/Syphon".to_string(),
            format!(
                "{}/Library/Frameworks/Syphon.framework/Syphon",
                std::env::var("HOME").unwrap_or_default()
            ),
        ];
        for p in candidates {
            if std::path::Path::new(&p).exists() {
                match unsafe { libloading::Library::new(&p) } {
                    Ok(l) => return Some(l),
                    Err(e) => log::warn!("Syphon: dlopen of {p} failed: {e}"),
                }
            }
        }
        None
    });
    lib.is_some()
}

/// Read an NSString value out of a server-description dictionary.
fn nsstring_value(dict: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<String> {
    unsafe {
        let val: *mut AnyObject = msg_send![dict, objectForKey: key];
        if val.is_null() {
            return None;
        }
        let s: &NSString = &*(val as *const NSString);
        Some(s.to_string())
    }
}

/// Allocate the RGBA wgpu texture + view a Syphon client uploads into.
fn make_texture(
    device: &wgpu::Device,
    server_name: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("Syphon Client: {}", server_name)),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Syphon getBytes hands back BGRA byte order (servers publish BGRA8Unorm);
        // Bgra8UnormSrgb matches the existing client expectation.
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl Drop for SyphonManager {
    fn drop(&mut self) {
        for r in &mut self.receivers {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
        }
        for h in self.servers.values() {
            unsafe {
                let _: () = msg_send![&h.server, stop];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These exercise the host-independent paths only: a fresh manager holds no
    // clients/sources, so none of these touch Syphon.framework or Metal. They
    // pass whether or not Syphon is installed on the test machine.

    #[test]
    fn syphon_manager_new_no_crash() {
        // new() probes for Syphon.framework; on machines without it, `available`
        // is simply false. Just verify construction and the query don't panic.
        let mgr = SyphonManager::new();
        let _ = mgr.is_available();
    }

    #[test]
    fn syphon_manager_disabled_is_unavailable() {
        let mgr = SyphonManager::new_disabled();
        assert!(!mgr.is_available());
    }

    #[test]
    fn syphon_manager_sources_empty() {
        let mgr = SyphonManager::new();
        assert!(mgr.sources().is_empty());
        assert!(mgr.discovered_sources().is_empty());
    }

    #[test]
    fn syphon_manager_texture_view_out_of_bounds() {
        let mgr = SyphonManager::new();
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.texture_view(999).is_none());
    }

    #[test]
    fn syphon_manager_client_dimensions_out_of_bounds() {
        let mgr = SyphonManager::new();
        assert!(mgr.client_dimensions(0).is_none());
        assert!(mgr.client_dimensions(999).is_none());
    }

    #[test]
    fn syphon_manager_discover_noop_when_unavailable() {
        // discover() early-returns when the framework is absent; on a machine
        // with Syphon it may populate sources, but must never panic either way.
        let mut mgr = SyphonManager::new_disabled();
        mgr.discover();
        assert!(mgr.sources().is_empty());
    }

    #[test]
    fn syphon_manager_is_connected_out_of_bounds() {
        let mgr = SyphonManager::new();
        assert!(!mgr.is_connected(0));
        assert!(!mgr.is_connected(999));
    }

    #[test]
    fn syphon_manager_disabled_has_no_servers() {
        // A disabled manager must never create publishers.
        let mgr = SyphonManager::new_disabled();
        assert!(!mgr.is_available());
        assert!(mgr.servers.is_empty());
    }

    #[test]
    fn publish_scheduler_write_publish_cycle() {
        let mut s = PublishScheduler::new(PUBLISH_RING);
        // Nothing pending → nothing to publish; first write takes slot 0.
        assert_eq!(s.poll_publish(&[false, false, false]), None);
        assert_eq!(s.poll_write(), Some(0));
        s.mark_written(0);
        // Slot 0's blit not finished → no publish, and writes are blocked.
        assert_eq!(s.poll_publish(&[false, false, false]), None);
        assert_eq!(s.poll_write(), None);
        // Blit done → publish 0, then the next write advances to slot 1.
        assert_eq!(s.poll_publish(&[true, false, false]), Some(0));
        assert_eq!(s.poll_write(), Some(1));
        s.mark_written(1);
        assert_eq!(s.poll_publish(&[false, true, false]), Some(1));
        assert_eq!(s.poll_write(), Some(2));
        s.mark_written(2);
        assert_eq!(s.poll_publish(&[false, false, true]), Some(2));
        // Cursor wraps back to slot 0.
        assert_eq!(s.poll_write(), Some(0));
    }

    #[test]
    fn publish_scheduler_no_publish_without_pending() {
        let mut s = PublishScheduler::new(PUBLISH_RING);
        assert_eq!(s.poll_publish(&[true, true, true]), None);
    }

    #[test]
    fn publish_scheduler_backpressure_blocks_write_until_published() {
        let mut s = PublishScheduler::new(2);
        assert_eq!(s.poll_write(), Some(0));
        s.mark_written(0);
        // Unpublished pending slot → writes blocked (frame dropped, not clobbered).
        assert_eq!(s.poll_write(), None);
        assert_eq!(s.poll_write(), None);
        assert_eq!(s.poll_publish(&[true, false]), Some(0));
        assert_eq!(s.poll_write(), Some(1));
    }
}
