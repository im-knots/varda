//! Headless GPU benchmark for the fractal explorer's host-perturbation path.
//!
//! This bypasses `Mixer` frame skipping and blocks on the exact submission each
//! frame produced, so the measured duration follows shader work rather than the
//! adaptive frame budget or an unrelated earlier submission.
//!
//! Only the shipped showcase configuration is timed: the certifiable Mandelbulb
//! flight with a mandatory host reference-orbit payload. If the host
//! preprocessor will not publish a payload the benchmark fails, because a
//! fallback timing measures a different renderer than the one being reported.

use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use std::hint::black_box;
use std::time::{Duration, Instant};
use varda::{
    audio::AudioData, deck::Deck, isf::ISFShader, modulation::ModulationEngine,
    renderer::context::GpuContext,
};

const SHADER: &str = include_str!("../shaders/fractal_explorer.fs");
const WIDTH_720P: u32 = 1280;
const HEIGHT_720P: u32 = 720;
const WIDTH_1080P: u32 = 1920;
const HEIGHT_1080P: u32 = 1080;
const WARMUP_FRAMES: usize = 3;
const DEFAULT_PREPROCESSOR_TIMEOUT: Duration = Duration::from_secs(120);
const ZOOM_EXPONENTS: [f32; 2] = [6.0, 12.0];

fn render_once(context: &GpuContext, deck: &mut Deck) {
    let mut command_buffers = Vec::new();
    deck.render(
        context,
        &AudioData::default(),
        &ModulationEngine::new(),
        0,
        &mut command_buffers,
    )
    .expect("fractal deck render");
    assert!(
        deck.gpu_error().is_none(),
        "fractal deck is quarantined and replays its last frame instead of rendering: {}",
        deck.gpu_error().unwrap_or_default()
    );
    assert!(
        !command_buffers.is_empty(),
        "fractal deck encoded no GPU work; the timing would measure an empty submission"
    );

    // Waiting on the most recent submission rather than this one lets the wait
    // resolve against work that was already complete, which reports a fraction
    // of the real frame cost. Block on the index this submit returned.
    let submission = context.queue.submit(command_buffers);
    context
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .expect("wait for the fractal deck submission to complete");
}

fn make_deck(
    context: &GpuContext,
    width: u32,
    height: u32,
    configure: impl FnOnce(&mut Deck),
) -> Deck {
    let shader = ISFShader::from_string(SHADER).expect("fractal shader");
    let mut deck = Deck::new(context, shader, width, height).expect("fractal deck");
    configure(&mut deck);
    deck.start_declared_preprocessors();
    deck
}

fn warm_up(context: &GpuContext, deck: &mut Deck) {
    for _ in 0..WARMUP_FRAMES {
        render_once(context, deck);
    }
}

/// Pin the camera and select the host-perturbation showcase at a fixed depth.
fn configure_showcase(deck: &mut Deck, zoom: f32) {
    deck.generator_params.set_float("fly_speed", 0.0);
    deck.generator_params.set_float("dz_zoom_exp", zoom);
}

fn configure_full_beauty(deck: &mut Deck) {
    deck.generator_params.set_long("dz_debug", 0);
    for (name, value) in [
        ("ao_strength", 0.75),
        ("shadow_strength", 0.55),
        ("fog_amount", 0.55),
        ("emissive", 0.4),
        ("head_light", 0.45),
        ("fog_iter_amount", 0.9),
        ("star_amount", 0.35),
        ("dof_amount", 0.55),
        ("bloom", 0.7),
        ("aberration", 0.18),
        ("ghost", 0.14),
        ("motion_blur", 0.15),
        ("vignette", 0.4),
        ("clarity", 0.25),
        ("shafts", 0.25),
    ] {
        deck.generator_params.set_float(name, value);
    }
}

fn configure_core_march_diagnostic(deck: &mut Deck) {
    // `dz_debug` returns immediately after the primary march, before normals,
    // shadows, lighting, fog, and surface shading. The remaining zeroes make the
    // unavoidable second ISF pass a minimal scene-buffer sample.
    deck.generator_params.set_long("dz_debug", 1);
    for name in [
        "ao_strength",
        "shadow_strength",
        "fog_amount",
        "emissive",
        "head_light",
        "fog_iter_amount",
        "star_amount",
        "dof_amount",
        "bloom",
        "aberration",
        "ghost",
        "motion_blur",
        "vignette",
        "clarity",
        "shafts",
    ] {
        deck.generator_params.set_float(name, 0.0);
    }
}

/// Every optional shading and lens term at zero, with the full pipeline still
/// running. This is the floor the bisect measures each term against.
fn configure_all_shading_off(deck: &mut Deck) {
    deck.generator_params.set_long("dz_debug", 0);
    for name in SHADING_TERMS {
        deck.generator_params.set_float(name, 0.0);
    }
}

const SHADING_TERMS: [&str; 15] = [
    "ao_strength",
    "shadow_strength",
    "fog_amount",
    "emissive",
    "head_light",
    "fog_iter_amount",
    "star_amount",
    "dof_amount",
    "bloom",
    "aberration",
    "ghost",
    "motion_blur",
    "vignette",
    "clarity",
    "shafts",
];

