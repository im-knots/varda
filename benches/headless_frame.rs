/// Per-frame cost of the engine's own frame sequence with no window: timing,
/// notifications, the command drain, inputs, then rendering the mixer, the
/// outputs, and the interactive window. The default two-channel scene with one
/// solid deck, so what is measured is the sequence and its fixed costs.
/// Skipped when no GPU adapter is available.
use criterion::{Criterion, criterion_group, criterion_main};
use varda::app::VardaApp;
use varda::engine::EngineCommand;
use varda::renderer::context::GpuContext;

fn app() -> Option<VardaApp> {
    let gpu = GpuContext::new_headless().ok()?;
    let mut app = VardaApp::new(gpu, &varda::testing::headless_config()).ok()?;
    let channel = app.build_engine_state().mixer.channels[0].uuid.clone();
    let sender = app.command_sender();
    let _ = sender.send((
        EngineCommand::AddSolidColorDeck {
            channel_uuid: channel,
            color: [1.0, 0.5, 0.0, 1.0],
        },
        None,
    ));
    frame(&mut app);
    Some(app)
}

fn frame(app: &mut VardaApp) {
    app.begin_frame();
    app.render_frame();
}

fn bench_headless_frame(c: &mut Criterion) {
    let Some(mut app) = app() else {
        eprintln!("headless_frame: no GPU adapter, skipping");
        return;
    };
    let mut g = c.benchmark_group("headless_frame");
    g.bench_function("default_scene", |b| {
        b.iter(|| frame(std::hint::black_box(&mut app)));
    });
    g.finish();
}

criterion_group!(benches, bench_headless_frame);
criterion_main!(benches);
