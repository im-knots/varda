//! Syphon — macOS inter-app GPU texture sharing.
//!
//! varda is both client and server, and both halves share memory with the peer
//! process rather than copying frames through the CPU.
//!
//! Receive: `[SyphonMetalClient newFrameImage]` hands back an `IOSurface`-backed
//! `MTLTexture`. varda takes the `IOSurface` off it and builds its own texture
//! over the same surface on wgpu's `MTLDevice`, which is then imported into wgpu
//! and bound directly. Nothing is read back and nothing is uploaded. The client
//! rebinds its surface only when the server resizes or restarts, so the import is
//! rebuilt on that event rather than per frame.
//!
//! Send: a ring of `BGRA8Unorm` textures on wgpu's device, written by a conversion
//! blit and handed to `SyphonMetalServer`, which copies them into its own surface.
//!
//! See /spec/syphon-zero-copy.md for why the receive path builds its own texture
//! instead of wrapping Syphon's, and why the publish format is not `_sRGB`.
//!
//! macOS only. Syphon.framework is loaded at runtime via `dlopen` (see
//! `framework_loaded`), not linked — a Mac without Syphon installed still builds
//! and runs, with Syphon features disabled.

use std::collections::HashMap;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, ClassType, extern_class, msg_send};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
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

/// A Syphon client and the wgpu texture sharing its `IOSurface`.
///
/// Unlike `NdiReceiver` there is no thread here. The client and its texture both
/// live on the render thread, because with nothing to copy the only work left is
/// a cheap poll, and `SyphonClientBase` wants a run loop anyway.
struct SyphonReceiver {
    server_name: String,
    /// Render-thread only, like every other Metal object this manager owns.
    client: Retained<SyphonMetalClient>,
    /// Address of the `IOSurface` currently imported. Syphon rebinds its surface
    /// when the server resizes or restarts and its client rebuilds the texture
    /// only then, so comparing addresses is enough to know when to re-import.
    /// Zero means nothing imported yet.
    surface_addr: usize,
    connected: bool,
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

// The manager lives on (and is only touched from) varda's render thread. Every
// Metal object it owns is render-thread-only: the publisher `servers`, the
// `wgpu_metal_device`, the `publish_queue`, the retained server descriptions, and
// since the zero-copy receive landed, the client handles too. Nothing is shared
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

