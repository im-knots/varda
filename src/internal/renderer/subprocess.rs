//! `FfmpegSubprocess` — shared ffmpeg lifecycle for recording and SRT streaming.
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
                r"if(Hls.isSupported()){{var h=new Hls({{lowLatencyMode:true,liveSyncDurationCount:2,liveMaxLatencyDurationCount:4,maxBufferLength:4,backBufferLength:0}});h.loadSource('{manifest_filename}');h.attachMedia(v);}}else if(v.canPlayType('application/vnd.apple.mpegurl')){{v.src='{manifest_filename}';}}",
            ),
        ),
        "hls" => (
            "https://cdn.jsdelivr.net/npm/hls.js@latest",
            format!(
                r"if(Hls.isSupported()){{var h=new Hls();h.loadSource('{manifest_filename}');h.attachMedia(v);}}else if(v.canPlayType('application/vnd.apple.mpegurl')){{v.src='{manifest_filename}';}}",
            ),
        ),
        _ => (
            "https://cdn.jsdelivr.net/npm/dashjs@latest/dist/dash.all.min.js",
            format!(
                r"var p=dashjs.MediaPlayer().create();p.updateSettings({{streaming:{{delay:{{liveDelay:2}},buffer:{{fastSwitchEnabled:true}}}}}});p.initialize(v,'{manifest_filename}',true);v.play().catch(function(){{}});",
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
    );
    let path = format!("{dir}/player.html");
    if let Err(e) = std::fs::write(&path, html) {
        log::warn!("Failed to write stream player to '{path}': {e}");
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
    /// Frame tallies, shared with the writer thread.
    counters: FrameCounters,
    /// Writer thread error flag (set when write fails during normal operation)
    write_failed: Arc<AtomicBool>,
    /// Set by `stop()` before killing ffmpeg — tells the writer thread that a
    /// broken pipe is expected and should not be logged as ERROR.
    shutting_down: Arc<AtomicBool>,
    /// Human-readable label (path or URL)
    label: String,
    /// Start time (for duration display)
    start_time: std::time::Instant,
    /// Whether `stop()` has already been called (prevent double-wait)
    stopped: bool,
    /// Optional audio passthrough side-channel (None = video-only).
    audio: Option<AudioPipe>,
    /// When true, `stop()` closes stdin and waits for ffmpeg to exit naturally
    /// (so it can finalize the container — e.g. write the MP4 moov atom).
    /// When false, `stop()` kills ffmpeg immediately (safe for streams, required
    /// when the writer thread may be blocked on a full network pipe).
    graceful_shutdown: bool,
}

/// Bounded channel capacity — 2 frames of buffer allows the writer thread
/// to stay one frame ahead without accumulating unbounded latency.
const FRAME_CHANNEL_CAPACITY: usize = 2;

/// Ceiling on repeated frames emitted to cover one gap, in
/// [`FfmpegSubprocess::start_writer_thread`]. Half a second at 60 fps: long
/// enough to ride out any hitch worth correcting, short enough that a genuine
/// freeze degrades into a shortened timeline rather than a burst of writes into
/// a pipe that is already struggling.
const MAX_PAD_FRAMES_PER_ARRIVAL: u64 = 30;

/// What the writer thread has put down the pipe, split by where it came from.
///
/// `written` counts frames the renderer actually produced and is the health
/// stat; `padded` counts repeats the writer inserted to cover gaps where it
/// did not. The two together are the length of the video timeline.
#[derive(Clone)]
struct FrameCounters {
    written: Arc<AtomicU64>,
    padded: Arc<AtomicU64>,
}

impl FrameCounters {
    fn new() -> Self {
        Self {
            written: Arc::new(AtomicU64::new(0)),
            padded: Arc::new(AtomicU64::new(0)),
        }
    }
}

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
    /// Samples the capture callback discarded because this subscriber's channel
    /// was full. The writer replaces them with silence so the sample count keeps
    /// matching elapsed time — see [`AudioPipe::start`].
    pub lost_samples: Arc<AtomicU64>,
}

