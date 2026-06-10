//! Camera capture manager — shared camera sessions for N deck consumers.
//!
//! One `CameraManager` owns all camera capture sessions. Each physical camera
//! produces one shared GPU texture that any number of decks can read from.
//!
//! Capture runs on a dedicated thread per camera. YUV→RGBA conversion uses
//! SIMD-accelerated yuvutils-rs for NV12 (macOS) and YUYV (macOS/Linux).
//! MJPEG frames (common on Linux V4L2) use nokhwa's built-in mozjpeg decoder.

use anyhow::{Context, Result};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use yuvutils_rs::{
    yuv_nv12_to_rgba, yuyv422_to_rgba, YuvBiPlanarImage, YuvConversionMode, YuvPackedImage,
    YuvRange, YuvStandardMatrix,
};

/// Opaque camera identifier (matches the OS-assigned index).
pub type CameraId = u32;

/// Information about a detected camera device.
#[derive(Debug, Clone)]
pub struct CameraDeviceInfo {
    pub id: CameraId,
    pub name: String,
    pub index: CameraIndex,
}

/// An active camera capture session with its shared GPU texture.
struct ActiveCamera {
    /// Shared GPU texture that decks read from.
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// How many decks are using this camera.
    ref_count: u32,
    /// Latest decoded RGBA frame — capture thread swaps in, main thread takes.
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    /// Signal to stop the capture thread.
    stop_flag: Arc<AtomicBool>,
    /// Whether the camera is actively producing frames.
    connected: Arc<AtomicBool>,
    /// Capture thread handle.
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Manages camera device enumeration, capture sessions, and shared textures.
pub struct CameraManager {
    /// Detected camera devices (refreshed periodically).
    devices: Vec<CameraDeviceInfo>,
    /// Active camera sessions keyed by CameraId.
    active: HashMap<CameraId, ActiveCamera>,
    /// Whether nokhwa has been initialized.
    initialized: bool,
}

impl Default for CameraManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            devices: Vec::new(),
            active: HashMap::new(),
            initialized: false,
        };
        mgr.initialize();
        mgr.scan_devices();
        mgr
    }

    fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        // On macOS, requests AVFoundation camera permission.
        // On Linux, this is a no-op (V4L2 doesn't need explicit permission).
        nokhwa::nokhwa_initialize(|granted| {
            if granted {
                log::info!("Camera access granted");
            } else {
                log::warn!("Camera access denied by OS");
            }
        });
        self.initialized = true;
    }

    /// Scan for available camera devices.
    pub fn scan_devices(&mut self) {
        match nokhwa::query(nokhwa::utils::ApiBackend::Auto) {
            Ok(cameras) => {
                self.devices = cameras
                    .iter()
                    .enumerate()
                    .map(|(i, info)| CameraDeviceInfo {
                        id: i as CameraId,
                        name: info.human_name().to_string(),
                        index: info.index().clone(),
                    })
                    .collect();
                log::info!("Camera scan: found {} device(s)", self.devices.len());
                for dev in &self.devices {
                    log::info!("  Camera {}: {}", dev.id, dev.name);
                }
            }
            Err(e) => {
                log::warn!("Camera enumeration failed: {}", e);
                self.devices.clear();
            }
        }
    }

    /// Get the list of detected camera devices.
    pub fn devices(&self) -> &[CameraDeviceInfo] {
        &self.devices
    }

    /// Open a camera and start capturing on a dedicated thread.
    /// Returns the camera's resolution. If already open, just increments the ref count.
    pub fn open_camera(&mut self, id: CameraId, device: &wgpu::Device) -> Result<(u32, u32)> {
        if let Some(active) = self.active.get_mut(&id) {
            active.ref_count += 1;
            return Ok((active.width, active.height));
        }

        let dev_info = self
            .devices
            .iter()
            .find(|d| d.id == id)
            .context("Camera device not found")?
            .clone();

        let format =
            RequestedFormat::new::<RgbAFormat>(RequestedFormatType::AbsoluteHighestFrameRate);

        let mut camera = Camera::new(dev_info.index.clone(), format)
            .map_err(|e| anyhow::anyhow!("Failed to open camera '{}': {}", dev_info.name, e))?;

        camera
            .open_stream()
            .map_err(|e| anyhow::anyhow!("Failed to start camera stream: {}", e))?;

        let res = camera.resolution();
        let width = res.width_x;
        let height = res.height_y;
        let cam_fmt = camera.frame_format();
        log::info!(
            "Opened camera '{}': {}x{}, format={:?}, frame_rate={}",
            dev_info.name,
            width,
            height,
            cam_fmt,
            camera.frame_rate()
        );

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Camera {} Texture", dev_info.name)),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Shared frame slot — capture thread swaps in decoded RGBA, main thread takes it
        let frame_data: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let frame_data_tx = Arc::clone(&frame_data);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = Arc::clone(&connected);
        let cam_id = id;
        let cam_w = width;
        let cam_h = height;

        let thread = std::thread::Builder::new()
            .name(format!("camera-{}", cam_id))
            .spawn(move || {
                Self::capture_loop(
                    camera,
                    cam_id,
                    cam_w,
                    cam_h,
                    frame_data_tx,
                    stop_clone,
                    connected_clone,
                );
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn camera thread: {}", e))?;

        self.active.insert(
            id,
            ActiveCamera {
                texture,
                texture_view,
                width,
                height,
                ref_count: 1,
                frame_data,
                stop_flag,
                connected,
                thread: Some(thread),
            },
        );

        Ok((width, height))
    }

    /// Background capture loop — runs on a dedicated thread per camera.
    fn capture_loop(
        mut camera: Camera,
        cam_id: CameraId,
        w: u32,
        h: u32,
        frame_data: Arc<Mutex<Option<Vec<u8>>>>,
        stop: Arc<AtomicBool>,
        connected: Arc<AtomicBool>,
    ) {
        let expected_rgba = (w * h * 4) as usize;
        // Pre-allocated decode buffer — never freed, reused every frame
        let mut rgba_buf = vec![0u8; expected_rgba];
        let mut frame_count: u64 = 0;
        let mut consecutive_errors: u64 = 0;
        let mut backoff_us: u64 = 500; // 500µs initial backoff
        const MAX_BACKOFF_US: u64 = 500_000; // 500ms cap
        const ERROR_THRESHOLD: u64 = 100;
        let start = std::time::Instant::now();

        log::info!("Camera {} capture thread started ({}x{})", cam_id, w, h);

        while !stop.load(Ordering::Relaxed) {
            let buf = match camera.frame() {
                Ok(b) => b,
                Err(_) => {
                    consecutive_errors += 1;
                    if consecutive_errors == ERROR_THRESHOLD {
                        log::warn!(
                            "Camera {}: {} consecutive frame errors — marking disconnected",
                            cam_id,
                            ERROR_THRESHOLD
                        );
                        connected.store(false, Ordering::SeqCst);
                    }
                    std::thread::sleep(std::time::Duration::from_micros(backoff_us));
                    backoff_us = (backoff_us * 2).min(MAX_BACKOFF_US);
                    continue;
                }
            };

            let raw = buf.buffer();
            let fmt = buf.source_frame_format();

            let ok = match fmt {
                FrameFormat::NV12 => {
                    // SIMD NV12→RGBA via yuvutils-rs
                    let y_size = (w * h) as usize;
                    if raw.len() >= y_size + y_size / 2 {
                        let bi = YuvBiPlanarImage {
                            y_plane: &raw[..y_size],
                            y_stride: w,
                            uv_plane: &raw[y_size..],
                            uv_stride: w,
                            width: w,
                            height: h,
                        };
                        yuv_nv12_to_rgba(
                            &bi,
                            &mut rgba_buf,
                            w * 4,
                            YuvRange::Limited,
                            YuvStandardMatrix::Bt709,
                            YuvConversionMode::Balanced,
                        )
                        .is_ok()
                    } else {
                        false
                    }
                }
                FrameFormat::YUYV => {
                    // SIMD YUYV→RGBA via yuvutils-rs
                    let expected_yuyv = (w * h * 2) as usize;
                    if raw.len() >= expected_yuyv {
                        let packed = YuvPackedImage {
                            yuy: &raw[..expected_yuyv],
                            yuy_stride: w * 2,
                            width: w,
                            height: h,
                        };
                        yuyv422_to_rgba(
                            &packed,
                            &mut rgba_buf,
                            w * 4,
                            YuvRange::Limited,
                            YuvStandardMatrix::Bt709,
                        )
                        .is_ok()
                    } else {
                        false
                    }
                }
                _ => {
                    // Fallback for MJPEG/GRAY/etc
                    buf.decode_image_to_buffer::<RgbAFormat>(&mut rgba_buf)
                        .is_ok()
                }
            };

            if ok {
                consecutive_errors = 0;
                backoff_us = 500;
                connected.store(true, Ordering::SeqCst);

                // Swap decoded frame into shared slot (fast — just pointer swap)
                if let Ok(mut lock) = frame_data.lock() {
                    let new_buf = std::mem::take(&mut rgba_buf);
                    let old = lock.replace(new_buf);
                    // Reclaim the old buffer for reuse (avoids allocation)
                    rgba_buf = old.unwrap_or_else(|| vec![0u8; expected_rgba]);
                    if rgba_buf.len() < expected_rgba {
                        rgba_buf.resize(expected_rgba, 0);
                    }
                }

                frame_count += 1;
                if frame_count.is_multiple_of(300) {
                    let elapsed = start.elapsed().as_secs_f64();
                    let fps = frame_count as f64 / elapsed;
                    log::debug!(
                        "Camera {}: {:.1} fps ({} frames in {:.1}s, fmt={:?})",
                        cam_id,
                        fps,
                        frame_count,
                        elapsed,
                        fmt
                    );
                }
            } else {
                consecutive_errors += 1;
                if consecutive_errors == ERROR_THRESHOLD {
                    log::warn!(
                        "Camera {}: {} consecutive decode errors — marking disconnected",
                        cam_id,
                        ERROR_THRESHOLD
                    );
                    connected.store(false, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_micros(backoff_us));
                backoff_us = (backoff_us * 2).min(MAX_BACKOFF_US);
            }
        }

        let _ = camera.stop_stream();
        log::info!("Camera {} capture thread stopped", cam_id);
    }

    /// Release a camera reference. Stops capture thread when ref_count hits 0.
    pub fn release_camera(&mut self, id: CameraId) {
        if let Some(active) = self.active.get_mut(&id) {
            active.ref_count = active.ref_count.saturating_sub(1);
            if active.ref_count == 0 {
                log::info!("Closing camera {} (no more references)", id);
                let Some(mut removed) = self.active.remove(&id) else {
                    log::warn!("Camera {} not found in active map during release", id);
                    return;
                };
                removed.stop_flag.store(true, Ordering::Relaxed);
                if let Some(t) = removed.thread.take() {
                    let _ = t.join();
                }
            }
        }
    }

    /// Get the shared texture view for a camera (decks read from this).
    pub fn texture_view(&self, id: CameraId) -> Option<&wgpu::TextureView> {
        self.active.get(&id).map(|a| &a.texture_view)
    }

    /// Get the resolution of an active camera.
    pub fn resolution(&self, id: CameraId) -> Option<(u32, u32)> {
        self.active.get(&id).map(|a| (a.width, a.height))
    }

    /// Check if a camera is currently connected and producing frames.
    /// Returns `false` if the camera is not active or is experiencing errors.
    pub fn is_connected(&self, id: CameraId) -> bool {
        self.active
            .get(&id)
            .map(|a| a.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Upload latest camera frames to GPU. Non-blocking — just grabs whatever
    /// the background capture threads have produced since the last call.
    /// Call once per frame from the main loop.
    pub fn update(&mut self, queue: &wgpu::Queue) {
        self.update_all(queue);
    }

    /// Upload frames for all active cameras.
    fn update_all(&mut self, queue: &wgpu::Queue) {
        for (_id, active) in self.active.iter_mut() {
            Self::upload_frame(active, queue);
        }
    }

    /// Upload frames only for cameras whose IDs are in the provided set.
    /// Cameras not in the set skip the GPU upload, saving bandwidth.
    pub fn update_selective(
        &mut self,
        queue: &wgpu::Queue,
        needed_ids: &std::collections::HashSet<CameraId>,
    ) {
        for (id, active) in self.active.iter_mut() {
            if needed_ids.contains(id) {
                Self::upload_frame(active, queue);
            }
        }
    }

    fn upload_frame(active: &mut ActiveCamera, queue: &wgpu::Queue) {
        // Take the latest frame from the shared buffer (non-blocking)
        let frame = if let Ok(mut lock) = active.frame_data.try_lock() {
            lock.take()
        } else {
            None
        };

        if let Some(data) = frame {
            let expected = (active.width * active.height * 4) as usize;
            if data.len() >= expected {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &active.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &data[..expected],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(active.width * 4),
                        rows_per_image: Some(active.height),
                    },
                    wgpu::Extent3d {
                        width: active.width,
                        height: active.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }

    /// Check if a camera is currently active (has an open capture session).
    pub fn is_active(&self, id: CameraId) -> bool {
        self.active.contains_key(&id)
    }

    /// Snapshot the current frame from an active camera without consuming it.
    /// Uses `try_lock()` for non-blocking access so we don't stall the render thread.
    /// Returns `Some((data, width, height))` if a frame is available, `None` otherwise.
    pub fn snapshot_frame(&self, id: CameraId) -> Option<(Vec<u8>, u32, u32)> {
        let cam = self.active.get(&id)?;
        let guard = cam.frame_data.try_lock().ok()?;
        let data = guard.as_ref()?.clone();
        Some((data, cam.width, cam.height))
    }

    /// Returns the first active camera ID, if any.
    pub fn first_active_id(&self) -> Option<CameraId> {
        self.active.keys().next().copied()
    }

    /// Returns all active camera IDs as a sorted vec.
    pub fn active_ids(&self) -> Vec<CameraId> {
        let mut ids: Vec<CameraId> = self.active.keys().copied().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_frame_on_empty_manager_returns_none() {
        let mgr = CameraManager::new();
        assert!(mgr.snapshot_frame(0).is_none());
        assert!(mgr.snapshot_frame(42).is_none());
    }

    #[test]
    fn first_active_id_on_empty_manager_returns_none() {
        let mgr = CameraManager::new();
        assert!(mgr.first_active_id().is_none());
    }

    #[test]
    fn active_ids_on_empty_manager_returns_empty() {
        let mgr = CameraManager::new();
        assert!(mgr.active_ids().is_empty());
    }
}
