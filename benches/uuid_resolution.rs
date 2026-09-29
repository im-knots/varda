/// UUID → index resolution cost on the command path.
///
/// Write commands resolve their target UUID to an index right before use, by a
/// linear scan with string comparison, so cost grows with scene size.
///
///   `resolve_channel` — scan over channels.
///   `resolve_deck`    — nested scan over channels × decks.
///   `resolve_effect`  — scan over every chain: master, then each channel's chain,
///                     then each deck's chain. The widest scan.
///
/// Each group measures the worst case: the target is the last entity scanned,
/// and a `_miss` variant scans everything and finds nothing (a stale client
/// UUID). Sizes cover a typical show (2–8 channels) and an extreme one (32).
///
/// The per-frame `tick_sequence` fade resolution is the same channel scan: it
/// costs 2 × `resolve_channel` (from + to) per playing sequence.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::{
    deck::{Deck, Effect},
    isf::ISFShader,
    mixer::Mixer,
    renderer::context::GpuContext,
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;

/// Worst-case budget for one resolution: 1µs is 0.006% of a 60fps frame.
/// Disable with `VARDA_BENCH_SKIP_SLO=1`.
const RESOLVE_BUDGET_NS: u128 = 1_000;

const INVERT_SHADER: &str = include_str!("../shaders/invert.fs");

fn make_context() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

/// A mixer with `n_channels` channels, `decks_per_channel` decks in each, and
/// one effect on every deck, every channel, and the master chain.
fn setup_mixer(ctx: &GpuContext, n_channels: usize, decks_per_channel: usize) -> Mixer {
    let mut mixer = Mixer::new(ctx, WIDTH, HEIGHT).expect("mixer");
    while mixer.channels().len() < n_channels {
        mixer.add_channel(ctx, WIDTH, HEIGHT).expect("add channel");
    }

    let effect = |ctx: &GpuContext| {
        let shader = ISFShader::from_string(INVERT_SHADER).expect("invert shader");
        Effect::new(ctx, shader).expect("effect")
    };

    for ch_idx in 0..n_channels {
        for _ in 0..decks_per_channel {
            let deck = Deck::solid_color(ctx, [0.5, 0.5, 0.5, 1.0], WIDTH, HEIGHT);
            let ch = mixer.channel_mut(ch_idx).expect("channel");
            ch.add_deck(deck);
            let slot = ch.decks.last_mut().expect("deck slot");
            slot.deck.effects.push(effect(ctx));
        }
        let ch = mixer.channel_mut(ch_idx).expect("channel");
        ch.add_effect(effect(ctx));
    }
    mixer.master_effects_mut().push(effect(ctx));

    mixer
}

/// UUID of the last channel, the far end of the scan.
fn last_channel_uuid(mixer: &Mixer) -> String {
    mixer
        .channels()
        .last()
        .expect("at least one channel")
        .uuid()
        .to_string()
}

/// UUID of the last deck in the last channel, the far end of the nested scan.
fn last_deck_uuid(mixer: &Mixer) -> String {
    mixer
        .channels()
        .last()
        .expect("at least one channel")
        .decks
        .last()
        .expect("at least one deck")
        .deck
        .uuid()
        .to_string()
}

/// UUID of the effect the chain walk reaches last.
fn last_effect_uuid(mixer: &Mixer) -> String {
    mixer
        .channels()
        .last()
        .expect("at least one channel")
        .decks
        .last()
        .expect("at least one deck")
        .deck
        .effects
        .last()
        .expect("at least one effect")
        .uuid()
        .to_owned()
}

/// Preflight: assert worst-case resolution in an extreme scene fits the budget.
fn preflight_slo(ctx: &GpuContext) {
    if std::env::var_os("VARDA_BENCH_SKIP_SLO").is_some() {
        return;
    }
    let mixer = setup_mixer(ctx, 32, 8);
    let target = last_effect_uuid(&mixer);

    for _ in 0..100 {
        std::hint::black_box(mixer.find_effect_by_uuid(&target));
    }
    let samples = 10_000;
    let t0 = std::time::Instant::now();
    for _ in 0..samples {
        std::hint::black_box(mixer.find_effect_by_uuid(&target));
    }
    let per_call = t0.elapsed().as_nanos() / samples;

    assert!(
        per_call <= RESOLVE_BUDGET_NS,
        "SLO violation: worst-case effect resolution across 32 channels × 8 decks \
         = {per_call}ns, exceeds {RESOLVE_BUDGET_NS}ns. Resolution needs an index."
    );
    eprintln!(
        "preflight: worst-case effect resolution (32ch × 8 decks, 289 effects) \
         = {per_call}ns (budget {RESOLVE_BUDGET_NS}ns)"
    );
}

fn bench_resolve_channel(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    preflight_slo(&ctx);

    let mut group = c.benchmark_group("resolve_channel");
    for n_channels in [2, 8, 32] {
        let mixer = setup_mixer(&ctx, n_channels, 1);
        let target = last_channel_uuid(&mixer);
        group.bench_with_input(
            BenchmarkId::new("channels", n_channels),
            &n_channels,
            |b, _| b.iter(|| std::hint::black_box(mixer.find_channel_by_uuid(&target))),
        );
        group.bench_with_input(
            BenchmarkId::new("channels_miss", n_channels),
            &n_channels,
            |b, _| b.iter(|| std::hint::black_box(mixer.find_channel_by_uuid("deadbeef"))),
        );
    }
    group.finish();
}

fn bench_resolve_deck(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let mut group = c.benchmark_group("resolve_deck");
    for (n_channels, decks) in [(2, 4), (8, 4), (8, 16), (32, 8)] {
        let mixer = setup_mixer(&ctx, n_channels, decks);
        let target = last_deck_uuid(&mixer);
        let label = format!("{n_channels}ch_x_{decks}decks");
        group.bench_with_input(BenchmarkId::new("hit", &label), &label, |b, _| {
            b.iter(|| std::hint::black_box(mixer.find_deck_by_uuid(&target)));
        });
        group.bench_with_input(BenchmarkId::new("miss", &label), &label, |b, _| {
            b.iter(|| std::hint::black_box(mixer.find_deck_by_uuid("deadbeef")));
        });
    }
    group.finish();
}

fn bench_resolve_effect(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let mut group = c.benchmark_group("resolve_effect");
    for (n_channels, decks) in [(2, 4), (8, 4), (8, 16), (32, 8)] {
        let mixer = setup_mixer(&ctx, n_channels, decks);
        let target = last_effect_uuid(&mixer);
        let label = format!("{n_channels}ch_x_{decks}decks");
        group.bench_with_input(BenchmarkId::new("hit", &label), &label, |b, _| {
            b.iter(|| std::hint::black_box(mixer.find_effect_by_uuid(&target)));
        });
        group.bench_with_input(BenchmarkId::new("miss", &label), &label, |b, _| {
            b.iter(|| std::hint::black_box(mixer.find_effect_by_uuid("deadbeef")));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_resolve_channel,
    bench_resolve_deck,
    bench_resolve_effect
);
criterion_main!(benches);
