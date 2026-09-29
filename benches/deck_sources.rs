/// Per-frame cost of a scene holding one deck of each hardware-free source
/// kind: solid color, raster image, ISF generator, program tap, and HAP video.
/// Measures per-deck source dispatch on top of the fixed frame cost
/// `headless_frame` measures.
///
///   `mixed_frame` — one full headless frame with all five decks visible.
///   `text_static`, `text_crawl_40`, `text_step_fade`, `text_step_modulated` —
///   one frame with a single text deck: one line, a 40-line crawl, stepping
///   with a fade, and stepping with size and weight modulated across raster
///   buckets.
///
/// Skipped without a GPU adapter.
use criterion::{Criterion, criterion_group, criterion_main};
use varda::app::VardaApp;
use varda::engine::EngineCommand;
use varda::source::SourceConfig;

const DECKS: usize = 5;

fn frame(app: &mut VardaApp) {
    app.begin_frame();
    app.render_frame();
}

fn app() -> Option<VardaApp> {
    let mut app = varda::testing::headless_app()?;
    let channel = app.build_engine_state().mixer.channels[0].uuid.clone();
    let root = env!("CARGO_MANIFEST_DIR");
    let sources = [
        SourceConfig::new("SolidColor").with("color", [1.0, 0.5, 0.0, 1.0]),
        SourceConfig::new("Image").with("path", format!("{root}/assets/icon.png")),
        SourceConfig::new("Shader").with("name", "bars"),
        SourceConfig::new("Tap").with("source", serde_json::json!({ "kind": "master_program" })),
        SourceConfig::new("Video")
            .with("path", format!("{root}/tests/media/birds_combined_hap.mov")),
    ];
    let sender = app.command_sender();
    for source in sources {
        let _ = sender.send((
            EngineCommand::AddDeck {
                channel_uuid: channel.clone(),
                source,
            },
            None,
        ));
    }
    // Image, shader and video decks build on the background loader.
    for _ in 0..600 {
        frame(&mut app);
        if app.build_engine_state().mixer.channels[0].decks.len() == DECKS {
            return Some(app);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!("deck_sources: decks did not all load, skipping");
    None
}

fn bench_deck_sources(c: &mut Criterion) {
    let Some(mut app) = app() else {
        eprintln!("deck_sources: no GPU adapter, skipping");
        return;
    };
    let mut g = c.benchmark_group("deck_sources");
    g.bench_function("mixed_frame", |b| {
        b.iter(|| frame(std::hint::black_box(&mut app)));
    });
    g.finish();
}

/// An app with one text deck built from `config`, plus an LFO on `modulated`
/// routes of it.
fn text_app(config: &serde_json::Value, modulated: &[&str]) -> Option<VardaApp> {
    let mut app = varda::testing::headless_app()?;
    let channel = app.build_engine_state().mixer.channels[0].uuid.clone();
    let sender = app.command_sender();
    let mut source = SourceConfig::new("Text");
    for (key, value) in config.as_object()? {
        source = source.with(key, value);
    }
    let _ = sender.send((
        EngineCommand::AddDeck {
            channel_uuid: channel,
            source,
        },
        None,
    ));
    if !modulated.is_empty() {
        let _ = sender.send((
            EngineCommand::AddLfo {
                waveform: varda::modulation::LFOWaveform::Sine,
                frequency: 0.5,
            },
            None,
        ));
    }
    for _ in 0..600 {
        frame(&mut app);
        let state = app.build_engine_state();
        if let Some(deck) = state.mixer.channels[0].decks.first() {
            if let Some(lfo) = state.modulation.sources.first() {
                for route in modulated {
                    let _ = sender.send((
                        EngineCommand::AssignModulation {
                            target: format!("deck/{}/{route}", deck.uuid),
                            source_id: lfo.uuid.clone(),
                            amount: 1.0,
                        },
                        None,
                    ));
                }
            }
            // Let the first frames rasterize before measuring.
            for _ in 0..30 {
                frame(&mut app);
            }
            return Some(app);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!("deck_sources: text deck did not load, skipping");
    None
}

fn bench_text_decks(c: &mut Criterion) {
    let lyrics: String = (1..=40)
        .map(|i| format!("line {i} of the lyric sheet goes here"))
        .collect::<Vec<_>>()
        .join("\n");
    let cases: [(&str, serde_json::Value, &[&str]); 4] = [
        (
            "text_static",
            serde_json::json!({ "text": "HEADLINE" }),
            &[],
        ),
        (
            "text_crawl_40",
            serde_json::json!({ "text": lyrics, "mode": "Crawl", "size": 0.05, "speed": 2.0 }),
            &[],
        ),
        (
            "text_step_fade",
            serde_json::json!({ "text": lyrics, "mode": "Step", "speed": 4.0,
                "transition": "Fade", "transition_time": 0.2 }),
            &[],
        ),
        (
            "text_step_modulated",
            serde_json::json!({ "text": "BREATHE", "mode": "Step", "speed": 0.0 }),
            &["size", "weight"],
        ),
    ];
    let mut g = c.benchmark_group("deck_sources");
    for (name, config, modulated) in cases {
        let Some(mut app) = text_app(&config, modulated) else {
            return;
        };
        g.bench_function(name, |b| {
            b.iter(|| frame(std::hint::black_box(&mut app)));
        });
    }
    g.finish();
}

criterion_group!(benches, bench_deck_sources, bench_text_decks);
criterion_main!(benches);
