/// Cost of producing and reading `EngineState`, the snapshot every consumer
/// reads. See /spec/state-publication.md.
///
///   `build`         — `VardaApp::build_engine_state` on a representative scene.
///   `deep_clone`    — what a REST read and a WebSocket tick pay today.
///   `arc_clone`     — what they pay once the snapshot is shared.
///   `to_json`       — serializing the snapshot, once per client per tick today.
///   `diff_unchanged`— a WebSocket tick that finds nothing new, as it runs today.
///
/// With a shared publication (`StatePublication`):
///
///   `published_tick_unchanged` — a WebSocket tick with no new generation.
///   `published_tick_changed`   — a tick with one: a diff of two shared values.
///   `published_rest_read`      — a REST read of the cached full-state JSON.
///   `publish`                  — handing a built snapshot to the publication.
///
/// `build` against `deep_clone` also settles the GUI option in the spec: sharing
/// one build costs the GUI a clone of what it now moves out of an owned state.
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use varda::app::publish::StatePublication;
use varda::app::{AppConfig, VardaApp};
use varda::engine::{CommandResult, EngineCommand};
use varda::modulation::LFOWaveform;
use varda::renderer::context::GpuContext;

use clap::Parser;

/// Four channels of four decks each, and four LFOs: about the size of a
/// working set.
fn scene() -> Option<VardaApp> {
    let gpu = GpuContext::new_headless().ok()?;
    let config =
        AppConfig::parse_from(["varda", "--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
    let mut app = VardaApp::new(gpu, &config).ok()?;
    let send = |app: &mut VardaApp, cmd: EngineCommand| {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.command_sender().send((cmd, Some(tx))).ok()?;
        app.process_commands();
        rx.blocking_recv().ok()
    };
    for _ in 0..2 {
        send(&mut app, EngineCommand::AddChannel)?;
    }
    let channels: Vec<String> = app
        .build_engine_state()
        .mixer
        .channels
        .iter()
        .map(|c| c.uuid.clone())
        .collect();
    for channel_uuid in channels {
        for i in 0..4 {
            let result = send(
                &mut app,
                EngineCommand::AddSolidColorDeck {
                    channel_uuid: channel_uuid.clone(),
                    color: [i as f32 / 4.0, 0.5, 0.5, 1.0],
                },
            )?;
            if !matches!(result, CommandResult::OkWithId { .. }) {
                return None;
            }
        }
    }
    for i in 0..4 {
        send(
            &mut app,
            EngineCommand::AddLfo {
                waveform: LFOWaveform::Sine,
                frequency: 0.25 * (i + 1) as f32,
            },
        )?;
    }
    Some(app)
}

fn bench_snapshot(c: &mut Criterion) {
    let Some(app) = scene() else {
        eprintln!("no GPU adapter — skipping engine_snapshot");
        return;
    };
    let state = app.build_engine_state();
    let shared = Arc::new(state.clone());
    let json = serde_json::to_value(&state).expect("state serializes");

    let mut group = c.benchmark_group("engine_snapshot");
    group.bench_function("build", |b| b.iter(|| black_box(app.build_engine_state())));
    group.bench_function("deep_clone", |b| b.iter(|| black_box(state.clone())));
    group.bench_function("arc_clone", |b| b.iter(|| black_box(Arc::clone(&shared))));
    group.bench_function("to_json", |b| {
        b.iter(|| black_box(serde_json::to_value(&state).expect("state serializes")));
    });
    group.bench_function("diff_unchanged", |b| {
        b.iter(|| black_box(json_patch::diff(&json, &json)));
    });

    let publication = StatePublication::with_state(state.clone());
    let last = publication.latest().expect("published");
    let _ = last.json();
    group.bench_function("published_tick_unchanged", |b| {
        b.iter(|| {
            let current = publication.latest().expect("published");
            black_box(current.generation != last.generation)
        });
    });
    publication.publish(state.clone());
    let next = publication.latest().expect("published");
    let _ = next.json();
    group.bench_function("published_tick_changed", |b| {
        b.iter(|| black_box(json_patch::diff(last.json(), next.json())));
    });
    group.bench_function("published_rest_read", |b| {
        b.iter(|| {
            let current = publication.latest().expect("published");
            black_box(current.json().is_object())
        });
    });
    group.bench_function("publish", |b| {
        b.iter_batched(
            || state.clone(),
            |s| publication.publish(s),
            criterion::BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_snapshot);
criterion_main!(benches);
