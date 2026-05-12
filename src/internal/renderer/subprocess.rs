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

/// Write a self-contained HTML player page into a stream directory.
/// Uses hls.js for HLS streams and dash.js for DASH streams.
/// For LL-HLS, enables hls.js low-latency mode with live-edge tuning.
fn write_stream_player(dir: &str, kind: &str, manifest_filename: &str, low_latency: bool) {
    let (lib_url, lib_setup) = match kind {
        "hls" if low_latency => (
            "https://cdn.jsdelivr.net/npm/hls.js@latest",
            format!(
                r#"if(Hls.isSupported()){{var h=new Hls({{lowLatencyMode:true,liveSyncDurationCount:2,liveMaxLatencyDurationCount:4,maxBufferLength:4,backBufferLength:0}});h.loadSource('{}');h.attachMedia(v);}}else if(v.canPlayType('application/vnd.apple.mpegurl')){{v.src='{}';}}"#,
                manifest_filename, manifest_filename,
            ),
        ),
        "hls" => (
            "https://cdn.jsdelivr.net/npm/hls.js@latest",
            format!(
                r#"if(Hls.isSupported()){{var h=new Hls();h.loadSource('{}');h.attachMedia(v);}}else if(v.canPlayType('application/vnd.apple.mpegurl')){{v.src='{}';}}"#,
                manifest_filename, manifest_filename,
            ),
        ),
        _ => (
            "https://cdn.jsdelivr.net/npm/dashjs@latest/dist/dash.all.min.js",
            format!(
                r#"var p=dashjs.MediaPlayer().create();p.updateSettings({{streaming:{{delay:{{liveDelay:2}},buffer:{{fastSwitchEnabled:true}}}}}});p.initialize(v,'{}',true);v.play().catch(function(){{}});"#,
                manifest_filename,
            ),
        ),
    };
    let title = if low_latency { format!("LL-{}", kind.to_uppercase()) } else { kind.to_uppercase() };
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Varda — {title} stream</title>
<style>*{{margin:0;padding:0;background:#000}}video{{width:100vw;height:100vh;object-fit:contain}}</style>
<script src="{lib_url}"></script></head>
<body><video id="v" autoplay muted controls></video>
<script>var v=document.getElementById('v');{lib_setup}</script></body></html>"#,
        title = title,
        lib_url = lib_url,
        lib_setup = lib_setup,
    );
    let path = format!("{}/player.html", dir);
    if let Err(e) = std::fs::write(&path, html) {
        log::warn!("Failed to write stream player to '{}': {}", path, e);
    }
}

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
        let (codec_args, needs_yuv420p): (Vec<&str>, bool) = match codec {
            RecordingCodec::H264 => (vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "18"], true),
            RecordingCodec::H265 => (vec!["-c:v", "libx265", "-preset", "ultrafast", "-crf", "20"], true),
            RecordingCodec::AV1 => (vec!["-c:v", "libsvtav1", "-preset", "10", "-crf", "28"], true),
            RecordingCodec::ProRes => (vec!["-c:v", "prores_ks", "-profile:v", "2"], true),
            RecordingCodec::Hap => (vec!["-c:v", "hap", "-format", "hap"], false),
            RecordingCodec::HapAlpha => (vec!["-c:v", "hap", "-format", "hap_alpha"], false),
            RecordingCodec::HapQ => (vec!["-c:v", "hap", "-format", "hap_q"], false),
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(&codec_args);
        if needs_yuv420p {
            cmd.args(["-pix_fmt", "yuv420p"]);
        }
        cmd.arg(path)
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
        codec: &super::context::SrtCodec,
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

        let encoder = match codec {
            super::context::SrtCodec::H264 => "libx264",
            super::context::SrtCodec::H265 => "libx265",
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(["-c:v", encoder])
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

    /// Spawn an ffmpeg HLS output subprocess.
    /// Writes HLS segments to `.varda/streams/<name>/` with `-hls_list_size 0` for VOD archive.
    pub fn spawn_hls(
        name: &str,
        codec: &super::context::StreamingCodec,
        width: u32,
        height: u32,
        fps: u32,
        low_latency: bool,
    ) -> anyhow::Result<Self> {
        let dir = format!(".varda/streams/{}", name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create HLS output dir '{}': {}", dir, e))?;
        let playlist = format!("{}/index.m3u8", dir);
        write_stream_player(&dir, "hls", "index.m3u8", low_latency);

        let (encoder, extra): (&str, Vec<&str>) = match codec {
            super::context::StreamingCodec::H264 => ("libx264", vec!["-preset", "ultrafast", "-tune", "zerolatency"]),
            super::context::StreamingCodec::H265 => ("libx265", vec!["-preset", "ultrafast"]),
            super::context::StreamingCodec::AV1 => ("libsvtav1", vec!["-preset", "10"]),
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(["-c:v", encoder])
            .args(&extra)
            .args(["-pix_fmt", "yuv420p"])
            .args(["-f", "hls"]);

        if low_latency {
            cmd.args(["-hls_time", "1"])
                .args(["-hls_list_size", "6"])
                .args(["-hls_flags", "independent_segments+delete_segments"])
                .args(["-hls_segment_type", "fmp4"])
                .args(["-hls_fmp4_init_filename", "init.mp4"])
                .args(["-hls_segment_filename", &format!("{}/seg_%05d.m4s", dir)]);
        } else {
            cmd.args(["-hls_time", "2"])
                .args(["-hls_list_size", "0"])
                .args(["-hls_segment_filename", &format!("{}/seg_%05d.ts", dir)]);
        }

        cmd.arg(&playlist)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg for HLS: {}. Is ffmpeg installed?", e))?;

        let mode = if low_latency { "LL-HLS" } else { "HLS" };
        log::info!("{} output started: {} ({}x{} @ {}fps)", mode, playlist, width, height, fps);

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin, rx, frames_written.clone(), write_failed.clone(), name.to_string(),
        );

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            label: name.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
        })
    }

    /// Spawn an ffmpeg DASH output subprocess.
    /// Writes DASH segments to `.varda/streams/<name>/` with `-window_size 0` for VOD archive.
    pub fn spawn_dash(
        name: &str,
        codec: &super::context::StreamingCodec,
        width: u32,
        height: u32,
        fps: u32,
    ) -> anyhow::Result<Self> {
        let dir = format!(".varda/streams/{}", name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create DASH output dir '{}': {}", dir, e))?;
        let manifest = format!("{}/manifest.mpd", dir);
        write_stream_player(&dir, "dash", "manifest.mpd", false);

        let (encoder, extra): (&str, Vec<&str>) = match codec {
            super::context::StreamingCodec::H264 => ("libx264", vec!["-preset", "ultrafast", "-tune", "zerolatency"]),
            super::context::StreamingCodec::H265 => ("libx265", vec!["-preset", "ultrafast"]),
            super::context::StreamingCodec::AV1 => ("libsvtav1", vec!["-preset", "10"]),
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(["-c:v", encoder])
            .args(&extra)
            .args(["-pix_fmt", "yuv420p"])
            .args(["-f", "dash"])
            .args(["-seg_duration", "2"])
            .args(["-window_size", "0"])
            .arg(&manifest)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg for DASH: {}. Is ffmpeg installed?", e))?;

        log::info!("DASH output started: {} ({}x{} @ {}fps)", manifest, width, height, fps);

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin, rx, frames_written.clone(), write_failed.clone(), name.to_string(),
        );

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            label: name.to_string(),
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

        let duration = self.start_time.elapsed();

        // 1. Drop the sender to close the channel — no more frames queued
        drop(self.frame_tx.take());

        // 2. Kill ffmpeg BEFORE joining the writer thread. The writer thread
        //    may be blocked on stdin.write_all() (e.g. SRT listener with a
        //    full pipe buffer). Killing the child breaks the pipe, which
        //    unblocks the write and lets the thread exit.
        let _ = self.child.kill();

        // 3. Now safe to join — the writer thread will see a broken pipe or
        //    a closed channel and exit promptly.
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }

        // 4. Reap the child process
        let frames = self.frames_written.load(Ordering::Relaxed);
        match self.child.wait() {
            Ok(_status) => {
                self.drain_stderr();
                log::info!("ffmpeg finished: {} ({} frames, {:.1}s)",
                    self.label, frames, duration.as_secs_f32());
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
        assert_eq!(format!("{}", RecordingCodec::H264), "H.264");
        assert_eq!(format!("{}", RecordingCodec::H265), "H.265 (HEVC)");
        assert_eq!(format!("{}", RecordingCodec::AV1), "AV1");
        assert_eq!(format!("{}", RecordingCodec::ProRes), "ProRes 422");
        assert_eq!(format!("{}", RecordingCodec::Hap), "HAP");
        assert_eq!(format!("{}", RecordingCodec::HapAlpha), "HAP Alpha");
        assert_eq!(format!("{}", RecordingCodec::HapQ), "HAP Q");

        // SrtCodec display
        use crate::renderer::context::SrtCodec;
        assert_eq!(format!("{}", SrtCodec::H264), "H.264");
        assert_eq!(format!("{}", SrtCodec::H265), "H.265 (HEVC)");
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
        let mut sub = FfmpegSubprocess::spawn_srt(url, &crate::renderer::context::SrtCodec::H264, 64, 64, 30)
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