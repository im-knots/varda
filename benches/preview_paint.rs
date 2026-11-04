//! Cost of the UI's previews in a real egui frame: 160x90 point thumbnails of
//! 1080p sources, at 2 pixels per point.
//!
//!   `preview_paint_cpu` — render-thread time of a frame, GPU wait not timed.
//!   `preview_paint_gpu` — a frame through the GPU wait.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::time::{Duration, Instant};
use varda::renderer::context::GpuContext;
use varda::testing::PreviewPaintBench;

/// (previews, previews drawn): a small show, then 16 decks, 4 channels, the
/// program and 2 outputs, with one panel's worth or everything on screen.
const CASES: [(usize, usize); 3] = [(4, 4), (23, 3), (23, 23)];

fn preview_paint(c: &mut Criterion) {
    let Ok(context) = GpuContext::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut group = c.benchmark_group("preview_paint_cpu");
    for (count, drawn) in CASES {
        let mut bench = PreviewPaintBench::new(context.clone(), count);
        bench.frame(drawn);
        bench.wait();
        group.bench_with_input(
            BenchmarkId::new(format!("{count} previews"), format!("{drawn} drawn")),
            &drawn,
            |b, &drawn| {
                b.iter_custom(|iterations| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iterations {
                        let start = Instant::now();
                        bench.frame(drawn);
                        total += start.elapsed();
                        bench.wait();
                    }
                    total
                });
            },
        );
    }
    group.finish();

    let mut group = c.benchmark_group("preview_paint_gpu");
    for (count, drawn) in CASES {
        let mut bench = PreviewPaintBench::new(context.clone(), count);
        bench.frame(drawn);
        bench.wait();
        group.bench_with_input(
            BenchmarkId::new(format!("{count} previews"), format!("{drawn} drawn")),
            &drawn,
            |b, &drawn| {
                b.iter(|| {
                    bench.frame(drawn);
                    bench.wait();
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, preview_paint);
criterion_main!(benches);