    /// Scan for available Syphon servers via `SyphonServerDirectory`.
    /// wgpu's own `MTLDevice`, extracted once through `as_hal` and cached.
    ///
    /// Both halves of the Syphon path need it. An `MTLTexture` belongs to exactly
    /// one device, so anything Varda intends to bind, whether a publish slot or an
    /// imported `IOSurface`, has to be created on the device wgpu is rendering on.
    fn wgpu_metal_device(
        &mut self,
        device: &wgpu::Device,
    ) -> Option<Retained<ProtocolObject<dyn MTLDevice>>> {
        if self.wgpu_metal_device.is_none() {
            let dev =
                unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.map(|d| d.raw_device().clone());
            let Some(d) = dev else {
                log::error!("Syphon: device.as_hal::<Metal> returned None");
                return None;
            };
            self.wgpu_metal_device = Some(d);
        }
        self.wgpu_metal_device.clone()
    }

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
    ///
    /// # Panics
    ///
    /// Panics only if the cached device is absent immediately after this call
    /// populated it, which cannot happen.
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
            log::warn!("Syphon: no discovered server named '{server_name}'");
            return None;
        };

        // wgpu's own MTLDevice, not MTLCreateSystemDefaultDevice(). A texture
        // belongs to exactly one device, so a client on any other device could
        // only ever be copied from, never bound.
        let metal = self.wgpu_metal_device(device)?;
        let client: Retained<SyphonMetalClient> = unsafe {
            let alloc = SyphonMetalClient::alloc();
            let nil_opts: *const NSDictionary<NSString, AnyObject> = std::ptr::null();
            // newFrameHandler: nil — update() polls on the render thread.
            let handler: *const AnyObject = std::ptr::null();
            msg_send![
                alloc,
                initWithServerDescription: &*desc,
                device: &*metal,
                options: nil_opts,
                newFrameHandler: handler,
            ]
        };

        // Placeholder until the first surface arrives, so `texture_view` has
        // something to hand a deck that binds before the server publishes.
        let (width, height) = (1920u32, 1080u32);
        let (texture, view) = make_placeholder_texture(device, server_name, width, height);

        let idx = self.receivers.len();
        self.receivers.push(SyphonReceiver {
            server_name: server_name.to_string(),
            client,
            surface_addr: 0,
            connected: false,
            width,
            height,
        });
        self.textures.push((texture, view));
        log::info!("Syphon client connected to '{server_name}'");
        Some(idx)
    }

    /// Re-import any receiver whose server has rebound its `IOSurface`.
    ///
    /// Render-thread, and in the steady state it does nothing: the surface is
    /// shared memory that the producer updates in place, so an unchanged surface
    /// address means the bound texture is already showing the current frame.
    /// Work happens only when a server starts, resizes, or restarts.
    pub fn update(&mut self, device: &wgpu::Device) {
        if self.receivers.is_empty() {
            return;
        }
        let Some(mtl_dev) = self.wgpu_metal_device(device) else {
            return;
        };

        for i in 0..self.receivers.len() {
            let tex: Option<Retained<ProtocolObject<dyn MTLTexture>>> =
                unsafe { msg_send![&self.receivers[i].client, newFrameImage] };
            // The framework's own answer, rather than inferring a dead producer
            // from a run of nil frames as the old receive thread had to.
            self.receivers[i].connected = unsafe { msg_send![&self.receivers[i].client, isValid] };

            let Some(tex) = tex else { continue };
            let Some(surface) = tex.iosurface() else {
                continue;
            };
            let addr = std::ptr::from_ref(&*surface).addr();
            if addr == self.receivers[i].surface_addr {
                continue; // same shared memory; nothing to rebuild
            }

            let (w, h) = (tex.width() as u32, tex.height() as u32);
            if w == 0 || h == 0 {
                continue;
            }
            let name = self.receivers[i].server_name.clone();
            let Some((texture, view)) =
                import_surface_texture(&mtl_dev, device, &surface, &name, w, h)
            else {
                log::warn!("Syphon '{name}': could not import the server's IOSurface");
                continue;
            };
            self.textures[i] = (texture, view);
            self.receivers[i].surface_addr = addr;
            self.receivers[i].width = w;
            self.receivers[i].height = h;
            log::debug!("Syphon '{name}': bound shared surface at {w}x{h}");
        }
    }

    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.textures.get(idx).map(|(_, v)| v)
    }

    pub fn client_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.receivers.get(idx).map(|c| (c.width, c.height))
    }

    /// Whether the client for `idx` is currently attached to a live server.
    pub fn is_connected(&self, idx: usize) -> bool {
        self.receivers.get(idx).is_some_and(|r| r.connected)
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
    ///
    /// # Panics
    ///
    /// Panics only if the cached wgpu Metal device is absent immediately after
    /// this call populated it, which cannot happen.
    pub fn publish_frame_gpu(
        &mut self,
        context: &crate::renderer::context::GpuContext,
        server_name: &str,
        src_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        if !self.available || width == 0 || height == 0 {
            return;
        }

        let device = &context.device;

        // 1. wgpu's MTLDevice — unify so the imported textures, the server, and
        //    the publish queue all live on one device.
        let Some(mtl_dev) = self.wgpu_metal_device(device) else {
            return;
        };

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
                log::error!("Syphon publish: failed to create server '{server_name}'");
                return;
            };
            let mut slots = Vec::with_capacity(PUBLISH_RING);
            for _ in 0..PUBLISH_RING {
                if let Some(s) = Self::make_publish_slot(&mtl_dev, device, width, height) {
                    slots.push(s);
                } else {
                    log::error!("Syphon publish: failed to create publish texture");
                    return;
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
            log::info!("Syphon server publishing as '{server_name}'");
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
                        width: f64::from(width),
                        height: f64::from(height),
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
            context.submit(std::iter::once(encoder.finish()));
            context
                .queue
                .on_submitted_work_done(move || flag.store(true, Ordering::SeqCst));
            handle.sched.mark_written(w);
        }
    }

    /// Create one publish texture on wgpu's `MTLDevice` and import it into wgpu
    /// so the conversion blit can target it while Syphon reads the same native
    /// texture.
    ///
    /// The Metal format is `BGRA8Unorm`, exactly what `SyphonMetalServer` creates
    /// its own destination surface as. That equality is the sole condition Varda
    /// was failing, and failing it sent every frame down Syphon's render fallback,
    /// which applies an sRGB decode on sample and never re-encodes: 128 published
    /// as 55. See /spec/syphon-zero-copy.md.
    ///
    /// The sRGB encode Varda still needs is kept by rendering through a
    /// `Bgra8UnormSrgb` **view** of the same linear texture, so the hardware
    /// performs the transfer and there is no hand-written curve to drift. That
    /// needs `PixelFormatView` usage on the Metal side and `view_formats` on the
    /// wgpu side; wgpu only calls `newTextureViewWithPixelFormat` when the view
    /// format differs from the texture format, which is precisely this case.
    fn make_publish_slot(
        mtl_dev: &Retained<ProtocolObject<dyn MTLDevice>>,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Option<PublishSlot> {
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width as usize,
                height as usize,
                false,
            )
        };
        desc.setUsage(
            MTLTextureUsage::ShaderRead
                | MTLTextureUsage::RenderTarget
                | MTLTextureUsage::PixelFormatView,
        );
        desc.setStorageMode(MTLStorageMode::Private);
        let mtl = mtl_dev.newTextureWithDescriptor(&desc)?;

        // Import the same MTLTexture into wgpu; keep a retained clone for Syphon.
        let hal_texture = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(
                mtl.clone(),
                wgpu::TextureFormat::Bgra8Unorm,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
                None,
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
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[wgpu::TextureFormat::Bgra8UnormSrgb],
                },
                wgpu::TextureUses::COLOR_TARGET,
            )
        };
        // The blit target: sRGB view over the linear texture Syphon will read.
        let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Syphon Publish sRGB View"),
            format: Some(wgpu::TextureFormat::Bgra8UnormSrgb),
            ..Default::default()
        });
        Some(PublishSlot {
            mtl,
            _wgpu_texture: wgpu_texture,
            view,
            write_done: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get(idx) {
            unsafe {
                let _: () = msg_send![&r.client, stop];
            }
        }
    }

    fn check_framework() -> bool {
        framework_loaded()
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

/// Read an `NSString` value out of a server-description dictionary.
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
/// A black stand-in bound until the server publishes its first frame.
///
/// Decks bind a Syphon source the moment it is added, which can be before the
/// producer has started, so `texture_view` must always have something to hand
/// back. Replaced wholesale by [`import_surface_texture`] on the first surface.
fn make_placeholder_texture(
    device: &wgpu::Device,
    server_name: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("Syphon Client Placeholder: {server_name}")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Bind a server's `IOSurface` as a wgpu texture, sharing its memory.
///
/// The Metal texture is created as `BGRA8Unorm_sRGB`, even though the client's
/// own texture over the same surface is plain `BGRA8Unorm`. An `IOSurface` is
/// just bytes; the transfer function belongs to the view of it, and Syphon's
/// convention is display-encoded 8-bit BGRA.
///
/// Building our own texture rather than wrapping the client's is load-bearing
/// rather than tidiness. `wgpu::hal::metal` stores whatever format the caller
/// declares with no validation against the raw texture, and `create_texture_view`
/// returns the raw texture untouched whenever the declared formats agree. So
/// importing Syphon's linear texture while claiming `Bgra8UnormSrgb` would skip
/// the sRGB decode entirely, silently, and nothing would catch it. Owning the
/// Metal texture makes the format we declare the format we actually have.
/// See /spec/syphon-zero-copy.md § Why the receive path must re-wrap the `IOSurface`.
fn import_surface_texture(
    mtl_dev: &Retained<ProtocolObject<dyn MTLDevice>>,
    device: &wgpu::Device,
    surface: &IOSurfaceRef,
    server_name: &str,
    width: u32,
    height: u32,
) -> Option<(wgpu::Texture, wgpu::TextureView)> {
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm_sRGB,
            width as usize,
            height as usize,
            false,
        )
    };
    desc.setUsage(MTLTextureUsage::ShaderRead);
    let mtl = mtl_dev.newTextureWithDescriptor_iosurface_plane(&desc, surface, 0)?;

    let hal_texture = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            mtl,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
            None,
        )
    };
    let texture = unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Metal>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some(&format!("Syphon Client: {server_name}")),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::TextureUses::RESOURCE,
        )
    };
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Some((texture, view))
}

