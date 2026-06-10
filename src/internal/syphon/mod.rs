//! Syphon — macOS inter-app GPU texture sharing.
//!
//! Syphon allows applications to share rendered frames in real-time via IOSurface.
//! Input follows the CameraManager pattern: receive thread → Arc<Mutex<>> → GPU upload.
//! Output publishes frames from HeadlessOutput readback.
//!
//! macOS only. Requires Syphon.framework installed on the system.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// Discovered Syphon server.
#[derive(Debug, Clone)]
pub struct SyphonSource {
    /// Server name
    pub name: String,
    /// Application name publishing the server
    pub app_name: String,
}

/// Manages Syphon server discovery, client connections, and server publishing.
pub struct SyphonManager {
    available: bool,
    sources: Vec<SyphonSource>,
    clients: Vec<SyphonClient>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
}

struct SyphonClient {
    #[allow(dead_code)]
    server_name: String,
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<std::thread::JoinHandle<()>>,
    width: u32,
    height: u32,
}

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
            clients: Vec::new(),
            textures: Vec::new(),
        }
    }

    /// Create a disabled Syphon manager (CLI `--no-syphon` flag).
    pub fn new_disabled() -> Self {
        Self {
            available: false,
            sources: Vec::new(),
            clients: Vec::new(),
            textures: Vec::new(),
        }
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

    /// Scan for available Syphon servers.
    pub fn discover(&mut self) {
        if !self.available {
            return;
        }
        log::debug!("Syphon discover: scanning...");
        self.sources.clear();
        // TODO: Use SyphonServerDirectory to enumerate servers
    }

    /// Start receiving from a named Syphon server.
    pub fn start_receive(&mut self, server_name: &str, device: &wgpu::Device) -> Option<usize> {
        if !self.available {
            log::warn!("Cannot receive Syphon: framework not available");
            return None;
        }
        let frame_data: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (width, height) = (1920, 1080);

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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let idx = self.clients.len();
        self.clients.push(SyphonClient {
            server_name: server_name.to_string(),
            frame_data,
            stop_flag,
            _thread: None,
            width,
            height,
        });
        self.textures.push((texture, texture_view));
        log::info!("Syphon client connected to '{}'", server_name);
        Some(idx)
    }

    /// Upload latest frames from all clients to GPU.
    pub fn update(&self, queue: &wgpu::Queue) {
        for (i, client) in self.clients.iter().enumerate() {
            if let Ok(mut guard) = client.frame_data.try_lock() {
                if let Some(frame) = guard.take() {
                    let expected = (client.width * client.height * 4) as usize;
                    if frame.len() >= expected {
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &self.textures[i].0,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            &frame[..expected],
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(client.width * 4),
                                rows_per_image: Some(client.height),
                            },
                            wgpu::Extent3d {
                                width: client.width,
                                height: client.height,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                }
            }
        }
    }

    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.textures.get(idx).map(|(_, v)| v)
    }

    pub fn client_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.clients.get(idx).map(|c| (c.width, c.height))
    }

    /// Publish a frame via Syphon server (for HeadlessOutput with SyphonServer target).
    pub fn publish_frame(&mut self, rgba: &[u8], width: u32, height: u32) {
        if !self.available {
            return;
        }
        let _ = (rgba, width, height); // TODO: Publish via Syphon.framework
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(c) = self.clients.get_mut(idx) {
            c.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = c._thread.take() {
                let _ = t.join();
            }
        }
    }

    fn check_framework() -> bool {
        // Check standard Syphon.framework locations
        let paths = [
            "/Library/Frameworks/Syphon.framework",
            &format!(
                "{}/Library/Frameworks/Syphon.framework",
                std::env::var("HOME").unwrap_or_default()
            ),
        ];
        paths.iter().any(|p| std::path::Path::new(p).exists())
    }
}

impl Drop for SyphonManager {
    fn drop(&mut self) {
        for c in &mut self.clients {
            c.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = c._thread.take() {
                let _ = t.join();
            }
        }
    }
}
