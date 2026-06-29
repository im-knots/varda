//! FfmpegSubprocess — shared ffmpeg lifecycle for recording and SRT streaming.
//!
//! Spawns an ffmpeg process with a background writer thread that feeds frames
//! via a bounded channel. The render thread never blocks on pipe writes — if
//! ffmpeg can't keep up (e.g. SRT listener waiting for client), frames are dropped.

use std::io::Write;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use crate::audio::PcmChunk;
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
    let title = if low_latency {
        format!("LL-{}", kind.to_uppercase())
    } else {
        kind.to_uppercase()
    };
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
    /// Writer thread error flag (set when write fails during normal operation)
    write_failed: Arc<AtomicBool>,
    /// Set by stop() before killing ffmpeg — tells the writer thread that a
    /// broken pipe is expected and should not be logged as ERROR.
    shutting_down: Arc<AtomicBool>,
    /// Human-readable label (path or URL)
    label: String,
    /// Start time (for duration display)
    start_time: std::time::Instant,
    /// Whether stop() has already been called (prevent double-wait)
    stopped: bool,
    /// Optional audio passthrough side-channel (None = video-only).
    audio: Option<AudioPipe>,
    /// When true, stop() closes stdin and waits for ffmpeg to exit naturally
    /// (so it can finalize the container — e.g. write the MP4 moov atom).
    /// When false, stop() kills ffmpeg immediately (safe for streams, required
    /// when the writer thread may be blocked on a full network pipe).
    graceful_shutdown: bool,
}

/// Bounded channel capacity — 2 frames of buffer allows the writer thread
/// to stay one frame ahead without accumulating unbounded latency.
const FRAME_CHANNEL_CAPACITY: usize = 2;

/// Compute video/buffer bitrate in kbps for RTMP output based on resolution and frame rate.
fn compute_rtmp_bitrate(width: u32, height: u32, fps: u32) -> (u32, u32) {
    let pixels = width * height;
    let base = match pixels {
        p if p <= 921_600 => 3000,   // ≤720p
        p if p <= 2_073_600 => 6000, // ≤1080p
        p if p <= 3_686_400 => 9000, // ≤1440p
        _ => 15000,                  // 4K+
    };
    let maxrate = if fps > 30 { base * 3 / 2 } else { base };
    (maxrate, maxrate * 2)
}

/// AAC output bitrate for passthrough audio.
const AUDIO_BITRATE: &str = "192k";
/// Normalized sample rate for streaming targets (Twitch/YouTube expect 48k AAC).
const STREAM_SAMPLE_RATE: &str = "48000";

/// Optional second (audio) input for an ffmpeg subprocess: a stream of raw
/// interleaved `f32` PCM plus the capture device's native format. Built from an
/// `AudioManager` PCM subscription; `None` keeps the byte-for-byte video-only path.
pub struct AudioInput {
    /// Raw interleaved PCM, drained by the audio writer thread into the socket.
    pub rx: crossbeam_channel::Receiver<PcmChunk>,
    /// Device native sample rate (Hz).
    pub sample_rate: u32,
    /// Device native channel count.
    pub channels: u16,
}

/// ffmpeg argument vectors + the live listener/receiver, computed before the
/// `Command` is assembled so audio input args can be interleaved after the video
/// input and audio output args before the destination.
struct PreparedAudio {
    in_args: Vec<String>,
    out_args: Vec<String>,
    listener: TcpListener,
    rx: crossbeam_channel::Receiver<PcmChunk>,
}

/// Build the ffmpeg audio input/output args and bind the loopback TCP endpoint
/// for an optional audio passthrough. `is_stream` selects the sample-rate policy:
/// native rate for Recording, normalized 48k for streaming targets (Decision 5).
fn prepare_audio(
    audio: Option<AudioInput>,
    is_stream: bool,
) -> anyhow::Result<Option<PreparedAudio>> {
    let Some(audio) = audio else {
        return Ok(None);
    };
    let (listener, audio_url) = create_audio_endpoint()?;
    // Input opts (must precede the audio `-i`): wallclock timestamps anchor the
    // first sample near the first video frame; f32le matches the raw PCM tap.
    let in_args = vec![
        "-use_wallclock_as_timestamps".into(),
        "1".into(),
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        audio.sample_rate.to_string(),
        "-ac".into(),
        audio.channels.to_string(),
        "-i".into(),
        audio_url,
    ];
    // Output opts: AAC, stereo downmix (Decision: stereo for v1), async resample
    // to absorb A/V drift; force 48k on streams, leave native on recordings.
    let mut out_args = vec![
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        AUDIO_BITRATE.into(),
        "-ac".into(),
        "2".into(),
        "-af".into(),
        "aresample=async=1:first_pts=0".into(),
    ];
    if is_stream {
        out_args.push("-ar".into());
        out_args.push(STREAM_SAMPLE_RATE.into());
    }
    // Explicit stream mapping once a second input exists.
    out_args.push("-map".into());
    out_args.push("0:v:0".into());
    out_args.push("-map".into());
    out_args.push("1:a:0".into());
    Ok(Some(PreparedAudio {
        in_args,
        out_args,
        listener,
        rx: audio.rx,
    }))
}

