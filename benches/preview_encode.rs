//! GPU cost of the UI's preview encoding: each deck, channel, program and
//! output preview is gamma-encoded into a target of up to 960 pixels a frame.
//!
//!   `preview_encode` — 4 and 23 1080p previews, with 3 of them on screen or
//!                      all of them, encoded and drained each iteration.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::renderer::context::GpuContext;
use varda::testing::PreviewEncodeBench;

/// (previews, previews drawn): a small show, then 16 decks, 4 channels, the
/// program and 2 outputs, with one panel's worth or everything on screen.
const CASES: [(usize, usize); 3] = [(4, 4), (23, 3), (23, 23)];

fn preview_encode(c: &mut Criterion) {
    let Ok(context) = GpuContext::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut group = c.benchmark_group("preview_encode");
    for (count, drawn) in CASES {
        let mut bench = PreviewEncodeBench::new(context.clone(), count);
        bench.frame(drawn);
        group.bench_with_input(
            BenchmarkId::new(format!("{count} previews"), format!("{drawn} drawn")),
            &drawn,
            |b, &drawn| b.iter(|| bench.frame(drawn)),
        );
    }
    group.finish();
}

criterion_group!(benches, preview_encode);
criterion_main!(benches);
