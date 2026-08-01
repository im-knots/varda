//! Scratch harness: where does `warped_grid.fs` spend its frame at 1080p?
//!
//! Reports the working tree against HEAD, then renders the shader with
//! individual pieces of the massing and the lighting budget disabled, so the
//! cost is attributed by measurement rather than by reading the code.
//!
//! Every ablation runs through a uniform rather than by patching the source,
//! because each feature sits behind a branch that a zeroed uniform skips
//! outright — `building()` tests `tierH > 0.0` before extruding a tier, and the
//! facade block tests `window_amount` before touching the pane grid. So the
//! measurement reflects work actually not done.

use std::time::Instant;

use varda::{
    audio::AudioData,
    deck::Deck,
    mixer::Mixer,
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
    renderer::tonemap::TonemapMode,
};

const W: u32 = 1920;
const H: u32 = 1080;
const WARMUP: usize = 15;
const FRAMES: usize = 60;
const SPEED: f32 = 3.0;
/// Variants are measured round-robin and reduced by median, because the GPU
/// drifts over a run: measuring every variant back to back once had the same
/// configuration come out at 49 ms early on and 33 ms a minute later. Only
/// interleaving makes two variants comparable.
const ROUNDS: usize = 5;

/// Label, float uniform overrides, bool uniform overrides.
type Ablation = (
    &'static str,
    &'static [(&'static str, f32)],
    &'static [(&'static str, bool)],
);

/// Feature ablations, each driven by the uniform that gates it.
const ABLATIONS: &[Ablation] = &[
    ("no setback tiers", &[("setback_amount", 0.0)], &[]),
    ("no spires", &[("spire_amount", 0.0)], &[]),
    ("no districts", &[("district_variation", 0.0)], &[]),
    ("no lit windows", &[("window_amount", 0.0)], &[]),
    ("no AO", &[], &[("ambient_occlusion", false)]),
    (
        "bare mass only",
        &[
            ("setback_amount", 0.0),
            ("spire_amount", 0.0),
            ("window_amount", 0.0),
        ],
        &[],
    ),
];

/// A thing to time: label, shader source, and the uniforms to override.
type Variant = (
    String,
    String,
    Vec<(&'static str, f32)>,
    Vec<(&'static str, bool)>,
);

fn measure(
    ctx: &GpuContext,
    source: &str,
    floats: &[(&str, f32)],
    bools: &[(&str, bool)],
) -> (f64, f64) {
    let shader = varda::isf::ISFShader::from_string(source).expect("parse");
    let mut mixer = Mixer::new(ctx, W, H).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let mut deck = Deck::new(ctx, shader, W, H).expect("deck");
    deck.generator_params.set_float("speed", SPEED);
    for (name, value) in floats {
        deck.generator_params.set_float(name, *value);
    }
    for (name, value) in bools {
        deck.generator_params.set_bool(name, *value);
    }
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(deck);
    // Pin the deck to the target rate. Left on Auto, the channel skips frames
    // for a deck that overruns its budget share, which both hides the true cost
    // and makes the phase accumulator advance by a load-dependent amount.
    ch.decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(60);

    let mut times = Vec::with_capacity(FRAMES);
    for frame in 0..(WARMUP + FRAMES) {
        let t0 = Instant::now();
        mixer
            .render(
                ctx,
                &AudioData::default(),
                &AudioValues {
                    sources: std::collections::HashMap::default(),
                },
                &AnalyzerValues::default(),
                60,
                &[],
            )
            .expect("render");
        ctx.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .ok();
        if frame >= WARMUP {
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(f64::total_cmp);
    (
        times.iter().sum::<f64>() / times.len() as f64,
        times[times.len() / 2],
    )
}

fn main() {
    let ctx = GpuContext::new_headless().expect("headless GPU");
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/warped_grid.fs"),
    )
    .expect("read warped_grid.fs");

    let head = String::from_utf8(
        std::process::Command::new("git")
            .args(["show", "HEAD:shaders/warped_grid.fs"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("git show")
            .stdout,
    )
    .expect("HEAD shader is UTF-8");

    let mut variants: Vec<Variant> = vec![
        ("HEAD pylons (shadows)".into(), head, vec![], vec![]),
        ("working (defaults)".into(), src.clone(), vec![], vec![]),
        (
            "working + shadows".into(),
            src.clone(),
            vec![],
            vec![("soft_shadows", true)],
        ),
    ];
    for (label, floats, bools) in ABLATIONS {
        variants.push((
            (*label).to_string(),
            src.clone(),
            floats.to_vec(),
            bools.to_vec(),
        ));
    }

    let mut samples = vec![Vec::with_capacity(ROUNDS); variants.len()];
    for _ in 0..ROUNDS {
        for (i, (_, source, floats, bools)) in variants.iter().enumerate() {
            samples[i].push(measure(&ctx, source, floats, bools).0);
        }
    }

    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let times: Vec<f64> = samples.iter_mut().map(median).collect();
    // "baseline" is the unpatched working tree, which is also index 1.
    let baseline = times[1];

    println!("{:<22} {:>9} {:>12}", "variant", "median ms", "vs baseline");
    for (i, (label, _, _, _)) in variants.iter().enumerate() {
        let delta = times[i] - baseline;
        println!(
            "{label:<22} {:>9.2} {delta:>+8.2} ({:>+3.0}%)",
            times[i],
            delta / baseline * 100.0
        );
    }
}
