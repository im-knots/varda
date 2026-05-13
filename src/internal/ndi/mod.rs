//! NDI (Network Device Interface) — send and receive video over LAN.
//!
//! Uses dynamic loading (`libloading`) so the NDI SDK is only required at runtime.
//! Input follows the CameraManager pattern: background receive thread →
//! Arc<Mutex<Option<Vec<u8>>>> → main-thread GPU upload.

#[allow(non_camel_case_types, non_snake_case, dead_code)]
pub mod ffi;
pub mod sdk;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

/// Discovered NDI source on the network.
#[derive(Debug, Clone)]
pub struct NdiSource {
    /// Source name (e.g. "MY-PC (Source 1)")
    pub name: String,
}

/// Manages NDI discovery, receive, and send.
pub struct NdiManager {
    sdk: Option<sdk::NdiSdk>,
    sources: Vec<NdiSource>,
    receivers: Vec<NdiReceiver>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// Active NDI senders keyed by sender name.
    senders: HashMap<String, NdiSender>,
}

/// Shared frame payload passed from receive thread → main thread.
struct NdiFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

struct NdiReceiver {
    #[allow(dead_code)]
    source_name: String,
    frame_data: Arc<Mutex<Option<NdiFrame>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    _thread: Option<std::thread::JoinHandle<()>>,
    #[allow(dead_code)]
    recv_instance: ffi::NDIlib_recv_instance_t,
    width: u32,
    height: u32,
}

struct NdiSender {
    instance: ffi::NDIlib_send_instance_t,
    /// Reusable UYVY conversion buffer (avoids per-frame allocation).
    uyvy_buf: Vec<u8>,
}

impl NdiManager {
    pub fn new() -> Self {
        let sdk = sdk::NdiSdk::load();
        if let Some(ref sdk) = sdk {
            let ok = unsafe { (sdk.initialize)() };
            if ok {
                log::info!("NDI SDK initialized successfully");
            } else {
                log::error!("NDI SDK initialize() returned false");
            }
        } else {
            log::info!("NDI SDK not found — NDI features disabled");
        }
        Self {
            sdk,
            sources: Vec::new(),
            receivers: Vec::new(),
            textures: Vec::new(),
            senders: HashMap::new(),
        }
    }

    /// Create a disabled NDI manager (CLI `--no-ndi` flag).
    pub fn new_disabled() -> Self {
        Self {
            sdk: None,
            sources: Vec::new(),
            receivers: Vec::new(),
            textures: Vec::new(),
            senders: HashMap::new(),
        }
    }

    pub fn is_available(&self) -> bool { self.sdk.is_some() }
    pub fn sources(&self) -> &[NdiSource] { &self.sources }