impl Drop for SyphonManager {
    fn drop(&mut self) {
        for r in &self.receivers {
            unsafe {
                let _: () = msg_send![&r.client, stop];
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
    // Readback types the production path no longer needs: nothing is copied out
    // of a Metal texture at runtime any more, only in these tests.
    use objc2_metal::{MTLOrigin, MTLRegion, MTLSize};

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

    /// Syphon's fast blit is gated on an exact pixel-format match against its own
    /// destination, which `SyphonMetalServer` always creates as
    /// `MTLPixelFormatBGRA8Unorm`. Publishing a `_sRGB` texture instead sends every
    /// frame down the render fallback, which samples with an sRGB decode and writes
    /// to a linear target, linearising the picture: 128 arrives as 55.
    ///
    /// See /spec/syphon-zero-copy.md § The send path corrupts colour today.
    #[test]
    fn publish_slot_matches_syphons_destination_pixel_format() {
        let Ok(ctx) = crate::renderer::context::GpuContext::new_headless() else {
            return; // no GPU on this machine
        };
        let Some(mtl_dev) =
            unsafe { ctx.device.as_hal::<wgpu::hal::api::Metal>() }.map(|d| d.raw_device().clone())
        else {
            return; // not the Metal backend
        };
        let slot =
            SyphonManager::make_publish_slot(&mtl_dev, &ctx.device, 64, 64).expect("publish slot");
        assert_eq!(
            slot.mtl.pixelFormat(),
            MTLPixelFormat::BGRA8Unorm,
            "publish slots must match SyphonMetalServer's BGRA8Unorm destination, \
             or every frame takes the linearising render fallback"
        );
    }

    /// End to end against the installed framework: publish a known ramp and read
    /// the published `IOSurface` straight back off the server.
    ///
    /// This is the shape of test that catches the linearisation defect. Checking
    /// the arguments we hand Syphon cannot, because the corruption happens inside
    /// Syphon, one step past anything we can inspect.
    #[test]
    fn published_bytes_survive_the_publish_path() {
        const SERVER: &str = "Varda Publish Self Test";
        const N: u32 = 64;
        // Neutral greys. 128 is the value the defect turned into 55.
        const BANDS: [u8; 5] = [0, 64, 128, 192, 255];

        let mut mgr = SyphonManager::new();
        if !mgr.is_available() {
            return; // Syphon.framework not installed on this machine
        }
        let Ok(ctx) = crate::renderer::context::GpuContext::new_headless() else {
            return;
        };

        // A stand-in for a headless output, which is Rgba8UnormSrgb (see
        // HeadlessOutput::storage_formats): display-encoded bytes, sRGB tagged.
        let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("syphon self test source"),
            size: wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut pixels = vec![0u8; (N * N * 4) as usize];
        for y in 0..N {
            let v = BANDS[(y as usize * BANDS.len()) / N as usize];
            for x in 0..N {
                let p = ((y * N + x) * 4) as usize;
                pixels[p] = v;
                pixels[p + 1] = v;
                pixels[p + 2] = v;
                pixels[p + 3] = 255;
            }
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(N * 4),
                rows_per_image: Some(N),
            },
            wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
        );
        let view = src.create_view(&wgpu::TextureViewDescriptor::default());

        // The scheduler publishes a slot only once its blit has signalled
        // completion, so this needs several rounds. The image is static, so any
        // completed publish is the whole answer.
        let mut out = vec![0u8; (N * N * 4) as usize];
        let mut matched = false;
        for _ in 0..16 {
            mgr.publish_frame_gpu(&ctx, SERVER, &view, N, N);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            std::thread::sleep(std::time::Duration::from_millis(4));
            let Some(handle) = mgr.servers.get(SERVER) else {
                continue;
            };
            let dst: Option<Retained<ProtocolObject<dyn MTLTexture>>> =
                unsafe { msg_send![&handle.server, newFrameImage] };
            let Some(dst) = dst else { continue };
            let region = MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: N as usize,
                    height: N as usize,
                    depth: 1,
                },
            };
            unsafe {
                dst.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    std::ptr::NonNull::new(out.as_mut_ptr().cast::<std::ffi::c_void>()).unwrap(),
                    (N * 4) as usize,
                    region,
                    0,
                );
            }
            if BANDS.iter().enumerate().all(|(i, expected)| {
                let y = (i as u32 * N) / BANDS.len() as u32 + (N / BANDS.len() as u32) / 2;
                out[((y * N + N / 2) * 4) as usize] == *expected
            }) {
                matched = true;
                break;
            }
        }

