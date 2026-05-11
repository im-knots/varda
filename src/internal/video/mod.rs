//! Video playback support for Varda
//!
//! Two codec paths:
//! - **HAP path**: GPU-native BCn compressed textures — near-zero CPU decode cost.
//!   Supports Hap (BC1), Hap Alpha (BC3), Hap R (BC7).
//! - **ffmpeg path**: CPU decode for H.264, ProRes, VP9, etc. — fallback for all other codecs.

pub mod hap;

use anyhow::{Context, Result};
use std::path::Path;

extern crate ffmpeg_next as ffmpeg;

use ffmpeg::format::{input, Pixel};
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;

/// Loop mode for video playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum LoopMode {
    /// Standard loop — restart from in-point when reaching out-point.
    Loop,
    /// Play forward then reverse repeatedly.
    PingPong,
    /// Play once and stop at the out-point.
    OneShot,
    /// Play once and hold the last frame.
    HoldLast,
}

/// Result of advancing playback state.
pub struct AdvanceResult {
    /// Whether a seek is needed (loop restart, etc.).
    pub needs_seek: bool,
    /// Number of video frames to decode (0 = hold current frame, 1+ = decode new frames).
    pub frames_to_decode: u32,
}

/// Shared playback state for all video sources (ffmpeg and HAP).
#[derive(Debug, Clone)]
pub struct PlaybackState {
    /// Whether the video is currently playing.
    pub playing: bool,
    /// Loop mode.
    pub loop_mode: LoopMode,
    /// Speed multiplier (1.0 = normal, 0.5 = half, 2.0 = double, negative = reverse).
    pub speed: f64,
    /// In-point in seconds (start of playback range). 0.0 = beginning.
    pub in_point: f64,
    /// Out-point in seconds (end of playback range). 0.0 = use duration.
    pub out_point: f64,
    /// Current playback position in seconds.
    pub position: f64,
    /// Whether we're currently playing in reverse (for ping-pong).
    pub reverse: bool,
    /// Video duration in seconds.
    pub duration: f64,
    /// Video frame rate.
    pub frame_rate: f64,
    /// Set to true for one frame when playback reaches the out-point/EOF.
    /// Used by auto-transition ClipEnd trigger. Cleared each frame before advance.
    pub reached_end: bool,
    /// Last wall-clock time advance_frame was called (for real-time delta).
    last_advance: std::time::Instant,
    /// Fractional frame accumulator — tracks sub-frame position for pacing.
    frame_accumulator: f64,
}

impl PlaybackState {
    pub fn new(duration: f64, frame_rate: f64) -> Self {
        Self {
            playing: true,
            loop_mode: LoopMode::Loop,
            speed: 1.0,
            in_point: 0.0,
            out_point: 0.0,
            position: 0.0,
            reverse: false,
            duration,
            frame_rate,
            reached_end: false,
            last_advance: std::time::Instant::now(),
            frame_accumulator: 0.0,
        }
    }

    /// Effective out-point (uses duration if out_point is 0).
    pub fn effective_out(&self) -> f64 {
        if self.out_point > 0.0 { self.out_point } else { self.duration }
    }

