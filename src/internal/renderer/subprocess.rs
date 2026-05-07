//! FfmpegSubprocess — shared ffmpeg lifecycle for recording and SRT streaming.
//!
//! Spawns an ffmpeg process with a background writer thread that feeds frames
//! via a bounded channel. The render thread never blocks on pipe writes — if
//! ffmpeg can't keep up (e.g. SRT listener waiting for client), frames are dropped.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::renderer::context::RecordingCodec;

/// Shared ffmpeg subprocess for recording and SRT streaming.
///
/// Frames are sent to a background writer thread via a bounded channel.
/// This prevents the render thread from blocking when ffmpeg's stdin pipe is full
/// (e.g. SRT listener waiting for a client connection).
pub struct FfmpegSubprocess {
    child: Child,
    /// Channel sender for frame data → writer thread
    frame_tx: Option<mpsc::SyncSender<Vec<u8>>>,
    /// Writer thread handle
    writer_thread: Option<std::thread::JoinHandle<()>>,
    /// Shared frame counter (updated by writer thread)
    frames_written: Arc<AtomicU64>,
    /// Writer thread error flag (set when write fails)
    write_failed: Arc<AtomicBool>,
    /// Human-readable label (path or URL)
    label: String,
    /// Start time (for duration display)
    start_time: std::time::Instant,
    /// Whether stop() has already been called (prevent double-wait)
    stopped: bool,
}

/// Bounded channel capacity — 2 frames of buffer allows the writer thread
/// to stay one frame ahead without accumulating unbounded latency.
const FRAME_CHANNEL_CAPACITY: usize = 2;

impl FfmpegSubprocess {
    /// Start the background writer thread that drains the channel into ffmpeg stdin.
    fn start_writer_thread(
        mut stdin: std::process::ChildStdin,
        rx: mpsc::Receiver<Vec<u8>>,
        frames_written: Arc<AtomicU64>,
        write_failed: Arc<AtomicBool>,
        label: String,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name(format!("ffmpeg-writer-{}", label))
            .spawn(move || {
                for frame in rx {
                    if let Err(e) = stdin.write_all(&frame) {
                        log::error!("ffmpeg write error for '{}': {}", label, e);
                        write_failed.store(true, Ordering::SeqCst);
                        return;
                    }
                    frames_written.fetch_add(1, Ordering::Relaxed);
                }
                // Channel closed — normal shutdown, flush stdin
                let _ = stdin.flush();
            })
            .expect("failed to spawn ffmpeg writer thread")
    }

