//! SRT receive — background ffmpeg decode thread → shared buffer → GPU upload.
//!
//! SRT output streaming is handled per-output via `FfmpegSubprocess` in
//! `src/renderer/subprocess.rs`. This module handles SRT *input* (receiving
//! video from an SRT source and displaying it as a deck source).
//!
//! Architecture mirrors `NdiManager`: background thread decodes frames into
//! `Arc<Mutex<Option<Vec<u8>>>>`, main thread uploads to GPU each frame.

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;

/// SRT connection mode — listener waits for connections, caller connects out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SrtMode {
    /// Bind to a port and wait for incoming connections.
    Listener,
    /// Connect to a remote SRT endpoint.
    Caller,
}

impl std::fmt::Display for SrtMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SrtMode::Listener => write!(f, "Listener"),
            SrtMode::Caller => write!(f, "Caller"),
        }
    }
}

#[allow(dead_code)]
struct SrtReceiver {
    url: String,
    mode: SrtMode,
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
    width: u32,
    height: u32,
}

/// Manages SRT input receivers (background decode threads + GPU textures).
pub struct SrtManager {
    receivers: Vec<SrtReceiver>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
}

impl SrtManager {
    pub fn new() -> Self {
        Self {
            receivers: Vec::new(),
            textures: Vec::new(),
        }
    }

    /// Start receiving from an SRT URL. Spawns a background ffmpeg decode thread.
    /// Returns receiver index on success.
    pub fn start_receive(&mut self, url: &str, mode: SrtMode, device: &wgpu::Device) -> Option<usize> {
        let frame_data: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let (width, height) = (1920u32, 1080u32);

        // Build the full SRT URL with mode query param
        let full_url = build_srt_url(url, mode);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("SRT Receive: {}", url)),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let frame_clone = Arc::clone(&frame_data);
        let stop_clone = Arc::clone(&stop_flag);
        let connected_clone = Arc::clone(&connected);
        let url_for_thread = full_url.clone();
        let url_display = url.to_string();

        let thread = std::thread::Builder::new()
            .name(format!("srt-recv-{}", url))
            .spawn(move || {
                srt_receive_thread(url_for_thread, url_display, frame_clone, stop_clone, connected_clone);
            })
            .ok();

        if thread.is_none() {
            log::error!("Failed to spawn SRT receive thread for '{}'", url);
            return None;
        }

