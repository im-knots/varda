//! Depth-sensor capture manager — shared sensor sessions for N deck consumers.
//!
//! One `DepthSensorManager` owns all depth capture sessions. Each physical
//! sensor produces shared GPU textures (depth `R16Uint` + optional RGBA) that
//! any number of decks can read from and reproject as a point cloud.
//!
//! Capture runs on a dedicated thread per device, mirroring `CameraManager`:
//! the thread owns the `DepthBackend`, polls `next_frame()`, and swaps each
//! frame into an `Arc<Mutex<Option<DepthFrame>>>`. The render thread only
//! uploads (non-blocking `try_lock`) — it never calls into the driver.
//!
//! See spec/depth-sensors.md.

pub mod backend;
#[cfg(feature = "depth")]
pub mod freenect_backend;
pub mod point_cloud;
pub mod preprocess;

#[cfg(feature = "depth")]
use anyhow::Context;
use anyhow::Result;
use backend::{DepthBackend, DepthDeviceInfo, DepthFrame, DepthIntrinsics, MockBackend};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Opaque depth-sensor identifier.
pub type DepthSensorId = u32;

/// Assumed inter-frame interval before two frames have been observed
/// (Kinect v1 runs at ~30 Hz).
const DEFAULT_FRAME_DT: f32 = 1.0 / 30.0;

/// An active depth capture session with its shared GPU textures.
struct ActiveSensor {
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    rgb_texture: wgpu::Texture,
    rgb_view: wgpu::TextureView,
    width: u32,
    height: u32,
    intrinsics: DepthIntrinsics,
    ref_count: u32,
    /// Latest decoded frame — capture thread swaps in, render thread takes.
    frame_data: Arc<Mutex<Option<DepthFrame>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Bumped on every GPU upload. Consumers that derive work from the sensor
    /// image gate on this so a 30 Hz sensor does not drive 60 Hz of GPU passes.
    generation: u64,
    /// Wall-clock instant of the previous upload, for real inter-frame `dt`.
    last_upload: Option<std::time::Instant>,
    /// Seconds between the last two uploads. Falls back to the Kinect v1 rate
    /// until two frames have arrived.
    frame_dt: f32,
}

/// Manages depth-sensor enumeration, capture sessions, and shared textures.
pub struct DepthSensorManager {
    devices: Vec<DepthDeviceInfo>,
    active: HashMap<DepthSensorId, ActiveSensor>,
}

