/// GPU compositing benchmarks at 1080p.
///
///   `channel_composite_solid`  — solid-color decks (`LoadOp::Clear`, no fragment shader).
///                              The slope across deck counts isolates per-deck
///                              copy-on-composite cost. The level at decks/1
///                              includes fixed render-pass setup, not just
///                              compositing.
///
///   `channel_composite_shader` — same shape but with bars.fs running on every
///                              pixel. Difference vs solid at N decks ≈
///                              N × per-deck shader execution cost.
///
///   `mixer_crossfade`          — two channels through the crossfader at 50%.
///
///   `composite_resolution`     — solid decks at 1080p and 4K. Compositing traffic
///                              scales with texel count, so the 4K/1080p ratio at
///                              a fixed deck count isolates how bandwidth-bound
///                              the stage is. Solid decks keep fragment cost flat
///                              so the difference is data movement, not shading.
///
/// After the criterion groups complete, the per-deck slope (decks/8 minus
/// decks/1, divided by 7) is computed from fresh samples and printed.
use std::time::Instant;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::{
    BlendMode,
    audio::AudioData,
    deck::Deck,
    isf::ISFShader,
    mixer::{FrameInputs, Mixer},
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
};

/// Silent audio, no modulation, no clock: the benchmark measures compositing,
/// not signal handling.
fn frame_inputs<'a>(
    audio_data: &'a AudioData,
    audio_values: &'a AudioValues,
    analyzer_values: &'a AnalyzerValues,
) -> FrameInputs<'a> {
    FrameInputs {
        audio_data,
        audio_values,
        analyzer_values,
        beat_time: None,
        transport: None,
        free_run_time: None,
    }
}

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

const UHD_WIDTH: u32 = 3840;
const UHD_HEIGHT: u32 = 2160;

/// 60fps frame budget. Preflight panics if 8-deck solid composite at 1080p
/// exceeds this. Disable with `VARDA_BENCH_SKIP_SLO=1`.
const FRAME_BUDGET_US: u128 = 16_670;

const BARS_SHADER: &str = include_str!("../shaders/bars.fs");

/// Generator with one `PERSISTENT` pass — the shape that forces per-iteration
/// submits in `render_multi_pass`.
const TURING_SHADER: &str = include_str!("../shaders/turing_patterns.fs");

fn make_context() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

fn poll(ctx: &GpuContext) {
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
}

fn setup_mixer_solid_at(context: &GpuContext, n_decks: usize, width: u32, height: u32) -> Mixer {
    let mut mixer = Mixer::new(context, width, height).expect("mixer");
    let ch = mixer.channel_mut(0).expect("channel 0");
    for i in 0..n_decks {
        let t = i as f32 / n_decks.max(1) as f32;
        let deck = Deck::new_solid_color(context, [t, 0.5, 1.0 - t, 1.0], width, height)
            .expect("solid color deck");
        ch.add_deck(deck);
    }
    mixer
}

fn setup_mixer_solid(context: &GpuContext, n_decks: usize) -> Mixer {
    setup_mixer_solid_at(context, n_decks, WIDTH, HEIGHT)
}

fn setup_mixer_shader(context: &GpuContext, n_decks: usize) -> Mixer {
    let mut mixer = Mixer::new(context, WIDTH, HEIGHT).expect("mixer");
    let ch = mixer.channel_mut(0).expect("channel 0");
    for _ in 0..n_decks {
        let shader = ISFShader::from_string(BARS_SHADER).expect("bars shader");
        let deck = Deck::new(context, shader, WIDTH, HEIGHT).expect("shader deck");
        ch.add_deck(deck);
    }
    mixer
}

fn time_render_us(ctx: &GpuContext, mixer: &mut Mixer, samples: usize) -> u128 {
    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();
    for _ in 0..3 {
        mixer
            .render(
                ctx,
                &frame_inputs(&audio, &audio_values, &analyzer_values),
                60,
                &[],
            )
            .expect("warmup");
        poll(ctx);
    }
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let t0 = Instant::now();
        mixer
            .render(
                ctx,
                &frame_inputs(&audio, &audio_values, &analyzer_values),
                60,
                &[],
            )
            .expect("render");
        poll(ctx);
        times.push(t0.elapsed().as_micros());
    }
    times.sort_unstable();
    times[samples / 2]
}