/// Marginal cost of each shading term, one at a time against a common floor.
///
/// The full beauty frame measured about twenty-five times the core march, so
/// the frame is dominated by something other than the fractal. Enabling terms
/// individually attributes that cost instead of inferring it: each row is the
/// floor plus exactly one term, so subtracting the floor gives what that term
/// charges per frame.
fn benchmark_shading_bisect(
    group: &mut BenchmarkGroup<'_, WallTime>,
    context: &GpuContext,
    deck: &mut Deck,
    prefix: &str,
) {
    let terms: [(&str, &[(&str, f32)]); 9] = [
        ("floor", &[]),
        ("shadow", &[("shadow_strength", 0.55)]),
        ("ao", &[("ao_strength", 0.75)]),
        ("fog_iter", &[("fog_iter_amount", 0.9)]),
        ("fog_distance", &[("fog_amount", 0.55)]),
        ("shafts", &[("shafts", 0.25)]),
        ("head_light", &[("head_light", 0.45)]),
        ("stars", &[("star_amount", 0.35)]),
        (
            "lens",
            &[
                ("bloom", 0.7),
                ("aberration", 0.18),
                ("ghost", 0.14),
                ("motion_blur", 0.15),
                ("dof_amount", 0.55),
                ("vignette", 0.4),
                ("clarity", 0.25),
            ],
        ),
    ];
    for (name, settings) in terms {
        configure_all_shading_off(deck);
        for (parameter, value) in settings {
            deck.generator_params.set_float(parameter, *value);
        }
        warm_up(context, deck);
        group.bench_function(format!("{prefix}/bisect/{name}"), |b| {
            b.iter(|| render_once(context, deck));
        });
    }
}

fn benchmark_profiles(
    group: &mut BenchmarkGroup<'_, WallTime>,
    context: &GpuContext,
    deck: &mut Deck,
    prefix: &str,
) {
    configure_full_beauty(deck);
    warm_up(context, deck);
    group.bench_function(format!("{prefix}/full_beauty"), |b| {
        b.iter(|| render_once(context, deck));
    });

    // The same frame as `full_beauty` with only the two-dimensional post chain
    // skipped. Full beauty minus this is what post costs; this minus the core
    // march is what three-dimensional shading costs.
    configure_full_beauty(deck);
    deck.generator_params.set_long("dz_debug", 12);
    warm_up(context, deck);
    group.bench_function(format!("{prefix}/shading_no_post"), |b| {
        b.iter(|| render_once(context, deck));
    });

    configure_core_march_diagnostic(deck);
    warm_up(context, deck);
    group.bench_function(format!("{prefix}/core_march_diagnostic"), |b| {
        b.iter(|| render_once(context, deck));
    });
}

fn preprocessor_timeout() -> Duration {
    std::env::var("VARDA_FRACTAL_AP_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(DEFAULT_PREPROCESSOR_TIMEOUT, Duration::from_secs)
}

fn wait_for_host_payload(
    context: &GpuContext,
    deck: &mut Deck,
) -> Result<varda::testing::FractalPreprocessorMetrics, String> {
    // One render publishes the effective shader parameters to the frameless
    // analyzer. Further renders are deliberately withheld while AP generation
    // runs, so setup cannot be mistaken for GPU warmup or benchmark work.
    render_once(context, deck);
    let timeout = preprocessor_timeout();
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(metrics) = varda::testing::fractal_preprocessor_metrics(deck) {
            if metrics.payload_ready && metrics.boundary_ready {
                // Upload the newly published payload before timed warmup begins.
                render_once(context, deck);
                return Ok(metrics);
            }
            if metrics.boundary_reason != 0 {
                return Err(format!(
                    "host preprocessor rejected AP generation (boundary reason {}) after {:.1} ms",
                    metrics.boundary_reason, metrics.boundary_runtime_ms
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "host preprocessor did not publish a ready AP payload within {:.1}s",
                timeout.as_secs_f32()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn bench_fractal_deep_zoom(c: &mut Criterion) {
    c.bench_function("fractal_reference_orbit/512", |b| {
        b.iter(|| {
            black_box(varda::testing::benchmark_fractal_reference_orbit(
                black_box(512),
            ))
        });
    });
    c.bench_function("fractal_directional_certificate/15_rays", |b| {
        b.iter(|| black_box(varda::testing::benchmark_fractal_directional_certificates()));
    });

    let Ok(context) = GpuContext::new_headless() else {
        eprintln!("no GPU adapter, skipping the GPU group");
        return;
    };
    let mut group = c.benchmark_group("fractal_deep_zoom");
    group.sample_size(10);

    // The default active stack is the certifiable Mandelbulb flight shipped to
    // new decks. A stack with no sufficiently long-lived camera-ray anchor
    // cannot produce a host payload, and that is a benchmark failure rather
    // than a case to time some other way.
    for zoom in ZOOM_EXPONENTS {
        let zoom_name = format!("bulb/zoom_{zoom:.0}");
        for (resolution, width, height) in [
            ("720p", WIDTH_720P, HEIGHT_720P),
            ("1080p", WIDTH_1080P, HEIGHT_1080P),
        ] {
            let mut deck = make_deck(&context, width, height, |deck| {
                configure_showcase(deck, zoom);
            });
            let metrics = match wait_for_host_payload(&context, &mut deck) {
                Ok(metrics) => metrics,
                Err(reason) => panic!("{zoom_name}/{resolution}: {reason}"),
            };
            eprintln!(
                "{zoom_name}/{resolution} host setup: boundary {:.1} ms, \
                 segment_ready={}, segment_coverage={:.3}",
                metrics.boundary_runtime_ms, metrics.segment_ready, metrics.segment_coverage
            );
            let prefix = format!("{zoom_name}/host_orbit/{resolution}");
            benchmark_profiles(&mut group, &context, &mut deck, &prefix);
            // One configuration is enough to attribute the shading cost, and
            // running the bisect everywhere would multiply the suite's runtime
            // for no extra information.
            if zoom == 12.0 && resolution == "720p" {
                benchmark_shading_bisect(&mut group, &context, &mut deck, &prefix);
            }
        }
    }

    group.finish();
}

criterion_group!(benches, bench_fractal_deep_zoom);
criterion_main!(benches);