    /// Spawn an ffmpeg recording subprocess.
    pub fn spawn_recording(
        path: &str,
        codec: &RecordingCodec,
        width: u32,
        height: u32,
        fps: u32,
    ) -> anyhow::Result<Self> {
        let codec_args: Vec<&str> = match codec {
            RecordingCodec::H264 => vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "18"],
            RecordingCodec::ProRes => vec!["-c:v", "prores_ks", "-profile:v", "2"],
            RecordingCodec::HapQ => vec!["-c:v", "hap", "-format", "hap_q"],
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(&codec_args)
            .args(["-pix_fmt", "yuv420p"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {}. Is ffmpeg installed?", e))?;

        log::info!("Recording started: {} ({}, {}x{} @ {}fps)", path, codec, width, height, fps);

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin, rx, frames_written.clone(), write_failed.clone(), path.to_string(),
        );

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            label: path.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
        })
    }

    /// Spawn an ffmpeg SRT streaming subprocess in listener (server) mode.
    /// Starts an SRT server on the specified port and broadcasts frames to connected clients.
    pub fn spawn_srt(
        url: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> anyhow::Result<Self> {
        // Ensure listener mode so ffmpeg acts as an SRT server
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{}&mode=listener", url)
        } else {
            format!("{}?mode=listener", url)
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(["-c:v", "libx264"])
            .args(["-preset", "ultrafast"])
            .args(["-tune", "zerolatency"])
            .args(["-pix_fmt", "yuv420p"])
            .args(["-f", "mpegts"])
            .arg(&srt_url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg for SRT: {}. Is ffmpeg installed?", e))?;

        log::info!("SRT server started: {} ({}x{} @ {}fps)", srt_url, width, height, fps);

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin, rx, frames_written.clone(), write_failed.clone(), url.to_string(),
        );

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            label: url.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
        })
    }

    /// Feed a frame of RGBA data to the subprocess.
    /// Never blocks — drops the frame if the writer thread can't keep up.
    /// Returns false if the subprocess has failed (write error or process exited).
    pub fn feed_frame(&mut self, rgba: &[u8]) -> bool {
        // Check if writer thread reported an error
        if self.write_failed.load(Ordering::SeqCst) {
            self.drain_stderr();
            return false;
        }
        // Check if ffmpeg already exited (non-blocking)
        if let Some(status) = self.child.try_wait().ok().flatten() {
            if !status.success() {
                self.drain_stderr();
                log::error!("ffmpeg exited with status {} for '{}' before frame could be written", status, self.label);
            }
            return false;
        }
        if let Some(ref tx) = self.frame_tx {
            match tx.try_send(rgba.to_vec()) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    // Frame dropped — ffmpeg can't keep up, but that's OK
                    true
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    // Writer thread exited (write error)
                    self.drain_stderr();
                    false
                }
            }
        } else {
            false
        }
    }

    /// Read and log any stderr output from ffmpeg.
    /// Each line is classified individually: lines containing error indicators
    /// are logged at ERROR, everything else (version info, codec config) at DEBUG.
    fn drain_stderr(&mut self) {
        use std::io::Read;
        if let Some(ref mut stderr) = self.child.stderr {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            if !buf.is_empty() {
                for line in buf.lines().take(30) {
                    let lower = line.to_ascii_lowercase();
                    if lower.contains("error") || lower.contains("failed")
                        || lower.contains("invalid") || lower.contains("fatal")
                    {
                        log::error!("ffmpeg [{}]: {}", self.label, line);
                    } else {
                        log::debug!("ffmpeg [{}]: {}", self.label, line);
                    }
                }
            }
        }
    }

    /// Stop the subprocess. Closes the frame channel, joins the writer thread,
    /// then kills ffmpeg if it doesn't exit promptly.
    /// Idempotent — safe to call multiple times.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;

        // Drop the sender to close the channel — writer thread will finish
        drop(self.frame_tx.take());

        // Join the writer thread (it will exit once channel is closed or on write error)
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }

        // Close stdin to signal ffmpeg to finish (writer thread already dropped it, but be safe)
        drop(self.child.stdin.take());
        let duration = self.start_time.elapsed();

        // Give ffmpeg a brief moment to exit gracefully
        let exited = match self.child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(200));
                matches!(self.child.try_wait(), Ok(Some(_)))
            }
            Err(_) => false,
        };

        // If still running (e.g. SRT listener blocking), kill it
        if !exited {
            let _ = self.child.kill();
        }

        let frames = self.frames_written.load(Ordering::Relaxed);
        match self.child.wait() {
            Ok(status) => {
                self.drain_stderr();
                if status.success() || !exited {
                    log::info!("ffmpeg finished: {} ({} frames, {:.1}s)",
                        self.label, frames, duration.as_secs_f32());
                } else {
                    log::error!("ffmpeg exited with status {} for '{}'", status, self.label);
                }
            }
            Err(e) => {
                log::error!("Failed to wait for ffmpeg '{}': {}", self.label, e);
            }
        }
    }

    /// Duration since the subprocess was started.
    pub fn duration(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Number of frames written so far.
    pub fn frames_written(&self) -> u64 {
        self.frames_written.load(Ordering::Relaxed)
    }

    /// The label (path or URL) for this subprocess.
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl Drop for FfmpegSubprocess {
    fn drop(&mut self) {
        self.stop();
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Check if ffmpeg is available on this system.
    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg").arg("-version").output().is_ok()
    }

    // ── SRT URL mode injection tests (pure logic) ──────────────────

    #[test]
    fn spawn_srt_url_adds_listener_mode() {
        // Verify the URL mode injection logic without spawning
        let url = "srt://127.0.0.1:9001";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{}&mode=listener", url)
        } else {
            format!("{}?mode=listener", url)
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?mode=listener");
    }

    #[test]
    fn spawn_srt_url_preserves_existing_mode() {
        let url = "srt://127.0.0.1:9001?mode=caller";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{}&mode=listener", url)
        } else {
            format!("{}?mode=listener", url)
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?mode=caller");
    }

    #[test]
    fn spawn_srt_url_appends_to_existing_params() {
        let url = "srt://127.0.0.1:9001?latency=0";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{}&mode=listener", url)
        } else {
            format!("{}?mode=listener", url)
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?latency=0&mode=listener");
    }

    // ── Recording codec display ────────────────────────────────────

    #[test]
    fn recording_codec_display() {
        assert_eq!(format!("{}", RecordingCodec::H264), "H.264 (fast)");
        assert_eq!(format!("{}", RecordingCodec::ProRes), "ProRes 422");
        assert_eq!(format!("{}", RecordingCodec::HapQ), "HAP Q");
    }

    // ── Subprocess lifecycle (requires ffmpeg) ─────────────────────

    #[test]
    fn spawn_recording_h264_and_feed_frames() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_recording.mp4");
        let path_str = path.to_str().unwrap();

        let mut sub = FfmpegSubprocess::spawn_recording(
            path_str, &RecordingCodec::H264, 64, 64, 30,
        ).expect("failed to spawn recording");

        assert_eq!(sub.label(), path_str);
        assert_eq!(sub.frames_written(), 0);

        // Feed a few frames
        let frame = vec![0u8; 64 * 64 * 4]; // black RGBA
        for _ in 0..5 {
            let ok = sub.feed_frame(&frame);
            assert!(ok, "feed_frame should succeed");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Stop and verify
        sub.stop();
        assert!(sub.duration().as_millis() > 0);

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_recording_stop_is_idempotent() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_idempotent.mp4");
        let path_str = path.to_str().unwrap();

        let mut sub = FfmpegSubprocess::spawn_recording(
            path_str, &RecordingCodec::H264, 64, 64, 30,
        ).unwrap();

        // Stop twice — should not panic
        sub.stop();
        sub.stop();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_srt_and_stop() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        // Use a high port unlikely to conflict
        let url = "srt://127.0.0.1:19876";
        let mut sub = FfmpegSubprocess::spawn_srt(url, 64, 64, 30)
            .expect("failed to spawn SRT");

        assert_eq!(sub.label(), url);
        assert_eq!(sub.frames_written(), 0);

        // Feed a frame (won't block because of background writer thread)
        let frame = vec![128u8; 64 * 64 * 4];
        let _ = sub.feed_frame(&frame);

        // Stop cleanly
        sub.stop();
    }

    #[test]
    fn feed_frame_returns_false_after_stop() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_after_stop.mp4");
        let path_str = path.to_str().unwrap();

        let mut sub = FfmpegSubprocess::spawn_recording(
            path_str, &RecordingCodec::H264, 64, 64, 30,
        ).unwrap();

        sub.stop();

        // After stop, feed_frame should return false (channel closed)
        let frame = vec![0u8; 64 * 64 * 4];
        assert!(!sub.feed_frame(&frame));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn recording_prores_codec() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_prores.mov");
        let path_str = path.to_str().unwrap();

        let mut sub = FfmpegSubprocess::spawn_recording(
            path_str, &RecordingCodec::ProRes, 64, 64, 30,
        ).expect("failed to spawn ProRes recording");

        let frame = vec![0u8; 64 * 64 * 4];
        for _ in 0..3 {
            sub.feed_frame(&frame);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        sub.stop();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn frame_channel_capacity_is_bounded() {
        // Verify the channel capacity constant
        assert_eq!(FRAME_CHANNEL_CAPACITY, 2);
    }
}