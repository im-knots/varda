//! Camera capture. Each physical camera has one capture thread and one shared
//! GPU texture that any number of decks read.
//!
//! NV12 (macOS) and YUYV (macOS/Linux) convert to RGBA with yuvutils-rs.
//! MJPEG frames (common on Linux V4L2) use nokhwa's mozjpeg decoder.

pub mod provider;

use anyhow::{Context, Result};
use nokhwa::Camera;
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use yuvutils_rs::{
    YuvBiPlanarImage, YuvConversionMode, YuvPackedImage, YuvRange, YuvStandardMatrix,
    yuv_nv12_to_rgba, yuyv422_to_rgba,
};

/// Camera identifier; matches the OS-assigned index.
pub type CameraId = u32;

#[derive(Debug, Clone)]
pub struct CameraDeviceInfo {
    pub id: CameraId,
    pub name: String,
    pub index: CameraIndex,
}

/// An open capture session and its shared GPU texture.
struct ActiveCamera {
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// Number of decks using this camera.
    ref_count: u32,
    /// Latest decoded RGBA frame. The capture thread swaps it in, the main thread takes it.
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Camera enumeration, capture sessions and shared textures.
pub struct CameraManager {
    /// Detected devices, refreshed periodically.
    devices: Vec<CameraDeviceInfo>,
    active: HashMap<CameraId, ActiveCamera>,
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
        // Requests AVFoundation camera permission on macOS; no-op on Linux.
        nokhwa::nokhwa_initialize(|granted| {
            if granted {
                log::info!("Camera access granted");
            } else {
                log::warn!("Camera access denied by OS");
            }
        });
        self.initialized = true;
    }

    pub fn scan_devices(&mut self) {
        match nokhwa::query(nokhwa::utils::ApiBackend::Auto) {
            Ok(cameras) => {
                self.devices = cameras
                    .iter()
                    .enumerate()
                    .map(|(i, info)| CameraDeviceInfo {
                        id: i as CameraId,
                        name: info.human_name().clone(),
                        index: info.index().clone(),
                    })
                    .collect();
                log::info!("Camera scan: found {} device(s)", self.devices.len());
                for dev in &self.devices {
                    log::info!("  Camera {}: {}", dev.id, dev.name);
                }
            }
            Err(e) => {
                log::warn!("Camera enumeration failed: {e}");
                self.devices.clear();
            }
        }
    }

    pub fn devices(&self) -> &[CameraDeviceInfo] {
        &self.devices
    }

    /// Opens a camera and starts its capture thread, returning its resolution.
    /// If already open, increments the ref count.
    ///
    /// # Errors
    ///
    /// Returns an error if no device matches `id`, the backend cannot open or
    /// start the stream, or the capture thread cannot be spawned.
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
            .map_err(|e| anyhow::anyhow!("Failed to start camera stream: {e}"))?;

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

