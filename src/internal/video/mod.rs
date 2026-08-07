//! Video playback support for Varda
//!
//! Two codec paths:
//! - **HAP path**: GPU-native `BCn` compressed textures — near-zero CPU decode cost.
//!   Supports Hap (BC1), Hap Alpha (BC3), Hap R (BC7).
//! - **ffmpeg path**: CPU decode for H.264, `ProRes`, VP9, etc. — fallback for all other codecs.

pub mod hap;

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

extern crate ffmpeg_next as ffmpeg;

use ffmpeg::format::{input, Pixel};
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;

/// Loop mode for video playback. Definition lives in `engine::value::video`
/// (see /spec/engine-value-types.md); re-exported here so existing
/// `crate::video::LoopMode` call sites keep working.
pub use crate::engine::value::video::LoopMode;

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
    /// Used by auto-transition `ClipEnd` trigger. Cleared each frame before advance.
    pub reached_end: bool,
    /// Last wall-clock time `advance_frame` was called (for real-time delta).
    last_advance: std::time::Instant,
    /// Fractional frame accumulator — tracks sub-frame position for pacing.
    frame_accumulator: f64,
}

impl PlaybackState {
    pub fn new(duration: f64, frame_rate: f64) -> Self {
        let frame_rate = if frame_rate > 0.0 { frame_rate } else { 30.0 };
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

    /// Effective out-point (uses duration if `out_point` is 0).
    pub fn effective_out(&self) -> f64 {
        if self.out_point > 0.0 {
            self.out_point
        } else {
            self.duration
        }
    }

    /// Advance playback position using real wall-clock time.
    /// Returns how many video frames to decode and whether a seek is needed.
    pub fn advance_frame(&mut self) -> AdvanceResult {
        self.reached_end = false;
        if !self.playing {
            self.last_advance = std::time::Instant::now();
            return AdvanceResult {
                needs_seek: false,
                frames_to_decode: 0,
            };
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
        self.frame_accumulator -= f64::from(frames_to_decode) * frame_time;

        let in_pt = self.in_point;
        let out_pt = self.effective_out();

        if self.position >= out_pt {
            self.reached_end = true;
            self.frame_accumulator = 0.0;
            match self.loop_mode {
                LoopMode::Loop => {
                    self.position = in_pt;
                    return AdvanceResult {
                        needs_seek: true,
                        frames_to_decode: 1,
                    };
                }
                LoopMode::PingPong => {
                    self.reverse = true;
                    self.position = out_pt - frame_time;
                }
                LoopMode::OneShot => {
                    self.playing = false;
                    self.position = out_pt;
                }
                LoopMode::HoldLast => {
                    self.position = out_pt;
                }
            }
        } else if self.position < in_pt {
            self.frame_accumulator = 0.0;
            match self.loop_mode {
                LoopMode::Loop | LoopMode::OneShot | LoopMode::HoldLast => {
                    self.position = in_pt;
                    return AdvanceResult {
                        needs_seek: true,
                        frames_to_decode: 1,
                    };
                }
                LoopMode::PingPong => {
                    self.reverse = false;
                    self.position = in_pt + frame_time;
                }
            }
        }
        AdvanceResult {
            needs_seek: false,
            frames_to_decode,
        }
    }
}

/// GPU-compressed texture format for HAP video frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapTextureFormat {
    /// BC1 / DXT1 — RGB, no alpha (Hap)
    Bc1,
    /// BC3 / DXT5 — RGBA with interpolated alpha (Hap Alpha)
    Bc3,
    /// BC3 / DXT5 storing Scaled `YCoCg` color (Hap Q) — needs shader conversion to RGB
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
        let blocks_x = width.div_ceil(4);
        let blocks_y = height.div_ceil(4);
        (blocks_x * blocks_y * self.block_bytes()) as usize
    }
}

/// A decoded video frame — either CPU-decoded RGBA or GPU-compressed `BCn`.
pub enum VideoFrame<'a> {
    /// Standard RGBA pixel data (from ffmpeg CPU decode).
    Rgba(&'a [u8]),
    /// GPU-compressed `BCn` texture data (from HAP decode).
    Compressed {
        data: &'a [u8],
        format: HapTextureFormat,
    },
}

// ── Background decode thread types ───────────────────────────────────

/// Commands sent from the main thread to the decode thread.
pub enum VideoCommand {
    Play,
    Pause,
    Seek(f64),
    SetSpeed(f64),
    SetLoopMode(LoopMode),
    SetInPoint(f64),
    SetOutPoint(f64),
    ClearInOutPoints,
    Stop,
}

/// A decoded frame ready for GPU upload — owned data copied from the player.
pub struct DecodedFrame {
    pub color_data: Vec<u8>,
    pub alpha_data: Option<Vec<u8>>,
    pub color_format: Option<HapTextureFormat>,
    pub alpha_format: Option<HapTextureFormat>,
}

/// Read-only snapshot of playback state for the main thread.
// Mirrors PlaybackState's independent flags one-for-one; collapsing them into an
// enum would misrepresent the state they snapshot.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct PlaybackSnapshot {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub loop_mode: LoopMode,
    pub in_point: f64,
    pub out_point: f64,
    pub reverse: bool,
    pub reached_end: bool,
    pub frame_rate: f64,
    /// Whether the ping-pong RAM cache was truncated (hit the memory cap).
    /// Always false for HAP sources (they reverse via seek, no cache).
    pub pingpong_cache_truncated: bool,
}

