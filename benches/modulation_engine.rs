/// Per-frame CPU cost of `ModulationEngine::update`, the hot loop that
/// evaluates every modulation source once per rendered frame.
///
/// Four variants per source count:
///   `lfo_only`   — the common case. Pure functions of time, no clone path.
///   `mixed`      — LFO / step sequencer / ADSR / audio band, as a real scene has.
///   `mod_on_mod` — every source carries a modulator-on-modulator assignment,
///                  which forces the clone-and-copy-back branch.
///   `assignments`— sources plus parameter assignments, so `recompute_order`
///                  has a real dependency graph to walk.
///
/// Source counts run to 256 deliberately. Performance-mode scenes use a
/// handful of modulators, but an arrangement produces one automation envelope
/// per automated parameter, so the engine has to stay flat at that scale
/// (/spec/modulation-engine-perf.md, /spec/automation.md § Performance).
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::collections::HashMap;
use varda::audio::AudioSourceId;
use varda::modulation::{
    ADSRStage, AnalyzerValues, AssignmentMode, AudioReactMode, AudioSourceValues, AudioValues,
    Breakpoint, CurveKind, LFOWaveform, ModulationEngine, ModulationSource, StepInterpolation,
};

const SOURCE_COUNTS: [usize; 4] = [8, 32, 128, 256];

fn audio_values() -> AudioValues {
    // 1024 bins of pink-ish noise: enough structure that `energy_in_range`
    // does real work rather than bailing on the silence check.
    let fft: Vec<f32> = (0..1024).map(|i| 0.5 / ((i as f32) + 1.0).sqrt()).collect();
    let mut sources = HashMap::default();
    sources.insert(
        AudioSourceId::default(),
        AudioSourceValues {
            fft,
            level: 0.4,
            sample_rate: 48_000.0,
        },
    );
    AudioValues { sources }
}

fn lfo(i: usize) -> ModulationSource {
    ModulationSource::LFO {
        waveform: match i % 5 {
            0 => LFOWaveform::Sine,
            1 => LFOWaveform::Square,
            2 => LFOWaveform::Triangle,
            3 => LFOWaveform::Sawtooth,
            _ => LFOWaveform::Random,
        },
        frequency: 0.25 + (i % 8) as f32 * 0.25,
        phase: (i % 16) as f32 / 16.0,
        amplitude: 1.0,
        bipolar: i.is_multiple_of(2),
    }
}

fn step_seq() -> ModulationSource {
    ModulationSource::StepSequencer {
        steps: (0..8).map(|s| s as f32 / 7.0).collect(),
        rate: 2.0,
        interpolation: StepInterpolation::Smooth,
        bipolar: false,
    }
}

fn adsr() -> ModulationSource {
    ModulationSource::ADSR {
        attack: 0.05,
        decay: 0.2,
        sustain: 0.6,
        release: 0.4,
        stage: ADSRStage::default(),
        stage_time: 0.0,
        gate: false,
        current_level: 0.0,
    }
}

fn audio_band() -> ModulationSource {
    ModulationSource::AudioBand {
        source_id: Some(AudioSourceId::default()),
        freq_low: 20.0,
        freq_high: 250.0,
        gain: 1.0,
        smoothing: 0.3,
        mode: AudioReactMode::Direct,
        noise_gate: 0.1,
    }
}

fn engine_lfo_only(n: usize) -> ModulationEngine {
    let mut engine = ModulationEngine::new();
    for i in 0..n {
        engine.add_source(lfo(i));
    }
    engine
}

fn engine_mixed(n: usize) -> ModulationEngine {
    let mut engine = ModulationEngine::new();
    for i in 0..n {
        engine.add_source(match i % 4 {
            0 => lfo(i),
            1 => step_seq(),
            2 => adsr(),
            _ => audio_band(),
        });
    }
    engine
}

/// Every source is both a target and a driver, so `has_mod_on_mod` is true
/// throughout and `update` takes the clone-and-copy-back branch every time.
fn engine_mod_on_mod(n: usize) -> ModulationEngine {
    let mut engine = ModulationEngine::new();
    let uuids: Vec<String> = (0..n).map(|i| engine.add_source(lfo(i))).collect();
    for i in 0..n {
        let driver = &uuids[(i + 1) % n];
        engine.assign(&format!("mod:{}:frequency", uuids[i]), driver, 0.3, None);
    }
    engine
}