/// Preflight: assert the 8-deck render fits the 60fps budget. Runs once
/// before any criterion sampling. Skipped when `VARDA_BENCH_SKIP_SLO` is set.
fn preflight_slo(ctx: &GpuContext) {
    if std::env::var_os("VARDA_BENCH_SKIP_SLO").is_some() {
        return;
    }
    let mut mixer = setup_mixer_solid(ctx, 8);
    let median = time_render_us(ctx, &mut mixer, 11);
    assert!(
        median <= FRAME_BUDGET_US,
        "SLO violation: 8-deck solid composite at {WIDTH}x{HEIGHT} \
         median = {median}µs, exceeds 60fps budget {FRAME_BUDGET_US}µs"
    );
    eprintln!("preflight: 8-deck solid composite median = {median}µs (budget {FRAME_BUDGET_US}µs)");
}

fn bench_channel_composite_solid(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    preflight_slo(&ctx);

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let mut group = c.benchmark_group("channel_composite_solid");
    group.sample_size(50);

    for n_decks in [1, 2, 4, 8] {
        let mut mixer = setup_mixer_solid(&ctx, n_decks);
        group.bench_with_input(BenchmarkId::new("decks", n_decks), &n_decks, |b, _| {
            b.iter(|| {
                mixer
                    .render(
                        &ctx,
                        &frame_inputs(&audio, &audio_values, &analyzer_values),
                        60,
                        &[],
                    )
                    .expect("render");
                poll(&ctx);
            });
        });
    }

    group.finish();
}

/// Cost of a single full-frame `copy_texture_to_texture` between two
/// `Rgba16Float` compositing textures, isolated from the render path.
///
/// Compositing currently issues one such copy per layer above the first,
/// plus one each for tonemap and LUT. Whole-frame benchmarks cannot resolve
/// that cost — it sits below the run-to-run noise floor of a laptop under
/// thermal drift. Batching `COPIES_PER_ITER` copies into one submit amortises
/// encode/submit/poll so the measurement is dominated by the copies
/// themselves; divide the reported time by `COPIES_PER_ITER` for per-copy cost.
const COPIES_PER_ITER: usize = 32;

fn bench_raw_copy_cost(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let mut group = c.benchmark_group("raw_copy_cost");
    group.sample_size(50);

    // Distinct destinations break the write-after-write chain that a single
    // dst imposes, separating serialisation latency from real per-copy cost.
    let dsts: Vec<wgpu::Texture> = (0..8)
        .map(|_| ctx.create_compositing_texture(WIDTH, HEIGHT))
        .collect();
    let src_1080 = ctx.create_compositing_texture(WIDTH, HEIGHT);
    group.bench_function(
        BenchmarkId::new("1080p_distinct_dst", COPIES_PER_ITER),
        |b| {
            b.iter(|| {
                let mut encoder =
                    ctx.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Raw Copy Bench (distinct dst)"),
                        });
                for i in 0..COPIES_PER_ITER {
                    encoder.copy_texture_to_texture(
                        src_1080.as_image_copy(),
                        dsts[i % dsts.len()].as_image_copy(),
                        src_1080.size(),
                    );
                }
                ctx.queue.submit(Some(encoder.finish()));
                poll(&ctx);
            });
        },
    );

    for (label, w, h) in [("1080p", WIDTH, HEIGHT), ("4k", UHD_WIDTH, UHD_HEIGHT)] {
        let src = ctx.create_compositing_texture(w, h);
        let dst = ctx.create_compositing_texture(w, h);
        group.bench_function(BenchmarkId::new(label, COPIES_PER_ITER), |b| {
            b.iter(|| {
                let mut encoder =
                    ctx.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Raw Copy Bench"),
                        });
                for _ in 0..COPIES_PER_ITER {
                    encoder.copy_texture_to_texture(
                        src.as_image_copy(),
                        dst.as_image_copy(),
                        src.size(),
                    );
                }
                ctx.queue.submit(Some(encoder.finish()));
                poll(&ctx);
            });
        });
    }

    group.finish();
}

fn bench_composite_resolution(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let mut group = c.benchmark_group("composite_resolution");
    group.sample_size(30);

    for (label, w, h) in [("1080p", WIDTH, HEIGHT), ("4k", UHD_WIDTH, UHD_HEIGHT)] {
        for n_decks in [4usize, 8] {
            let mut mixer = setup_mixer_solid_at(&ctx, n_decks, w, h);
            group.bench_with_input(BenchmarkId::new(label, n_decks), &n_decks, |b, _| {
                b.iter(|| {
                    mixer
                        .render(
                            &ctx,
                            &frame_inputs(&audio, &audio_values, &analyzer_values),
                            60,
                            &[],
                        )
                        .expect("render");
                    poll(&ctx);
                });
            });
        }
    }

    group.finish();
}