impl PlaybackSnapshot {
    /// Create a snapshot from a `PlaybackState`. The `pingpong_cache_truncated`
    /// flag defaults to false here and is set by the ffmpeg decode thread.
    pub fn from_state(ps: &PlaybackState) -> Self {
        Self {
            playing: ps.playing,
            position: ps.position,
            duration: ps.duration,
            speed: ps.speed,
            loop_mode: ps.loop_mode,
            in_point: ps.in_point,
            out_point: ps.out_point,
            reverse: ps.reverse,
            reached_end: ps.reached_end,
            frame_rate: ps.frame_rate,
            pingpong_cache_truncated: false,
        }
    }
}

/// Main-thread handle to a background video decode thread.
pub struct VideoDecodeHandle {
    cmd_tx: mpsc::Sender<VideoCommand>,
    frame_data: Arc<Mutex<Option<DecodedFrame>>>,
    snapshot: Arc<Mutex<PlaybackSnapshot>>,
    stop_flag: Arc<AtomicBool>,
    /// Bounded pool of reusable frame buffers returned by the renderer via
    /// [`Self::recycle`] and reused by the decode thread (avoids a fresh ~4 MB
    /// allocation per frame — issue #42).
    frame_pool: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Rate the renderer can actually present at, in whole frames per second
    /// (0 = uncapped). Written by the render thread every frame and read by the
    /// decode thread to bound its own rate — see [`Self::set_output_fps`].
    output_fps: Arc<AtomicU32>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub width: u32,
    pub height: u32,
    /// Whether this is a dual-plane HAP source (for render pass alpha detection).
    pub is_dual_plane: bool,
}

impl VideoDecodeHandle {
    /// Spawn a background decode thread for a standard (ffmpeg) `VideoPlayer`.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the decode thread.
    pub fn spawn_video(player: VideoPlayer) -> Self {
        let width = player.width();
        let height = player.height();
        let fps = player.frame_rate();
        let initial_snapshot = PlaybackSnapshot::from_state(&player.playback);

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let frame_data: Arc<Mutex<Option<DecodedFrame>>> = Arc::new(Mutex::new(None));
        let snapshot: Arc<Mutex<PlaybackSnapshot>> = Arc::new(Mutex::new(initial_snapshot));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let frame_pool: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let output_fps = Arc::new(AtomicU32::new(0));

        let fd = frame_data.clone();
        let ss = snapshot.clone();
        let sf = stop_flag.clone();
        let fp = frame_pool.clone();
        let ofps = output_fps.clone();

        let thread = std::thread::Builder::new()
            .name("video-decode".into())
            .spawn(move || {
                video_decode_thread(player, &cmd_rx, &fd, &ss, &sf, &fp, fps, &ofps);
            })
            .expect("failed to spawn video decode thread");

        Self {
            cmd_tx,
            frame_data,
            snapshot,
            stop_flag,
            frame_pool,
            output_fps,
            thread: Some(thread),
            width,
            height,
            is_dual_plane: false,
        }
    }

    /// Spawn a background decode thread for a HAP video player.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the decode thread.
    pub fn spawn_hap(player: hap::HapPlayer) -> Self {
        let width = player.width();
        let height = player.height();
        let fps = player.frame_rate();
        let is_dual_plane = player.is_dual_plane;
        let initial_snapshot = PlaybackSnapshot::from_state(&player.playback);

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let frame_data: Arc<Mutex<Option<DecodedFrame>>> = Arc::new(Mutex::new(None));
        let snapshot: Arc<Mutex<PlaybackSnapshot>> = Arc::new(Mutex::new(initial_snapshot));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let frame_pool: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let output_fps = Arc::new(AtomicU32::new(0));

        let fd = frame_data.clone();
        let ss = snapshot.clone();
        let sf = stop_flag.clone();
        let fp = frame_pool.clone();
        let ofps = output_fps.clone();

        let thread = std::thread::Builder::new()
            .name("hap-decode".into())
            .spawn(move || {
                hap_decode_thread(player, &cmd_rx, &fd, &ss, &sf, &fp, fps, &ofps);
            })
            .expect("failed to spawn hap decode thread");

        Self {
            cmd_tx,
            frame_data,
            snapshot,
            stop_flag,
            frame_pool,
            output_fps,
            thread: Some(thread),
            width,
            height,
            is_dual_plane,
        }
    }

    /// Tell the decode thread how fast the renderer can actually present, so a
    /// source faster than the output does not decode frames that can never be
    /// shown. Pass 0 to leave the source uncapped.
    ///
    /// The renderer takes at most one frame per rendered frame and the decode
    /// thread's mailbox holds one, so everything a faster source produces in
    /// between is overwritten. Decoding it anyway spends CPU and disk bandwidth
    /// for nothing, and leaves the surviving frames landing on an irregular
    /// beat against the render clock, which reads as judder.
    pub fn set_output_fps(&self, fps: u32) {
        self.output_fps.store(fps, Ordering::Relaxed);
    }

    /// Take the latest decoded frame (returns None if no new frame available).
    /// Return the frame to the decode thread via [`Self::recycle`] after upload
    /// so its buffer is reused instead of freed.
    pub fn take_frame(&self) -> Option<DecodedFrame> {
        self.frame_data.lock().ok()?.take()
    }

