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

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{extern_class, msg_send, AnyThread, ClassType};
use objc2_foundation::{NSArray, NSDictionary, NSString};
use objc2_metal::{
    MTLCreateSystemDefaultDevice, MTLDevice, MTLOrigin, MTLRegion, MTLSize, MTLTexture,
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

/// Manages Syphon server discovery and client connections.
pub struct SyphonManager {
    available: bool,
    sources: Vec<SyphonSource>,
    /// Server-description dicts kept alongside `sources` by name, needed to init
    /// a `SyphonMetalClient`.
    descriptions: Vec<(String, Retained<NSDictionary<NSString, AnyObject>>)>,
    clients: Vec<SyphonClient>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// Our own Metal device for the clients (one GPU on M-series). Lazily created.
    metal_device: Option<Retained<ProtocolObject<dyn MTLDevice>>>,
    /// wgpu device clone, captured at first `start_receive`, so `update()` can
    /// (re)create textures when a server's frame size becomes known/changes.
    device: Option<wgpu::Device>,
}

struct SyphonClient {
    #[allow(dead_code)]
    server_name: String,
    client: Retained<SyphonMetalClient>,
    stop_flag: Arc<AtomicBool>,
    width: u32,
    height: u32,
}

// Single-threaded ownership: the manager lives on (and is only touched from)
// varda's render thread. Metal objects aren't Sync but are never shared.
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
            clients: Vec::new(),
            textures: Vec::new(),
            metal_device: None,
            device: None,
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

    /// Start receiving from a named Syphon server. Returns a client index used by
    /// `texture_view()` / `client_dimensions()`.
    pub fn start_receive(&mut self, server_name: &str, device: &wgpu::Device) -> Option<usize> {
        if !self.available {
            log::warn!("Cannot receive Syphon: framework not available");
            return None;
        }
        if self.device.is_none() {
            self.device = Some(device.clone());
        }

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
            // newFrameHandler: nil — we poll in update() instead of using the block.
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

        let idx = self.clients.len();
        self.clients.push(SyphonClient {
            server_name: server_name.to_string(),
            client,
            stop_flag: Arc::new(AtomicBool::new(false)),
            width,
            height,
        });
        self.textures.push((texture, view));
        log::info!("Syphon client connected to '{}'", server_name);
        Some(idx)
    }

    /// Pull the newest frame from each client and upload it to its wgpu texture.
    /// Runs on the render thread (callers already hold `&mut self`).
    pub fn update(&mut self, queue: &wgpu::Queue) {
        for i in 0..self.clients.len() {
            if self.clients[i].stop_flag.load(Ordering::Relaxed) {
                continue;
            }
            // Latest completed frame, or None if nothing new.
            let tex: Option<Retained<ProtocolObject<dyn MTLTexture>>> =
                unsafe { msg_send![&self.clients[i].client, newFrameImage] };
            let Some(tex) = tex else { continue };

            let w = tex.width() as u32;
            let h = tex.height() as u32;
            if w == 0 || h == 0 {
                continue;
            }

            // Resize the wgpu texture if the server's frame size changed.
            if w != self.clients[i].width || h != self.clients[i].height {
                if let Some(dev) = self.device.clone() {
                    let name = self.clients[i].server_name.clone();
                    let (t, v) = make_texture(&dev, &name, w, h);
                    self.textures[i] = (t, v);
                    self.clients[i].width = w;
                    self.clients[i].height = h;
                }
            }

            // CPU readback from the IOSurface-backed Metal texture.
            // NOTE: requires the source texture to be Shared/Managed storage
            // (Syphon's surface textures are Shared on Apple silicon).
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

            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.textures[i].0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &buf,
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
        self.clients.get(idx).map(|c| (c.width, c.height))
    }

    /// Publishing from varda is out of scope here (varda is the client).
    pub fn publish_frame(&mut self, _rgba: &[u8], _width: u32, _height: u32) {}

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(c) = self.clients.get_mut(idx) {
            c.stop_flag.store(true, Ordering::SeqCst);
            unsafe {
                let _: () = msg_send![&c.client, stop];
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
        for c in &mut self.clients {
            c.stop_flag.store(true, Ordering::SeqCst);
            unsafe {
                let _: () = msg_send![&c.client, stop];
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
}