impl Default for DepthSensorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DepthSensorManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            devices: Vec::new(),
            active: HashMap::new(),
        };
        mgr.scan_devices();
        mgr
    }

    /// Scan for connected depth sensors.
    ///
    /// With the `depth` feature this enumerates real Kinect devices via
    /// libfreenect. Without it, no devices are reported (the mock backend is
    /// only used for tests / explicit `open_mock`).
    pub fn scan_devices(&mut self) {
        self.devices.clear();
        #[cfg(feature = "depth")]
        {
            match freenect_backend::FreenectBackend::enumerate() {
                Ok(n) => {
                    for id in 0..n {
                        self.devices.push(DepthDeviceInfo {
                            id,
                            name: format!("Kinect v1 (#{id})"),
                        });
                    }
                    log::info!("Depth scan: found {} sensor(s)", self.devices.len());
                }
                Err(e) => log::warn!("Depth enumeration failed: {}", e),
            }
        }
        #[cfg(not(feature = "depth"))]
        {
            log::debug!("Depth scan: `depth` feature disabled — no sensors enumerated");
        }
    }

    /// Get the list of detected depth devices.
    pub fn devices(&self) -> &[DepthDeviceInfo] {
        &self.devices
    }

    /// Create the shared GPU textures for a sensor of the given resolution.
    fn create_textures(
        device: &wgpu::Device,
        id: DepthSensorId,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::Texture) {
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Depth {id} R16Uint")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let rgb_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Depth {id} RGBA")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::renderer::context::COLOR_PATH_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        (depth_texture, rgb_texture)
    }

    /// Open a real depth sensor and start capturing on a dedicated thread.
    /// Returns the sensor's resolution. If already open, increments ref count.
    #[cfg(feature = "depth")]
    pub fn open(&mut self, id: DepthSensorId, device: &wgpu::Device) -> Result<(u32, u32)> {
        if let Some(active) = self.active.get_mut(&id) {
            active.ref_count += 1;
            return Ok((active.width, active.height));
        }
        let backend = freenect_backend::FreenectBackend::open(id)
            .with_context(|| format!("opening depth sensor {id}"))?;
        self.start_session(id, Box::new(backend), device)
    }

    /// Open a synthetic sensor (mock backend). Always available; used for tests
    /// and for exercising the point-cloud pass without hardware.
    pub fn open_mock(
        &mut self,
        id: DepthSensorId,
        width: u32,
        height: u32,
        device: &wgpu::Device,
    ) -> Result<(u32, u32)> {
        if let Some(active) = self.active.get_mut(&id) {
            active.ref_count += 1;
            return Ok((active.width, active.height));
        }
        let backend = MockBackend::new(format!("Mock Depth (#{id})"), width, height);
        self.start_session(id, Box::new(backend), device)
    }

    /// Spawn the capture thread and register the session. Shared by real/mock.
    fn start_session(
        &mut self,
        id: DepthSensorId,
        mut backend: Box<dyn DepthBackend>,
        device: &wgpu::Device,
    ) -> Result<(u32, u32)> {
        let (width, height) = backend.resolution();
        let intrinsics = backend.intrinsics();
        let (depth_texture, rgb_texture) = Self::create_textures(device, id, width, height);
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let rgb_view = rgb_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let frame_data: Arc<Mutex<Option<DepthFrame>>> = Arc::new(Mutex::new(None));
        let frame_tx = Arc::clone(&frame_data);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = Arc::clone(&connected);

        let thread = std::thread::Builder::new()
            .name(format!("depth-{id}"))
            .spawn(move || {
                Self::capture_loop(id, backend.as_mut(), frame_tx, stop_clone, connected_clone);
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn depth thread: {}", e))?;

        self.active.insert(
            id,
            ActiveSensor {
                depth_texture,
                depth_view,
                rgb_texture,
                rgb_view,
                width,
                height,
                intrinsics,
                ref_count: 1,
                frame_data,
                stop_flag,
                connected,
                thread: Some(thread),
                generation: 0,
                last_upload: None,
                frame_dt: DEFAULT_FRAME_DT,
            },
        );
        Ok((width, height))
    }

    /// Background capture loop — runs on a dedicated thread per sensor.
    fn capture_loop(
        id: DepthSensorId,
        backend: &mut dyn DepthBackend,
        frame_data: Arc<Mutex<Option<DepthFrame>>>,
        stop: Arc<AtomicBool>,
        connected: Arc<AtomicBool>,
    ) {
        log::info!("Depth {} capture thread started ({})", id, backend.name());
        while !stop.load(Ordering::Relaxed) {
            match backend.next_frame() {
                Some(frame) => {
                    connected.store(true, Ordering::SeqCst);
                    if let Ok(mut lock) = frame_data.lock() {
                        lock.replace(frame);
                    }
                }
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
        log::info!("Depth {} capture thread stopped", id);
    }

    /// Upload latest frames to GPU. Non-blocking. Call once per frame.
    pub fn update(&mut self, queue: &wgpu::Queue) {
        for active in self.active.values_mut() {
            let frame = match active.frame_data.try_lock() {
                Ok(mut lock) => lock.take(),
                Err(_) => None,
            };
            let Some(frame) = frame else { continue };
            Self::upload_depth(active, &frame, queue);
            if let Some(rgb) = &frame.rgb {
                Self::upload_rgb(active, rgb, queue);
            }
            let now = std::time::Instant::now();
            if let Some(prev) = active.last_upload {
                active.frame_dt = now.duration_since(prev).as_secs_f32().max(1.0e-4);
            }
            active.last_upload = Some(now);
            active.generation = active.generation.wrapping_add(1);
        }
    }

    fn upload_depth(active: &ActiveSensor, frame: &DepthFrame, queue: &wgpu::Queue) {
        let expected = (active.width * active.height) as usize;
        if frame.depth.len() < expected {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &active.depth_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&frame.depth[..expected]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(active.width * 2),
                rows_per_image: Some(active.height),
            },
            wgpu::Extent3d {
                width: active.width,
                height: active.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn upload_rgb(active: &ActiveSensor, rgb: &[u8], queue: &wgpu::Queue) {
        let expected = (active.width * active.height * 4) as usize;
        if rgb.len() < expected {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &active.rgb_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgb[..expected],
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

    /// Depth texture view (`R16Uint`) for the point-cloud pass.
    pub fn depth_view(&self, id: DepthSensorId) -> Option<&wgpu::TextureView> {
        self.active.get(&id).map(|a| &a.depth_view)
    }

    /// RGB texture view (`Rgba8Unorm`) for per-point colour.
    pub fn rgb_view(&self, id: DepthSensorId) -> Option<&wgpu::TextureView> {
        self.active.get(&id).map(|a| &a.rgb_view)
    }

    /// Intrinsics for deprojecting an active sensor's depth image.
    pub fn intrinsics(&self, id: DepthSensorId) -> Option<DepthIntrinsics> {
        self.active.get(&id).map(|a| a.intrinsics)
    }

    /// Resolution of an active sensor.
    pub fn resolution(&self, id: DepthSensorId) -> Option<(u32, u32)> {
        self.active.get(&id).map(|a| (a.width, a.height))
    }

    /// Upload counter for an active sensor, bumped once per new frame.
    ///
    /// Consumers that derive GPU work from the sensor image compare this against
    /// the value they last processed and skip when it is unchanged, so a 30 Hz
    /// sensor does not drive 60 Hz of redundant passes.
    pub fn frame_generation(&self, id: DepthSensorId) -> Option<u64> {
        self.active.get(&id).map(|a| a.generation)
    }

    /// Measured seconds between the last two frame uploads. Use this rather than
    /// the render `dt` for rate calculations — they differ whenever the deck runs
    /// faster than the sensor.
    pub fn frame_dt(&self, id: DepthSensorId) -> Option<f32> {
        self.active.get(&id).map(|a| a.frame_dt)
    }

    /// Whether a sensor is currently producing frames.
    pub fn is_connected(&self, id: DepthSensorId) -> bool {
        self.active
            .get(&id)
            .map(|a| a.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Whether a sensor has an open capture session.
    pub fn is_active(&self, id: DepthSensorId) -> bool {
        self.active.contains_key(&id)
    }

    /// Reference count for an active sensor (0 if not active).
    pub fn ref_count(&self, id: DepthSensorId) -> u32 {
        self.active.get(&id).map(|a| a.ref_count).unwrap_or(0)
    }

    /// Release a sensor reference. Stops the capture thread at ref_count 0.
    pub fn release(&mut self, id: DepthSensorId) {
        if let Some(active) = self.active.get_mut(&id) {
            active.ref_count = active.ref_count.saturating_sub(1);
            if active.ref_count == 0 {
                log::info!("Closing depth sensor {} (no more references)", id);
                if let Some(mut removed) = self.active.remove(&id) {
                    removed.stop_flag.store(true, Ordering::Relaxed);
                    if let Some(t) = removed.thread.take() {
                        let _ = t.join();
                    }
                }
            }
        }
    }

    /// All active sensor IDs, sorted.
    pub fn active_ids(&self) -> Vec<DepthSensorId> {
        let mut ids: Vec<DepthSensorId> = self.active.keys().copied().collect();
        ids.sort();
        ids
    }
}

/// Open a depth sensor for use, abstracting over the `depth` feature gate.
///
/// With the `depth` feature this opens the real Kinect backend. Without it,
/// there are no real devices, so this returns an error (callers skip the deck
/// with a warning, matching camera-not-found behaviour).
pub fn open_depth_sensor(
    manager: &mut DepthSensorManager,
    id: DepthSensorId,
    device: &wgpu::Device,
) -> Result<(u32, u32)> {
    #[cfg(feature = "depth")]
    {
        manager.open(id, device)
    }
    #[cfg(not(feature = "depth"))]
    {
        let _ = (manager, id, device);
        anyhow::bail!("depth sensor support not compiled in (enable the `depth` feature)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::GpuContext;

    fn headless() -> Option<GpuContext> {
        GpuContext::new_headless().ok()
    }

    #[test]
    fn empty_manager_has_no_active_sensors() {
        let mgr = DepthSensorManager::new();
        assert!(mgr.active_ids().is_empty());
        assert!(!mgr.is_active(0));
        assert_eq!(mgr.ref_count(0), 0);
        assert!(mgr.depth_view(0).is_none());
    }

    #[test]
    fn open_mock_creates_session_with_textures_and_intrinsics() {
        let Some(gpu) = headless() else { return };
        let mut mgr = DepthSensorManager::new();
        let (w, h) = mgr.open_mock(0, 64, 48, &gpu.device).expect("open mock");
        assert_eq!((w, h), (64, 48));
        assert!(mgr.is_active(0));
        assert_eq!(mgr.ref_count(0), 1);
        assert!(mgr.depth_view(0).is_some());
        assert!(mgr.rgb_view(0).is_some());
        assert_eq!(mgr.intrinsics(0), Some(DepthIntrinsics::kinect_v1()));
        assert_eq!(mgr.resolution(0), Some((64, 48)));
        mgr.release(0);
        assert!(!mgr.is_active(0));
    }

    #[test]
    fn frame_generation_advances_only_on_a_new_frame() {
        let Some(gpu) = headless() else { return };
        let mut mgr = DepthSensorManager::new();
        mgr.open_mock(0, 32, 24, &gpu.device).expect("open mock");
        assert_eq!(mgr.frame_generation(0), Some(0));
        assert!(mgr.frame_dt(0).is_some());

        // The mock capture thread needs a moment to publish its first frame.
        let mut first = 0;
        for _ in 0..200 {
            mgr.update(&gpu.queue);
            first = mgr.frame_generation(0).expect("active");
            if first > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(first > 0, "mock backend never produced a frame");

        // A second update with no newly captured frame must not advance the
        // counter — that is what lets consumers skip redundant GPU work.
        mgr.update(&gpu.queue);
        let second = mgr.frame_generation(0).expect("active");
        assert!(
            second == first || second == first + 1,
            "generation jumped {first} -> {second} without new frames"
        );

        // Unknown sensors report nothing rather than a misleading zero.
        assert_eq!(mgr.frame_generation(99), None);
        assert_eq!(mgr.frame_dt(99), None);
        mgr.release(0);
    }

    #[test]
    fn refcount_shares_one_session_across_consumers() {
        let Some(gpu) = headless() else { return };
        let mut mgr = DepthSensorManager::new();
        mgr.open_mock(1, 32, 32, &gpu.device).expect("first open");
        mgr.open_mock(1, 32, 32, &gpu.device).expect("second open");
        assert_eq!(mgr.ref_count(1), 2);
        assert_eq!(mgr.active_ids(), vec![1]);
        mgr.release(1);
        assert!(mgr.is_active(1), "still held by second consumer");
        assert_eq!(mgr.ref_count(1), 1);
        mgr.release(1);
        assert!(!mgr.is_active(1), "released by last consumer");
    }

    #[test]
    fn update_uploads_without_panicking() {
        let Some(gpu) = headless() else { return };
        let mut mgr = DepthSensorManager::new();
        mgr.open_mock(0, 16, 16, &gpu.device).expect("open mock");
        // Give the capture thread a moment to produce a frame.
        std::thread::sleep(std::time::Duration::from_millis(20));
        mgr.update(&gpu.queue);
        gpu.queue.submit(std::iter::empty());
        mgr.release(0);
    }
}