    /// Advance playback position using real wall-clock time.
    /// Returns how many video frames to decode and whether a seek is needed.
    pub fn advance_frame(&mut self) -> AdvanceResult {
        self.reached_end = false;
        if !self.playing {
            self.last_advance = std::time::Instant::now();
            return AdvanceResult { needs_seek: false, frames_to_decode: 0 };
        }

        let now = std::time::Instant::now();
        let wall_dt = now.duration_since(self.last_advance).as_secs_f64();
        self.last_advance = now;
        // Clamp dt to avoid huge jumps after pauses/stalls
        let dt = wall_dt.min(0.1);

        let delta = dt * self.speed.abs() * if self.reverse { -1.0 } else { 1.0 };
        self.position += delta;

        // Accumulate frames: how many video frames does this time step cover?
        let frame_time = 1.0 / self.frame_rate;
        self.frame_accumulator += dt * self.speed.abs();
        let frames_to_decode = (self.frame_accumulator / frame_time).floor() as u32;
        self.frame_accumulator -= frames_to_decode as f64 * frame_time;

        let in_pt = self.in_point;
        let out_pt = self.effective_out();

        if self.position >= out_pt {
            self.reached_end = true;
            self.frame_accumulator = 0.0;
            match self.loop_mode {
                LoopMode::Loop => { self.position = in_pt; return AdvanceResult { needs_seek: true, frames_to_decode: 1 }; }
                LoopMode::PingPong => { self.reverse = true; self.position = out_pt - frame_time; }
                LoopMode::OneShot => { self.playing = false; self.position = out_pt; }
                LoopMode::HoldLast => { self.position = out_pt; }
            }
        } else if self.position < in_pt {
            self.frame_accumulator = 0.0;
            match self.loop_mode {
                LoopMode::Loop | LoopMode::OneShot | LoopMode::HoldLast => {
                    self.position = in_pt;
                    return AdvanceResult { needs_seek: true, frames_to_decode: 1 };
                }
                LoopMode::PingPong => { self.reverse = false; self.position = in_pt + frame_time; }
            }
        }
        AdvanceResult { needs_seek: false, frames_to_decode }
    }
}

/// GPU-compressed texture format for HAP video frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapTextureFormat {
    /// BC1 / DXT1 — RGB, no alpha (Hap)
    Bc1,
    /// BC3 / DXT5 — RGBA with interpolated alpha (Hap Alpha)
    Bc3,
    /// BC3 / DXT5 storing Scaled YCoCg color (Hap Q) — needs shader conversion to RGB
    Bc3YCoCg,
    /// BC4 / RGTC1 — single-channel alpha (Hap Alpha-Only, or alpha plane of Hap Q Alpha)
    Bc4,
    /// BC7 / BPTC — RGBA, best quality (Hap R)
    Bc7,
}

impl HapTextureFormat {
    /// Bytes per 4×4 block for this format.
    pub fn block_bytes(self) -> u32 {
        match self {
            Self::Bc1 | Self::Bc4 => 8,
            Self::Bc3 | Self::Bc3YCoCg | Self::Bc7 => 16,
        }
    }

    /// Corresponding wgpu texture format.
    pub fn wgpu_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Bc1 => wgpu::TextureFormat::Bc1RgbaUnorm,
            Self::Bc3 | Self::Bc3YCoCg => wgpu::TextureFormat::Bc3RgbaUnorm,
            Self::Bc4 => wgpu::TextureFormat::Bc4RUnorm,
            Self::Bc7 => wgpu::TextureFormat::Bc7RgbaUnorm,
        }
    }

    /// Whether this format requires YCoCg→RGB conversion in a shader.
    pub fn needs_ycocg_convert(self) -> bool {
        matches!(self, Self::Bc3YCoCg)
    }

    /// Calculate the byte size of a full frame in this compressed format.
    pub fn frame_byte_size(self, width: u32, height: u32) -> usize {
        let blocks_x = (width + 3) / 4;
        let blocks_y = (height + 3) / 4;
        (blocks_x * blocks_y * self.block_bytes()) as usize
    }
}

/// A decoded video frame — either CPU-decoded RGBA or GPU-compressed BCn.
pub enum VideoFrame<'a> {
    /// Standard RGBA pixel data (from ffmpeg CPU decode).
    Rgba(&'a [u8]),
    /// GPU-compressed BCn texture data (from HAP decode).
    Compressed {
        data: &'a [u8],
        format: HapTextureFormat,
    },
}