/// Bind a loopback TCP listener on an ephemeral port and return it with the
/// `tcp://127.0.0.1:<port>` URL ffmpeg connects to as the audio input. Loopback
/// TCP is the cross-platform second-input transport (no `mkfifo`/named pipes and
/// no new crate, per the audio-passthrough transport decision).
fn create_audio_endpoint() -> anyhow::Result<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| anyhow::anyhow!("Failed to bind audio TCP listener: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("Failed to read audio listener address: {}", e))?
        .port();
    Ok((listener, format!("tcp://127.0.0.1:{}", port)))
}

/// Start the audio writer thread for a prepared passthrough, if any. Called
/// after the ffmpeg child is spawned so the writer can accept ffmpeg's connection.
fn finalize_audio(
    prepared: Option<PreparedAudio>,
    label: String,
) -> anyhow::Result<Option<AudioPipe>> {
    match prepared {
        Some(p) => Ok(Some(AudioPipe::start(p.listener, p.rx, label)?)),
        None => Ok(None),
    }
}

/// Audio side-channel for an [`FfmpegSubprocess`]: a loopback TCP connection plus
/// a writer thread that drains raw PCM into it, symmetric with the video writer.
pub struct AudioPipe {
    /// Set before teardown so an expected broken pipe isn't logged as ERROR.
    shutting_down: Arc<AtomicBool>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    /// PCM chunks written to the socket so far (health stat).
    frames_written: Arc<AtomicU64>,
}

impl AudioPipe {
    /// Start the audio writer thread. It accepts ffmpeg's connection to the
    /// loopback listener, then drains `rx` into the stream as f32le bytes.
    fn start(
        listener: TcpListener,
        rx: crossbeam_channel::Receiver<PcmChunk>,
        label: String,
    ) -> anyhow::Result<Self> {
        let shutting_down = Arc::new(AtomicBool::new(false));
        let frames_written = Arc::new(AtomicU64::new(0));
        let sd = shutting_down.clone();
        let fw = frames_written.clone();
        // Non-blocking accept so teardown can interrupt a wait for an ffmpeg that
        // never connects (e.g. it died at startup) instead of a wedged thread.
        listener
            .set_nonblocking(true)
            .map_err(|e| anyhow::anyhow!("Failed to set audio listener non-blocking: {}", e))?;
        let writer_thread = std::thread::Builder::new()
            .name(format!("ffmpeg-audio-{}", label))
            .spawn(move || {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if sd.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        Err(e) => {
                            if !sd.load(Ordering::SeqCst) {
                                log::error!("audio TCP accept failed for '{}': {}", label, e);
                            }
                            return;
                        }
                    }
                };
                // Blocking writes once connected; disable Nagle to minimize latency.
                if let Err(e) = stream.set_nonblocking(false) {
                    log::error!("audio TCP set-blocking failed for '{}': {}", label, e);
                    return;
                }
                let _ = stream.set_nodelay(true);
                loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(chunk) => {
                            let bytes: &[u8] = bytemuck::cast_slice(&chunk.samples);
                            if let Err(e) = stream.write_all(bytes) {
                                if sd.load(Ordering::SeqCst) {
                                    log::debug!(
                                        "audio pipe closed during shutdown for '{}': {}",
                                        label,
                                        e
                                    );
                                } else {
                                    log::error!("audio pipe write error for '{}': {}", label, e);
                                }
                                return;
                            }
                            fw.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            if sd.load(Ordering::SeqCst) {
                                let _ = stream.flush();
                                return;
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            let _ = stream.flush();
                            return;
                        }
                    }
                }
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn audio writer thread: {}", e))?;