/// Sources plus ordinary parameter assignments, which is what a loaded scene
/// actually looks like.
fn engine_with_assignments(n: usize) -> ModulationEngine {
    let mut engine = ModulationEngine::new();
    let uuids: Vec<String> = (0..n).map(|i| engine.add_source(lfo(i))).collect();
    for (i, uuid) in uuids.iter().enumerate() {
        engine.assign(&format!("deck_{i:04x}:opacity"), uuid, 1.0, None);
    }
    engine
}

/// A 32-breakpoint automation curve with a mix of segment shapes, which is a
/// generous count for one parameter over one show.
fn envelope(i: usize) -> ModulationSource {
    let breakpoints = (0..32)
        .map(|k| {
            let curve = match k % 3 {
                0 => CurveKind::Step,
                1 => CurveKind::Linear { tension: 0.4 },
                _ => CurveKind::Smooth,
            };
            Breakpoint::new(k as f64 * 4.0, ((k + i) % 8) as f32 / 7.0).with_curve(curve)
        })
        .collect();
    ModulationSource::envelope(breakpoints)
}

/// An arrangement: one absolute-mode envelope per automated parameter. This is
/// the density /spec/automation.md § Performance has to hold at, and the group
/// that would catch a regression in the segment cache or in absolute
/// resolution.
fn engine_envelopes(n: usize) -> ModulationEngine {
    let mut engine = ModulationEngine::new();
    for i in 0..n {
        let uuid = engine.add_source(envelope(i));
        engine.assign_with_mode(
            &format!("deck_{i:04x}:opacity"),
            &uuid,
            1.0,
            None,
            AssignmentMode::Absolute,
        );
    }
    engine
}

fn bench_update(c: &mut Criterion) {
    let audio = audio_values();
    let analyzers = AnalyzerValues::default();

    let mut g = c.benchmark_group("modulation_update");
    g.sample_size(200);

    for n in SOURCE_COUNTS {
        for (name, mut engine) in [
            ("lfo_only", engine_lfo_only(n)),
            ("mixed", engine_mixed(n)),
            ("mod_on_mod", engine_mod_on_mod(n)),
            ("assignments", engine_with_assignments(n)),
            ("envelopes", engine_envelopes(n)),
        ] {
            // Warm the cached evaluation order so the benchmark measures steady
            // state rather than the one-off `recompute_order`.
            engine.update_free_running(0.0, &audio, &analyzers);

            g.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
                let mut t = 0.0f32;
                b.iter(|| {
                    t += 1.0 / 60.0;
                    engine.update_free_running(t, &audio, &analyzers);
                    criterion::black_box(engine.get_modulation("deck_0000:opacity"))
                });
            });
        }
    }

    g.finish();
}

/// Per-frame cost of the residency predicate, which `Mixer::apply_arrangement`
/// runs once per lane to decide whether that deck's source can stop pulling
/// frames. The win it buys is decode threads rather than frame time, so what
/// this group has to prove is that the check itself stays free.
/// See /spec/deck-residency.md.
fn bench_residency(c: &mut Criterion) {
    use varda::arrangement::residency;

    let mut g = c.benchmark_group("residency_demand");
    g.sample_size(200);

    for n in SOURCE_COUNTS {
        // One region-shaped opacity curve per deck, staggered across a show so
        // the playhead is inside a few and far from most.
        let curves: Vec<Vec<Breakpoint>> = (0..n)
            .map(|i| {
                let start = i as f64 * 30.0;
                vec![
                    Breakpoint::new(0.0, 0.0),
                    Breakpoint::new(start, 0.0),
                    Breakpoint::new(start + 1.0, 1.0),
                    Breakpoint::new(start + 20.0, 1.0),
                    Breakpoint::new(start + 21.0, 0.0),
                    Breakpoint::new(start + 3600.0, 0.0),
                ]
            })
            .collect();

        g.bench_with_input(BenchmarkId::new("lanes", n), &n, |b, _| {
            let mut position = 0.0f64;
            b.iter(|| {
                position += 1.0 / 60.0;
                for curve in &curves {
                    criterion::black_box(residency::demand([Some(curve.as_slice())], position));
                }
            });
        });
    }

    g.finish();
}

criterion_group!(benches, bench_update, bench_residency);
criterion_main!(benches);