/// Detect whether a video file uses a HAP codec.
/// Returns the HAP texture format if it is HAP, or None for standard codecs.
pub fn detect_hap_codec<P: AsRef<Path>>(path: P) -> Result<Option<HapTextureFormat>> {
    ffmpeg::init().context("Failed to initialize FFmpeg")?;
    let ictx = input(&path).context("Failed to open video file for codec detection")?;

    let video_stream = ictx
        .streams()
        .best(Type::Video)
        .context("No video stream found")?;

    // Get the codec tag (FourCC) from stream parameters
    let params = video_stream.parameters();
    let codec_ctx = ffmpeg::codec::context::Context::from_parameters(params)?;
    let codec_id = codec_ctx.id();

    // Check if codec is HAP by examining the codec ID
    // ffmpeg maps HAP variants to AV_CODEC_ID_HAP
    let codec_name = codec_id.name();
    if codec_name == "hap" {
        // Determine the specific HAP variant from the codec tag
        // We need to probe the first frame to determine the exact texture format
        // since ffmpeg groups all HAP variants under one codec ID.
        // We'll determine the format when we parse the first frame in HapPlayer.
        return Ok(Some(HapTextureFormat::Bc7)); // default; HapPlayer refines this
    }

    Ok(None)
}

/// Maximum frame cache memory in bytes (2 GB).
/// Frames are cached during forward playback and served in reverse for ping-pong.
/// At 1080p (~2.5 MB/frame) this holds ~800 frames (~13s at 60fps).
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// A video player that decodes frames from a video file using ffmpeg (CPU decode).
///
/// # Safety: Send
/// The ffmpeg types (`Input`, `Video` decoder, `Scaler`) contain raw pointers to C-allocated
/// state. These pointers represent exclusive ownership of heap allocations — there is no
/// shared mutable state across instances. Transferring a `VideoPlayer` between threads is
/// safe because Rust's ownership system guarantees exclusive access (no concurrent use).
/// The player is always used from a single thread at a time.
pub struct VideoPlayer {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    scaler: Scaler,
    video_stream_index: usize,
    width: u32,
    height: u32,
    /// Shared playback state (loop mode, speed, in/out points, position).
    pub playback: PlaybackState,
    /// Current frame data (RGBA).
    frame_data: Vec<u8>,
    /// Frame cache for reverse playback (ping-pong).
    /// Filled during forward play, drained in reverse order.
    frame_cache: Vec<Vec<u8>>,
    /// Current read index into frame_cache during reverse playback.
    cache_read_idx: usize,
    /// Whether we're actively caching frames (disabled when memory cap hit).
    caching_enabled: bool,
    /// Set permanently once the cache overflows — prevents re-filling on subsequent passes.
    cache_overflowed: bool,
    /// Bytes per frame for cache budget calculation.
    frame_byte_size: usize,
}

// SAFETY: See doc comment on VideoPlayer. Exclusive ownership of C allocations, no concurrent use.
unsafe impl Send for VideoPlayer {}