        Ok(Self {
            shutting_down,
            writer_thread: Some(writer_thread),
            frames_written,
        })
    }

    /// Tear down the writer thread. Idempotent. Setting `shutting_down` unblocks
    /// a pending accept-poll (~20ms) or `recv_timeout` drain (~100ms).
    fn stop(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }

    /// PCM chunks written to the socket so far.
    fn frames_written(&self) -> u64 {
        self.frames_written.load(Ordering::Relaxed)
    }
}

impl Drop for AudioPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FfmpegSubprocess {
    /// Start the background writer thread that drains the channel into ffmpeg stdin.
    fn start_writer_thread(
        mut stdin: std::process::ChildStdin,
        rx: mpsc::Receiver<Vec<u8>>,
        frames_written: Arc<AtomicU64>,
        write_failed: Arc<AtomicBool>,
        shutting_down: Arc<AtomicBool>,
        label: String,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name(format!("ffmpeg-writer-{}", label))
            .spawn(move || {
                for frame in rx {
                    if let Err(e) = stdin.write_all(&frame) {
                        if shutting_down.load(Ordering::SeqCst) {
                            log::debug!(
                                "ffmpeg pipe closed during shutdown for '{}': {}",
                                label,
                                e
                            );
                        } else {
                            log::error!("ffmpeg write error for '{}': {}", label, e);
                            write_failed.store(true, Ordering::SeqCst);
                        }
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
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Recording keeps the device's native sample rate (Decision 5).
        let prepared = prepare_audio(audio, false)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        // (codec args, needs yuv420p output, alpha-capable). Alpha-capable codecs
        // get an `unpremultiply` filter because the program output is
        // premultiplied-alpha (see /spec/html-source.md §2); for fully opaque
        // pixels unpremultiply is a no-op, so existing opaque recordings are
        // unchanged.
        let (codec_args, needs_yuv420p, alpha): (Vec<&str>, bool, bool) = match codec {
            RecordingCodec::H264 => (
                vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "18"],
                true,
                false,
            ),
            RecordingCodec::H265 => (
                vec!["-c:v", "libx265", "-preset", "ultrafast", "-crf", "20"],
                true,
                false,
            ),
            RecordingCodec::AV1 => (
                vec!["-c:v", "libsvtav1", "-preset", "10", "-crf", "28"],
                true,
                false,
            ),
            RecordingCodec::ProRes => (vec!["-c:v", "prores_ks", "-profile:v", "2"], true, false),
            RecordingCodec::ProRes4444 => (
                vec![
                    "-c:v",
                    "prores_ks",
                    "-profile:v",
                    "4",
                    "-pix_fmt",
                    "yuva444p10le",
                ],
                false,
                true,
            ),
            RecordingCodec::Hap => (vec!["-c:v", "hap", "-format", "hap"], false, false),
            RecordingCodec::HapAlpha => (vec!["-c:v", "hap", "-format", "hap_alpha"], false, true),
            RecordingCodec::HapQ => (vec!["-c:v", "hap", "-format", "hap_q"], false, true),
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in);
        if alpha {
            cmd.args(["-vf", "unpremultiply=inplace=1"]);
        }
        cmd.args(&codec_args);
        if needs_yuv420p {
            cmd.args(["-pix_fmt", "yuv420p"]);
        }
        cmd.args(a_out)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {}. Is ffmpeg installed?", e))?;

        log::info!(
            "Recording started: {} ({}, {}x{} @ {}fps)",
            path,
            codec,
            width,
            height,
            fps
        );

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            frames_written.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            path.to_string(),
        );
        let audio = finalize_audio(prepared, path.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            shutting_down,
            label: path.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: true,
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
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
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
            .args(a_in)
            .args(["-c:v", encoder])
            .args(["-preset", "ultrafast"])
            .args(["-tune", "zerolatency"])
            .args(["-pix_fmt", "yuv420p"])
            .args(a_out)
            .args(["-f", "mpegts"])
            .arg(&srt_url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn ffmpeg for SRT: {}. Is ffmpeg installed?",
                e
            )
        })?;

        log::info!(
            "SRT server started: {} ({}x{} @ {}fps)",
            srt_url,
            width,
            height,
            fps
        );

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            frames_written.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            url.to_string(),
        );
        let audio = finalize_audio(prepared, url.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            shutting_down,
            label: url.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
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
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let dir = format!(".varda/streams/{}", name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create HLS output dir '{}': {}", dir, e))?;
        let playlist = format!("{}/index.m3u8", dir);
        write_stream_player(&dir, "hls", "index.m3u8", low_latency);

        let (encoder, extra): (&str, Vec<&str>) = match codec {
            super::context::StreamingCodec::H264 => (
                "libx264",
                vec!["-preset", "ultrafast", "-tune", "zerolatency"],
            ),
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
            .args(a_in)
            .args(["-c:v", encoder])
            .args(&extra)
            .args(["-pix_fmt", "yuv420p"])
            .args(a_out)
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
                .args(["-hls_list_size", "30"])
                .args(["-hls_flags", "delete_segments"])
                .args(["-hls_segment_filename", &format!("{}/seg_%05d.ts", dir)]);
        }

        cmd.arg(&playlist)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn ffmpeg for HLS: {}. Is ffmpeg installed?",
                e
            )
        })?;

