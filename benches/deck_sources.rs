/// Per-frame cost of a scene holding one deck of each source kind that needs
/// no hardware: solid color, raster image, ISF generator, program tap, and HAP
/// video. What it isolates is the per-deck source dispatch in the render path,
/// on top of the fixed frame sequence `headless_frame` measures.
/// See /spec/deck-source-providers.md § Performance.
///
///   `mixed_frame` — one full headless frame with all five decks visible.
///
/// Skipped when no GPU adapter is available.
use criterion::{Criterion, criterion_group, criterion_main};
use varda::app::VardaApp;
use varda::engine::EngineCommand;
use varda::renderer::context::GpuContext;
use varda::source::SourceConfig;

const DECKS: usize = 5;

fn frame(app: &mut VardaApp) {
    app.begin_frame();
    app.render_frame();
}

fn app() -> Option<VardaApp> {
    let gpu = GpuContext::new_headless().ok()?;
    let mut app = VardaApp::new(gpu, &varda::testing::headless_config()).ok()?;
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

criterion_group!(benches, bench_deck_sources);
criterion_main!(benches);