fn bench_channel_composite_shader(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let mut group = c.benchmark_group("channel_composite_shader");
    group.sample_size(50);

    for n_decks in [1, 2, 4, 8] {
        let mut mixer = setup_mixer_shader(&ctx, n_decks);
        group.bench_with_input(BenchmarkId::new("decks", n_decks), &n_decks, |b, _| {
            b.iter(|| {
                mixer
                    .render(
                        &ctx,
                        &frame_inputs(&audio, &audio_values, &analyzer_values),
                        60,
                        &[],
                    )
                    .expect("render");
                poll(&ctx);
            });
        });
    }

    group.finish();
}

fn bench_mixer_crossfade(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let mut mixer = setup_mixer_solid(&ctx, 1);
    let deck =
        Deck::new_solid_color(&ctx, [0.0, 1.0, 0.5, 1.0], WIDTH, HEIGHT).expect("solid color deck");
    mixer.channel_mut(1).unwrap().add_deck(deck);
    mixer.set_crossfader(0.5);

    let mut group = c.benchmark_group("mixer_crossfade");
    group.sample_size(50);
    group.bench_function("2ch_50pct", |b| {
        b.iter(|| {
            mixer
                .render(
                    &ctx,
                    &frame_inputs(&audio, &audio_values, &analyzer_values),
                    60,
                    &[],
                )
                .expect("render");
            poll(&ctx);
        });
    });
    group.finish();
}

/// Solid decks with a pivot blend mode on every layer above the base.
///
/// Overlay/Hard Light/Soft Light encode their operands before the blend math
/// (spec/blend-modes.md § Blend Space), so they carry two `pow`s per channel
/// that the physical modes do not. This group isolates that cost — compare
/// against `channel_composite_solid` at the same deck count for the delta.
fn setup_mixer_blend(context: &GpuContext, n_decks: usize, mode: BlendMode) -> Mixer {
    let mut mixer = Mixer::new(context, WIDTH, HEIGHT).expect("mixer");
    let ch = mixer.channel_mut(0).expect("channel 0");
    for i in 0..n_decks {
        let t = i as f32 / n_decks.max(1) as f32;
        let deck = Deck::new_solid_color(context, [t, 0.5, 1.0 - t, 1.0], WIDTH, HEIGHT)
            .expect("solid color deck");
        ch.add_deck(deck);
        // Deck 0 is a plain blit (LoadOp::Clear); only layers above it blend.
        if i > 0 {
            ch.set_deck_blend_mode(i, mode);
        }
    }
    mixer
}

fn bench_channel_composite_blend(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let mut group = c.benchmark_group("channel_composite_blend");
    group.sample_size(50);

    for (label, mode) in [
        ("overlay", BlendMode::Overlay),
        ("soft_light", BlendMode::SoftLight),
        ("screen", BlendMode::Screen),
    ] {
        for n_decks in [4, 8] {
            let mut mixer = setup_mixer_blend(&ctx, n_decks, mode);
            group.bench_with_input(BenchmarkId::new(label, n_decks), &n_decks, |b, _| {
                b.iter(|| {
                    mixer
                        .render(
                            &ctx,
                            &frame_inputs(&audio, &audio_values, &analyzer_values),
                            60,
                            &[],
                        )
                        .expect("render");
                    poll(&ctx);
                });
            });
        }
    }

    group.finish();
}

/// Decks running a shader with a `PERSISTENT` pass.
///
/// `render_multi_pass` runs `SIMULATION_ITERATIONS` (4) iterations per
/// persistent pass and calls `queue.submit()` inside that loop, because
/// `UnifiedPipeline` owns a single uniform buffer that each iteration
/// overwrites. So every such deck costs 4 command buffer commits per frame on
/// top of the batched deck/composite submits. Compare against
/// `channel_composite_shader` at the same deck count for the delta.
fn setup_mixer_multipass(context: &GpuContext, n_decks: usize, src: &str) -> Mixer {
    let mut mixer = Mixer::new(context, WIDTH, HEIGHT).expect("mixer");
    let ch = mixer.channel_mut(0).expect("channel 0");
    for _ in 0..n_decks {
        let shader = ISFShader::from_string(src).expect("multipass shader");
        let deck = Deck::new(context, shader, WIDTH, HEIGHT).expect("multipass deck");
        ch.add_deck(deck);
    }
    mixer
}

