/// Per-frame cost of the macro fan-out: every knob with a modulation source on
/// its value writes each of its targets through the parameter router, once per
/// rendered frame, before compositing.
///
/// `macros/N` runs N modulated knobs, each driving four deck opacities, so the
/// frame writes 4N parameters. Skipped when no GPU adapter is available.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::deck::Deck;
use varda::macros::{Macro, MacroKind, MacroTarget};
use varda::mixer::Mixer;
use varda::modulation::{AnalyzerValues, AudioValues, ModulationSource};
use varda::renderer::context::GpuContext;

const MACRO_COUNTS: [usize; 3] = [1, 8, 32];
const TARGETS_PER_MACRO: usize = 4;
const DECKS: usize = 8;

fn mixer_with_macros(gpu: &GpuContext, macros: usize) -> Mixer {
    let mut mixer = Mixer::new(gpu, 64, 64).expect("mixer");
    for _ in 0..DECKS {
        let deck = Deck::new_solid_color(gpu, [1.0; 4], 64, 64).expect("solid deck");
        mixer.channel_mut(0).expect("channel 0").add_deck(deck);
    }
    let decks: Vec<String> = mixer
        .channel(0)
        .expect("channel 0")
        .decks
        .iter()
        .map(|s| s.deck.uuid().to_string())
        .collect();
    let source = mixer
        .modulation_mut()
        .add_source(ModulationSource::sine_lfo(1.0));
    for m in 0..macros {
        let uuid = mixer.macros_mut().add_macro(MacroKind::Knob);
        let knob = mixer.macros_mut().find_mut(&uuid).expect("macro");
        for t in 0..TARGETS_PER_MACRO {
            let deck = &decks[(m * TARGETS_PER_MACRO + t) % DECKS];
            knob.targets
                .push(MacroTarget::new(format!("deck/{deck}/opacity")));
        }
        mixer
            .modulation_mut()
            .assign(&Macro::value_mod_key(&uuid), &source, 1.0, None);
    }
    mixer.update_modulation(
        None,
        None,
        &AudioValues::default(),
        &AnalyzerValues::default(),
    );
    mixer
}

fn fan_out(mixer: &mut Mixer) {
    mixer.apply_macro_modulation(varda::param_router::write_macro_target);
}

fn bench_macro_modulation(c: &mut Criterion) {
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("macro_modulation: no GPU adapter, skipping");
        return;
    };
    let mut g = c.benchmark_group("macro_modulation");
    for &count in &MACRO_COUNTS {
        let mut mixer = mixer_with_macros(&gpu, count);
        g.bench_with_input(BenchmarkId::new("macros", count), &count, |b, _| {
            b.iter(|| fan_out(std::hint::black_box(&mut mixer)));
        });
    }
    g.finish();
}

criterion_group!(benches, bench_macro_modulation);
criterion_main!(benches);
