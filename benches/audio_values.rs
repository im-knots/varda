/// Per-frame cost of gathering audio for modulation: each active source's
/// analysis into `AudioValues`, and the copy of the primary source's data the
/// mixer reads BPM and beat phase from. See /spec/performance-hot-paths.md
/// item I.
///
///   `collect/N` — `AudioValues` from N sources with full-size spectra.
///   `primary` — the copy of one source's `AudioData`.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::audio::AudioData;
use varda::modulation::AudioValues;

fn bench_audio_values(c: &mut Criterion) {
    let data = AudioData::default();
    let mut g = c.benchmark_group("audio_values");
    for count in [1u32, 4] {
        g.bench_with_input(BenchmarkId::new("collect", count), &count, |b, &n| {
            b.iter(|| AudioValues::collect((0..n).map(|id| (id, &data))));
        });
    }
    g.bench_function("primary", |b| {
        b.iter(|| std::hint::black_box(&data).clone());
    });
    g.finish();
}

criterion_group!(benches, bench_audio_values);
criterion_main!(benches);
