/// Per-frame CPU cost of building the shader parameter buffer.
///
/// Three variants per param count:
///   `no_mod`     — std140 byte buffer serialization only.
///   `empty_mod`  — modulation engine present but no assignments; isolates
///                the per-param key construction.
///   `active_lfo` — full modulation path: lookup, LFO read, clamp, write.
///
/// The (`empty_mod` − `no_mod`) gap is the per-param cost paid even when
/// nothing is modulated. Multiply by params × decks × effects for a full
/// scene's per-frame floor.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::{
    isf::ISFInput,
    modulation::{AudioValues, LFOWaveform, ModulationEngine, ModulationSource},
    params::ShaderParams,
};

fn float_input(name: &str) -> ISFInput {
    ISFInput {
        name: name.to_string(),
        input_type: "float".to_string(),
        default: None,
        min: Some(0.0),
        max: Some(1.0),
        label: None,
        values: None,
        labels: None,
        identity: None,
        group: None,
        specialize: false,
    }
}

fn color_input(name: &str) -> ISFInput {
    ISFInput {
        name: name.to_string(),
        input_type: "color".to_string(),
        default: None,
        min: None,
        max: None,
        label: None,
        values: None,
        labels: None,
        identity: None,
        group: None,
        specialize: false,
    }
}

fn point2d_input(name: &str) -> ISFInput {
    ISFInput {
        name: name.to_string(),
        input_type: "point2D".to_string(),
        default: None,
        min: None,
        max: None,
        label: None,
        values: None,
        labels: None,
        identity: None,
        group: None,
        specialize: false,
    }
}

fn make_params(n_floats: usize) -> ShaderParams {
    let mut inputs: Vec<ISFInput> = (0..n_floats)
        .map(|i| float_input(&format!("p{i}")))
        .collect();
    inputs.push(color_input("tint"));
    inputs.push(point2d_input("center"));
    ShaderParams::from_inputs(&inputs)
}

fn engine_with_lfo(param_key: &str) -> ModulationEngine {
    let mut engine = ModulationEngine::new();
    let src = engine.add_source(ModulationSource::LFO {
        waveform: LFOWaveform::Sine,
        frequency: 1.0,
        phase: 0.0,
        amplitude: 0.5,
        bipolar: false,
    });
    engine.assign(param_key, &src, 1.0);
    engine.update_free_running(
        0.5,
        &AudioValues {
            sources: std::collections::HashMap::default(),
        },
        &varda::modulation::AnalyzerValues::default(),
    );
    engine
}

fn bench_shader_params_buffer(c: &mut Criterion) {
    let mut g = c.benchmark_group("shader_params_buffer");
    g.sample_size(500);

    for n_floats in [2usize, 6, 14] {
        let total = n_floats + 2;
        let lfo_key = "deck0/p0".to_string();
        let eng_empty = ModulationEngine::new();
        let eng_lfo = engine_with_lfo(&lfo_key);

        let mut params_no = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("no_mod", total), &total, |b, _| {
            b.iter(|| {
                params_no.build_buffer_data();
                criterion::black_box(params_no.scratch().len())
            });
        });
        let mut params_em = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("empty_mod", total), &total, |b, _| {
            b.iter(|| {
                params_em.build_modulated_buffer_data(&eng_empty, Some("deck0"));
                criterion::black_box(params_em.scratch().len())
            });
        });
        let mut params_lfo = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("active_lfo", total), &total, |b, _| {
            b.iter(|| {
                params_lfo.build_modulated_buffer_data(&eng_lfo, Some("deck0"));
                criterion::black_box(params_lfo.scratch().len())
            });
        });
    }

    g.finish();
}