/// ffmpeg argument vectors + the live listener/receiver, computed before the
/// `Command` is assembled so audio input args can be interleaved after the video
/// input and audio output args before the destination.
struct PreparedAudio {
    in_args: Vec<String>,
    out_args: Vec<String>,
    listener: TcpListener,
    rx: crossbeam_channel::Receiver<PcmChunk>,
    lost_samples: Arc<AtomicU64>,
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
    // Input opts (must precede the audio `-i`); f32le matches the raw PCM tap.
    //
    // Timestamps come from the sample count, which is ffmpeg's default for a raw
    // input: sample N sits at N/sample_rate. This used to pass
    // `-use_wallclock_as_timestamps 1`, stamping each buffer with the moment it
    // arrived over the socket, and that was the cause of audio breaking up
    // whenever the renderer hitched. Arrival time is not a clock — it carries
    // scheduler jitter, and it stalls outright when ffmpeg stops draining the
    // socket to wait on the video pipe. Every one of those stalls was written
    // into the file as a timing hole.
    //
    // The capture device's sample clock has none of those problems: the hardware
    // delivers exactly `sample_rate` samples per second no matter what the rest
    // of the process is doing. It is the most accurate clock available here, so
    // it is the one the recording is built on. See /spec/av-sync.md.
    let in_args = vec![
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
        lost_samples: audio.lost_samples,
    }))
}

/// Bind a loopback TCP listener on an ephemeral port and return it with the
/// `tcp://127.0.0.1:<port>` URL ffmpeg connects to as the audio input. Loopback
/// TCP is the cross-platform second-input transport (no `mkfifo`/named pipes and
/// no new crate, per the audio-passthrough transport decision).
fn create_audio_endpoint() -> anyhow::Result<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| anyhow::anyhow!("Failed to bind audio TCP listener: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("Failed to read audio listener address: {e}"))?
        .port();
    Ok((listener, format!("tcp://127.0.0.1:{port}")))
}

