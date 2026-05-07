//! FfmpegSubprocess — shared ffmpeg lifecycle for recording and SRT streaming.
//!
//! Extracted from RecordingManager and SrtManager into a single reusable type.
//! Spawns an ffmpeg process, pipes raw RGBA frames to stdin.

use std::io::Write;
use std::process::{Child, Command, Stdio};

use crate::renderer::context::RecordingCodec;

/// Shared ffmpeg subprocess for recording and SRT streaming.
pub struct FfmpegSubprocess {
    child: Child,
    /// Frame count written (for stats)
    frames_written: u64,
    /// Human-readable label (path or URL)
    label: String,
    /// Start time (for duration display)
    start_time: std::time::Instant,
}

impl FfmpegSubprocess {
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

        let child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {}. Is ffmpeg installed?", e))?;

        log::info!("Recording started: {} ({}, {}x{} @ {}fps)", path, codec, width, height, fps);

        Ok(Self {
            child,
            frames_written: 0,
            label: path.to_string(),
            start_time: std::time::Instant::now(),
        })
    }

    /// Spawn an ffmpeg SRT streaming subprocess.
    pub fn spawn_srt(
        url: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> anyhow::Result<Self> {
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
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg for SRT: {}. Is ffmpeg installed?", e))?;

        log::info!("SRT stream started: {} ({}x{} @ {}fps)", url, width, height, fps);

        Ok(Self {
            child,
            frames_written: 0,
            label: url.to_string(),
            start_time: std::time::Instant::now(),
        })
    }

    /// Feed a frame of RGBA data to the subprocess.
    /// Returns false if the write failed (subprocess likely crashed).
    pub fn feed_frame(&mut self, rgba: &[u8]) -> bool {
        // Check if ffmpeg already exited (non-blocking)
        if let Some(status) = self.child.try_wait().ok().flatten() {
            if !status.success() {
                self.drain_stderr();
                log::error!("ffmpeg exited with status {} for '{}' before frame could be written", status, self.label);
            }
            return false;
        }
        if let Some(ref mut stdin) = self.child.stdin {
            if let Err(e) = stdin.write_all(rgba) {
                self.drain_stderr();
                log::error!("ffmpeg write error for '{}': {}", self.label, e);
                return false;
            }
            self.frames_written += 1;
            true
        } else {
            false
        }
    }

    /// Read and log any stderr output from ffmpeg (for diagnostics on failure).
    fn drain_stderr(&mut self) {
        use std::io::Read;
        if let Some(ref mut stderr) = self.child.stderr {
            let mut buf = String::new();
            // Read available stderr (non-blocking best-effort)
            let _ = stderr.read_to_string(&mut buf);
            if !buf.is_empty() {
                for line in buf.lines().take(20) {
                    log::error!("ffmpeg stderr [{}]: {}", self.label, line);
                }
            }
        }
    }

    /// Stop the subprocess. Closes stdin and waits for ffmpeg to finish.
    pub fn stop(&mut self) {
        drop(self.child.stdin.take());
        let duration = self.start_time.elapsed();
        match self.child.wait() {
            Ok(status) => {
                self.drain_stderr();
                if status.success() {
                    log::info!("ffmpeg finished: {} ({} frames, {:.1}s)",
                        self.label, self.frames_written, duration.as_secs_f32());
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
        self.frames_written
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
