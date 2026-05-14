//! Stream receive — background ffmpeg decode thread → shared buffer → GPU upload.
//!
//! SRT output streaming is handled per-output via `FfmpegSubprocess` in
//! `src/renderer/subprocess.rs`. This module handles stream *input* (receiving
//! video from SRT/HLS/DASH sources and displaying them as deck sources).
//!
//! Architecture mirrors `NdiManager`: background thread decodes frames into
//! `Arc<Mutex<Option<Vec<u8>>>>`, main thread uploads to GPU each frame.

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;

/// Serializes ffmpeg context creation (input_with_interrupt + decoder open).
/// ffmpeg's `avcodec_open2` is NOT thread-safe — two threads opening decoders
/// simultaneously can corrupt the internal codec registry, producing silent
/// black output. The mutex is released before entering the decode loop so
/// frame decoding remains fully parallel across receivers.
static FFMPEG_INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Stream protocol for input receivers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum StreamProtocol {
    /// SRT (Secure Reliable Transport) with connection mode.
    Srt { mode: SrtMode },
    /// HLS (HTTP Live Streaming) — reads `.m3u8` manifest.
    Hls,
    /// DASH (Dynamic Adaptive Streaming over HTTP) — reads `.mpd` manifest.
    Dash,
    /// RTMP stream source with connection mode.
    Rtmp { mode: RtmpMode },
}

/// SRT connection mode — listener waits for connections, caller connects out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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

/// RTMP connection mode: pull from a remote server or listen for incoming pushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum RtmpMode {
    /// Connect to a remote RTMP server and pull the stream.
    Pull,
    /// Listen on a local RTMP port for incoming pushes (e.g. from OBS).
    Listen,
}

impl std::fmt::Display for RtmpMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtmpMode::Pull => write!(f, "Pull"),
            RtmpMode::Listen => write!(f, "Listen"),
        }
    }
}

#[allow(dead_code)]
struct StreamReceiver {
    url: String,
    protocol: StreamProtocol,
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
    width: u32,
    height: u32,
}

/// Manages stream input receivers (background decode threads + GPU textures).
/// Handles SRT, HLS, and DASH input protocols.
pub struct StreamManager {
    receivers: Vec<StreamReceiver>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
}

impl StreamManager {
    pub fn new() -> Self {
        // Init ffmpeg once on the main thread before any receiver threads spawn.
        ffmpeg_next::init().ok();
        Self {
            receivers: Vec::new(),
            textures: Vec::new(),
        }
    }

    /// Start receiving from a stream URL. Spawns a background ffmpeg decode thread.
    /// Returns receiver index on success. If an active receiver for the same URL
    /// already exists, returns its index.
    pub fn start_receive(&mut self, url: &str, protocol: StreamProtocol, device: &wgpu::Device) -> Option<usize> {
        // Reuse existing receiver for the same URL if it's still alive
        if let Some(idx) = self.receivers.iter().position(|r| r.url == url && !r.stop_flag.load(Ordering::SeqCst)) {
            log::info!("Reusing existing stream receiver {} for '{}'", idx, url);
            return Some(idx);
        }

        let frame_data: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let (width, height) = (1920u32, 1080u32);

        // Build the full URL with protocol-specific params
        let full_url = match &protocol {
            StreamProtocol::Srt { mode } => build_srt_url(url, *mode),
            StreamProtocol::Hls | StreamProtocol::Dash => url.to_string(),
            StreamProtocol::Rtmp { mode } => build_rtmp_url(url, *mode),
        };

        let label = match &protocol {
            StreamProtocol::Srt { .. } => format!("SRT Receive: {}", url),
            StreamProtocol::Hls => format!("HLS Receive: {}", url),
            StreamProtocol::Dash => format!("DASH Receive: {}", url),
            StreamProtocol::Rtmp { .. } => format!("RTMP Receive: {}", url),
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
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

        let thread_name = match &protocol {
            StreamProtocol::Srt { .. } => format!("srt-recv-{}", url),
            StreamProtocol::Hls => format!("hls-recv-{}", url),
            StreamProtocol::Dash => format!("dash-recv-{}", url),
            StreamProtocol::Rtmp { .. } => format!("rtmp-recv-{}", url),
        };

        let thread = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                stream_receive_thread(url_for_thread, url_display, frame_clone, stop_clone, connected_clone);
            })
            .ok();

        if thread.is_none() {
            log::error!("Failed to spawn stream receive thread for '{}'", url);
            return None;
        }