/// Start the audio writer thread for a prepared passthrough, if any. Called
/// after the ffmpeg child is spawned so the writer can accept ffmpeg's connection.
fn finalize_audio(
    prepared: Option<PreparedAudio>,
    label: String,
) -> anyhow::Result<Option<AudioPipe>> {
    match prepared {
        Some(p) => Ok(Some(AudioPipe::start(
            p.listener,
            p.rx,
            p.lost_samples,
            label,
        )?)),
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
    /// Samples of silence spliced in to replace PCM lost to backpressure.
    silence_spliced: Arc<AtomicU64>,
}

/// Samples of silence written per `write_all` when filling a gap. Only used on
/// the rare backpressure path, so a modest buffer is plenty.
const SILENCE_BLOCK: usize = 4096;

impl AudioPipe {
    /// Start the audio writer thread. It accepts ffmpeg's connection to the
    /// loopback listener, then drains `rx` into the stream as f32le bytes.
    ///
    /// `lost_samples` counts PCM the capture callback had to discard because
    /// this pipe was backed up. The writer replaces each lost sample with a
    /// sample of silence before writing the next real chunk. That matters now
    /// that timestamps come from the sample count: a gap left unfilled does not
    /// read as a gap, it pulls every later sample earlier, so a single dropout
    /// would desynchronise the rest of the recording. Filling it costs a brief
    /// mute and keeps the timeline exact.
    fn start(
        listener: TcpListener,
        rx: crossbeam_channel::Receiver<PcmChunk>,
        lost_samples: Arc<AtomicU64>,
        label: String,
    ) -> anyhow::Result<Self> {
        let shutting_down = Arc::new(AtomicBool::new(false));
        let frames_written = Arc::new(AtomicU64::new(0));
        let silence_spliced = Arc::new(AtomicU64::new(0));
        let sd = shutting_down.clone();
        let fw = frames_written.clone();
        let spliced = silence_spliced.clone();
        // Non-blocking accept so teardown can interrupt a wait for an ffmpeg that
        // never connects (e.g. it died at startup) instead of a wedged thread.
        listener
            .set_nonblocking(true)
            .map_err(|e| anyhow::anyhow!("Failed to set audio listener non-blocking: {e}"))?;
        let writer_thread = std::thread::Builder::new()
            .name(format!("ffmpeg-audio-{label}"))
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
                                log::error!("audio TCP accept failed for '{label}': {e}");
                            }
                            return;
                        }
                    }
                };
                // Blocking writes once connected; disable Nagle to minimize latency.
                if let Err(e) = stream.set_nonblocking(false) {
                    log::error!("audio TCP set-blocking failed for '{label}': {e}");
                    return;
                }
                let _ = stream.set_nodelay(true);
                let silence = [0f32; SILENCE_BLOCK];
                loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(chunk) => {
                            // Restore any time the capture callback had to throw
                            // away, before the samples that follow it.
                            let mut lost = lost_samples.swap(0, Ordering::Relaxed);
                            if lost > 0 {
                                spliced.fetch_add(lost, Ordering::Relaxed);
                                log::warn!(
                                    "audio backpressure on '{label}': spliced {lost} samples of \
                                     silence to hold the timeline"
                                );
                            }
                            while lost > 0 {
                                let n = lost.min(SILENCE_BLOCK as u64) as usize;
                                let bytes: &[u8] = bytemuck::cast_slice(&silence[..n]);
                                if stream.write_all(bytes).is_err() {
                                    return;
                                }
                                lost -= n as u64;
                            }

                            let bytes: &[u8] = bytemuck::cast_slice(&chunk.samples);
                            if let Err(e) = stream.write_all(bytes) {
                                if sd.load(Ordering::SeqCst) {
                                    log::debug!(
                                        "audio pipe closed during shutdown for '{label}': {e}"
                                    );
                                } else {
                                    log::error!("audio pipe write error for '{label}': {e}");
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
            .map_err(|e| anyhow::anyhow!("Failed to spawn audio writer thread: {e}"))?;

        Ok(Self {
            shutting_down,
            silence_spliced,
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

    /// Samples of silence written in place of PCM lost to backpressure.
    fn silence_spliced(&self) -> u64 {
        self.silence_spliced.load(Ordering::Relaxed)
    }
}

impl Drop for AudioPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FfmpegSubprocess {
    /// Start the background writer thread that drains the channel into ffmpeg stdin.
    /// Start the video writer thread.
    ///
    /// The raw video input is declared at a constant `fps`, so ffmpeg times each
    /// frame by its position in the stream: frame N is at N/fps regardless of
    /// when it was produced. That makes a missing frame invisible in the video
    /// but *silent* about time — the recorded timeline simply comes out shorter
    /// than the session was. The audio track is built on the capture device's
    /// sample clock and stays true to real time, so every skipped frame used to
    /// pull the two apart a little more, and the drift accumulated for as long
    /// as the recording ran.
    ///
    /// So when the renderer misses its slot, the writer repeats the previous
    /// frame often enough to cover the gap. Repeating a frame is nearly free to
    /// encode — it differs from its predecessor in nothing — and it keeps the
    /// file constant-frame-rate, which is what editors want. See
    /// /spec/av-sync.md.
    fn start_writer_thread(
        mut stdin: std::process::ChildStdin,
        rx: mpsc::Receiver<Vec<u8>>,
        fps: u32,
        counters: FrameCounters,
        write_failed: Arc<AtomicBool>,
        shutting_down: Arc<AtomicBool>,
        label: String,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name(format!("ffmpeg-writer-{label}"))
            .spawn(move || {
                let fps = f64::from(fps.max(1));
                // Frames emitted so far, padding included. Counted separately
                // from `frames_written` so the health stat still reports what
                // the renderer actually produced.
                let mut emitted: u64 = 0;
                // Anchored on the *first* frame, not on this thread starting.
                // ffmpeg takes a moment to come up and the caller may not have
                // a frame ready the instant it does; timing from spawn would
                // read that startup as a gap and open every recording with a
                // frozen still.
                let mut started: Option<std::time::Instant> = None;

                for frame in rx {
                    let pad = started.map_or_else(
                        || {
                            started = Some(std::time::Instant::now());
                            0
                        },
                        |t| Self::pad_count(t.elapsed(), fps, emitted),
                    );
                    for _ in 0..pad {
                        if let Err(e) = stdin.write_all(&frame) {
                            Self::report_write_error(&e, &shutting_down, &write_failed, &label);
                            return;
                        }
                        emitted += 1;
                    }
                    if pad > 0 {
                        counters.padded.fetch_add(pad, Ordering::Relaxed);
                    }

                    if let Err(e) = stdin.write_all(&frame) {
                        Self::report_write_error(&e, &shutting_down, &write_failed, &label);
                        return;
                    }
                    emitted += 1;
                    counters.written.fetch_add(1, Ordering::Relaxed);
                }
                // Channel closed — normal shutdown, flush stdin
                let _ = stdin.flush();
            })
            .expect("failed to spawn ffmpeg writer thread")
    }

    /// How many repeated frames to emit before the frame that just arrived.
    ///
    /// `emitted` is everything written so far, padding included. If real time
    /// has moved further than that, the difference is the renderer's shortfall
    /// and repeating the previous frame covers it. Capped at
    /// [`MAX_PAD_FRAMES_PER_ARRIVAL`]: past that the app was not really
    /// recording anyway, and a burst of writes into a pipe that is already
    /// behind would make things worse rather than better.
    fn pad_count(elapsed: std::time::Duration, fps: f64, emitted: u64) -> u64 {
        let due = (elapsed.as_secs_f64() * fps) as u64;
        due.saturating_sub(emitted).min(MAX_PAD_FRAMES_PER_ARRIVAL)
    }

    fn report_write_error(
        e: &std::io::Error,
        shutting_down: &Arc<AtomicBool>,
        write_failed: &Arc<AtomicBool>,
        label: &str,
    ) {
        if shutting_down.load(Ordering::SeqCst) {
            log::debug!("ffmpeg pipe closed during shutdown for '{label}': {e}");
        } else {
            log::error!("ffmpeg write error for '{label}': {e}");
            write_failed.store(true, Ordering::SeqCst);
        }
    }

    /// Spawn an ffmpeg recording subprocess.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, or if the `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
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
            .args(["-s", &format!("{width}x{height}")])
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
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {e}. Is ffmpeg installed?"))?;

        log::info!("Recording started: {path} ({codec}, {width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            path.to_string(),
        );
        let audio = finalize_audio(prepared, path.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, or if the `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
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
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };

        let encoder = match codec {
            super::context::SrtCodec::H264 => "libx264",
            super::context::SrtCodec::H265 => "libx265",
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "rgba"])
            .args(["-s", &format!("{width}x{height}")])
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
            anyhow::anyhow!("Failed to spawn ffmpeg for SRT: {e}. Is ffmpeg installed?")
        })?;

        log::info!("SRT server started: {srt_url} ({width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            url.to_string(),
        );
        let audio = finalize_audio(prepared, url.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, if the stream output directory cannot be created, or if the
    /// `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
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
        let dir = format!(".varda/streams/{name}");
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create HLS output dir '{dir}': {e}"))?;
        let playlist = format!("{dir}/index.m3u8");
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
            .args(["-s", &format!("{width}x{height}")])
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
                .args(["-hls_segment_filename", &format!("{dir}/seg_%05d.m4s")]);
        } else {
            cmd.args(["-hls_time", "2"])
                .args(["-hls_list_size", "30"])
                .args(["-hls_flags", "delete_segments"])
                .args(["-hls_segment_filename", &format!("{dir}/seg_%05d.ts")]);
        }

        cmd.arg(&playlist)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn ffmpeg for HLS: {e}. Is ffmpeg installed?")
        })?;

        let mode = if low_latency { "LL-HLS" } else { "HLS" };
        log::info!("{mode} output started: {playlist} ({width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            name.to_string(),
        );
        let audio = finalize_audio(prepared, name.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, or if the `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
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
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in)
            .args(["-c:v", encoder])
            .args(&extra)
            .args(["-pix_fmt", "yuv420p"])
            .args(["-b:v", &format!("{maxrate}k")])
            .args(["-maxrate", &format!("{maxrate}k")])
            .args(["-bufsize", &format!("{bufsize}k")])
            .args(["-g", &gop.to_string()])
            .args(a_out)
            .args(["-f", "flv"])
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn ffmpeg for RTMP: {e}. Is ffmpeg installed?")
        })?;

        log::info!("RTMP output started: {url} ({width}x{height} @ {fps}fps, {maxrate}kbps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let label = url.to_string();
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            label.clone(),
        );
        let audio = finalize_audio(prepared, label.clone())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, if the stream output directory cannot be created, or if the
    /// `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
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
        let dir = format!(".varda/streams/{name}");
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create DASH output dir '{dir}': {e}"))?;
        let manifest = format!("{dir}/manifest.mpd");
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
            .args(["-s", &format!("{width}x{height}")])
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
            anyhow::anyhow!("Failed to spawn ffmpeg for DASH: {e}. Is ffmpeg installed?")
        })?;

        log::info!("DASH output started: {manifest} ({width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            name.to_string(),
        );
        let audio = finalize_audio(prepared, name.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
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
                // Full means the frame was dropped — ffmpeg can't keep up, but that's OK
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
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
                    log::error!("ffmpeg [{label}]: {line}");
                } else {
                    log::debug!("ffmpeg [{label}]: {line}");
                }
            }
        }
    }

    /// Stop the subprocess. For recordings (`graceful_shutdown`), the heavy
    /// work (joining threads, waiting for ffmpeg to write the moov atom) runs
    /// on a detached background thread so the caller (UI / main thread) returns
    /// immediately. For streams, kills ffmpeg inline (fast).
    /// Idempotent — safe to call multiple times.
    ///
    /// # Panics
    ///
    /// On the recording path, panics if the placeholder child process or the
    /// background finalize thread cannot be spawned.
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
            let counters = self.counters.clone();
            let stderr = child.stderr.take();

            std::thread::Builder::new()
                .name(format!("ffmpeg-finalize-{label}"))
                .spawn(move || {
                    const FINALIZE_TIMEOUT: std::time::Duration =
                        std::time::Duration::from_secs(30);

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
                                log::error!("Failed to wait for ffmpeg '{label}': {e}");
                                break;
                            }
                        }
                    }

                    // 5. Log completion
                    if let Some(mut pipe) = stderr {
                        Self::drain_stderr_pipe(&mut pipe, &label);
                    }
                    let frames = counters.written.load(Ordering::Relaxed);
                    log::info!(
                        "ffmpeg finished: {} ({} frames{}, {:.1}s)",
                        label,
                        frames,
                        Self::pad_summary(counters.padded.load(Ordering::Relaxed)),
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

            let frames = self.counters.written.load(Ordering::Relaxed);
            let padded = self.counters.padded.load(Ordering::Relaxed);
            self.drain_stderr();
            log::info!(
                "ffmpeg finished: {} ({} frames{}, {:.1}s)",
                self.label,
                frames,
                Self::pad_summary(padded),
                duration.as_secs_f32()
            );
        }
    }

    /// Completion-log fragment naming repeated frames, empty when there were
    /// none. Padding is not an error — it is how a hitchy session still comes
    /// out in sync — but it is worth knowing the renderer struggled.
    fn pad_summary(padded: u64) -> String {
        if padded == 0 {
            String::new()
        } else {
            format!(" + {padded} repeated to cover renderer gaps")
        }
    }

    /// Duration since the subprocess was started.
    pub fn duration(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Number of frames written so far.
    pub fn frames_written(&self) -> u64 {
        self.counters.written.load(Ordering::Relaxed)
    }

    /// Number of audio PCM chunks written to the socket so far, or `None` for a
    /// video-only output (no audio passthrough).
    pub fn audio_frames_written(&self) -> Option<u64> {
        self.audio.as_ref().map(AudioPipe::frames_written)
    }

    /// Repeated frames emitted to cover gaps where the renderer missed its
    /// slot. Nonzero means the session dropped frames; the recording is still
    /// in sync, but the visible result is a brief freeze.
    pub fn frames_padded(&self) -> u64 {
        self.counters.padded.load(Ordering::Relaxed)
    }

    /// Samples of silence spliced into the audio to replace PCM lost to
    /// backpressure. Nonzero means audio was audibly interrupted, as opposed to
    /// merely delayed. `None` for a video-only output.
    pub fn audio_silence_spliced(&self) -> Option<u64> {
        self.audio.as_ref().map(AudioPipe::silence_spliced)
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
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?mode=listener");
    }

    #[test]
    fn spawn_srt_url_preserves_existing_mode() {
        let url = "srt://127.0.0.1:9001?mode=caller";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?mode=caller");
    }

    #[test]
    fn spawn_srt_url_appends_to_existing_params() {
        let url = "srt://127.0.0.1:9001?latency=0";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?latency=0&mode=listener");
    }

    // ── Recording codec display ────────────────────────────────────

    #[test]
    fn recording_codec_display() {
        use crate::renderer::context::SrtCodec;

        assert_eq!(format!("{}", RecordingCodec::H264), "H.264");
        assert_eq!(format!("{}", RecordingCodec::H265), "H.265 (HEVC)");
        assert_eq!(format!("{}", RecordingCodec::AV1), "AV1");
        assert_eq!(format!("{}", RecordingCodec::ProRes), "ProRes 422");
        assert_eq!(format!("{}", RecordingCodec::ProRes4444), "ProRes 4444");
        assert_eq!(format!("{}", RecordingCodec::Hap), "HAP");
        assert_eq!(format!("{}", RecordingCodec::HapAlpha), "HAP Alpha");
        assert_eq!(format!("{}", RecordingCodec::HapQ), "HAP Q");

        // SrtCodec display
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
            lost_samples: Arc::new(AtomicU64::new(0)),
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
        // Output: AAC, stereo downmix, async resample, explicit mapping.
        assert!(has_pair(&p.out_args, "-c:a", "aac"));
        assert!(has_pair(&p.out_args, "-ac", "2"));
        assert!(has_pair(&p.out_args, "-map", "0:v:0"));
        assert!(has_pair(&p.out_args, "-map", "1:a:0"));
        // Recording must NOT force 48k — native rate is preserved (Decision 5).
        assert!(!has_pair(&p.out_args, "-ar", "48000"));
    }

    /// Audio must be timed by its own sample count, never by when it turned up.
    ///
    /// This is the whole of the recording's timebase. A raw f32le input with no
    /// timestamp option gets PTS from the sample count, which is exactly the
    /// device's clock; adding `-use_wallclock_as_timestamps` replaces that with
    /// the moment each buffer reached the socket. The two look identical while
    /// everything is keeping up and diverge the instant anything stalls —
    /// notably when ffmpeg stops draining the audio socket because it is
    /// waiting on the video pipe. Every such stall lands in the file as a hole
    /// in the audio timeline, which is what "the music goes off-beat and drops
    /// bits when the renderer hitches" sounded like.
    ///
    /// Checked on both policies because all five spawners share `prepare_audio`
    /// and the streaming ones would otherwise regress unnoticed.
    #[test]
    fn audio_is_timed_by_its_sample_clock_not_by_arrival() {
        for is_stream in [false, true] {
            let p = prepare_audio(Some(dummy_audio(48000, 2)), is_stream)
                .unwrap()
                .expect("audio prepared");
            assert!(
                !p.in_args
                    .contains(&"-use_wallclock_as_timestamps".to_string()),
                "audio timestamps must come from the sample count, not arrival time \
                 (is_stream={is_stream}); see /spec/av-sync.md"
            );
        }
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
        let expected = format!("tcp://127.0.0.1:{port}");
        assert!(
            p.in_args.contains(&expected),
            "audio input should be the bound loopback TCP URL"
        );
        // Mono device still reported faithfully on the input side.
        assert!(has_pair(&p.in_args, "-ac", "1"));
    }

    // ── A/V sync: holding the video timeline against the audio clock ────

    /// A renderer keeping up must not be padded at all.
    ///
    /// Padding is repair work; doing it when nothing is broken would inflate
    /// every recording with duplicate frames and make the encoder do work for
    /// nothing.
    #[test]
    fn a_renderer_on_time_is_never_padded() {
        let fps = 60.0;
        // Frame N arrives exactly on its slot, having already emitted N frames.
        for n in 0..120u64 {
            let elapsed =
                std::time::Duration::from_secs_f64(f64::from(u32::try_from(n).unwrap()) / fps);
            assert_eq!(
                FfmpegSubprocess::pad_count(elapsed, fps, n),
                0,
                "frame {n} arrived on time and should need no padding"
            );
        }
    }

    /// A gap in the renderer is covered by exactly as many frames as it swallowed.
    ///
    /// This is what keeps the video timeline honest. Raw video is timed by
    /// position — frame N is at N/fps whenever it was made — so a frame that is
    /// never written does not register as a pause, it shortens the recording.
    /// The audio track is timed by the capture device's sample clock and stays
    /// true to real time, so every unwritten frame used to slide the two apart,
    /// permanently and cumulatively. Repeating the last frame across the gap
    /// costs almost nothing to encode and keeps the two clocks agreeing.
    #[test]
    fn a_renderer_gap_is_covered_frame_for_frame() {
        let fps = 60.0;
        // 10 frames written, then a 100 ms stall: 6 frames' worth of real time
        // has passed unrecorded (10/60 s = 166.7 ms, +100 ms = 266.7 ms = 16
        // frames due), so 6 repeats bring the timeline back to real time.
        let elapsed = std::time::Duration::from_secs_f64(10.0 / fps + 0.1);
        assert_eq!(FfmpegSubprocess::pad_count(elapsed, fps, 10), 6);
    }

    /// A long freeze degrades gracefully instead of flooding the pipe.
    #[test]
    fn a_long_freeze_is_capped_rather_than_burst_written() {
        let fps = 60.0;
        let elapsed = std::time::Duration::from_secs(30);
        assert_eq!(
            FfmpegSubprocess::pad_count(elapsed, fps, 0),
            MAX_PAD_FRAMES_PER_ARRIVAL,
            "a 30 s stall must not emit 1800 frames in one go"
        );
    }

    /// Running ahead of the clock never produces negative padding.
    #[test]
    fn a_renderer_ahead_of_the_clock_is_not_padded() {
        let fps = 60.0;
        assert_eq!(
            FfmpegSubprocess::pad_count(std::time::Duration::from_millis(1), fps, 100),
            0
        );
    }

    /// PCM lost to backpressure is replaced by an equal span of silence.
    ///
    /// Now that audio is timed by its sample count, a dropped chunk does not
    /// leave a hole — it pulls everything after it earlier, so one dropout
    /// would put the rest of the recording out of sync with the picture.
    /// Writing silence in its place costs a brief mute and keeps the count
    /// exact. The assertion is on the byte count for that reason: it is the
    /// only thing the timeline is made of.
    #[test]
    fn lost_audio_is_replaced_by_an_equal_span_of_silence() {
        use std::io::Read as _;

        const CHUNK: usize = 64;
        const LOST: u64 = 500;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = crossbeam_channel::bounded::<PcmChunk>(4);
        let lost_samples = Arc::new(AtomicU64::new(LOST));

        let mut pipe = AudioPipe::start(listener, rx, lost_samples, "test".into()).expect("pipe");

        // Stand in for ffmpeg: connect and read everything written.
        let mut client = std::net::TcpStream::connect(addr).expect("connect");
        tx.send(PcmChunk {
            samples: vec![0.5; CHUNK],
        })
        .expect("send");

        let expected_samples = LOST as usize + CHUNK;
        let mut buf = vec![0u8; expected_samples * 4];
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        client.read_exact(&mut buf).expect("read all PCM");

        let samples: &[f32] = bytemuck::cast_slice(&buf);
        assert!(
            samples[..LOST as usize].iter().all(|s| *s == 0.0),
            "the gap should be filled with silence"
        );
        assert!(
            samples[LOST as usize..].iter().all(|s| *s == 0.5),
            "the real chunk should follow the silence, intact"
        );
        assert_eq!(pipe.silence_spliced(), LOST);

        pipe.stop();
    }

    /// With nothing lost, not a single extra sample is invented.
    #[test]
    fn an_unimpeded_audio_pipe_writes_only_captured_samples() {
        use std::io::Read as _;

        const CHUNK: usize = 32;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = crossbeam_channel::bounded::<PcmChunk>(4);
        let mut pipe = AudioPipe::start(
            listener,
            rx,
            Arc::new(AtomicU64::new(0)),
            "test-clean".into(),
        )
        .expect("pipe");

        let mut client = std::net::TcpStream::connect(addr).expect("connect");
        tx.send(PcmChunk {
            samples: vec![0.25; CHUNK],
        })
        .expect("send");

        let mut buf = vec![0u8; CHUNK * 4];
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        client.read_exact(&mut buf).expect("read chunk");

        let samples: &[f32] = bytemuck::cast_slice(&buf);
        assert!(samples.iter().all(|s| *s == 0.25));
        assert_eq!(pipe.silence_spliced(), 0);

        pipe.stop();
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