        assert!(
            matched,
            "published values were altered in transit. Sampled: {:?}, expected {BANDS:?}. \
             A 128 arriving as 55 is Syphon's render fallback applying an sRGB decode \
             with no matching encode.",
            BANDS
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let y = (i as u32 * N) / BANDS.len() as u32 + (N / BANDS.len() as u32) / 2;
                    out[((y * N + N / 2) * 4) as usize]
                })
                .collect::<Vec<_>>()
        );
    }

    /// The trap `import_surface_texture` exists to avoid, tested by behaviour
    /// rather than by inspecting a format constant.
    ///
    /// `wgpu::hal::metal` believes whatever format an imported texture declares
    /// and hands back the raw Metal texture whenever the declared formats agree,
    /// so importing Syphon's linear `BGRA8Unorm` texture while claiming
    /// `Bgra8UnormSrgb` would skip the sRGB decode with nothing to catch it.
    /// Varda builds its own `BGRA8Unorm_sRGB` texture over the surface instead.
    ///
    /// Sampling a value of 128 must therefore yield linear 0.216, which lands at
    /// 55 in a linear target. If the decode were being skipped it would arrive
    /// back as 128. See /spec/syphon-zero-copy.md.
    #[test]
    fn an_imported_surface_actually_applies_the_srgb_decode() {
        const SERVER: &str = "Varda Import Self Test";
        const N: u32 = 64; // N * 4 == 256, the required copy row alignment
        const MID: u8 = 128;
        const EXPECTED_LINEAR: u8 = 55;

        let mut mgr = SyphonManager::new();
        if !mgr.is_available() {
            return; // Syphon.framework not installed on this machine
        }
        let Ok(ctx) = crate::renderer::context::GpuContext::new_headless() else {
            return;
        };
        let Some(mtl_dev) = mgr.wgpu_metal_device(&ctx.device) else {
            return; // not the Metal backend
        };

        // A flat mid-grey source, published so Syphon owns a real surface.
        let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("syphon import test source"),
            size: wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let flat = vec![MID; (N * N * 4) as usize];
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &flat,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(N * 4),
                rows_per_image: Some(N),
            },
            wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
        );
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());

        let mut imported = None;
        for _ in 0..16 {
            mgr.publish_frame_gpu(&ctx, SERVER, &src_view, N, N);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            std::thread::sleep(std::time::Duration::from_millis(4));
            let Some(handle) = mgr.servers.get(SERVER) else {
                continue;
            };
            let published: Option<Retained<ProtocolObject<dyn MTLTexture>>> =
                unsafe { msg_send![&handle.server, newFrameImage] };
            let Some(published) = published else { continue };
            let Some(surface) = published.iosurface() else {
                continue;
            };
            imported = import_surface_texture(&mtl_dev, &ctx.device, &surface, "test", N, N);
            if imported.is_some() {
                break;
            }
        }
        let Some((_shared, shared_view)) = imported else {
            panic!("could not import the published IOSurface");
        };

        // Sample the imported texture into a *linear* target: whatever decode the
        // texture applies on sample is what lands in the bytes read back.
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("syphon import test target"),
            size: wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let pipeline =
            crate::renderer::blit::BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgba8Unorm)
                .expect("blit pipeline");
        let bind = pipeline.create_bind_group(&ctx.device, &shared_view);
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("syphon import test readback"),
            size: u64::from(N * N * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("syphon import test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
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
            pipeline.render(&mut pass, &bind);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(N * 4),
                    rows_per_image: Some(N),
                },
            },
            wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let centre = mapped[((N / 2 * N + N / 2) * 4) as usize];

        assert!(
            centre.abs_diff(EXPECTED_LINEAR) <= 2,
            "imported surface sampled as {centre}, expected about {EXPECTED_LINEAR}. \
             A value of {MID} coming back unchanged means the sRGB decode was skipped, \
             which is what importing Syphon's linear texture under an sRGB label does."
        );
    }

    /// `start_receive` plus `update` against a live server, end to end.
    ///
    /// Discovery is bypassed by handing the manager the server's own description,
    /// because `SyphonServerDirectory` resolves names through distributed
    /// notifications that a test has no run loop to deliver. The client handshake
    /// does need the run loop, so this pumps it.
    ///
    /// The assertion is that the placeholder texture gets replaced by one sized to
    /// the actual shared surface, which only happens if the poll, the `IOSurface`
    /// address compare, and the import all worked.
    #[test]
    fn a_receiver_binds_the_servers_shared_surface() {
        use objc2_foundation::{NSDate, NSRunLoop};

        const SERVER: &str = "Varda Receive Self Test";
        const N: u32 = 64;

        let mut mgr = SyphonManager::new();
        if !mgr.is_available() {
            return; // Syphon.framework not installed on this machine
        }
        let Ok(ctx) = crate::renderer::context::GpuContext::new_headless() else {
            return;
        };

        let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("syphon receive test source"),
            size: wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());

        // One publish to bring the server, and therefore a surface, into being.
        mgr.publish_frame_gpu(&ctx, SERVER, &src_view, N, N);
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        let Some(handle) = mgr.servers.get(SERVER) else {
            panic!("publish did not create a server");
        };
        let desc: Retained<NSDictionary<NSString, AnyObject>> =
            unsafe { msg_send![&handle.server, serverDescription] };
        mgr.descriptions.push((SERVER.to_string(), desc));

        let Some(idx) = mgr.start_receive(SERVER, &ctx.device) else {
            panic!("start_receive refused a server it had a description for");
        };
        // The placeholder, until a surface arrives.
        assert_eq!(mgr.client_dimensions(idx), Some((1920, 1080)));

        let mut bound = false;
        for _ in 0..40 {
            mgr.publish_frame_gpu(&ctx, SERVER, &src_view, N, N);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            // Syphon's connection manager settles on the run loop.
            let until = NSDate::dateWithTimeIntervalSinceNow(0.02);
            NSRunLoop::currentRunLoop().runUntilDate(&until);
            mgr.update(&ctx.device);
            if mgr.client_dimensions(idx) == Some((N, N)) {
                bound = true;
                break;
            }
        }

        assert!(
            bound,
            "receiver never bound the shared surface; still at {:?}",
            mgr.client_dimensions(idx)
        );
        assert!(
            mgr.is_connected(idx),
            "a receiver holding a live surface must report connected"
        );
        assert!(mgr.texture_view(idx).is_some());
    }
}
