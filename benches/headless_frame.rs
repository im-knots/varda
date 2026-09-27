/// Per-frame cost of the engine's own frame sequence with no window: timing,
/// notifications, the command drain, inputs, then rendering the mixer, the
/// outputs, and the interactive window.
///
///   `default_scene` — the default two-channel scene with one solid deck, so
///                     what is measured is the sequence and its fixed costs.
///   `syphon_output` — the default scene plus one live Syphon output showing
///                     the whole program (macOS only).
///   `syphon_output_surface` — the same output with one surface assigned, so
///                     the output composes surfaces before delivering.
///   `heavy_modulation` — the same scene with 128 LFOs, each modulating the
///                     next one's frequency and all driving the channel's
///                     opacity. See /spec/performance-hot-paths.md item G.
///
/// Skipped when no GPU adapter is available.
use criterion::{Criterion, criterion_group, criterion_main};
use varda::app::VardaApp;
use varda::engine::EngineCommand;
use varda::modulation::LFOWaveform;
use varda::renderer::context::GpuContext;

fn app() -> Option<VardaApp> {
    let gpu = GpuContext::new_headless().ok()?;
    let mut app = VardaApp::new(gpu, &varda::testing::headless_config()).ok()?;
    let channel = app.build_engine_state().mixer.channels[0].uuid.clone();
    let sender = app.command_sender();
    let _ = sender.send((
        EngineCommand::AddDeck {
            channel_uuid: channel,
            source: varda::solid_color::SolidColor::config_for([1.0, 0.5, 0.0, 1.0]),
        },
        None,
    ));
    frame(&mut app);
    Some(app)
}

/// The default scene plus 128 chained LFOs.
fn heavy_modulation_app() -> Option<VardaApp> {
    const SOURCES: usize = 128;
    let mut app = app()?;
    let sender = app.command_sender();
    for i in 0..SOURCES {
        let _ = sender.send((
            EngineCommand::AddLfo {
                waveform: LFOWaveform::Sine,
                frequency: 0.1 + i as f32 * 0.01,
            },
            None,
        ));
    }
    frame(&mut app);
    let state = app.build_engine_state();
    let opacity = format!("ch/{}/opacity", state.mixer.channels[0].uuid);
    let ids: Vec<String> = state
        .modulation
        .sources
        .iter()
        .map(|s| s.uuid.clone())
        .collect();
    assert_eq!(ids.len(), SOURCES);
    for (i, id) in ids.iter().enumerate() {
        let _ = sender.send((
            EngineCommand::AssignModulation {
                target: opacity.clone(),
                source_id: id.clone(),
                amount: 0.01,
            },
            None,
        ));
        if i > 0 {
            let _ = sender.send((
                EngineCommand::AssignModOnMod {
                    target_source_id: id.clone(),
                    param_name: "frequency".to_string(),
                    modulator_id: ids[i - 1].clone(),
                    amount: 0.1,
                },
                None,
            ));
        }
    }
    frame(&mut app);
    Some(app)
}

/// The default scene plus one live Syphon output, with one surface assigned
/// when `surface` is set. Syphon is the output a test machine can always run.
#[cfg(target_os = "macos")]
fn output_app(surface: bool) -> Option<VardaApp> {
    use clap::Parser;
    let gpu = GpuContext::new_headless().ok()?;
    let workspace = varda::testing::temp_workspace();
    let config = varda::app::AppConfig::parse_from([
        "varda",
        "--headless",
        "--no-osc",
        "--no-ndi",
        "--workspace",
        &workspace,
    ]);
    let mut app = VardaApp::new(gpu, &config).ok()?;
    let channel = app.build_engine_state().mixer.channels[0].uuid.clone();
    let sender = app.command_sender();
    let _ = sender.send((
        EngineCommand::AddDeck {
            channel_uuid: channel,
            source: varda::solid_color::SolidColor::config_for([1.0, 0.5, 0.0, 1.0]),
        },
        None,
    ));
    let _ = sender.send((
        EngineCommand::CreateOutput {
            sink: varda::engine::value::provider::ProviderConfig::new("syphon_server")
                .with("server_name", "Varda Bench"),
        },
        None,
    ));
    frame(&mut app);
    let outputs = app.build_engine_state().outputs;
    let output = outputs.windows.first()?.uuid.clone();
    if surface {
        let _ = sender.send((
            EngineCommand::AddSurface {
                name: "Bench".to_string(),
                source: varda::engine::value::render::OutputSource::Master,
            },
            None,
        ));
        frame(&mut app);
        let surface = app
            .build_engine_state()
            .outputs
            .surfaces
            .first()?
            .uuid
            .clone();
        let _ = sender.send((
            EngineCommand::AssignSurfaceToOutput {
                output_uuid: output,
                surface_uuid: surface,
            },
            None,
        ));
    }
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
    #[cfg(target_os = "macos")]
    for (name, surface) in [("syphon_output", false), ("syphon_output_surface", true)] {
        if let Some(mut app) = output_app(surface) {
            g.bench_function(name, |b| {
                b.iter(|| frame(std::hint::black_box(&mut app)));
            });
        }
    }
    if let Some(mut heavy) = heavy_modulation_app() {
        g.bench_function("heavy_modulation", |b| {
            b.iter(|| frame(std::hint::black_box(&mut heavy)));
        });
    }
    g.finish();
}

criterion_group!(benches, bench_headless_frame);
criterion_main!(benches);
