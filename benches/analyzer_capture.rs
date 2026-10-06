//! Render-thread cost of feeding pixel-reading analyzers.
//!
//!   `analyzer_capture` — one capture per deck with one and four analyzed decks
//!                        at 1080p and 4K. Each iteration times only the
//!                        render thread's share; the GPU is drained untimed
//!                        between iterations, so every capture finds the
//!                        previous frame's readback ready.

use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::renderer::context::GpuContext;
use varda::testing::AnalyzerCaptureBench;

const SIZES: [(&str, u32, u32); 2] = [("1080p", 1920, 1080), ("4K", 3840, 2160)];
const DECK_COUNTS: [usize; 2] = [1, 4];

fn analyzer_capture(c: &mut Criterion) {
    let Ok(context) = GpuContext::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut group = c.benchmark_group("analyzer_capture");
    group.sample_size(20);
    for (label, width, height) in SIZES {
        for decks in DECK_COUNTS {
            let mut bench = AnalyzerCaptureBench::new(context.clone(), width, height, decks);
            // Fill the readback pipeline so the timed calls deliver frames.
            for _ in 0..3 {
                bench.capture();
                bench.wait();
            }
            group.bench_with_input(
                BenchmarkId::new(label, format!("{decks} decks")),
                &decks,
                |b, _| {
                    b.iter_custom(|iterations| {
                        let mut total = Duration::ZERO;
                        for _ in 0..iterations {
                            let start = Instant::now();
                            bench.capture();
                            total += start.elapsed();
                            bench.wait();
                        }
                        total
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, analyzer_capture);
criterion_main!(benches);