    /// Return discovered source names for UI display.
    pub fn discovered_sources(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name.clone()).collect()
    }

    /// Scan for NDI sources on the network (2s timeout).
    pub fn discover(&mut self) {
        let sdk = match &self.sdk {
            Some(s) => s,
            None => return,
        };
        self.sources.clear();

        unsafe {
            let find_settings = ffi::NDIlib_find_create_t::default();
            let finder = (sdk.find_create_v2)(&find_settings);
            if finder.is_null() {
                log::warn!("NDI find_create_v2 returned null");
                return;
            }

            // Wait up to 2 seconds for sources to appear
            (sdk.find_wait_for_sources)(finder, 2000);

            let mut count: std::os::raw::c_uint = 0;
            let sources_ptr = (sdk.find_get_current_sources)(finder, &mut count);

            if !sources_ptr.is_null() && count > 0 {
                let sources_slice = std::slice::from_raw_parts(sources_ptr, count as usize);
                for src in sources_slice {
                    if !src.p_ndi_name.is_null() {
                        let name = std::ffi::CStr::from_ptr(src.p_ndi_name)
                            .to_string_lossy()
                            .into_owned();
                        self.sources.push(NdiSource { name });
                    }
                }
            }
            log::info!("NDI discovery found {} sources", self.sources.len());
            (sdk.find_destroy)(finder);
        }
    }

    /// Start receiving from a named NDI source.
    /// Spawns a background thread that captures frames into shared memory.
    pub fn start_receive(&mut self, source_name: &str, device: &wgpu::Device) -> Option<usize> {
        let sdk = match &self.sdk {
            Some(s) => s,
            None => {
                log::warn!("Cannot receive NDI: SDK not available");
                return None;
            }
        };

        let frame_data: Arc<Mutex<Option<NdiFrame>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (width, height) = (1920u32, 1080u32);

        // Create the NDI source struct
        let name_c = std::ffi::CString::new(source_name).ok()?;
        let ndi_source = ffi::NDIlib_source_t {
            p_ndi_name: name_c.as_ptr(),
            p_url_address: std::ptr::null(),
        };

        let recv_name = std::ffi::CString::new("Varda Receiver").unwrap();
        let recv_settings = ffi::NDIlib_recv_create_v3_t {
            source_to_connect_to: ndi_source,
            color_format: 0, // BGRX/BGRA — most reliable for conversion
            bandwidth: 100, // highest quality
            allow_video_fields: false,
            p_ndi_recv_name: recv_name.as_ptr(),
        };

        let recv_instance = unsafe { (sdk.recv_create_v3)(&recv_settings) };
        if recv_instance.is_null() {
            log::error!("NDI recv_create_v3 returned null for '{}'", source_name);
            return None;
        }

        let width = width.max(1);
        let height = height.max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("NDI Receive: {}", source_name)),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let connected = Arc::new(AtomicBool::new(false));

        // Spawn background receive thread
        let frame_clone = Arc::clone(&frame_data);
        let stop_clone = Arc::clone(&stop_flag);
        let connected_clone = Arc::clone(&connected);
        // Note: recv_instance is a raw pointer, sent across thread boundary.
        // Safe because NDI SDK guarantees thread safety for recv instances.
        let recv_ptr = recv_instance as usize;
        let recv_destroy_fn = sdk.recv_destroy as usize;
        let recv_capture_fn = sdk.recv_capture_v3 as usize;
        let recv_free_fn = sdk.recv_free_video_v2 as usize;

        let source_name_log = source_name.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("ndi-recv-{}", source_name))
            .spawn(move || {
                let recv = recv_ptr as ffi::NDIlib_recv_instance_t;
                let capture_fn: unsafe extern "C" fn(ffi::NDIlib_recv_instance_t, *mut ffi::NDIlib_video_frame_v2_t, *mut std::ffi::c_void, *mut std::ffi::c_void, std::os::raw::c_uint) -> ffi::NDIlib_frame_type_e
                    = unsafe { std::mem::transmute(recv_capture_fn) };
                let free_fn: unsafe extern "C" fn(ffi::NDIlib_recv_instance_t, *const ffi::NDIlib_video_frame_v2_t)
                    = unsafe { std::mem::transmute(recv_free_fn) };

                let mut frame_count: u64 = 0;
                let mut none_count: u64 = 0;

                while !stop_clone.load(Ordering::SeqCst) {
                    let mut video_frame = std::mem::MaybeUninit::<ffi::NDIlib_video_frame_v2_t>::zeroed();
                    let frame_type = unsafe {
                        capture_fn(recv, video_frame.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), 100)
                    };

                    if frame_type == ffi::NDIlib_frame_type_e::VIDEO {
                        let vf = unsafe { video_frame.assume_init() };
                        if !vf.p_data.is_null() && vf.xres > 0 && vf.yres > 0 {
                            let w = vf.xres as u32;
                            let h = vf.yres as u32;
                            if frame_count == 0 {
                                log::info!("NDI '{}': first frame {}×{} FourCC={:?} stride={}", source_name_log, w, h, vf.FourCC, vf.line_stride_in_bytes);
                            }
                            frame_count += 1;
                            none_count = 0;
                            connected_clone.store(true, Ordering::SeqCst);
                            let rgba = convert_ndi_frame_to_rgba(&vf, w, h);
                            if let Ok(mut guard) = frame_clone.lock() {
                                *guard = Some(NdiFrame { data: rgba, width: w, height: h });
                            }
                        }
                        unsafe { free_fn(recv, &vf) };
                    } else if frame_type == ffi::NDIlib_frame_type_e::NONE {
                        none_count += 1;
                        if none_count == 30 {
                            log::warn!("NDI '{}': 30 consecutive empty captures — source may not be sending", source_name_log);
                        }
                        if none_count >= 50 {
                            connected_clone.store(false, Ordering::SeqCst);
                        }
                    } else if frame_type == ffi::NDIlib_frame_type_e::STATUS_CHANGE {
                        log::info!("NDI '{}': connection status changed", source_name_log);
                    } else if frame_type == ffi::NDIlib_frame_type_e::ERROR {
                        log::warn!("NDI '{}': received ERROR frame", source_name_log);
                        connected_clone.store(false, Ordering::SeqCst);
                    }
                }

                log::info!("NDI '{}': receive thread stopping (captured {} frames)", source_name_log, frame_count);
                let destroy_fn: unsafe extern "C" fn(ffi::NDIlib_recv_instance_t)
                    = unsafe { std::mem::transmute(recv_destroy_fn) };
                unsafe { destroy_fn(recv) };
            })
            .ok();

        let idx = self.receivers.len();
        self.receivers.push(NdiReceiver {
            source_name: source_name.to_string(),
            frame_data,
            stop_flag,
            connected,
            _thread: thread,
            recv_instance: std::ptr::null_mut(), // Owned by the thread now
            width,
            height,
        });
        self.textures.push((texture, texture_view));
        log::info!("NDI receiver started for '{}'", source_name);
        Some(idx)
    }

    /// Upload latest frames from all receivers to GPU.
    /// Dynamically recreates textures when the received resolution changes.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for i in 0..self.receivers.len() {
            let new_frame = {
                if let Ok(mut guard) = self.receivers[i].frame_data.try_lock() {
                    guard.take()
                } else {
                    None
                }
            };
            if let Some(frame) = new_frame {
                let fw = frame.width;
                let fh = frame.height;
                let expected = (fw * fh * 4) as usize;
                if frame.data.len() < expected {
                    continue;
                }
                // Recreate texture if dimensions changed
                let fw = fw.max(1);
                let fh = fh.max(1);
                if fw != self.receivers[i].width || fh != self.receivers[i].height {
                    log::info!("NDI receiver {}: resolution changed {}×{} → {}×{}", i, self.receivers[i].width, self.receivers[i].height, fw, fh);
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("NDI Receive (resized)"),
                        size: wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
                        mip_level_count: 1, sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    self.textures[i] = (texture, texture_view);
                    self.receivers[i].width = fw;
                    self.receivers[i].height = fh;
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.textures[i].0, mip_level: 0,
                        origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
                    },
                    &frame.data[..expected],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0, bytes_per_row: Some(fw * 4),
                        rows_per_image: Some(fh),
                    },
                    wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
                );
            }
        }
    }

    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.textures.get(idx).map(|(_, v)| v)
    }

    pub fn receiver_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.receivers.get(idx).map(|r| (r.width, r.height))
    }

    /// Check if a receiver is currently connected (receiving video frames).
    /// Returns `false` for out-of-bounds indices.
    pub fn is_connected(&self, idx: usize) -> bool {
        self.receivers
            .get(idx)
            .map(|r| r.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Send a frame via NDI for a specific sender name.
    /// Creates the sender instance on first call for a given name.
    pub fn send_frame(&mut self, sender_name: &str, rgba: &[u8], width: u32, height: u32) {
        let sdk = match &self.sdk {
            Some(s) => s,
            None => return,
        };

        // Get or create sender for this name
        if !self.senders.contains_key(sender_name) {
            let name_c = match std::ffi::CString::new(sender_name) {
                Ok(c) => c,
                Err(_) => return,
            };
            let settings = ffi::NDIlib_send_create_t {
                p_ndi_name: name_c.as_ptr(),
                p_groups: std::ptr::null(),
                clock_video: true,
                clock_audio: false,
            };
            let instance = unsafe { (sdk.send_create)(&settings) };
            if instance.is_null() {
                log::error!("NDI send_create returned null for '{}'", sender_name);
                return;
            }
            let uyvy_size = (width as usize) * (height as usize) * 2;
            self.senders.insert(sender_name.to_string(), NdiSender {
                instance,
                uyvy_buf: vec![0u8; uyvy_size],
            });
            log::info!("NDI sender created: '{}'", sender_name);
        }

        let sender = if let Some(s) = self.senders.get_mut(sender_name) {
            s
        } else {
            log::error!("NDI sender '{}' not found after creation", sender_name);
            return;
        };

        // Convert RGBA → UYVY
        let uyvy_size = (width as usize) * (height as usize) * 2;
        if sender.uyvy_buf.len() != uyvy_size {
            sender.uyvy_buf.resize(uyvy_size, 0);
        }
        rgba_to_uyvy(rgba, &mut sender.uyvy_buf, width, height);

        let frame = ffi::NDIlib_video_frame_v2_t {
            xres: width as i32,
            yres: height as i32,
            FourCC: ffi::NDIlib_FourCC_video_type_e::UYVY,
            frame_rate_N: 30,
            frame_rate_D: 1,
            picture_aspect_ratio: 0.0,
            frame_format_type: 1, // progressive
            timecode: 0, // auto
            p_data: sender.uyvy_buf.as_mut_ptr(),
            line_stride_in_bytes: (width * 2) as i32,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        };

        unsafe { (sdk.send_send_video_v2)(sender.instance, &frame) };
    }

    /// Destroy a specific sender by name.
    pub fn destroy_sender(&mut self, sender_name: &str) {
        if let Some(sender) = self.senders.remove(sender_name) {
            if let Some(ref sdk) = self.sdk {
                unsafe { (sdk.send_destroy)(sender.instance) };
            }
            log::info!("NDI sender destroyed: '{}'", sender_name);
        }
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get_mut(idx) {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r._thread.take() { let _ = t.join(); }
        }
    }
}

impl Drop for NdiManager {
    fn drop(&mut self) {
        // Stop all receivers and join their threads before SDK cleanup
        for r in &mut self.receivers {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r._thread.take() { let _ = t.join(); }
        }
        // Destroy all senders
        let sender_names: Vec<String> = self.senders.keys().cloned().collect();
        for name in sender_names {
            self.destroy_sender(&name);
        }
        // Destroy NDI SDK
        if let Some(ref sdk) = self.sdk {
            unsafe { (sdk.destroy)() };
        }
    }
}

/// Convert an NDI video frame (UYVY, BGRA, BGRX, or RGBA) to RGBA.
fn convert_ndi_frame_to_rgba(vf: &ffi::NDIlib_video_frame_v2_t, w: u32, h: u32) -> Vec<u8> {
    let pixel_count = (w * h) as usize;
    let mut rgba = vec![0u8; pixel_count * 4];

    if vf.FourCC == ffi::NDIlib_FourCC_video_type_e::UYVY {
        let stride = if vf.line_stride_in_bytes > 0 { vf.line_stride_in_bytes as usize } else { w as usize * 2 };
        let uyvy_data = unsafe { std::slice::from_raw_parts(vf.p_data, h as usize * stride) };
        uyvy_to_rgba(uyvy_data, &mut rgba, w, h, stride);
    } else if vf.FourCC == ffi::NDIlib_FourCC_video_type_e::BGRA
           || vf.FourCC == ffi::NDIlib_FourCC_video_type_e::BGRX {
        let stride = if vf.line_stride_in_bytes > 0 { vf.line_stride_in_bytes as usize } else { w as usize * 4 };
        let bgra_data = unsafe { std::slice::from_raw_parts(vf.p_data, h as usize * stride) };
        for y in 0..h as usize {
            for x in 0..w as usize {
                let src = y * stride + x * 4;
                let dst = (y * w as usize + x) * 4;
                rgba[dst] = bgra_data[src + 2]; // R
                rgba[dst + 1] = bgra_data[src + 1]; // G
                rgba[dst + 2] = bgra_data[src]; // B
                rgba[dst + 3] = 255; // Alpha (forced opaque — BGRX has undefined alpha)
            }
        }
    } else if vf.FourCC == ffi::NDIlib_FourCC_video_type_e::RGBA {
        let data_size = pixel_count * 4;
        let src = unsafe { std::slice::from_raw_parts(vf.p_data, data_size) };
        rgba.copy_from_slice(src);
    } else {
        log::warn!("Unknown NDI FourCC: {:?}, filling black", vf.FourCC);
    }

    rgba
}

/// Convert UYVY 4:2:2 to RGBA.
fn uyvy_to_rgba(uyvy: &[u8], rgba: &mut [u8], w: u32, h: u32, stride: usize) {
    for y in 0..h as usize {
        for x in (0..w as usize).step_by(2) {
            let src = y * stride + x * 2;
            if src + 3 >= uyvy.len() { break; }
            let u = uyvy[src] as f32 - 128.0;
            let y0 = uyvy[src + 1] as f32;
            let v = uyvy[src + 2] as f32 - 128.0;
            let y1 = uyvy[src + 3] as f32;

            let dst0 = (y * w as usize + x) * 4;
            let dst1 = dst0 + 4;

            rgba[dst0] = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
            rgba[dst0 + 1] = (y0 - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
            rgba[dst0 + 2] = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;
            rgba[dst0 + 3] = 255;

            if x + 1 < w as usize && dst1 + 3 < rgba.len() {
                rgba[dst1] = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
                rgba[dst1 + 1] = (y1 - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
                rgba[dst1 + 2] = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;
                rgba[dst1 + 3] = 255;
            }
        }
    }
}

/// Convert RGBA to UYVY 4:2:2 for NDI sending.
fn rgba_to_uyvy(rgba: &[u8], uyvy: &mut [u8], w: u32, h: u32) {
    for y in 0..h as usize {
        for x in (0..w as usize).step_by(2) {
            let src0 = (y * w as usize + x) * 4;
            let src1 = src0 + 4;

            let r0 = rgba[src0] as f32;
            let g0 = rgba[src0 + 1] as f32;
            let b0 = rgba[src0 + 2] as f32;

            let (r1, g1, b1) = if x + 1 < w as usize && src1 + 2 < rgba.len() {
                (rgba[src1] as f32, rgba[src1 + 1] as f32, rgba[src1 + 2] as f32)
            } else {
                (r0, g0, b0)
            };

            let y0 = (0.299 * r0 + 0.587 * g0 + 0.114 * b0).clamp(0.0, 255.0);
            let y1 = (0.299 * r1 + 0.587 * g1 + 0.114 * b1).clamp(0.0, 255.0);
            let u = (-0.169 * r0 - 0.331 * g0 + 0.500 * b0 + 128.0).clamp(0.0, 255.0);
            let v = (0.500 * r0 - 0.419 * g0 - 0.081 * b0 + 128.0).clamp(0.0, 255.0);

            let dst = y * w as usize * 2 + x * 2;
            if dst + 3 < uyvy.len() {
                uyvy[dst] = u as u8;
                uyvy[dst + 1] = y0 as u8;
                uyvy[dst + 2] = v as u8;
                uyvy[dst + 3] = y1 as u8;
            }
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndi_manager_new_no_crash() {
        // NdiManager::new() attempts to load SDK dynamically.
        // On machines without NDI SDK, it gracefully returns with sdk=None.
        let mgr = NdiManager::new();
        // is_available depends on whether SDK is installed — just verify no panic
        let _ = mgr.is_available();
    }

    #[test]
    fn ndi_manager_sources_empty() {
        let mgr = NdiManager::new();
        assert!(mgr.sources().is_empty());
        assert!(mgr.discovered_sources().is_empty());
    }

    #[test]
    fn ndi_manager_texture_view_out_of_bounds() {
        let mgr = NdiManager::new();
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.texture_view(999).is_none());
    }

    #[test]
    fn ndi_manager_receiver_dimensions_out_of_bounds() {
        let mgr = NdiManager::new();
        assert!(mgr.receiver_dimensions(0).is_none());
    }

    #[test]
    fn ndi_manager_is_connected_out_of_bounds() {
        let mgr = NdiManager::new();
        assert!(!mgr.is_connected(0));
        assert!(!mgr.is_connected(999));
    }

    #[test]
    fn ndi_source_debug() {
        let src = NdiSource { name: "Test Source".to_string() };
        let debug = format!("{:?}", src);
        assert!(debug.contains("Test Source"));
    }

    #[test]
    fn ndi_source_clone() {
        let src = NdiSource { name: "Test".to_string() };
        let cloned = src.clone();
        assert_eq!(src.name, cloned.name);
    }

    // ── Color conversion tests ──────────────────────────────────────

    #[test]
    fn uyvy_to_rgba_pure_white() {
        // UYVY for white: Y=235, U=128, V=128
        // Two pixels per macropixel: [U, Y0, V, Y1]
        let uyvy = vec![128, 235, 128, 235];
        let mut rgba = vec![0u8; 2 * 4]; // 2 pixels
        uyvy_to_rgba(&uyvy, &mut rgba, 2, 1, 4);
        // Y=235, U=0 (128-128), V=0 (128-128) → R=235, G=235, B=235
        assert_eq!(rgba[0], 235); // R
        assert_eq!(rgba[1], 235); // G
        assert_eq!(rgba[2], 235); // B
        assert_eq!(rgba[3], 255); // A
        assert_eq!(rgba[4], 235); // R (second pixel)
        assert_eq!(rgba[7], 255); // A
    }

    #[test]
    fn uyvy_to_rgba_pure_black() {
        // UYVY for black: Y=0, U=128, V=128
        let uyvy = vec![128, 0, 128, 0];
        let mut rgba = vec![0u8; 2 * 4];
        uyvy_to_rgba(&uyvy, &mut rgba, 2, 1, 4);
        assert_eq!(rgba[0], 0); // R
        assert_eq!(rgba[1], 0); // G
        assert_eq!(rgba[2], 0); // B
        assert_eq!(rgba[3], 255); // A (always opaque)
    }

    #[test]
    fn uyvy_to_rgba_alpha_always_255() {
        // Regardless of input, alpha should always be 255
        let uyvy = vec![100, 150, 200, 100];
        let mut rgba = vec![0u8; 2 * 4];
        uyvy_to_rgba(&uyvy, &mut rgba, 2, 1, 4);
        assert_eq!(rgba[3], 255);
        assert_eq!(rgba[7], 255);
    }

    #[test]
    fn uyvy_to_rgba_multirow() {
        let w = 4u32;
        let h = 2u32;
        let stride = w as usize * 2;
        // Fill with neutral gray (Y=128, U=128, V=128)
        let uyvy = vec![128u8; (h as usize) * stride];
        let mut rgba = vec![0u8; (w * h) as usize * 4];
        uyvy_to_rgba(&uyvy, &mut rgba, w, h, stride);
        // Check all alphas are 255
        for i in 0..(w * h) as usize {
            assert_eq!(rgba[i * 4 + 3], 255, "alpha at pixel {} should be 255", i);
        }
    }

    #[test]
    fn rgba_to_uyvy_pure_black() {
        let rgba = vec![0u8; 2 * 4]; // 2 black pixels
        let mut uyvy = vec![0u8; 4]; // 1 macropixel
        rgba_to_uyvy(&rgba, &mut uyvy, 2, 1);
        // Black: Y≈0, U≈128, V≈128
        assert_eq!(uyvy[1], 0); // Y0
        assert_eq!(uyvy[3], 0); // Y1
        assert_eq!(uyvy[0], 128); // U
        assert_eq!(uyvy[2], 128); // V
    }

    #[test]
    fn rgba_to_uyvy_pure_white() {
        let rgba = vec![255, 255, 255, 255, 255, 255, 255, 255];
        let mut uyvy = vec![0u8; 4];
        rgba_to_uyvy(&rgba, &mut uyvy, 2, 1);
        // White: Y≈255, U≈128, V≈128
        assert_eq!(uyvy[1], 255); // Y0
        assert_eq!(uyvy[3], 255); // Y1
        assert_eq!(uyvy[0], 128); // U
        assert_eq!(uyvy[2], 128); // V
    }

    #[test]
    fn rgba_uyvy_roundtrip_gray() {
        // Roundtrip test: RGBA → UYVY → RGBA should be close to original
        let original_rgba = vec![128, 128, 128, 255, 128, 128, 128, 255];
        let mut uyvy = vec![0u8; 4];
        rgba_to_uyvy(&original_rgba, &mut uyvy, 2, 1);

        let mut restored = vec![0u8; 2 * 4];
        uyvy_to_rgba(&uyvy, &mut restored, 2, 1, 4);

        // Allow ±2 tolerance for floating point in color space conversion
        for i in 0..2 {
            for c in 0..3 {
                let orig = original_rgba[i * 4 + c] as i32;
                let rest = restored[i * 4 + c] as i32;
                assert!((orig - rest).abs() <= 2,
                    "pixel {} channel {} mismatch: {} vs {}", i, c, orig, rest);
            }
        }
    }

    #[test]
    fn rgba_uyvy_roundtrip_red() {
        let original = vec![255, 0, 0, 255, 255, 0, 0, 255];
        let mut uyvy = vec![0u8; 4];
        rgba_to_uyvy(&original, &mut uyvy, 2, 1);

        let mut restored = vec![0u8; 2 * 4];
        uyvy_to_rgba(&uyvy, &mut restored, 2, 1, 4);

        // Red should survive roundtrip within tolerance
        for i in 0..2 {
            let r = restored[i * 4] as i32;
            let g = restored[i * 4 + 1] as i32;
            let b = restored[i * 4 + 2] as i32;
            assert!(r > 200, "red channel should be high: {}", r);
            assert!(g < 50, "green channel should be low: {}", g);
            assert!(b < 50, "blue channel should be low: {}", b);
        }
    }

    #[test]
    fn uyvy_to_rgba_stride_larger_than_width() {
        // Stride can be larger than width*2 (padding)
        let w = 2u32;
        let h = 1u32;
        let stride = 8; // padded stride (width*2=4, but stride=8)
        let mut uyvy = vec![0u8; stride];
        // Set actual pixel data in first 4 bytes
        uyvy[0] = 128; // U
        uyvy[1] = 200; // Y0
        uyvy[2] = 128; // V
        uyvy[3] = 200; // Y1
        // Bytes 4-7 are padding

        let mut rgba = vec![0u8; 2 * 4];
        uyvy_to_rgba(&uyvy, &mut rgba, w, h, stride);
        assert_eq!(rgba[0], 200); // R = Y + 0 = 200
        assert_eq!(rgba[3], 255); // A
    }
}