fn bench_channel_composite_multipass(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let mut group = c.benchmark_group("channel_composite_multipass");
    group.sample_size(30);

    for n_decks in [1, 2, 4] {
        let mut mixer = setup_mixer_multipass(&ctx, n_decks, TURING_SHADER);
        group.bench_with_input(BenchmarkId::new("decks", n_decks), &n_decks, |b, _| {
            b.iter(|| {
                mixer
                    .render(
                        &ctx,
                        &frame_inputs(&audio, &audio_values, &analyzer_values),
                        60,
                        &[],
                    )
                    .expect("render");
                poll(&ctx);
            });
        });
    }

    group.finish();
}

/// Prints command buffer commits per `Mixer::render` for a few configurations.
///
/// Mixer scope only — the output, preview, and present submits happen in the
/// frame loop above the mixer and are not counted here. Read alongside the
/// `[PERF] frame | submits=` log, which covers a whole frame.
fn report_submits_per_frame(_c: &mut Criterion) {
    let Some(ctx) = make_context() else { return };
    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    let measure = |label: &str, mixer: &mut Mixer| {
        // Warm up: first frames allocate pass buffers and pipelines.
        for _ in 0..3 {
            mixer
                .render(
                    &ctx,
                    &frame_inputs(&audio, &audio_values, &analyzer_values),
                    60,
                    &[],
                )
                .expect("warmup");
            poll(&ctx);
        }
        ctx.submits.take();
        mixer
            .render(
                &ctx,
                &frame_inputs(&audio, &audio_values, &analyzer_values),
                60,
                &[],
            )
            .expect("render");
        poll(&ctx);
        eprintln!(
            "submits/frame (mixer scope) — {label}: {}",
            ctx.submits.take()
        );
    };

    measure("1 solid deck", &mut setup_mixer_solid(&ctx, 1));
    measure("8 solid decks, 1 channel", &mut setup_mixer_solid(&ctx, 8));
    measure("4 shader decks", &mut setup_mixer_shader(&ctx, 4));
    measure(
        "1 multipass deck (turing_patterns)",
        &mut setup_mixer_multipass(&ctx, 1, TURING_SHADER),
    );
    measure(
        "4 multipass decks (turing_patterns)",
        &mut setup_mixer_multipass(&ctx, 4, TURING_SHADER),
    );

    let mut two_ch = setup_mixer_solid(&ctx, 4);
    for _ in 0..4 {
        let deck = Deck::new_solid_color(&ctx, [0.0, 1.0, 0.5, 1.0], WIDTH, HEIGHT).expect("deck");
        two_ch.channel_mut(1).unwrap().add_deck(deck);
    }
    two_ch.set_crossfader(0.5);
    measure("2 channels x 4 solid decks", &mut two_ch);
}

/// Re-times solid composites at 1 and 8 decks (warmed, median of 11) and
/// prints the per-deck slope to stderr.
fn report_per_deck_slope(_c: &mut Criterion) {
    let Some(ctx) = make_context() else { return };
    let mut m1 = setup_mixer_solid(&ctx, 1);
    let mut m8 = setup_mixer_solid(&ctx, 8);
    let t1 = time_render_us(&ctx, &mut m1, 11);
    let t8 = time_render_us(&ctx, &mut m8, 11);
    let slope =
        (i128::try_from(t8).unwrap_or(i128::MAX) - i128::try_from(t1).unwrap_or(i128::MAX)) / 7;
    eprintln!(
        "per-deck copy-on-composite slope (solid, {WIDTH}x{HEIGHT}): \
         {slope}µs/deck   [decks/1={t1}µs, decks/8={t8}µs]"
    );
}

criterion_group!(
    benches,
    bench_channel_composite_solid,
    bench_composite_resolution,
    bench_raw_copy_cost,
    bench_channel_composite_shader,
    bench_channel_composite_blend,
    bench_channel_composite_multipass,
    bench_mixer_crossfade,
    report_submits_per_frame,
    report_per_deck_slope,
);
criterion_main!(benches);
