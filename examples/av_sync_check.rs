//! End-to-end check that a recording stays in sync when the renderer stumbles.
//!
//! Drives a real ffmpeg recording with a synthetic 48 kHz tone on the audio
//! side and a 60 fps video feed on the other, then deliberately stalls the
//! video feed the way a heavy shader does. Afterwards it asks ffprobe how long
//! each stream in the file actually is.
//!
//! What it is looking for: audio duration, video duration and wall-clock
//! duration should all agree. They used to diverge, because raw video is timed
//! by frame position — a frame that is never written silently shortens the
//! recording — while audio ran on the capture clock and stayed true to real
//! time. Every dropped frame pushed the two further apart for the rest of the
//! session.
//!
//! Run with:
//!   `LIBRARY_PATH="/opt/homebrew/lib:$LIBRARY_PATH" cargo run --release --example av_sync_check`

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use varda::audio::PcmChunk;
use varda::renderer::{AudioInput, FfmpegSubprocess, RecordingCodec};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FPS: u32 = 60;
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Audio callback period, matching a typical cpal buffer.
const AUDIO_CHUNK_MS: u64 = 10;
const RECORD_SECS: f64 = 6.0;
/// Every Nth frame the "renderer" stalls, standing in for a shader that blew
/// its budget.
const STALL_EVERY: u64 = 40;
const STALL_MS: u64 = 120;

fn main() {
    let path = std::env::temp_dir().join("varda_av_sync_check.mp4");
    let path = path.to_str().expect("utf-8 temp path");
    let _ = std::fs::remove_file(path);

    // Synthetic capture device: a 440 Hz tone delivered in real time, exactly
    // as a sound card would.
    let (pcm_tx, pcm_rx) = crossbeam_channel::bounded::<PcmChunk>(32);
    let audio_done = Arc::new(AtomicU64::new(0));
    let tone_samples = Arc::clone(&audio_done);
    let audio_thread = std::thread::spawn(move || {
        let per_chunk = usize::try_from(u64::from(SAMPLE_RATE) * AUDIO_CHUNK_MS / 1000)
            .expect("chunk fits in usize");
        let started = Instant::now();
        let mut n: u64 = 0;
        while started.elapsed().as_secs_f64() < RECORD_SECS {
            let mut samples = Vec::with_capacity(per_chunk * CHANNELS as usize);
            for _ in 0..per_chunk {
                #[allow(clippy::cast_precision_loss)]
                let t = n as f32 / SAMPLE_RATE as f32;
                let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
                samples.push(v);
                samples.push(v);
                n += 1;
            }
            if pcm_tx.send(PcmChunk { samples }).is_err() {
                break;
            }
            tone_samples.store(n, Ordering::Relaxed);
            // Pace to the wall clock, the way a device callback is paced.
            let due = Duration::from_millis((n * 1000) / u64::from(SAMPLE_RATE));
            if let Some(sleep) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(sleep);
            }
        }
    });

    let audio = AudioInput {
        rx: pcm_rx,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        lost_samples: Arc::new(AtomicU64::new(0)),
    };

    let mut rec = FfmpegSubprocess::spawn_recording(
        path,
        &RecordingCodec::H264,
        WIDTH,
        HEIGHT,
        FPS,
        Some(audio),
    )
    .expect("spawn recording (is ffmpeg installed?)");

    // Video feed: one frame per 60 fps slot, with periodic stalls. A renderer
    // that overruns does not catch up afterwards by rendering extra frames —
    // the slots it missed are simply gone — so the loop resynchronises to the
    // current slot after each stall rather than working through a backlog.
    let frame = vec![32u8; (WIDTH * HEIGHT * 4) as usize];
    let started = Instant::now();
    let mut produced: u64 = 0;
    let mut stalls: u64 = 0;
    let mut slot: u64 = 0;
    while started.elapsed().as_secs_f64() < RECORD_SECS {
        rec.feed_frame(&frame);
        produced += 1;
        if produced.is_multiple_of(STALL_EVERY) {
            std::thread::sleep(Duration::from_millis(STALL_MS));
            stalls += 1;
        }
        // Next slot strictly in the future; everything in between was missed.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let now_slot = (started.elapsed().as_secs_f64() * f64::from(FPS)) as u64;
        slot = slot.max(now_slot) + 1;
        let due = Duration::from_secs_f64(slot as f64 / f64::from(FPS));
        if let Some(sleep) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(sleep);
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let padded = rec.frames_padded();
    rec.stop();
    let _ = audio_thread.join();

    // ffmpeg finalizes the container on a background thread; give it a moment.
    std::thread::sleep(Duration::from_secs(2));

    let v_dur = probe(path, "v:0");
    let a_dur = probe(path, "a:0");

    // Without padding the file would hold exactly the frames the renderer
    // produced, and raw video timing would call that `produced / fps` seconds.
    let unpadded = produced as f64 / f64::from(FPS);

    println!("\n  wall clock          {wall:.3} s");
    println!("  frames produced     {produced} ({stalls} stalls of {STALL_MS} ms)");
    println!("  frames repeated     {padded}");
    println!("  video duration      {v_dur:.3} s");
    println!("  audio duration      {a_dur:.3} s");
    println!("  A/V difference      {:+.3} s", v_dur - a_dur);
    println!(
        "  video without the fix would have been {unpadded:.3} s ({:+.3} s vs audio)\n",
        unpadded - a_dur
    );
}

fn probe(path: &str, stream: &str) -> f64 {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            stream,
            "-show_entries",
            "stream=duration",
            "-of",
            "csv=p=0",
            path,
        ])
        .output()
        .expect("run ffprobe");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0)
}