        let idx = self.receivers.len();
        self.receivers.push(StreamReceiver {
            url: url.to_string(),
            protocol,
            frame_data,
            stop_flag,
            connected,
            _thread: thread,
            width,
            height,
        });
        self.textures.push((texture, texture_view));
        log::info!("Stream receiver started for '{}'", url);
        Some(idx)
    }

    /// Start receiving from an SRT URL (convenience wrapper).
    pub fn start_srt_receive(&mut self, url: &str, mode: SrtMode, device: &wgpu::Device) -> Option<usize> {
        self.start_receive(url, StreamProtocol::Srt { mode }, device)
    }

    /// Start receiving from an RTMP URL (convenience wrapper).
    pub fn start_rtmp_receive(&mut self, url: &str, mode: RtmpMode, device: &wgpu::Device) -> Option<usize> {
        self.start_receive(url, StreamProtocol::Rtmp { mode }, device)
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

    /// Get protocol of receiver at index.
    pub fn receiver_protocol(&self, idx: usize) -> Option<&StreamProtocol> {
        self.receivers.get(idx).map(|r| &r.protocol)
    }

    /// Get SRT mode of receiver at index (convenience for SRT receivers).
    pub fn receiver_mode(&self, idx: usize) -> Option<SrtMode> {
        self.receivers.get(idx).and_then(|r| match &r.protocol {
            StreamProtocol::Srt { mode } => Some(*mode),
            _ => None,
        })
    }

    /// Get RTMP mode of receiver at index (convenience for RTMP receivers).
    pub fn receiver_rtmp_mode(&self, idx: usize) -> Option<RtmpMode> {
        self.receivers.get(idx).and_then(|r| match &r.protocol {
            StreamProtocol::Rtmp { mode } => Some(*mode),
            _ => None,
        })
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get_mut(idx) {
            r.stop_flag.store(true, Ordering::SeqCst);
            // Don't join — the thread may be blocked in ffmpeg I/O.
            // The interrupt callback will cause ffmpeg to abort, and
            // the thread holds only Arc clones so it's safe to detach.
            if let Some(t) = r._thread.take() {
                drop(t);
            }
        }
    }
}

