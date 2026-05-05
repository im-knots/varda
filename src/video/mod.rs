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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        }
    }

    /// Effective out-point (uses duration if out_point is 0).
    pub fn effective_out(&self) -> f64 {
        if self.out_point > 0.0 { self.out_point } else { self.duration }
    }

    /// Advance position by one frame at current speed. Returns true if a seek is needed.
    pub fn advance_frame(&mut self) -> bool {
        self.reached_end = false;
        if !self.playing {
            return false;
        }
        let frame_time = 1.0 / self.frame_rate;
        let delta = frame_time * self.speed.abs() * if self.reverse { -1.0 } else { 1.0 };
        self.position += delta;

        let in_pt = self.in_point;
        let out_pt = self.effective_out();

        if self.position >= out_pt {
            self.reached_end = true;
            match self.loop_mode {
                LoopMode::Loop => { self.position = in_pt; return true; }
                LoopMode::PingPong => { self.reverse = true; self.position = out_pt - frame_time; }
                LoopMode::OneShot => { self.playing = false; self.position = out_pt; }
                LoopMode::HoldLast => { self.position = out_pt; }
            }
        } else if self.position < in_pt {
            match self.loop_mode {
                LoopMode::Loop | LoopMode::OneShot | LoopMode::HoldLast => {
                    self.position = in_pt;
                    return true;
                }
                LoopMode::PingPong => { self.reverse = false; self.position = in_pt + frame_time; }
            }
        }
        false
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
    pub fn next_frame(&mut self) -> Result<Option<&[u8]>> {
        if !self.playback.playing {
            return Ok(None);
        }
        let was_reverse = self.playback.reverse;
        let needs_seek = self.playback.advance_frame();

        // Detect ping-pong boundary flips from advance_frame:
        if !was_reverse && self.playback.reverse {
            // Forward→reverse flip (hit out-point). Serve from cache.
            if !self.frame_cache.is_empty() {
                self.cache_read_idx = self.frame_cache.len();
                // Fall through to reverse branch
            }
        } else if was_reverse && !self.playback.reverse {
            // Reverse→forward flip (hit in-point). Clear cache, seek to in-point.
            self.frame_cache.clear();
            self.caching_enabled = true;
            self.seek(self.playback.position)?;
            // Fall through to forward decode
        }

        // Reverse playback: serve frames from cache
        if self.playback.reverse {
            if self.cache_read_idx > 0 {
                self.cache_read_idx -= 1;
                self.frame_data.copy_from_slice(&self.frame_cache[self.cache_read_idx]);
                return Ok(Some(&self.frame_data));
            }
            // Cache exhausted — flip back to forward
            self.playback.reverse = false;
            self.playback.position = self.playback.in_point;
            self.frame_cache.clear();
            self.caching_enabled = !self.cache_overflowed;
            self.seek(self.playback.position)?;
            // Fall through to forward decode below
        } else if needs_seek {
            // Forward seek (loop restart, etc.) — clear cache since we're
            // starting a new forward pass
            self.frame_cache.clear();
            self.caching_enabled = !self.cache_overflowed;
            self.seek(self.playback.position)?;
        }

        // Forward playback
        loop {
            let mut decoded = Video::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
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
                                // Flip to reverse, start serving from cache end
                                self.playback.reverse = true;
                                self.cache_read_idx = self.frame_cache.len();
                                // Recurse once to serve the first reverse frame
                                return self.next_frame();
                            }
                            // No cache — fall back to loop
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
    fn test_playback_state_advance_normal() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        let needs_seek = ps.advance_frame();
        assert!(!needs_seek);
        assert!((ps.position - 1.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn test_playback_state_loop_restart() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.position = 0.99;
        let needs_seek = ps.advance_frame();
        assert!(needs_seek);
        assert_eq!(ps.position, 0.0);
    }

    #[test]
    fn test_playback_state_one_shot_stops() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.loop_mode = LoopMode::OneShot;
        ps.position = 0.99;
        ps.advance_frame();
        assert!(!ps.playing);
        assert_eq!(ps.position, 1.0);
    }

    #[test]
    fn test_playback_state_hold_last() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.loop_mode = LoopMode::HoldLast;
        ps.position = 0.99;
        ps.advance_frame();
        assert!(ps.playing);
        assert_eq!(ps.position, 1.0);
    }

    #[test]
    fn test_playback_state_ping_pong() {
        let mut ps = PlaybackState::new(1.0, 30.0);
        ps.loop_mode = LoopMode::PingPong;
        ps.position = 0.99;
        ps.advance_frame();
        assert!(ps.reverse);
        assert!(ps.position < 1.0);
    }

    #[test]
    fn test_playback_state_in_out_points() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.in_point = 2.0;
        ps.out_point = 5.0;
        ps.position = 4.99;
        let needs_seek = ps.advance_frame();
        assert!(needs_seek);
        assert_eq!(ps.position, 2.0);
    }

    #[test]
    fn test_playback_state_speed_multiplier() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.speed = 2.0;
        ps.advance_frame();
        assert!((ps.position - 2.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn test_playback_state_not_playing() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.playing = false;
        let needs_seek = ps.advance_frame();
        assert!(!needs_seek);
        assert_eq!(ps.position, 0.0);
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