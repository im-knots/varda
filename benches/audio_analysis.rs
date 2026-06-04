/// Audio analysis hot-path benchmarks.
///
/// Two functions run inside the cpal audio callback (~every 5ms at 48kHz/256):
///   onset_threshold  — median computation over spectral flux history (8 values)
///   bpm_estimation   — median + outlier rejection over beat intervals (4-16 values)
///
/// Both currently clone + full-sort their input to compute a median.
/// Benchmarks measure per-invocation cost at realistic window sizes.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use varda::audio::{compute_onset_threshold, estimate_bpm};

/// Realistic spectral flux values (positive, varying magnitude)
fn make_flux_history(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.01 + 0.05 * (i as f32 * 0.7).sin().abs())
        .collect()
}

/// Realistic beat intervals centered around 120 BPM (0.5s) with slight jitter
fn make_beat_intervals(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 + 0.02 * (i as f32 * 1.3).sin())
        .collect()
}

fn bench_onset_threshold(c: &mut Criterion) {
    let mut g = c.benchmark_group("onset_threshold");
    g.sample_size(1000);

    // ONSET_MEDIAN_WINDOW = 8, but test a range
    for n in [4, 8, 16] {
        let flux = make_flux_history(n);
        g.bench_with_input(BenchmarkId::new("clone_sort", n), &flux, |b, flux| {
            b.iter(|| compute_onset_threshold(flux))
        });
    }
    g.finish();
}

fn bench_bpm_estimation(c: &mut Criterion) {
    let mut g = c.benchmark_group("bpm_estimation");
    g.sample_size(1000);

    // BPM_HISTORY_SIZE = 16, but test a range
    for n in [4, 8, 16] {
        let intervals = make_beat_intervals(n);
        g.bench_with_input(
            BenchmarkId::new("clone_sort", n),
            &intervals,
            |b, intervals| b.iter(|| estimate_bpm(intervals)),
        );
    }
    g.finish();
}

criterion_group!(benches, bench_onset_threshold, bench_bpm_estimation);
criterion_main!(benches);