impl VideoPlayer {
    /// Create a new video player from a file path.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        ffmpeg::init().context("Failed to initialize FFmpeg")?;
        let ictx = input(&path).context("Failed to open video file")?;
        let video_stream = ictx.streams().best(Type::Video).context("No video stream found")?;
        let video_stream_index = video_stream.index();
        let context_decoder = ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())?;
        let decoder = context_decoder.decoder().video()?;
        let width = decoder.width();
        let height = decoder.height();
        let rate = video_stream.rate();
        let fps = rate.0 as f64 / rate.1 as f64;
        let duration = if ictx.duration() > 0 {
            ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64
        } else { 0.0 };
        let scaler = Scaler::get(
            decoder.format(), width, height,
            Pixel::RGBA, width, height, Flags::BILINEAR,
        )?;
        let frame_byte_size = (width * height * 4) as usize;
        let frame_data = vec![0u8; frame_byte_size];
        let max_cached_frames = MAX_CACHE_BYTES / frame_byte_size.max(1);
        log::info!("Loaded video: {}x{} @ {:.2} fps, duration: {:.2}s (ping-pong cache: {} frames)",
            width, height, fps, duration, max_cached_frames);
        Ok(Self {
            ictx, decoder, scaler, video_stream_index, width, height,
            playback: PlaybackState::new(duration, fps),
            frame_data,
            frame_cache: Vec::new(),
            cache_read_idx: 0,
            caching_enabled: true,
            cache_overflowed: false,
            frame_byte_size,
        })
    }

    /// Get the next frame as RGBA data.
    /// Uses wall-clock time pacing: only decodes new frames when enough real
    /// time has elapsed (respecting speed multiplier). At speed < 1.0, frames
    /// are held longer; at speed > 1.0, frames are skipped.
    pub fn next_frame(&mut self) -> Result<Option<&[u8]>> {
        if !self.playback.playing {
            return Ok(None);
        }
        let was_reverse = self.playback.reverse;
        let result = self.playback.advance_frame();

        // No frames to decode this tick — hold current frame
        if result.frames_to_decode == 0 && !result.needs_seek {
            return Ok(Some(&self.frame_data));
        }

        // Detect ping-pong boundary flips from advance_frame:
        if !was_reverse && self.playback.reverse {
            // Forward→reverse flip (hit out-point). Serve from cache.
            if !self.frame_cache.is_empty() {
                self.cache_read_idx = self.frame_cache.len();
            }
        } else if was_reverse && !self.playback.reverse {
            // Reverse→forward flip (hit in-point). Clear cache, seek to in-point.
            self.frame_cache.clear();
            self.caching_enabled = true;
            self.seek(self.playback.position)?;
        }

        // Reverse playback: serve frames from cache
        if self.playback.reverse {
            // Skip frames for speed > 1.0 in reverse
            let skip = result.frames_to_decode.max(1) as usize;
            if self.cache_read_idx >= skip {
                self.cache_read_idx -= skip;
                self.frame_data.copy_from_slice(&self.frame_cache[self.cache_read_idx]);
                return Ok(Some(&self.frame_data));
            }
            // Cache exhausted — flip back to forward
            self.playback.reverse = false;
            self.playback.position = self.playback.in_point;
            self.frame_cache.clear();
            self.caching_enabled = !self.cache_overflowed;
            self.seek(self.playback.position)?;
        } else if result.needs_seek {
            // Forward seek (loop restart, etc.)
            self.frame_cache.clear();
            self.caching_enabled = !self.cache_overflowed;
            self.seek(self.playback.position)?;
        }

        // Forward playback — decode frames_to_decode frames (skip intermediate ones)
        let target_frames = result.frames_to_decode.max(1);
        let mut decoded_count = 0u32;
        loop {
            let mut decoded = Video::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                decoded_count += 1;
                // Only convert the last frame we need (skip intermediate for speed > 1)
                if decoded_count >= target_frames {
                    let mut rgb_frame = Video::empty();
                    self.scaler.run(&decoded, &mut rgb_frame)?;
                    let data = rgb_frame.data(0);
                    let stride = rgb_frame.stride(0);
                    for y in 0..self.height as usize {
                        let src_offset = y * stride;
                        let dst_offset = y * (self.width as usize * 4);
                        let row_bytes = self.width as usize * 4;
                        self.frame_data[dst_offset..dst_offset + row_bytes]
                            .copy_from_slice(&data[src_offset..src_offset + row_bytes]);
                    }
                    // Cache frame for potential reverse playback
                    if self.caching_enabled && self.playback.loop_mode == LoopMode::PingPong {
                        if self.frame_cache.len() * self.frame_byte_size < MAX_CACHE_BYTES {
                            self.frame_cache.push(self.frame_data.clone());
                        } else {
                            self.caching_enabled = false;
                            if !self.cache_overflowed {
                                self.cache_overflowed = true;
                                log::warn!("Ping-pong frame cache full ({} frames, {} MB) — reverse will loop instead",
                                    self.frame_cache.len(),
                                    self.frame_cache.len() * self.frame_byte_size / (1024 * 1024));
                            }
                        }
                    }
                    return Ok(Some(&self.frame_data));
                }
                // Intermediate frame at speed > 1: still cache for ping-pong
                if self.caching_enabled && self.playback.loop_mode == LoopMode::PingPong {
                    // Lightweight: decode into scaler for cache but skip if over budget
                    let mut rgb_frame = Video::empty();
                    self.scaler.run(&decoded, &mut rgb_frame)?;
                    let data = rgb_frame.data(0);
                    let stride = rgb_frame.stride(0);
                    let mut cache_buf = vec![0u8; self.frame_byte_size];
                    for y in 0..self.height as usize {
                        let src_offset = y * stride;
                        let dst_offset = y * (self.width as usize * 4);
                        let row_bytes = self.width as usize * 4;
                        cache_buf[dst_offset..dst_offset + row_bytes]
                            .copy_from_slice(&data[src_offset..src_offset + row_bytes]);
                    }
                    if self.frame_cache.len() * self.frame_byte_size < MAX_CACHE_BYTES {
                        self.frame_cache.push(cache_buf);
                    }
                }
                continue;
            }
            match self.ictx.packets().next() {
                Some((stream, packet)) => {
                    if stream.index() == self.video_stream_index {
                        self.decoder.send_packet(&packet)?;
                    }
                }
                None => {
                    // End of stream
                    match self.playback.loop_mode {
                        LoopMode::Loop => {
                            self.playback.position = self.playback.in_point;
                            self.seek(self.playback.position)?;
                            continue;
                        }
                        LoopMode::PingPong => {
                            if !self.frame_cache.is_empty() {
                                self.playback.reverse = true;
                                self.cache_read_idx = self.frame_cache.len();
                                return self.next_frame();
                            }
                            self.playback.position = self.playback.in_point;
                            self.seek(self.playback.position)?;
                            continue;
                        }
                        LoopMode::OneShot => {
                            self.playback.playing = false;
                            return Ok(None);
                        }
                        LoopMode::HoldLast => {
                            return Ok(Some(&self.frame_data));
                        }
                    }
                }
            }
        }
    }

    /// Seek to a specific time in seconds (internal — does not clear cache).
    fn seek(&mut self, time_secs: f64) -> Result<()> {
        let timestamp = (time_secs * ffmpeg::ffi::AV_TIME_BASE as f64) as i64;
        self.ictx.seek(timestamp, ..timestamp)?;
        self.decoder.flush();
        self.playback.position = time_secs;
        Ok(())
    }

    /// Seek to a specific time and reset the frame cache.
    /// Use this for user-initiated seeks (scrub bar, etc.).
    pub fn seek_and_reset(&mut self, time_secs: f64) -> Result<()> {
        self.frame_cache.clear();
        self.cache_read_idx = 0;
        self.caching_enabled = true;
        self.cache_overflowed = false;
        self.playback.reverse = false;
        self.seek(time_secs)
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn frame_rate(&self) -> f64 { self.playback.frame_rate }
    pub fn duration(&self) -> f64 { self.playback.duration }
    pub fn is_playing(&self) -> bool { self.playback.playing }
    pub fn set_playing(&mut self, playing: bool) { self.playback.playing = playing; }
    pub fn is_looping(&self) -> bool { self.playback.loop_mode == LoopMode::Loop }
    pub fn set_looping(&mut self, looping: bool) {
        self.playback.loop_mode = if looping { LoopMode::Loop } else { LoopMode::OneShot };
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playback_state_defaults() {
        let ps = PlaybackState::new(10.0, 30.0);
        assert!(ps.playing);
        assert_eq!(ps.loop_mode, LoopMode::Loop);
        assert_eq!(ps.speed, 1.0);
        assert_eq!(ps.in_point, 0.0);
        assert_eq!(ps.out_point, 0.0);
        assert_eq!(ps.position, 0.0);
        assert!(!ps.reverse);
        assert_eq!(ps.effective_out(), 10.0);
    }

    #[test]
    fn test_playback_state_advance_moves_position() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        // Sleep briefly so wall-clock dt > 0
        std::thread::sleep(std::time::Duration::from_millis(20));
        let result = ps.advance_frame();
        assert!(!result.needs_seek);
        // Position should have advanced by ~20ms worth
        assert!(ps.position > 0.0);
        assert!(ps.position < 0.1); // sanity: not more than 100ms
    }

    #[test]
    fn test_playback_state_loop_restart() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.position = 1.1; // already past out-point
        // Ensure some dt elapses
        std::thread::sleep(std::time::Duration::from_millis(5));
        let result = ps.advance_frame();
        assert!(result.needs_seek);
        assert_eq!(ps.position, 0.0);
    }

    #[test]
    fn test_playback_state_one_shot_stops() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.loop_mode = LoopMode::OneShot;
        ps.position = 1.1;
        std::thread::sleep(std::time::Duration::from_millis(5));
        ps.advance_frame();
        assert!(!ps.playing);
    }

    #[test]
    fn test_playback_state_hold_last() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.loop_mode = LoopMode::HoldLast;
        ps.position = 1.1;
        std::thread::sleep(std::time::Duration::from_millis(5));
        ps.advance_frame();
        assert!(ps.playing);
        assert_eq!(ps.position, 1.0);
    }

    #[test]
    fn test_playback_state_ping_pong() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.loop_mode = LoopMode::PingPong;
        ps.position = 1.1;
        std::thread::sleep(std::time::Duration::from_millis(5));
        ps.advance_frame();
        assert!(ps.reverse);
        assert!(ps.position < 1.0);
    }

    #[test]
    fn test_playback_state_in_out_points() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.in_point = 2.0;
        ps.out_point = 5.0;
        ps.position = 5.1; // past out-point
        std::thread::sleep(std::time::Duration::from_millis(5));
        let result = ps.advance_frame();
        assert!(result.needs_seek);
        assert_eq!(ps.position, 2.0);
    }

    #[test]
    fn test_playback_state_speed_affects_position() {
        // Two states: one at speed 1, one at speed 3
        let mut ps_slow = PlaybackState::new(10.0, 30.0);
        let mut ps_fast = PlaybackState::new(10.0, 30.0);
        ps_fast.speed = 3.0;
        std::thread::sleep(std::time::Duration::from_millis(30));
        ps_slow.advance_frame();
        ps_fast.advance_frame();
        // Fast should advance ~3x further
        assert!(ps_fast.position > ps_slow.position * 2.0,
            "fast={} should be > 2x slow={}", ps_fast.position, ps_slow.position);
    }

    #[test]
    fn test_playback_state_not_playing() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.playing = false;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = ps.advance_frame();
        assert!(!result.needs_seek);
        assert_eq!(result.frames_to_decode, 0);
        assert_eq!(ps.position, 0.0);
    }

    #[test]
    fn test_playback_frame_pacing_slow_speed() {
        // At speed 0.1 with 30fps video, each frame should last ~333ms.
        // A 10ms advance should produce 0 frames to decode.
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.speed = 0.1;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = ps.advance_frame();
        assert_eq!(result.frames_to_decode, 0);
    }

    #[test]
    fn test_hap_texture_format_block_bytes() {
        assert_eq!(HapTextureFormat::Bc1.block_bytes(), 8);
        assert_eq!(HapTextureFormat::Bc3.block_bytes(), 16);
        assert_eq!(HapTextureFormat::Bc3YCoCg.block_bytes(), 16);
        assert_eq!(HapTextureFormat::Bc4.block_bytes(), 8);
        assert_eq!(HapTextureFormat::Bc7.block_bytes(), 16);
    }

    #[test]
    fn test_hap_texture_format_frame_byte_size() {
        assert_eq!(HapTextureFormat::Bc1.frame_byte_size(8, 8), 4 * 8);
        assert_eq!(HapTextureFormat::Bc7.frame_byte_size(8, 8), 4 * 16);
        assert_eq!(HapTextureFormat::Bc1.frame_byte_size(5, 5), 4 * 8);
    }

    #[test]
    fn test_hap_texture_format_needs_ycocg() {
        assert!(!HapTextureFormat::Bc1.needs_ycocg_convert());
        assert!(!HapTextureFormat::Bc3.needs_ycocg_convert());
        assert!(HapTextureFormat::Bc3YCoCg.needs_ycocg_convert());
        assert!(!HapTextureFormat::Bc4.needs_ycocg_convert());
        assert!(!HapTextureFormat::Bc7.needs_ycocg_convert());
    }
}