        let mode = if low_latency { "LL-HLS" } else { "HLS" };
        log::info!(
            "{} output started: {} ({}x{} @ {}fps)",
            mode,
            playlist,
            width,
            height,
            fps
        );

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            frames_written.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            name.to_string(),
        );
        let audio = finalize_audio(prepared, name.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            shutting_down,
            label: name.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
        })
    }

    /// Spawn an ffmpeg RTMP output subprocess.
    pub fn spawn_rtmp(
        url: &str,
        codec: &super::context::StreamingCodec,
        width: u32,
        height: u32,
        fps: u32,
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let (encoder, extra): (&str, Vec<&str>) = match codec {
            super::context::StreamingCodec::H264 => (
                "libx264",
                vec!["-preset", "ultrafast", "-tune", "zerolatency"],
            ),
            super::context::StreamingCodec::H265 => {
                ("libx265", vec!["-preset", "ultrafast", "-vtag", "hvc1"])
            }
            super::context::StreamingCodec::AV1 => ("libsvtav1", vec!["-preset", "10"]),
        };

        let (maxrate, bufsize) = compute_rtmp_bitrate(width, height, fps);
        let gop = fps * 2;

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", width, height)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in)
            .args(["-c:v", encoder])
            .args(&extra)
            .args(["-pix_fmt", "yuv420p"])
            .args(["-b:v", &format!("{}k", maxrate)])
            .args(["-maxrate", &format!("{}k", maxrate)])
            .args(["-bufsize", &format!("{}k", bufsize)])
            .args(["-g", &gop.to_string()])
            .args(a_out)
            .args(["-f", "flv"])
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn ffmpeg for RTMP: {}. Is ffmpeg installed?",
                e
            )
        })?;

        log::info!(
            "RTMP output started: {} ({}x{} @ {}fps, {}kbps)",
            url,
            width,
            height,
            fps,
            maxrate
        );

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let label = url.to_string();
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            frames_written.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            label.clone(),
        );
        let audio = finalize_audio(prepared, label.clone())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            shutting_down,
            label,
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
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
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let dir = format!(".varda/streams/{}", name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create DASH output dir '{}': {}", dir, e))?;
        let manifest = format!("{}/manifest.mpd", dir);
        write_stream_player(&dir, "dash", "manifest.mpd", false);

        let (encoder, extra): (&str, Vec<&str>) = match codec {
            super::context::StreamingCodec::H264 => (
                "libx264",
                vec!["-preset", "ultrafast", "-tune", "zerolatency"],
            ),
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
            .args(a_in)
            .args(["-c:v", encoder])
            .args(&extra)
            .args(["-pix_fmt", "yuv420p"])
            .args(a_out)
            .args(["-f", "dash"])
            .args(["-seg_duration", "2"])
            .args(["-window_size", "30"])
            .args(["-extra_window_size", "5"])
            .arg(&manifest)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn ffmpeg for DASH: {}. Is ffmpeg installed?",
                e
            )
        })?;

        log::info!(
            "DASH output started: {} ({}x{} @ {}fps)",
            manifest,
            width,
            height,
            fps
        );

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let frames_written = Arc::new(AtomicU64::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            frames_written.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            name.to_string(),
        );
        let audio = finalize_audio(prepared, name.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            frames_written,
            write_failed,
            shutting_down,
            label: name.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
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
                log::error!(
                    "ffmpeg exited with status {} for '{}' before frame could be written",
                    status,
                    self.label
                );
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
        if let Some(mut stderr) = self.child.stderr.take() {
            Self::drain_stderr_pipe(&mut stderr, &self.label);
        }
    }

    /// Static helper: drain an ffmpeg stderr pipe and log each line.
    fn drain_stderr_pipe(stderr: &mut std::process::ChildStderr, label: &str) {
        use std::io::Read;
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        if !buf.is_empty() {
            for line in buf.lines().take(30) {
                let lower = line.to_ascii_lowercase();
                if lower.contains("error")
                    || lower.contains("failed")
                    || lower.contains("invalid")
                    || lower.contains("fatal")
                {
                    log::error!("ffmpeg [{}]: {}", label, line);
                } else {
                    log::debug!("ffmpeg [{}]: {}", label, line);
                }
            }
        }
    }

    /// Stop the subprocess. For recordings (`graceful_shutdown`), the heavy
    /// work (joining threads, waiting for ffmpeg to write the moov atom) runs
    /// on a detached background thread so the caller (UI / main thread) returns
    /// immediately. For streams, kills ffmpeg inline (fast).
    /// Idempotent — safe to call multiple times.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;

        let duration = self.start_time.elapsed();

        // 1. Signal shutdown so writer threads know a broken pipe is expected
        self.shutting_down.store(true, Ordering::SeqCst);

        // 2. Drop the sender to close the channel — no more frames queued
        drop(self.frame_tx.take());

        if self.graceful_shutdown {
            // --- Recording path: finalize on a background thread ---
            // Move all owned resources out of `self` so the thread owns them.
            let mut audio = self.audio.take();
            let writer_thread = self.writer_thread.take();
            let mut child = std::mem::replace(
                &mut self.child,
                // Placeholder — never used again (stopped == true).
                Command::new("true")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("failed to spawn placeholder"),
            );
            let label = self.label.clone();
            let frames_written = self.frames_written.clone();
            let stderr = child.stderr.take();

            std::thread::Builder::new()
                .name(format!("ffmpeg-finalize-{}", label))
                .spawn(move || {
                    // 3a. Tear down the audio writer/socket so ffmpeg sees EOF on
                    //     both inputs.
                    if let Some(ref mut a) = audio {
                        a.stop();
                    }

                    // 3b. Join the video writer thread — drains remaining ≤2
                    //     frames, flushes & drops stdin → ffmpeg sees video EOF.
                    if let Some(handle) = writer_thread {
                        let _ = handle.join();
                    }

                    // 4. Wait for ffmpeg to finalize the container (moov atom).
                    const FINALIZE_TIMEOUT: std::time::Duration =
                        std::time::Duration::from_secs(30);
                    let deadline = std::time::Instant::now() + FINALIZE_TIMEOUT;
                    loop {
                        match child.try_wait() {
                            Ok(Some(_status)) => break,
                            Ok(None) => {
                                if std::time::Instant::now() >= deadline {
                                    log::warn!(
                                        "ffmpeg did not exit within {}s for '{}', killing",
                                        FINALIZE_TIMEOUT.as_secs(),
                                        label
                                    );
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    break;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            Err(e) => {
                                log::error!("Failed to wait for ffmpeg '{}': {}", label, e);
                                break;
                            }
                        }
                    }

                    // 5. Log completion
                    if let Some(mut pipe) = stderr {
                        Self::drain_stderr_pipe(&mut pipe, &label);
                    }
                    let frames = frames_written.load(Ordering::Relaxed);
                    log::info!(
                        "ffmpeg finished: {} ({} frames, {:.1}s)",
                        label,
                        frames,
                        duration.as_secs_f32()
                    );
                })
                .expect("failed to spawn ffmpeg finalize thread");
        } else {
            // --- Streaming path: kill immediately (inline, fast) ---

            // 3. Kill ffmpeg BEFORE joining the writer thread. The writer
            //    thread may be blocked on stdin.write_all() (e.g. SRT listener
            //    with a full pipe buffer). Killing the child breaks the pipe,
            //    which unblocks the write and lets the thread exit.
            let _ = self.child.kill();

            // 3b. Tear down the audio side-channel (socket + writer thread).
            //     Done after the kill so a writer blocked on a full socket sees
            //     a broken pipe.
            if let Some(audio) = self.audio.as_mut() {
                audio.stop();
            }

            // 4. Now safe to join — the writer thread will see a broken pipe
            //    or a closed channel and exit promptly.
            if let Some(handle) = self.writer_thread.take() {
                let _ = handle.join();
            }

            // 5. Reap the child process
            let _ = self.child.wait();

            let frames = self.frames_written.load(Ordering::Relaxed);
            self.drain_stderr();
            log::info!(
                "ffmpeg finished: {} ({} frames, {:.1}s)",
                self.label,
                frames,
                duration.as_secs_f32()
            );
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

    /// Number of audio PCM chunks written to the socket so far, or `None` for a
    /// video-only output (no audio passthrough).
    pub fn audio_frames_written(&self) -> Option<u64> {
        self.audio.as_ref().map(|a| a.frames_written())
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
        assert_eq!(format!("{}", RecordingCodec::ProRes4444), "ProRes 4444");
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

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::H264, 64, 64, 30, None)
                .expect("failed to spawn recording");

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

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::H264, 64, 64, 30, None)
                .unwrap();
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
        let mut sub = FfmpegSubprocess::spawn_srt(
            url,
            &crate::renderer::context::SrtCodec::H264,
            64,
            64,
            30,
            None,
        )
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

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::H264, 64, 64, 30, None)
                .unwrap();

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

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::ProRes, 64, 64, 30, None)
                .expect("failed to spawn ProRes recording");

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

    // ── Phase 19b: audio passthrough arg construction ──────────────

    fn dummy_audio(sample_rate: u32, channels: u16) -> AudioInput {
        let (_tx, rx) = crossbeam_channel::bounded::<PcmChunk>(4);
        AudioInput {
            rx,
            sample_rate,
            channels,
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn prepare_audio_none_is_video_only() {
        // No audio input → no socket, no args: the video-only path is unchanged.
        let prepared = prepare_audio(None, false).unwrap();
        assert!(prepared.is_none());
    }

    #[test]
    fn prepare_audio_recording_uses_native_rate() {
        let p = prepare_audio(Some(dummy_audio(44100, 2)), false)
            .unwrap()
            .expect("audio prepared");
        // Input: raw f32le at the device's native rate + channel count.
        assert!(has_pair(&p.in_args, "-f", "f32le"));
        assert!(has_pair(&p.in_args, "-ar", "44100"));
        assert!(has_pair(&p.in_args, "-ac", "2"));
        assert!(p.in_args.contains(&"-i".to_string()));
        assert!(p
            .in_args
            .contains(&"-use_wallclock_as_timestamps".to_string()));
        // Output: AAC, stereo downmix, async resample, explicit mapping.
        assert!(has_pair(&p.out_args, "-c:a", "aac"));
        assert!(has_pair(&p.out_args, "-ac", "2"));
        assert!(has_pair(&p.out_args, "-map", "0:v:0"));
        assert!(has_pair(&p.out_args, "-map", "1:a:0"));
        // Recording must NOT force 48k — native rate is preserved (Decision 5).
        assert!(!has_pair(&p.out_args, "-ar", "48000"));
    }

    #[test]
    fn prepare_audio_stream_forces_48k() {
        let p = prepare_audio(Some(dummy_audio(44100, 2)), true)
            .unwrap()
            .expect("audio prepared");
        // Stream targets normalize to 48k (Decision 5).
        assert!(has_pair(&p.out_args, "-ar", "48000"));
    }

    #[test]
    fn prepare_audio_binds_tcp_endpoint() {
        let p = prepare_audio(Some(dummy_audio(48000, 1)), false)
            .unwrap()
            .expect("audio prepared");
        // The second input is the loopback TCP URL of the bound listener.
        let port = p.listener.local_addr().expect("listener addr").port();
        let expected = format!("tcp://127.0.0.1:{}", port);
        assert!(
            p.in_args.contains(&expected),
            "audio input should be the bound loopback TCP URL"
        );
        // Mono device still reported faithfully on the input side.
        assert!(has_pair(&p.in_args, "-ac", "1"));
    }

    #[test]
    fn compute_rtmp_bitrate_720p() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(1280, 720, 30);
        assert_eq!(maxrate, 3000);
        assert_eq!(bufsize, 6000);
    }

    #[test]
    fn compute_rtmp_bitrate_1080p() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(1920, 1080, 30);
        assert_eq!(maxrate, 6000);
        assert_eq!(bufsize, 12000);
    }

    #[test]
    fn compute_rtmp_bitrate_1080p60() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(1920, 1080, 60);
        assert_eq!(maxrate, 9000);
        assert_eq!(bufsize, 18000);
    }

    #[test]
    fn compute_rtmp_bitrate_4k() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(3840, 2160, 30);
        assert_eq!(maxrate, 15000);
        assert_eq!(bufsize, 30000);
    }
}
