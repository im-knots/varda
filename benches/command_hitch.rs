//! Render-thread cost of commands that build a shader, on a headless engine
//! with one deck.
//!
//!   `add_effect`          — the `AddEffect` command itself.
//!   `add_effect_ready`    — from the command until the effect draws.
//!   `undo_effect_removal` — undoing an effect's removal, which rebuilds it.
//!
//! Skipped without a GPU adapter.
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use varda::testing::CommandHitchBench;

const SHADER: &str = "invert";

fn command_hitch(c: &mut Criterion) {
    let Some(mut bench) = CommandHitchBench::new() else {
        eprintln!("command_hitch: no GPU adapter, skipping");
        return;
    };
    let mut g = c.benchmark_group("command_hitch");
    g.sample_size(20);
    g.bench_function("add_effect", |b| {
        b.iter_custom(|iterations| {
            (0..iterations)
                .map(|_| bench.add_effect(SHADER).0)
                .sum::<Duration>()
        });
    });
    g.bench_function("add_effect_ready", |b| {
        b.iter_custom(|iterations| {
            (0..iterations)
                .map(|_| bench.add_effect(SHADER).1)
                .sum::<Duration>()
        });
    });
    g.bench_function("undo_effect_removal", |b| {
        b.iter_custom(|iterations| {
            (0..iterations)
                .map(|_| bench.undo_effect_removal(SHADER))
                .sum::<Duration>()
        });
    });
    g.finish();
}

criterion_group!(benches, command_hitch);
criterion_main!(benches);
