//! The ffmpeg decode thread's work per frame: decode, convert to RGBA, and
//! hand the frame over. Clips are generated with the ffmpeg CLI (H.264 with
//! B-frames, realistic bitrates) and the bench skips without it.
//!
//!   `video_decode` — one frame of a 1080p 12 Mbit/s and a 4K 45 Mbit/s clip.

use criterion::{Criterion, criterion_group, criterion_main};
use varda::testing::{VideoDecodeBench, generated_clip};

fn video_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("video_decode");
    group.sample_size(30);
    for (name, width, height, bitrate) in [("1080p", 1920, 1080, "12M"), ("4K", 3840, 2160, "45M")]
    {
        let Some(clip) = generated_clip(width, height, 5, bitrate) else {
            eprintln!("video_decode: no ffmpeg CLI, skipping");
            return;
        };
        let mut bench = VideoDecodeBench::new(&clip);
        bench.frame();
        group.bench_function(name, |b| b.iter(|| bench.frame()));
    }
    group.finish();
}

criterion_group!(benches, video_decode);
criterion_main!(benches);
