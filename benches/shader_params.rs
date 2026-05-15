/// Per-frame CPU cost of building the shader parameter buffer.
///
/// Three variants per param count:
///   no_mod     — std140 byte buffer serialization only.
///   empty_mod  — modulation engine present but no assignments.
///                Isolates the cost of the per-param key construction
///                (currently a `format!` allocation).
///   active_lfo — full modulation path: lookup, LFO read, clamp, write.
///
/// The (empty_mod − no_mod) gap is the per-param allocation cost paid even
/// when nothing is modulated. Multiply by params × decks × effects to
/// estimate the per-frame floor for a full scene.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
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
    }
}

fn color_input(name: &str) -> ISFInput {
    ISFInput {
        name: name.to_string(),
        input_type: "color".to_string(),
        default: None, min: None, max: None, label: None,
        values: None, labels: None, identity: None,
    }
}

fn point2d_input(name: &str) -> ISFInput {
    ISFInput {
        name: name.to_string(),
        input_type: "point2D".to_string(),
        default: None, min: None, max: None, label: None,
        values: None, labels: None, identity: None,
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
    engine.assign(param_key, &src, 1.0, None);
    engine.update(0.5, &AudioValues { sources: Default::default() });
    engine
}

fn bench_shader_params_buffer(c: &mut Criterion) {
    let mut g = c.benchmark_group("shader_params_buffer");
    g.sample_size(500);

    for n_floats in [2usize, 6, 14] {
        let params = make_params(n_floats);
        let total = n_floats + 2;
        let lfo_key = format!("deck0:p0");
        let eng_empty = ModulationEngine::new();
        let eng_lfo = engine_with_lfo(&lfo_key);

        g.bench_with_input(BenchmarkId::new("no_mod", total), &total,
            |b, _| b.iter(|| params.build_buffer_data()));
        g.bench_with_input(BenchmarkId::new("empty_mod", total), &total,
            |b, _| b.iter(|| params.build_modulated_buffer_data(&eng_empty, Some("deck0"))));
        g.bench_with_input(BenchmarkId::new("active_lfo", total), &total,
            |b, _| b.iter(|| params.build_modulated_buffer_data(&eng_lfo, Some("deck0"))));
    }

    g.finish();
}

criterion_group!(benches, bench_shader_params_buffer);
criterion_main!(benches);