/// Combined cost of prefix construction and modulated buffer build, as the
/// render loop does per deck/effect, against a cached prefix.
fn bench_prefix_construction(c: &mut Criterion) {
    let mut g = c.benchmark_group("prefix_construction");
    g.sample_size(500);

    // The deck render path: param prefix plus modulated buffer build.
    let deck_uuid = "a1b2c3d4";
    let fx_uuid = "e5f6a7b8";

    for n_floats in [2usize, 6, 14] {
        let total = n_floats + 2;
        let eng = ModulationEngine::new();

        // Deck param prefix each frame
        let mut params_deck_format = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("deck_format", total), &total, |b, _| {
            b.iter(|| {
                let prefix = format!("deck/{deck_uuid}/param");
                params_deck_format.build_modulated_buffer_data(&eng, Some(&prefix));
                criterion::black_box(params_deck_format.scratch().len())
            });
        });

        // Effect param prefix each frame
        let mut params_fx_format = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("fx_format", total), &total, |b, _| {
            b.iter(|| {
                let prefix = format!("effect/{fx_uuid}/param");
                params_fx_format.build_modulated_buffer_data(&eng, Some(&prefix));
                criterion::black_box(params_fx_format.scratch().len())
            });
        });

        // Cached: prefix already exists (no allocation)
        let cached_deck_prefix = format!("deck/{deck_uuid}/param");
        let cached_fx_prefix = format!("effect/{fx_uuid}/param");
        let mut params_deck_cached = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("deck_cached", total), &total, |b, _| {
            b.iter(|| {
                params_deck_cached.build_modulated_buffer_data(&eng, Some(&cached_deck_prefix));
                criterion::black_box(params_deck_cached.scratch().len())
            });
        });
        let mut params_fx_cached = make_params(n_floats);
        g.bench_with_input(BenchmarkId::new("fx_cached", total), &total, |b, _| {
            b.iter(|| {
                params_fx_cached.build_modulated_buffer_data(&eng, Some(&cached_fx_prefix));
                criterion::black_box(params_fx_cached.scratch().len())
            });
        });
    }

    g.finish();
}

/// Per-frame cost of reading the values that drive phase accumulators.
///
/// `base` is `get_float`. `empty_mod` and `active_lfo` are
/// `get_float_modulated` without and with an assignment on the parameter. The
/// gap is the cost of audio-reactive phase accumulation, paid at most four
/// times per deck and per effect.
fn bench_phase_accumulator_reads(c: &mut Criterion) {
    const PHASE_PARAMS: [&str; 4] = ["p0", "p1", "p2", "p3"];

    let mut g = c.benchmark_group("phase_accumulator_reads");
    g.sample_size(500);

    for n_inputs in [1usize, 4] {
        let names = &PHASE_PARAMS[..n_inputs];
        let eng_empty = ModulationEngine::new();
        let eng_lfo = engine_with_lfo("deck0/p0");

        let params_base = make_params(14);
        g.bench_with_input(BenchmarkId::new("base", n_inputs), &n_inputs, |b, _| {
            b.iter(|| {
                let mut acc = 0.0f32;
                for name in names {
                    acc += params_base.get_float(name).unwrap_or(1.0);
                }
                criterion::black_box(acc)
            });
        });

        let mut params_em = make_params(14);
        g.bench_with_input(
            BenchmarkId::new("empty_mod", n_inputs),
            &n_inputs,
            |b, _| {
                b.iter(|| {
                    let mut acc = 0.0f32;
                    for name in names {
                        acc += params_em
                            .get_float_modulated(name, &eng_empty, Some("deck0"))
                            .unwrap_or(1.0);
                    }
                    criterion::black_box(acc)
                });
            },
        );

        let mut params_lfo = make_params(14);
        g.bench_with_input(
            BenchmarkId::new("active_lfo", n_inputs),
            &n_inputs,
            |b, _| {
                b.iter(|| {
                    let mut acc = 0.0f32;
                    for name in names {
                        acc += params_lfo
                            .get_float_modulated(name, &eng_lfo, Some("deck0"))
                            .unwrap_or(1.0);
                    }
                    criterion::black_box(acc)
                });
            },
        );
    }

    g.finish();
}

criterion_group!(
    benches,
    bench_shader_params_buffer,
    bench_prefix_construction,
    bench_phase_accumulator_reads
);
criterion_main!(benches);
