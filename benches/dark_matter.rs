//! Frame cost of `shaders/dark_matter.comp` at its default settings, at 1080p and
//! 4K, through the real mixer, while the simulation is working.
//!
//! Each case renders 180 frames before timing, so the universe is built and the
//! sheet is on screen. Forces, the sheet and the resolve all run in the timed span.
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
};

struct Case {
    mixer: Mixer,
    time: f32,
}

impl Case {
    fn new(ctx: &GpuContext, size: (u32, u32), speed: f64, warmup: u32) -> Self {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/dark_matter.comp");
        let shader = ISFShader::from_file(path).expect("shader");
        let mut deck = Deck::from_shader(ctx, shader, size.0, size.1).expect("deck");
        varda::testing::set_param(&mut deck.generator_params, "speed", speed).expect("speed");
        let mut mixer = Mixer::new(ctx, size.0, size.1).expect("mixer");
        mixer.channel_mut(0).expect("channel").add_deck(deck);
        for channel in mixer.channels_mut() {
            for slot in &mut channel.decks {
                slot.render_fps = varda::channel::DeckRenderFps::Fixed(0);
            }
        }
        let mut case = Self { mixer, time: 0.0 };
        for _ in 0..warmup {
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
            free_run_time: Some(self.time),
            write_param: varda::param_router::write_macro_target,
        };
        self.mixer.render(ctx, &inputs, 60, &[]).expect("render");
        let _ = ctx.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        self.time += 1.0 / 60.0;
    }
}

fn bench(c: &mut Criterion) {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let mut group = c.benchmark_group("dark_matter");
    group.sample_size(30);
    let sizes = [("1080p", (1920, 1080)), ("4k", (3840, 2160))];
    for (label, size) in sizes {
        let mut case = Case::new(&ctx, size, 1.0, 180);
        group.bench_function(label, |b| b.iter(|| case.render(&ctx)));
    }
    // A finished universe costs the most to draw: its voids have stretched, so the
    // sheet holds more points.
    for (label, size) in sizes {
        let mut case = Case::new(&ctx, size, 12.0, 600);
        group.bench_function(format!("{label}_formed"), |b| {
            b.iter(|| case.render(&ctx));
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
