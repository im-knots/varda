/// Audio analysis hot-path benchmarks.
///
/// Two functions run inside the cpal audio callback (~every 5ms at 48kHz/256):
///   `onset_threshold`  — median over spectral flux history (8 values)
///   `bpm_estimation`   — autocorrelation comb over the onset envelope (run
///                        every 0.5 s on the analysis thread, not per hop)
///
///   `audio_callback`     — the capture callback's work for one 256-frame
///                          stereo buffer from the device.
///   `audio_hop_analysis` — one 256-sample hop of FFT, flux, onset and BPM
///                          analysis.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::audio::{TempoRange, compute_onset_threshold, estimate_bpm};
use varda::testing::{AudioCaptureBench, HopAnalysisBench};

/// Realistic spectral flux values (positive, varying magnitude)
fn make_flux_history(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.01 + 0.05 * (i as f32 * 0.7).sin().abs())
        .collect()
}

/// Hops per second at 48 kHz.
const HOP_RATE: f32 = 48_000.0 / 256.0;

/// `secs` of onset envelope with a pulse every beat at 120 BPM.
fn make_envelope(secs: f32) -> Vec<f32> {
    (0..(secs * HOP_RATE) as usize)
        .map(|i| if i % 94 < 2 { 1.0 } else { 0.02 })
        .collect()
}

fn bench_onset_threshold(c: &mut Criterion) {
    let mut g = c.benchmark_group("onset_threshold");
    g.sample_size(1000);

    // ONSET_MEDIAN_WINDOW = 8, but test a range
    for n in [4, 8, 16] {
        let flux = make_flux_history(n);
        g.bench_with_input(BenchmarkId::new("clone_sort", n), &flux, |b, flux| {
            b.iter(|| compute_onset_threshold(flux));
        });
    }
    g.finish();
}

fn bench_bpm_estimation(c: &mut Criterion) {
    let mut g = c.benchmark_group("bpm_estimation");
    g.sample_size(1000);

    // 4 s before the first estimate, 8 s once the window is full.
    for secs in [4, 8] {
        let envelope = make_envelope(secs as f32);
        g.bench_with_input(
            BenchmarkId::new("acf_comb", secs),
            &envelope,
            |b, envelope| b.iter(|| estimate_bpm(envelope, HOP_RATE, TempoRange::default())),
        );
    }
    g.finish();
}

fn bench_audio_callback(c: &mut Criterion) {
    let mut capture = AudioCaptureBench::new();
    let mut g = c.benchmark_group("audio_callback");
    g.bench_function("stereo_256", |b| b.iter(|| capture.callback()));
    g.finish();
}

fn bench_hop_analysis(c: &mut Criterion) {
    let mut analysis = HopAnalysisBench::new();
    let mut g = c.benchmark_group("audio_hop_analysis");
    g.bench_function("one_hop", |b| b.iter(|| analysis.analyze()));
    g.finish();
}

criterion_group!(
    benches,
    bench_onset_threshold,
    bench_bpm_estimation,
    bench_audio_callback,
    bench_hop_analysis
);
criterion_main!(benches);