impl Drop for StreamManager {
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

/// Build an RTMP URL, appending `listen=1` for Listen mode.
fn build_rtmp_url(url: &str, mode: RtmpMode) -> String {
    match mode {
        RtmpMode::Pull => url.to_string(),
        RtmpMode::Listen => {
            if url.contains('?') {
                format!("{}&listen=1", url)
            } else {
                format!("{}?listen=1", url)
            }
        }
    }
}

/// Background thread: decode stream via ffmpeg_next into RGBA frames.
/// Protocol-agnostic — ffmpeg handles SRT, HLS, DASH, and RTMP URLs natively.
fn stream_receive_thread(
    url: String,
    url_display: String,
    frame_data: Arc<Mutex<Option<Vec<u8>>>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
) {
    log::info!("Stream receive thread starting for '{}'", url_display);

    // Exponential backoff: starts at 500ms, caps at 10s, resets on successful frame
    let mut backoff_ms: u64 = 500;
    const MIN_BACKOFF_MS: u64 = 500;
    const MAX_BACKOFF_MS: u64 = 10_000;

    loop {
        if stop_flag.load(Ordering::SeqCst) { return; }

        // Serialize ffmpeg context creation — avcodec_open2 is not thread-safe.
        // Hold the lock through input open + decoder creation, release before
        // entering the decode loop so frame decoding is parallel.
        let setup_result = {
            let _guard = FFMPEG_INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

            let stop_for_interrupt = Arc::clone(&stop_flag);
            (|| -> Option<_> {
                let ctx = match ffmpeg_next::format::input_with_interrupt(
                    &url,
                    move || stop_for_interrupt.load(Ordering::SeqCst),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        log::warn!("Stream '{}' connection failed: {}, retrying in {}ms...", url_display, e, backoff_ms);
                        return None;
                    }
                };

                let stream_idx = ctx.streams().best(ffmpeg_next::media::Type::Video)
                    .map(|s| s.index())
                    .or_else(|| {
                        log::warn!("Stream '{}': no video stream found, retrying in {}ms...", url_display, backoff_ms);
                        None
                    })?;

                let codec_params = ctx.stream(stream_idx)
                    .or_else(|| {
                        log::warn!("Stream '{}': stream index {} not found, retrying in {}ms...", url_display, stream_idx, backoff_ms);
                        None
                    })?
                    .parameters();
                let codec_ctx = ffmpeg_next::codec::Context::from_parameters(codec_params)
                    .map_err(|e| log::warn!("Stream '{}': failed to create codec context: {}, retrying in {}ms...", url_display, e, backoff_ms))
                    .ok()?;
                let decoder = codec_ctx.decoder().video()
                    .map_err(|e| log::warn!("Stream '{}': failed to create video decoder: {}, retrying in {}ms...", url_display, e, backoff_ms))
                    .ok()?;

                let scaler = ffmpeg_next::software::scaling::Context::get(
                    decoder.format(), decoder.width(), decoder.height(),
                    ffmpeg_next::format::Pixel::RGBA, 1920, 1080,
                    ffmpeg_next::software::scaling::Flags::BILINEAR,
                ).ok();

                Some((ctx, stream_idx, decoder, scaler))
            })()
            // _guard dropped here — lock released before decode loop
        };

        let Some((mut input_ctx, stream_idx, mut decoder, mut scaler)) = setup_result else {
            connected.store(false, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
            backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
            continue;
        };

        log::info!("Stream '{}' connected, decoding video", url_display);

        let mut last_frame_time = std::time::Instant::now();
        // HLS/DASH segments can have multi-second gaps; use a longer timeout
        let is_segment_protocol = url.contains(".m3u8") || url.contains(".mpd")
            || url.starts_with("http://") || url.starts_with("https://");
        let stall_timeout = if is_segment_protocol {
            std::time::Duration::from_secs(15)
        } else {
            std::time::Duration::from_secs(5)
        };
        let mut consecutive_errors: u32 = 0;
        const MAX_CONSECUTIVE_ERRORS: u32 = 50;

        for (stream, packet) in input_ctx.packets() {
            if stop_flag.load(Ordering::SeqCst) { return; }
            if stream.index() != stream_idx { continue; }

            if decoder.send_packet(&packet).is_err() {
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    log::warn!("Stream '{}': {} consecutive decode errors, reconnecting", url_display, consecutive_errors);
                    break;
                }
                continue;
            }
            consecutive_errors = 0;

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
                        let width = rgb_frame.width() as usize;
                        let height = rgb_frame.height() as usize;
                        let stride = rgb_frame.stride(0);
                        let row_bytes = width * 4;
                        let expected = row_bytes * height;
                        let data = rgb_frame.data(0);

                        let rgba = if stride == row_bytes {
                            if data.len() >= expected {
                                data[..expected].to_vec()
                            } else {
                                continue;
                            }
                        } else {
                            let mut buf = Vec::with_capacity(expected);
                            for row in 0..height {
                                let start = row * stride;
                                let end = start + row_bytes;
                                if end > data.len() { break; }
                                buf.extend_from_slice(&data[start..end]);
                            }
                            if buf.len() != expected { continue; }
                            buf
                        };

                        if let Ok(mut guard) = frame_data.lock() {
                            *guard = Some(rgba);
                        }
                        last_frame_time = std::time::Instant::now();
                        // Reset backoff on successful frame delivery
                        backoff_ms = MIN_BACKOFF_MS;
                        if !connected.load(Ordering::SeqCst) {
                            connected.store(true, Ordering::SeqCst);
                            log::info!("Stream '{}' first frame received ({}x{}, stride={})",
                                url_display, width, height, stride);
                        }
                    }
                }
            }

            // If we haven't produced a frame in a while, the demuxer is
            // likely stuck (e.g. HLS segments deleted). Force reconnect.
            if last_frame_time.elapsed() > stall_timeout {
                log::warn!("Stream '{}': no frames for {:.1}s, reconnecting",
                    url_display, last_frame_time.elapsed().as_secs_f32());
                break;
            }
        }

        log::warn!("Stream '{}' ended, reconnecting in {}ms...", url_display, backoff_ms);
        connected.store(false, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_manager_new_empty() {
        let mgr = StreamManager::new();
        assert_eq!(mgr.receivers.len(), 0);
        assert_eq!(mgr.textures.len(), 0);
        assert!(!mgr.is_connected(0));
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.receiver_dimensions(0).is_none());
    }

    #[test]
    fn stream_manager_receiver_count_empty() {
        let mgr = StreamManager::new();
        assert_eq!(mgr.receiver_count(), 0);
    }

    #[test]
    fn stream_manager_receiver_url_out_of_bounds() {
        let mgr = StreamManager::new();
        assert!(mgr.receiver_url(0).is_none());
        assert!(mgr.receiver_url(999).is_none());
    }

    #[test]
    fn stream_manager_receiver_mode_out_of_bounds() {
        let mgr = StreamManager::new();
        assert!(mgr.receiver_mode(0).is_none());
        assert!(mgr.receiver_mode(999).is_none());
    }

    #[test]
    fn stream_manager_is_connected_out_of_bounds() {
        let mgr = StreamManager::new();
        assert!(!mgr.is_connected(0));
        assert!(!mgr.is_connected(100));
    }

    #[test]
    fn stream_manager_receiver_dimensions_out_of_bounds() {
        let mgr = StreamManager::new();
        assert!(mgr.receiver_dimensions(0).is_none());
    }

    #[test]
    fn stream_manager_stop_receive_out_of_bounds_no_panic() {
        let mut mgr = StreamManager::new();
        mgr.stop_receive(0);
        mgr.stop_receive(999);
    }

    #[test]
    fn stream_protocol_srt_serialization() {
        let proto = StreamProtocol::Srt { mode: SrtMode::Listener };
        let json = serde_json::to_string(&proto).unwrap();
        let restored: StreamProtocol = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, StreamProtocol::Srt { mode: SrtMode::Listener });
    }

    #[test]
    fn stream_protocol_hls_serialization() {
        let proto = StreamProtocol::Hls;
        let json = serde_json::to_string(&proto).unwrap();
        let restored: StreamProtocol = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, StreamProtocol::Hls);
    }

    #[test]
    fn stream_protocol_dash_serialization() {
        let proto = StreamProtocol::Dash;
        let json = serde_json::to_string(&proto).unwrap();
        let restored: StreamProtocol = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, StreamProtocol::Dash);
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
    fn srt_mode_equality() {
        assert_eq!(SrtMode::Listener, SrtMode::Listener);
        assert_eq!(SrtMode::Caller, SrtMode::Caller);
        assert_ne!(SrtMode::Listener, SrtMode::Caller);
    }

    #[test]
    fn srt_mode_clone() {
        let original = SrtMode::Caller;
        let cloned = original;
        assert_eq!(original, cloned);
    }

    #[test]
    fn srt_mode_debug() {
        let debug = format!("{:?}", SrtMode::Listener);
        assert!(debug.contains("Listener"));
        let debug = format!("{:?}", SrtMode::Caller);
        assert!(debug.contains("Caller"));
    }

    #[test]
    fn srt_mode_deserialize_from_string() {
        // Verify it can deserialize from JSON strings
        let listener: SrtMode = serde_json::from_str("\"Listener\"").unwrap();
        assert_eq!(listener, SrtMode::Listener);
        let caller: SrtMode = serde_json::from_str("\"Caller\"").unwrap();
        assert_eq!(caller, SrtMode::Caller);
    }

    #[test]
    fn srt_mode_deserialize_invalid() {
        let result: Result<SrtMode, _> = serde_json::from_str("\"InvalidMode\"");
        assert!(result.is_err());
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

    #[test]
    fn build_srt_url_localhost_default_port() {
        let url = build_srt_url("srt://127.0.0.1:9001", SrtMode::Caller);
        assert_eq!(url, "srt://127.0.0.1:9001?mode=caller");
    }

    #[test]
    fn build_srt_url_listener_wildcard() {
        let url = build_srt_url("srt://:9001", SrtMode::Listener);
        assert_eq!(url, "srt://:9001?mode=listener");
    }

    #[test]
    fn build_srt_url_multiple_existing_params() {
        let url = build_srt_url("srt://host:9000?latency=0&passphrase=test", SrtMode::Listener);
        assert_eq!(url, "srt://host:9000?latency=0&passphrase=test&mode=listener");
    }

    #[test]
    fn stream_protocol_rtmp_serialization() {
        let protocol = StreamProtocol::Rtmp { mode: RtmpMode::Pull };
        let json = serde_json::to_string(&protocol).unwrap();
        let deserialized: StreamProtocol = serde_json::from_str(&json).unwrap();
        assert_eq!(protocol, deserialized);
    }

    #[test]
    fn rtmp_mode_display() {
        assert_eq!(RtmpMode::Pull.to_string(), "Pull");
        assert_eq!(RtmpMode::Listen.to_string(), "Listen");
    }

    #[test]
    fn rtmp_mode_serialization_roundtrip() {
        let pull = RtmpMode::Pull;
        let listen = RtmpMode::Listen;
        let pull_json = serde_json::to_string(&pull).unwrap();
        let listen_json = serde_json::to_string(&listen).unwrap();
        assert_eq!(serde_json::from_str::<RtmpMode>(&pull_json).unwrap(), pull);
        assert_eq!(serde_json::from_str::<RtmpMode>(&listen_json).unwrap(), listen);
    }
}