        // Capture thread swaps in decoded RGBA; main thread takes it.
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
            .name(format!("camera-{cam_id}"))
            .spawn(move || {
                Self::capture_loop(
                    camera,
                    cam_id,
                    cam_w,
                    cam_h,
                    &frame_data_tx,
                    &stop_clone,
                    &connected_clone,
                );
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn camera thread: {e}"))?;

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

    /// Capture loop, one thread per camera.
    fn capture_loop(
        mut camera: Camera,
        cam_id: CameraId,
        w: u32,
        h: u32,
        frame_data: &Mutex<Option<Vec<u8>>>,
        stop: &AtomicBool,
        connected: &AtomicBool,
    ) {
        const MAX_BACKOFF_US: u64 = 500_000; // 500ms cap
        const ERROR_THRESHOLD: u64 = 100;

        let expected_rgba = (w * h * 4) as usize;
        // Reused for every frame.
        let mut rgba_buf = vec![0u8; expected_rgba];
        let mut frame_count: u64 = 0;
        let mut consecutive_errors: u64 = 0;
        let mut backoff_us: u64 = 500; // 500µs initial backoff
        let start = std::time::Instant::now();

        log::info!("Camera {cam_id} capture thread started ({w}x{h})");

        while !stop.load(Ordering::Relaxed) {
            let Ok(buf) = camera.frame() else {
                consecutive_errors += 1;
                if consecutive_errors == ERROR_THRESHOLD {
                    log::warn!(
                        "Camera {cam_id}: {ERROR_THRESHOLD} consecutive frame errors — marking disconnected"
                    );
                    connected.store(false, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_micros(backoff_us));
                backoff_us = (backoff_us * 2).min(MAX_BACKOFF_US);
                continue;
            };

            let raw = buf.buffer();
            let fmt = buf.source_frame_format();

            let ok = match fmt {
                FrameFormat::NV12 => {
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
                    // MJPEG, GRAY, etc.
                    buf.decode_image_to_buffer::<RgbAFormat>(&mut rgba_buf)
                        .is_ok()
                }
            };

            if ok {
                consecutive_errors = 0;
                backoff_us = 500;
                connected.store(true, Ordering::SeqCst);

                if let Ok(mut lock) = frame_data.lock() {
                    let new_buf = std::mem::take(&mut rgba_buf);
                    let old = lock.replace(new_buf);
                    // Reuse the old buffer to avoid an allocation.
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
                        "Camera {cam_id}: {fps:.1} fps ({frame_count} frames in {elapsed:.1}s, fmt={fmt:?})"
                    );
                }
            } else {
                consecutive_errors += 1;
                if consecutive_errors == ERROR_THRESHOLD {
                    log::warn!(
                        "Camera {cam_id}: {ERROR_THRESHOLD} consecutive decode errors — marking disconnected"
                    );
                    connected.store(false, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_micros(backoff_us));
                backoff_us = (backoff_us * 2).min(MAX_BACKOFF_US);
            }
        }

        let _ = camera.stop_stream();
        log::info!("Camera {cam_id} capture thread stopped");
    }

    /// Releases a camera reference. Stops the capture thread when `ref_count` hits 0.
    pub fn release_camera(&mut self, id: CameraId) {
        if let Some(active) = self.active.get_mut(&id) {
            active.ref_count = active.ref_count.saturating_sub(1);
            if active.ref_count == 0 {
                log::info!("Closing camera {id} (no more references)");
                let Some(mut removed) = self.active.remove(&id) else {
                    log::warn!("Camera {id} not found in active map during release");
                    return;
                };
                removed.stop_flag.store(true, Ordering::Relaxed);
                if let Some(t) = removed.thread.take() {
                    let _ = t.join();
                }
            }
        }
    }

    /// Shared texture view that decks read.
    pub fn texture_view(&self, id: CameraId) -> Option<&wgpu::TextureView> {
        self.active.get(&id).map(|a| &a.texture_view)
    }

    pub fn resolution(&self, id: CameraId) -> Option<(u32, u32)> {
        self.active.get(&id).map(|a| (a.width, a.height))
    }

    /// Whether the camera is open and producing frames.
    pub fn is_connected(&self, id: CameraId) -> bool {
        self.active
            .get(&id)
            .is_some_and(|a| a.connected.load(Ordering::SeqCst))
    }

    /// Uploads frames the capture threads produced since the last call.
    /// Non-blocking; call once per frame.
    pub fn update(&mut self, queue: &wgpu::Queue) {
        self.update_all(queue);
    }

    fn update_all(&mut self, queue: &wgpu::Queue) {
        for active in self.active.values_mut() {
            Self::upload_frame(active, queue);
        }
    }

    /// Uploads frames only for cameras in `needed_ids`.
    pub fn update_selective(
        &mut self,
        queue: &wgpu::Queue,
        needed_ids: &std::collections::HashSet<CameraId>,
    ) {
        for (id, active) in &mut self.active {
            if needed_ids.contains(id) {
                Self::upload_frame(active, queue);
            }
        }
    }

    fn upload_frame(active: &mut ActiveCamera, queue: &wgpu::Queue) {
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

    pub fn is_active(&self, id: CameraId) -> bool {
        self.active.contains_key(&id)
    }

    /// Copies the current frame without consuming it, as `(data, width, height)`.
    /// Uses `try_lock()` so the render thread never stalls.
    pub fn snapshot_frame(&self, id: CameraId) -> Option<(Vec<u8>, u32, u32)> {
        let cam = self.active.get(&id)?;
        let guard = cam.frame_data.try_lock().ok()?;
        let data = guard.as_ref()?.clone();
        Some((data, cam.width, cam.height))
    }

    pub fn first_active_id(&self) -> Option<CameraId> {
        self.active.keys().next().copied()
    }

    /// Active camera IDs, sorted.
    pub fn active_ids(&self) -> Vec<CameraId> {
        let mut ids: Vec<CameraId> = self.active.keys().copied().collect();
        ids.sort_unstable();
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
        assert_eq!(mgr.active_ids().len(), 0);
    }
}
