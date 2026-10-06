//! Frame cost of `shaders/fractal_explorer.fs` on the acceptance scenes in
//! `tests/fixtures/fractal_scenes.json`, at 720p and 1080p.
//!
//!   `<size>/<scene>`: the scene with every look setting as saved.
//!   `<size>/<scene>/geometry`: temporal smoothing, depth of field, bloom and
//!                    light shafts off, which separates the march from the look.
//!
//! Each case renders 30 frames before timing, so the history passes are full.
//! `tests/fractal_explorer.rs` checks that every scene draws structure: an
//! empty frame is cheap, so these timings mean something only while it passes.
//!
//! Skipped without a GPU adapter.
use std::collections::HashMap;

use criterion::{Criterion, criterion_group, criterion_main};
use varda::{
    audio::AudioData,
    deck::Deck,
    isf::ISFShader,
    mixer::{FrameInputs, Mixer},
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
    testing::{FractalScene, fractal_scenes, set_param},
};

struct Case {
    mixer: Mixer,
    frame: u32,
}

impl Case {
    fn new(ctx: &GpuContext, scene: &FractalScene, size: (u32, u32), geometry_only: bool) -> Self {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fractal_explorer.fs");
        let shader = ISFShader::from_file(path).expect("shader");
        let mut deck = Deck::from_shader(ctx, shader, size.0, size.1).expect("deck");
        for (name, value) in &scene.params {
            set_param(&mut deck.generator_params, name, *value).expect("scene input");
        }
        if geometry_only {
            for (name, value) in [
                ("temporal", 0.0),
                ("dof_mode", 0.0),
                ("bloom", 0.0),
                ("shafts", 0.0),
            ] {
                set_param(&mut deck.generator_params, name, value).expect("look input");
            }
        }
        deck.start_declared_preprocessors();
        deck.restore_preprocessor_state(&scene.state);
        let mut mixer = Mixer::new(ctx, size.0, size.1).expect("mixer");
        mixer.channel_mut(0).expect("channel").add_deck(deck);
        for channel in mixer.channels_mut() {
            for slot in &mut channel.decks {
                slot.render_fps = varda::channel::DeckRenderFps::Fixed(0);
            }
        }
        let mut case = Self { mixer, frame: 0 };
        for _ in 0..30 {
            case.render(ctx);
        }
        case
    }

    fn render(&mut self, ctx: &GpuContext) {
        let audio = AudioData::default();
        let audio_values = AudioValues {
            sources: HashMap::default(),
        };
        let analyzer_values = AnalyzerValues::default();
        let inputs = FrameInputs {
            audio_data: &audio,
            audio_values: &audio_values,
            analyzer_values: &analyzer_values,
            beat_time: None,
            transport: None,
            free_run_time: Some(self.frame as f32 / 60.0),
            write_param: varda::param_router::write_macro_target,
        };
        self.mixer.render(ctx, &inputs, 60, &[]).expect("render");
        let _ = ctx.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        self.frame += 1;
    }
}

fn bench(c: &mut Criterion) {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let mut group = c.benchmark_group("fractal_explorer");
    group.sample_size(20);
    for (label, size) in [("720p", (1280, 720)), ("1080p", (1920, 1080))] {
        for scene in fractal_scenes() {
            for geometry_only in [false, true] {
                let mut case = Case::new(&ctx, &scene, size, geometry_only);
                let name = if geometry_only {
                    format!("{label}/{}/geometry", scene.name)
                } else {
                    format!("{label}/{}", scene.name)
                };
                group.bench_function(&name, |b| b.iter(|| case.render(&ctx)));
            }
        }
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