        let idx = self.receivers.len();
        self.receivers.push(SrtReceiver {
            url: url.to_string(),
            mode,
            frame_data,
            stop_flag,
            connected,
            _thread: thread,
            width,
            height,
        });
        self.textures.push((texture, texture_view));
        log::info!("SRT receiver started for '{}' (mode: {:?})", url, mode);
        Some(idx)
    }

    /// Upload latest frames from all receivers to GPU.
    pub fn update(&self, queue: &wgpu::Queue) {
        for (i, receiver) in self.receivers.iter().enumerate() {
            if let Ok(mut guard) = receiver.frame_data.try_lock() {
                if let Some(frame) = guard.take() {
                    let expected = (receiver.width * receiver.height * 4) as usize;
                    if frame.len() >= expected {
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &self.textures[i].0, mip_level: 0,
                                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
                            },
                            &frame[..expected],
                            wgpu::TexelCopyBufferLayout {
                                offset: 0, bytes_per_row: Some(receiver.width * 4),
                                rows_per_image: Some(receiver.height),
                            },
                            wgpu::Extent3d { width: receiver.width, height: receiver.height, depth_or_array_layers: 1 },
                        );
                    }
                }
            }
        }
    }

    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.textures.get(idx).map(|(_, v)| v)
    }

    pub fn receiver_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.receivers.get(idx).map(|r| (r.width, r.height))
    }

    /// Whether the receiver at `idx` has successfully connected and received a frame.
    pub fn is_connected(&self, idx: usize) -> bool {
        self.receivers.get(idx)
            .map(|r| r.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Number of active receivers.
    pub fn receiver_count(&self) -> usize {
        self.receivers.len()
    }

    /// Get URL of receiver at index.
    pub fn receiver_url(&self, idx: usize) -> Option<&str> {
        self.receivers.get(idx).map(|r| r.url.as_str())
    }

    /// Get mode of receiver at index.
    pub fn receiver_mode(&self, idx: usize) -> Option<SrtMode> {
        self.receivers.get(idx).map(|r| r.mode)
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get_mut(idx) {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r._thread.take() { let _ = t.join(); }
        }
    }
}

impl Drop for SrtManager {
    fn drop(&mut self) {
        for r in &mut self.receivers {
            r.stop_flag.store(true, Ordering::SeqCst);
        }
    }
}

/// Build an SRT URL with mode query parameter.
fn build_srt_url(url: &str, mode: SrtMode) -> String {
    let mode_str = match mode {
        SrtMode::Listener => "listener",
        SrtMode::Caller => "caller",
    };
    if url.contains('?') {
        format!("{}&mode={}", url, mode_str)
    } else {
        format!("{}?mode={}", url, mode_str)
    }
}

/// Background thread: decode SRT stream via ffmpeg_next into RGBA frames.
fn srt_receive_thread(
    url: String,
    url_display: String,
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
) {
    log::info!("SRT receive thread starting for '{}'", url_display);

    // Initialize ffmpeg
    ffmpeg_next::init().ok();

    loop {
        if stop_flag.load(Ordering::SeqCst) { return; }

        match ffmpeg_next::format::input(&url) {
            Ok(mut input_ctx) => {
                let stream_idx = match input_ctx.streams().best(ffmpeg_next::media::Type::Video) {
                    Some(s) => s.index(),
                    None => {
                        log::error!("SRT '{}': no video stream found", url_display);
                        return;
                    }
                };

                let codec_params = input_ctx.stream(stream_idx).unwrap().parameters();
                let mut decoder = match ffmpeg_next::codec::Context::from_parameters(codec_params) {
                    Ok(ctx) => match ctx.decoder().video() {
                        Ok(d) => d,
                        Err(e) => {
                            log::error!("SRT '{}': failed to create video decoder: {}", url_display, e);
                            return;
                        }
                    },
                    Err(e) => {
                        log::error!("SRT '{}': failed to create codec context: {}", url_display, e);
                        return;
                    }
                };

                let mut scaler = ffmpeg_next::software::scaling::Context::get(
                    decoder.format(), decoder.width(), decoder.height(),
                    ffmpeg_next::format::Pixel::RGBA, 1920, 1080,
                    ffmpeg_next::software::scaling::Flags::BILINEAR,
                ).ok();

                log::info!("SRT '{}' connected, decoding video", url_display);

                for (stream, packet) in input_ctx.packets() {
                    if stop_flag.load(Ordering::SeqCst) { return; }
                    if stream.index() != stream_idx { continue; }

                    if decoder.send_packet(&packet).is_err() { continue; }

                    let mut decoded = ffmpeg_next::frame::Video::empty();
                    while decoder.receive_frame(&mut decoded).is_ok() {
                        if stop_flag.load(Ordering::SeqCst) { return; }

                        // Lazily init/reinit scaler if dimensions changed
                        if scaler.is_none() || decoded.width() != 1920 || decoded.height() != 1080 {
                            scaler = ffmpeg_next::software::scaling::Context::get(
                                decoded.format(), decoded.width(), decoded.height(),
                                ffmpeg_next::format::Pixel::RGBA, 1920, 1080,
                                ffmpeg_next::software::scaling::Flags::BILINEAR,
                            ).ok();
                        }

                        if let Some(ref mut sws) = scaler {
                            let mut rgb_frame = ffmpeg_next::frame::Video::empty();
                            if sws.run(&decoded, &mut rgb_frame).is_ok() {
                                let data = rgb_frame.data(0);
                                let expected = (1920 * 1080 * 4) as usize;
                                if data.len() >= expected {
                                    let rgba = data[..expected].to_vec();
                                    if let Ok(mut guard) = frame_data.lock() {
                                        *guard = Some(rgba);
                                    }
                                    if !connected.load(Ordering::SeqCst) {
                                        connected.store(true, Ordering::SeqCst);
                                        log::info!("SRT '{}' first frame received", url_display);
                                    }
                                }
                            }
                        }
                    }
                }

                // Stream ended — retry after a short delay
                log::warn!("SRT '{}' stream ended, retrying in 2s...", url_display);
                connected.store(false, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            Err(e) => {
                log::warn!("SRT '{}' connection failed: {}, retrying in 2s...", url_display, e);
                connected.store(false, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_manager_new_empty() {
        let mgr = SrtManager::new();
        assert_eq!(mgr.receivers.len(), 0);
        assert_eq!(mgr.textures.len(), 0);
        assert!(!mgr.is_connected(0));
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.receiver_dimensions(0).is_none());
    }

    #[test]
    fn srt_mode_serialization_roundtrip() {
        let listener = SrtMode::Listener;
        let json = serde_json::to_string(&listener).unwrap();
        let restored: SrtMode = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, SrtMode::Listener);

        let caller = SrtMode::Caller;
        let json = serde_json::to_string(&caller).unwrap();
        let restored: SrtMode = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, SrtMode::Caller);
    }

    #[test]
    fn srt_mode_display() {
        assert_eq!(format!("{}", SrtMode::Listener), "Listener");
        assert_eq!(format!("{}", SrtMode::Caller), "Caller");
    }

    #[test]
    fn build_srt_url_caller() {
        let url = build_srt_url("srt://192.168.1.100:9000", SrtMode::Caller);
        assert_eq!(url, "srt://192.168.1.100:9000?mode=caller");
    }

    #[test]
    fn build_srt_url_listener() {
        let url = build_srt_url("srt://:9000", SrtMode::Listener);
        assert_eq!(url, "srt://:9000?mode=listener");
    }

    #[test]
    fn build_srt_url_with_existing_params() {
        let url = build_srt_url("srt://host:9000?latency=0", SrtMode::Caller);
        assert_eq!(url, "srt://host:9000?latency=0&mode=caller");
    }
}