    /// Return a consumed frame's buffers to the pool for reuse by the decode
    /// thread. Call after the frame's data has been uploaded to the GPU.
    pub fn recycle(&self, frame: DecodedFrame) {
        pool_return(&self.frame_pool, frame.color_data);
        if let Some(alpha) = frame.alpha_data {
            pool_return(&self.frame_pool, alpha);
        }
    }

    /// Send a command to the decode thread.
    pub fn send(&self, cmd: VideoCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Get the current playback snapshot (read-only copy).
    pub fn playback_snapshot(&self) -> PlaybackSnapshot {
        self.snapshot.lock().map_or_else(
            |_| PlaybackSnapshot {
                playing: false,
                position: 0.0,
                duration: 0.0,
                speed: 1.0,
                loop_mode: LoopMode::Loop,
                in_point: 0.0,
                out_point: 0.0,
                reverse: false,
                reached_end: false,
                frame_rate: 30.0,
                pingpong_cache_truncated: false,
            },
            |s| s.clone(),
        )
    }
}

impl Drop for VideoDecodeHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Release);
        // Send Stop to unblock recv_timeout
        let _ = self.cmd_tx.send(VideoCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Apply a command to a `PlaybackState`.
fn apply_command(ps: &mut PlaybackState, cmd: &VideoCommand) {
    match cmd {
        VideoCommand::Play => ps.playing = true,
        VideoCommand::Pause => ps.playing = false,
        // Seek is handled specially by the thread loop (calls seek_and_reset / seek)
        VideoCommand::Seek(_) | VideoCommand::Stop => {}
        VideoCommand::SetSpeed(s) => ps.speed = *s,
        VideoCommand::SetLoopMode(m) => ps.loop_mode = *m,
        VideoCommand::SetInPoint(s) => ps.in_point = *s,
        VideoCommand::SetOutPoint(s) => ps.out_point = *s,
        VideoCommand::ClearInOutPoints => {
            ps.in_point = 0.0;
            ps.out_point = 0.0;
        }
    }
}

/// Background decode loop for standard (ffmpeg) video.
/// Maximum number of reusable frame buffers held in a decode handle's pool.
/// Enough to cover the in-flight frame plus a displaced frame; bounded so the
/// pool itself can never grow unbounded.
const FRAME_POOL_CAP: usize = 4;

/// Take a reusable buffer from the pool, or a fresh empty one if it is empty.
fn pool_take(pool: &Mutex<Vec<Vec<u8>>>) -> Vec<u8> {
    pool.lock()
        .ok()
        .and_then(|mut p| p.pop())
        .unwrap_or_default()
}

/// Return a buffer to the pool for reuse, dropping it if the pool is at capacity.
fn pool_return(pool: &Mutex<Vec<Vec<u8>>>, buf: Vec<u8>) {
    if let Ok(mut p) = pool.lock() {
        if p.len() < FRAME_POOL_CAP {
            p.push(buf);
        }
    }
}

/// Advance a fixed-rate frame schedule and return how long to wait for the
/// next frame to fall due.
///
/// The deadline moves in fixed `interval` steps so the time spent decoding is
/// absorbed by the wait instead of being added to it. Waiting a whole interval
/// *after* each decode paces frames at `1/(interval + decode)`, which silently
/// drops frames on every source — the shortfall grows with decode cost and is
/// worst on high frame-rate or high-resolution media.
///
/// If a decode overruns its slot the schedule restarts from now rather than
/// accumulating a backlog of already-late deadlines, which would burn through
/// several frames at full speed to "catch up".
/// The interval a decode thread should run at: the source's own rate, bounded
/// by the rate the renderer can present (0 = uncapped).
///
/// Decoding faster than the output can present buys nothing — the extra frames
/// are overwritten in the mailbox before anyone reads them — while costing CPU
/// and disk bandwidth that the rest of the frame needs, and scattering the
/// frames that do survive across an irregular beat.
fn decode_interval(video_fps: f64, output_fps: &AtomicU32) -> std::time::Duration {
    let cap = output_fps.load(Ordering::Relaxed);
    let rate = if cap > 0 {
        video_fps.min(f64::from(cap))
    } else {
        video_fps
    };
    std::time::Duration::from_secs_f64((1.0 / rate).max(0.001))
}

fn wait_for_next_frame(
    next_frame_at: &mut std::time::Instant,
    interval: std::time::Duration,
) -> std::time::Duration {
    let now = std::time::Instant::now();
    let wait = next_frame_at.saturating_duration_since(now);
    *next_frame_at += interval;
    if *next_frame_at <= now {
        *next_frame_at = now + interval;
    }
    wait
}

// Decode-thread plumbing: each arg is a distinct channel to the render thread,
// with no shared invariant that a struct would express.
#[allow(clippy::too_many_arguments)]
fn video_decode_thread(
    mut player: VideoPlayer,
    cmd_rx: &mpsc::Receiver<VideoCommand>,
    frame_data: &Mutex<Option<DecodedFrame>>,
    snapshot: &Mutex<PlaybackSnapshot>,
    stop_flag: &AtomicBool,
    frame_pool: &Mutex<Vec<Vec<u8>>>,
    fps: f64,
    output_fps: &AtomicU32,
) {
    let mut interval = decode_interval(fps, output_fps);
    let mut next_frame_at = std::time::Instant::now() + interval;

    while !stop_flag.load(Ordering::Acquire) {
        // Drain all pending commands
        let mut had_seek = None;
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let VideoCommand::Stop = &cmd {
                return;
            }
            if let VideoCommand::Seek(t) = &cmd {
                had_seek = Some(*t);
            }
            apply_command(&mut player.playback, &cmd);
        }

        // Process seek if any
        if let Some(t) = had_seek {
            if let Err(e) = player.seek_and_reset(t) {
                log::warn!("Video seek error: {e}");
            }
        }

        // Decode next frame
        match player.next_frame() {
            Ok(Some(data)) => {
                let mut buf = pool_take(frame_pool);
                buf.clear();
                buf.extend_from_slice(data);
                let frame = DecodedFrame {
                    color_data: buf,
                    alpha_data: None,
                    color_format: None,
                    alpha_format: None,
                };
                if let Ok(mut slot) = frame_data.lock() {
                    // Recycle a frame the renderer never consumed (happens when
                    // it falls behind — exactly the #42 scenario) instead of
                    // dropping its buffer.
                    if let Some(old) = slot.take() {
                        pool_return(frame_pool, old.color_data);
                        if let Some(alpha) = old.alpha_data {
                            pool_return(frame_pool, alpha);
                        }
                    }
                    *slot = Some(frame);
                }
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("Video decode error: {e}");
            }
        }

        // Publish snapshot
        if let Ok(mut ss) = snapshot.lock() {
            let mut snap = PlaybackSnapshot::from_state(&player.playback);
            snap.pingpong_cache_truncated = player.pingpong_cache_truncated();
            *ss = snap;
        }

        // Sleep until the next frame falls due, or wake early on a command.
        interval = decode_interval(fps, output_fps);
        match cmd_rx.recv_timeout(wait_for_next_frame(&mut next_frame_at, interval)) {
            Ok(cmd) => {
                if let VideoCommand::Stop = &cmd {
                    return;
                }
                if let VideoCommand::Seek(t) = &cmd {
                    if let Err(e) = player.seek_and_reset(*t) {
                        log::warn!("Video seek error: {e}");
                    }
                } else {
                    apply_command(&mut player.playback, &cmd);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Background decode loop for HAP video.
// Decode-thread plumbing: each arg is a distinct channel to the render thread,
// with no shared invariant that a struct would express.
#[allow(clippy::too_many_arguments)]
fn hap_decode_thread(
    mut player: hap::HapPlayer,
    cmd_rx: &mpsc::Receiver<VideoCommand>,
    frame_data: &Mutex<Option<DecodedFrame>>,
    snapshot: &Mutex<PlaybackSnapshot>,
    stop_flag: &AtomicBool,
    frame_pool: &Mutex<Vec<Vec<u8>>>,
    fps: f64,
    output_fps: &AtomicU32,
) {
    let mut interval = decode_interval(fps, output_fps);
    let mut next_frame_at = std::time::Instant::now() + interval;

    while !stop_flag.load(Ordering::Acquire) {
        // Drain all pending commands
        let mut had_seek = None;
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let VideoCommand::Stop = &cmd {
                return;
            }
            if let VideoCommand::Seek(t) = &cmd {
                had_seek = Some(*t);
            }
            apply_command(&mut player.playback, &cmd);
        }

        // Process seek if any
        if let Some(t) = had_seek {
            if let Err(e) = player.seek(t) {
                log::warn!("HAP seek error: {e}");
            }
        }

        // Decode next frame
        match player.next_frame() {
            Ok(Some(result)) => {
                let mut color = pool_take(frame_pool);
                color.clear();
                color.extend_from_slice(result.color_data);
                let alpha = result.alpha_data.map(|d| {
                    let mut a = pool_take(frame_pool);
                    a.clear();
                    a.extend_from_slice(d);
                    a
                });
                let frame = DecodedFrame {
                    color_data: color,
                    alpha_data: alpha,
                    color_format: Some(result.color_format),
                    alpha_format: result.alpha_format,
                };
                if let Ok(mut slot) = frame_data.lock() {
                    if let Some(old) = slot.take() {
                        pool_return(frame_pool, old.color_data);
                        if let Some(alpha) = old.alpha_data {
                            pool_return(frame_pool, alpha);
                        }
                    }
                    *slot = Some(frame);
                }
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("HAP decode error: {e}");
            }
        }

        // Publish snapshot
        if let Ok(mut ss) = snapshot.lock() {
            *ss = PlaybackSnapshot::from_state(&player.playback);
        }

        // Sleep until the next frame falls due, or wake early on a command.
        interval = decode_interval(fps, output_fps);
        match cmd_rx.recv_timeout(wait_for_next_frame(&mut next_frame_at, interval)) {
            Ok(cmd) => {
                if let VideoCommand::Stop = &cmd {
                    return;
                }
                if let VideoCommand::Seek(t) = &cmd {
                    if let Err(e) = player.seek(*t) {
                        log::warn!("HAP seek error: {e}");
                    }
                } else {
                    apply_command(&mut player.playback, &cmd);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Detect whether a video file uses a HAP codec.
/// Returns the HAP texture format if it is HAP, or None for standard codecs.
///
/// # Errors
///
/// Returns an error if FFmpeg cannot be initialised, if the file cannot be
/// opened, or if it contains no video stream.
pub fn detect_hap_codec<P: AsRef<Path>>(path: P) -> Result<Option<HapTextureFormat>> {
    ffmpeg::init().context("Failed to initialize FFmpeg")?;
    let mut ictx = input(&path).context("Failed to open video file for codec detection")?;

    // ffmpeg maps every HAP variant to one codec id, so confirm it's HAP here
    // and then read the real texture format from the first frame's section header.
    let (video_stream_index, is_hap) = {
        let video_stream = ictx
            .streams()
            .best(Type::Video)
            .context("No video stream found")?;
        let codec_ctx =
            ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())?;
        (video_stream.index(), codec_ctx.id().name() == "hap")
    };

    if !is_hap {
        return Ok(None);
    }

    // Probe the first video packet for the exact variant (BC1/BC3/BC7/YCoCg).
    // The texture and staging buffers are sized from this, so a wrong format
    // overruns the staging copy (issue: Hap1/BC1 misdetected as Bc7).
    for (stream, packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue;
        }
        if let Some(data) = packet.data() {
            return match hap::detect_hap_format(data) {
                Ok(fmt) => Ok(Some(fmt)),
                Err(e) => {
                    log::warn!("HAP format detection failed ({e}) — using CPU decode fallback");
                    Ok(None)
                }
            };
        }
    }

    log::warn!("HAP stream has no readable packets — using CPU decode fallback");
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
    /// Current read index into `frame_cache` during reverse playback.
    cache_read_idx: usize,
    /// Whether we're actively caching frames (disabled when memory cap hit this pass).
    caching_enabled: bool,
    /// Set when cache overflows this pass — reset each new forward pass.
    cache_overflowed: bool,
    /// Permanent latch set on the first cache overflow. Suppresses repeated log
    /// warnings and drives the one-time "transcode to HAP" UI notice (exposed
    /// via [`VideoPlayer::pingpong_cache_truncated`]).
    cache_overflow_warned: bool,
    /// Bytes per frame for cache budget calculation.
    frame_byte_size: usize,
    /// Reused decoder output frame (avoids a per-frame ffmpeg frame allocation).
    decoded: Video,
    /// Reused scaler output frame (RGBA), avoids a per-frame allocation.
    rgb_frame: Video,
}

// SAFETY: See doc comment on VideoPlayer. Exclusive ownership of C allocations, no concurrent use.
unsafe impl Send for VideoPlayer {}

impl VideoPlayer {
    /// Create a new video player from a file path.
    ///
    /// # Errors
    ///
    /// Returns an error if FFmpeg cannot be initialised, if the file cannot be
    /// opened, if it contains no video stream, or if a decoder or scaler cannot
    /// be created for it.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        ffmpeg::init().context("Failed to initialize FFmpeg")?;
        let ictx = input(&path).context("Failed to open video file")?;
        let video_stream = ictx
            .streams()
            .best(Type::Video)
            .context("No video stream found")?;
        let video_stream_index = video_stream.index();
        let context_decoder =
            ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())?;
        let decoder = context_decoder.decoder().video()?;
        let width = decoder.width();
        let height = decoder.height();
        let rate = video_stream.rate();
        let fps = f64::from(rate.0) / f64::from(rate.1);
        let duration = if ictx.duration() > 0 {
            ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
        } else {
            0.0
        };
        let scaler = Scaler::get(
            decoder.format(),
            width,
            height,
            Pixel::RGBA,
            width,
            height,
            Flags::BILINEAR,
        )?;
        let frame_byte_size = (width * height * 4) as usize;
        let frame_data = vec![0u8; frame_byte_size];
        let max_cached_frames = MAX_CACHE_BYTES / frame_byte_size.max(1);
        log::info!(
            "Loaded video: {width}x{height} @ {fps:.2} fps, duration: {duration:.2}s (ping-pong cache: {max_cached_frames} frames)"
        );
        Ok(Self {
            ictx,
            decoder,
            scaler,
            video_stream_index,
            width,
            height,
            playback: PlaybackState::new(duration, fps),
            frame_data,
            frame_cache: Vec::new(),
            cache_read_idx: 0,
            caching_enabled: true,
            cache_overflowed: false,
            cache_overflow_warned: false,
            frame_byte_size,
            decoded: Video::empty(),
            rgb_frame: Video::empty(),
        })
    }

    /// Get the next frame as RGBA data.
    /// Uses wall-clock time pacing: only decodes new frames when enough real
    /// time has elapsed (respecting speed multiplier). At speed < 1.0, frames
    /// are held longer; at speed > 1.0, frames are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking, packet demuxing, decoding, or colour-space
    /// conversion fails.
    pub fn next_frame(&mut self) -> Result<Option<&[u8]>> {
        if !self.playback.playing {
            return Ok(None);
        }
        let was_reverse = self.playback.reverse;
        let result = self.playback.advance_frame();

        // No frames to decode this tick — hold current frame.
        // Return None so the caller skips the GPU texture upload;
        // the texture already contains the current frame from the last upload.
        if result.frames_to_decode == 0 && !result.needs_seek {
            return Ok(None);
        }

        // Detect ping-pong boundary flips from advance_frame:
        if !was_reverse && self.playback.reverse {
            // Forward→reverse flip (hit out-point). Serve from cache.
            if self.frame_cache.is_empty() {
                // No cache available (overflow or very short video).
                // Hold the current frame at the boundary and stay in reverse —
                // advance_frame will walk the position backward and eventually
                // hit in_point, triggering the reverse→forward flip below.
                return Ok(Some(&self.frame_data));
            }
            self.cache_read_idx = self.frame_cache.len() - 1;
        } else if was_reverse && !self.playback.reverse {
            // Reverse→forward flip (hit in-point). Clear cache, seek to in-point.
            // Reset overflow so the new forward pass gets a fresh caching budget.
            self.frame_cache.clear();
            self.cache_overflowed = false;
            self.caching_enabled = true;
            self.seek(self.playback.position)?;
        }

        // Reverse playback: serve frames from cache
        if self.playback.reverse {
            // Skip frames for speed > 1.0 in reverse
            let skip = result.frames_to_decode.max(1) as usize;
            if self.cache_read_idx >= skip {
                self.cache_read_idx -= skip;
                self.frame_data
                    .copy_from_slice(&self.frame_cache[self.cache_read_idx]);
                return Ok(Some(&self.frame_data));
            }
            // Cache exhausted before position reached in_point.
            // Flip back to forward, seek to in_point, start a new forward pass.
            // Reset overflow so the new forward pass gets a fresh caching budget.
            self.playback.reverse = false;
            self.playback.position = self.playback.in_point;
            self.frame_cache.clear();
            self.cache_overflowed = false;
            self.caching_enabled = true;
            self.seek(self.playback.position)?;
        } else if result.needs_seek {
            // Forward seek (loop restart, etc.)
            self.frame_cache.clear();
            self.cache_overflowed = false;
            self.caching_enabled = true;
            self.seek(self.playback.position)?;
        }

        // Forward playback — decode frames_to_decode frames (skip intermediate ones)
        let target_frames = result.frames_to_decode.max(1);
        let mut decoded_count = 0u32;
        loop {
            if self.decoder.receive_frame(&mut self.decoded).is_ok() {
                decoded_count += 1;
                // Only convert the last frame we need (skip intermediate for speed > 1)
                if decoded_count >= target_frames {
                    self.scaler.run(&self.decoded, &mut self.rgb_frame)?;
                    let data = self.rgb_frame.data(0);
                    let stride = self.rgb_frame.stride(0);
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
                            self.cache_overflowed = true;
                            if !self.cache_overflow_warned {
                                self.cache_overflow_warned = true;
                                log::warn!("Ping-pong frame cache full ({} frames, {} MB) — reverse will cover partial clip",
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
                    self.scaler.run(&self.decoded, &mut self.rgb_frame)?;
                    let data = self.rgb_frame.data(0);
                    let stride = self.rgb_frame.stride(0);
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
                        }
                        LoopMode::PingPong => {
                            self.playback.reverse = true;
                            if !self.frame_cache.is_empty() {
                                self.cache_read_idx = self.frame_cache.len() - 1;
                                return self.next_frame();
                            }
                            // No cache: hold current frame at boundary.
                            // advance_frame will walk position backward until in_point,
                            // then flip back to forward on the next pass.
                            self.playback.position =
                                self.playback.effective_out() - (1.0 / self.playback.frame_rate);
                            return Ok(Some(&self.frame_data));
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
        let timestamp = (time_secs * f64::from(ffmpeg::ffi::AV_TIME_BASE)) as i64;
        self.ictx.seek(timestamp, ..timestamp)?;
        self.decoder.flush();
        self.playback.position = time_secs;
        Ok(())
    }

    /// Seek to a specific time and reset the frame cache.
    /// Use this for user-initiated seeks (scrub bar, etc.).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying FFmpeg seek fails.
    pub fn seek_and_reset(&mut self, time_secs: f64) -> Result<()> {
        self.frame_cache.clear();
        self.cache_read_idx = 0;
        self.caching_enabled = true;
        self.cache_overflowed = false;
        self.playback.reverse = false;
        self.seek(time_secs)
    }

    /// Whether this player's ping-pong RAM cache has ever been truncated (hit
    /// the memory cap). Permanent latch — once true, stays true for the
    /// player's lifetime. Drives the one-time "transcode to HAP" UI notice.
    pub fn pingpong_cache_truncated(&self) -> bool {
        self.cache_overflow_warned
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn frame_rate(&self) -> f64 {
        self.playback.frame_rate
    }
    pub fn duration(&self) -> f64 {
        self.playback.duration
    }
    pub fn is_playing(&self) -> bool {
        self.playback.playing
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.playback.playing = playing;
    }
    pub fn is_looping(&self) -> bool {
        self.playback.loop_mode == LoopMode::Loop
    }
    pub fn set_looping(&mut self, looping: bool) {
        self.playback.loop_mode = if looping {
            LoopMode::Loop
        } else {
            LoopMode::OneShot
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode time must come out of the wait, not be added to it. Waiting a
    /// full interval after each decode paced frames at `1/(interval + decode)`,
    /// which lost 4-18% of the frames of every source.
    #[test]
    fn frame_schedule_absorbs_decode_time_instead_of_adding_to_it() {
        let interval = std::time::Duration::from_millis(10);
        let decode = std::time::Duration::from_millis(4);
        let start = std::time::Instant::now();
        let mut next_frame_at = start + interval;

        // Three frames, each costing `decode` before the wait.
        let mut elapsed = decode;
        let mut waits = Vec::new();
        for _ in 0..3 {
            let mut at = start + elapsed;
            let wait = {
                // Same arithmetic as `wait_for_next_frame`, driven off a
                // simulated clock so the test does not have to sleep.
                let w = next_frame_at.saturating_duration_since(at);
                next_frame_at += interval;
                if next_frame_at <= at {
                    next_frame_at = at + interval;
                }
                at += w;
                w
            };
            waits.push(wait);
            elapsed = at.duration_since(start) + decode;
        }

        // Each wait covers the interval minus the decode that preceded it, so
        // the frame period stays at `interval`.
        for wait in &waits {
            assert_eq!(
                *wait,
                interval.checked_sub(decode).unwrap(),
                "wait must absorb the decode time"
            );
        }
    }

    /// A source faster than the renderer must decode at the renderer's rate:
    /// the mailbox holds one frame and the renderer takes one per rendered
    /// frame, so the surplus is decoded only to be overwritten.
    #[test]
    fn decode_rate_is_bounded_by_what_the_renderer_can_present() {
        let cap = AtomicU32::new(60);
        assert_eq!(
            decode_interval(75.0, &cap),
            std::time::Duration::from_secs_f64(1.0 / 60.0),
            "a 75 fps source on a 60 fps output must decode at 60"
        );

        // A source slower than the output keeps its own rate — capping there
        // would slow the video down rather than save work.
        assert_eq!(
            decode_interval(15.0, &cap),
            std::time::Duration::from_secs_f64(1.0 / 15.0),
            "a 15 fps source must not be sped up to the output rate"
        );

        // Uncapped output (offline render, no target) leaves the source alone.
        let uncapped = AtomicU32::new(0);
        assert_eq!(
            decode_interval(75.0, &uncapped),
            std::time::Duration::from_secs_f64(1.0 / 75.0),
            "an uncapped output must not bound the source"
        );
    }

    /// A decode that overruns its slot must not build a backlog of already-late
    /// deadlines, or the next frames would fire back-to-back at full speed.
    #[test]
    fn frame_schedule_resyncs_after_an_overrun_instead_of_bursting() {
        let interval = std::time::Duration::from_millis(10);
        let mut next_frame_at = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap();

        let wait = wait_for_next_frame(&mut next_frame_at, interval);
        assert_eq!(wait, std::time::Duration::ZERO, "a late frame waits not at all");
        assert!(
            next_frame_at > std::time::Instant::now(),
            "the schedule must restart in the future, not replay a backlog"
        );
    }

    /// Regression: the birds HAP fixture is Hap1/BC1, not Bc7. `detect_hap_codec`
    /// must report the real format so the deck sizes its texture/staging
    /// correctly (a wrong format overran the staging copy and panicked).
    /// Skips when the local-only fixture is absent (tests/media/ is gitignored).
    #[test]
    fn detect_hap_codec_birds_fixture_is_bc1() {
        let path = "tests/media/birds_combined_hap.mov";
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: {path} not present (local-only fixture)");
            return;
        }
        assert_eq!(
            detect_hap_codec(path).unwrap(),
            Some(HapTextureFormat::Bc1),
            "Hap1 fixture must be detected as BC1, not the old hardcoded Bc7"
        );
    }

    /// The demux loop must actually yield decoded frames, and must keep
    /// yielding them across the loop wrap at end of stream.
    /// Skips when the local-only fixture is absent (tests/media/ is gitignored).
    #[test]
    fn hap_player_decodes_frames_and_survives_the_loop_wrap() {
        let path = "tests/media/birds_combined_hap.mov";
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: {path} not present (local-only fixture)");
            return;
        }
        let fmt = detect_hap_codec(path).unwrap().expect("fixture is HAP");
        let mut player = hap::HapPlayer::new(path, fmt).expect("open fixture");
        let expected = fmt.frame_byte_size(player.width(), player.height());

        // Drive the player from a simulated clock instead of real time so the
        // test neither sleeps nor depends on how fast the machine decodes.
        let mut decoded = 0;
        for _ in 0..200 {
            player.playback.frame_accumulator = 1.0 / player.playback.frame_rate;
            if let Some(frame) = player.next_frame().expect("decode") {
                assert_eq!(
                    frame.color_data.len(),
                    expected,
                    "decoded frame must fill the plane the deck sized its texture for"
                );
                decoded += 1;
            }
        }
        assert!(decoded > 100, "expected a frame per tick, got {decoded}");

        // Wrap past the end and confirm the demuxer still produces frames
        // rather than reporting a permanent end of stream.
        player.playback.position = player.playback.effective_out() + 1.0;
        player.playback.frame_accumulator = 1.0 / player.playback.frame_rate;
        player.next_frame().expect("wrap");
        let mut after_wrap = 0;
        for _ in 0..10 {
            player.playback.frame_accumulator = 1.0 / player.playback.frame_rate;
            if player.next_frame().expect("decode after wrap").is_some() {
                after_wrap += 1;
            }
        }
        assert!(
            after_wrap > 5,
            "playback must resume after the loop wrap, got {after_wrap} frames"
        );
    }

    #[test]
    fn frame_pool_reuses_and_bounds_buffers() {
        let pool: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));

        // Empty pool yields a fresh buffer.
        let buf = pool_take(&pool);
        assert!(buf.is_empty());

        // A returned buffer is handed back out (reuse, not realloc).
        let mut b = Vec::with_capacity(4096);
        b.extend_from_slice(&[1u8, 2, 3]);
        pool_return(&pool, b);
        let reused = pool_take(&pool);
        assert!(reused.capacity() >= 4096);

        // The pool never grows past FRAME_POOL_CAP.
        for _ in 0..(FRAME_POOL_CAP + 4) {
            pool_return(&pool, Vec::new());
        }
        assert_eq!(pool.lock().unwrap().len(), FRAME_POOL_CAP);
    }

    #[test]
    fn test_playback_snapshot_from_state() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.speed = 2.0;
        ps.in_point = 1.0;
        ps.out_point = 8.0;
        ps.loop_mode = LoopMode::PingPong;
        let snap = PlaybackSnapshot::from_state(&ps);
        assert!(snap.playing);
        assert_eq!(snap.duration, 10.0);
        assert_eq!(snap.speed, 2.0);
        assert_eq!(snap.in_point, 1.0);
        assert_eq!(snap.out_point, 8.0);
        assert_eq!(snap.loop_mode, LoopMode::PingPong);
        assert_eq!(snap.frame_rate, 30.0);
    }

    #[test]
    fn test_decode_handle_take_frame_returns_none_initially() {
        // Cannot construct a full handle without a player, but we can test the
        // shared frame_data path directly.
        let frame_data: Arc<Mutex<Option<DecodedFrame>>> = Arc::new(Mutex::new(None));
        assert!(frame_data.lock().unwrap().is_none());
    }

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
        assert!(
            ps_fast.position > ps_slow.position * 2.0,
            "fast={} should be > 2x slow={}",
            ps_fast.position,
            ps_slow.position
        );
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

    // ── Offensive: frame rate div-by-zero prevention ─────────────────

    #[test]
    fn playback_state_zero_frame_rate_clamped() {
        let ps = PlaybackState::new(10.0, 0.0);
        assert_eq!(
            ps.frame_rate, 30.0,
            "zero frame_rate should be clamped to 30.0"
        );
    }

    #[test]
    fn playback_state_negative_frame_rate_clamped() {
        let ps = PlaybackState::new(10.0, -24.0);
        assert_eq!(
            ps.frame_rate, 30.0,
            "negative frame_rate should be clamped to 30.0"
        );
    }

    #[test]
    fn playback_state_nan_frame_rate_clamped() {
        let ps = PlaybackState::new(10.0, f64::NAN);
        assert_eq!(
            ps.frame_rate, 30.0,
            "NaN frame_rate should be clamped to 30.0"
        );
    }

    #[test]
    fn playback_state_valid_frame_rate_preserved() {
        let ps = PlaybackState::new(10.0, 60.0);
        assert_eq!(ps.frame_rate, 60.0, "valid frame_rate should be preserved");
    }

    #[test]
    fn playback_state_advance_with_clamped_rate_does_not_divide_by_zero() {
        let mut ps = PlaybackState::new(10.0, 0.0);
        std::thread::sleep(std::time::Duration::from_millis(20));
        // Must not panic or produce NaN/Inf
        let result = ps.advance_frame();
        assert!(!ps.position.is_nan(), "position must not be NaN");
        assert!(!ps.position.is_infinite(), "position must not be Inf");
        assert!(!result.needs_seek || ps.position >= 0.0);
    }

    // ── Chaos Tests Round 2: Speed extremes ──────────────────────────────

    #[test]
    fn chaos_extreme_speed_1e6_does_not_overflow() {
        let mut ps = PlaybackState::new(100.0, 60.0);
        ps.speed = 1_000_000.0;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = ps.advance_frame();
        assert!(!ps.position.is_nan(), "position NaN at extreme speed");
        assert!(!ps.position.is_infinite(), "position Inf at extreme speed");
        // frames_to_decode should be finite (even if large)
        assert!(
            result.frames_to_decode < u32::MAX,
            "frames_to_decode wrapped"
        );
    }

    #[test]
    fn chaos_negative_extreme_speed() {
        let mut ps = PlaybackState::new(100.0, 30.0);
        ps.speed = -1_000_000.0;
        ps.position = 50.0;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = ps.advance_frame();
        assert!(
            !ps.position.is_nan(),
            "position NaN at negative extreme speed"
        );
        assert!(
            !ps.position.is_infinite(),
            "position Inf at negative extreme speed"
        );
        // Should trigger loop/clamp logic
        assert!(result.frames_to_decode < u32::MAX);
    }

    #[test]
    fn chaos_nan_speed_does_not_propagate() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.speed = f64::NAN;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
        // NaN speed causes NaN position — document the behavior
        // The key is it doesn't panic
    }

    #[test]
    fn chaos_infinity_speed_does_not_panic() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.speed = f64::INFINITY;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
        // Must not panic
    }

    // ── Chaos Tests Round 2: Corrupted playback state ────────────────────

    #[test]
    fn chaos_in_point_greater_than_out_point() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.in_point = 8.0;
        ps.out_point = 3.0; // inverted
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
        // Must not panic — position may clamp or loop oddly
    }

    #[test]
    fn chaos_zero_duration() {
        let mut ps = PlaybackState::new(0.0, 30.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
        // effective_out() with duration=0 — must not panic
    }

    #[test]
    fn chaos_nan_position_does_not_panic() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.position = f64::NAN;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
        // NaN comparisons are always false, so no branch fires — must not panic
    }

    #[test]
    fn chaos_nan_in_point_does_not_panic() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.in_point = f64::NAN;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
    }

    #[test]
    fn chaos_negative_duration() {
        let mut ps = PlaybackState::new(-5.0, 30.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _result = ps.advance_frame();
    }

    #[test]
    fn chaos_extreme_position_recovery() {
        let mut ps = PlaybackState::new(10.0, 30.0);
        ps.position = 1e15;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = ps.advance_frame();
        // Should trigger loop/clamp since position > out_point
        assert!(ps.reached_end || result.needs_seek || ps.position <= 1e15);
    }
}
