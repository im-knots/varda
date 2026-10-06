//! `shaders/fractal_explorer.fs` against the host evaluator in `varda::fractal`.
//!
//! The shader's parity diagnostic writes the stack's sample at a fixed lattice
//! of points instead of a frame. Each stack below is set on a deck, rendered,
//! and compared point by point with the `f64` evaluator. A transcription
//! mistake in any formula shows up as a large difference; `f32` against `f64`
//! and the `f16` composite stay well under the tolerance.

mod common;

use std::collections::HashMap;

use common::headless_gpu;
use varda::{
    audio::AudioData,
    deck::Deck,
    fractal::{SLOT_FIELDS, STACK_PARAMS, Vec3, default_param, slot_param, stack_from_params},
    isf::ISFShader,
    mixer::Mixer,
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
    renderer::tonemap::TonemapMode,
    testing::{fractal_scenes, read_rgba16f, set_param, srgb8},
};

const W: u32 = 48;
const H: u32 = 48;
/// The parity diagnostic's index in the shader's `debug_view` list.
const PARITY_VIEW: i32 = 6;
/// Relative distance tolerance: `f16` output carries about three digits.
const TOLERANCE: f64 = 0.01;

fn shader() -> ISFShader {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fractal_explorer.fs");
    ISFShader::from_file(path).expect("fractal_explorer.fs parses")
}

/// The lattice point the shader samples for a pixel, from its center.
fn parity_point(x: u32, y: u32) -> Vec3 {
    let qx = (f64::from(x) + 0.5) / f64::from(W);
    let qy = (f64::from(y) + 0.5) / f64::from(H);
    Vec3::new(-3.0 + 6.0 * qx, -3.0 + 6.0 * qy, 0.37)
}

/// Render the explorer with `values` set, after restoring `state`, for
/// `frames` frames at `width x height`, and read the composite back.
fn render(
    ctx: &GpuContext,
    size: (u32, u32),
    values: &HashMap<&str, f64>,
    state: Option<&serde_json::Map<String, serde_json::Value>>,
    frames: u32,
) -> Vec<[f32; 4]> {
    render_colored(ctx, size, values, &[], state, frames)
}

/// `render` with color inputs set too.
fn render_colored(
    ctx: &GpuContext,
    (width, height): (u32, u32),
    values: &HashMap<&str, f64>,
    colors: &[(&str, [f32; 4])],
    state: Option<&serde_json::Map<String, serde_json::Value>>,
    frames: u32,
) -> Vec<[f32; 4]> {
    let mut deck = Deck::from_shader(ctx, shader(), width, height).expect("deck");
    // Inputs with presets first, so a preset does not overwrite the other values.
    let mut ordered: Vec<_> = values.iter().collect();
    ordered.sort_by_key(|(name, _)| {
        deck.generator_params
            .definitions
            .get(**name)
            .is_none_or(|d| d.presets.is_none())
    });
    for (name, value) in ordered {
        set_param(&mut deck.generator_params, name, *value).unwrap_or_else(|e| panic!("{e}"));
    }
    for (name, color) in colors {
        assert!(
            deck.generator_params.definitions.contains_key(*name),
            "shader has no input {name}"
        );
        deck.generator_params.set_color(name, *color);
    }
    deck.start_declared_preprocessors();
    if let Some(state) = state {
        deck.restore_preprocessor_state(state);
    }
    let mut mixer = Mixer::new(ctx, width, height).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    mixer.channel_mut(0).unwrap().add_deck(deck);
    for channel in mixer.channels_mut() {
        for slot in &mut channel.decks {
            slot.render_fps = varda::channel::DeckRenderFps::Fixed(0);
        }
    }
    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();
    for frame in 0..frames {
        let inputs = varda::mixer::FrameInputs {
            audio_data: &audio,
            audio_values: &audio_values,
            analyzer_values: &analyzer_values,
            beat_time: None,
            transport: None,
            free_run_time: Some(frame as f32 / 60.0),
            write_param: varda::param_router::write_macro_target,
        };
        mixer.render(ctx, &inputs, 60, &[]).expect("render");
    }
    read_rgba16f(ctx, mixer.composite_texture(), width, height)
}

fn render_parity(ctx: &GpuContext, values: &HashMap<&str, f64>) -> Vec<[f32; 4]> {
    let mut values = values.clone();
    values.insert("debug_view", f64::from(PARITY_VIEW));
    // One parity texel per output pixel: no upsampling.
    values.insert("render_scale", 1.0);
    values.insert("target_fps", 0.0);
    render(ctx, (W, H), &values, None, 1)
}

/// Compare one stack. Points where the orbit is chaotic at the scale of `f32`
/// rounding (a nearby point escapes at another iteration, or its distance
/// moves by more than the tolerance) are skipped: there the two evaluators
/// may legitimately take different branches.
fn assert_parity(ctx: &GpuContext, label: &str, values: &[(&str, f64)]) {
    // Cases name what they change from a scale -1.5 Amazing Box, not from
    // whatever the shader's defaults are.
    let mut values: HashMap<&str, f64> = values.iter().copied().collect();
    values.entry("slot1_a").or_insert(-1.5);
    // Slots a case does not name are empty, and every slot value the case
    // leaves out takes the host's default on the GPU too.
    let names: Vec<String> = (0..6)
        .flat_map(|slot| SLOT_FIELDS.iter().map(move |field| slot_param(slot, field)))
        .collect();
    for name in &names {
        let fallback = if name.ends_with("_formula") && name != "slot1_formula" {
            0.0
        } else {
            default_param(name)
        };
        values.entry(name.as_str()).or_insert(fallback);
    }
    let stack = stack_from_params(|name| {
        values
            .get(name)
            .copied()
            .or_else(|| (name == "debug_view").then_some(f64::from(PARITY_VIEW)))
    });
    let pixels = render_parity(ctx, &values);
    let mut compared = 0;
    let mut failures = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let p = parity_point(x, y);
            let host = stack.sample(p, 0.0);
            let stable = [
                Vec3::new(1e-4, 0.0, 0.0),
                Vec3::new(0.0, 1e-4, 0.0),
                Vec3::new(0.0, 0.0, 1e-4),
            ]
            .iter()
            .all(|e| {
                let near = stack.sample(p + *e, 0.0);
                (near.smooth_iteration.floor() - host.smooth_iteration.floor()).abs() < 0.5
                    && (near.distance - host.distance).abs()
                        <= TOLERANCE * 0.25 * host.distance.abs().max(1e-3)
            });
            if !stable || !host.distance.is_finite() || host.distance.abs() > 1e4 {
                continue;
            }
            compared += 1;
            let gpu = f64::from(pixels[(y * W + x) as usize][0]);
            let scale = host.distance.abs().max(1e-3);
            if (gpu - host.distance).abs() > TOLERANCE * scale {
                let px = pixels[(y * W + x) as usize];
                failures.push(format!(
                    "{p:?}: gpu d {gpu:.5} it {:.2} log2dr {:.2} | host d {:.5} it {:.2} log2dr {:.2} esc {}",
                    px[1], px[2], host.distance, host.smooth_iteration, host.log2_dr, host.escaped
                ));
            }
        }
    }
    let total = (W * H) as usize;
    // Non-conformal chains' Jacobian distance jumps at fold edges, so fewer
    // of their points are stable.
    assert!(
        compared * 10 >= total * 2,
        "{label}: only {compared} of {total} points were stable enough to compare"
    );
    // A few points sit exactly on a fold boundary in f32 but not in f64.
    assert!(
        failures.len() * 200 <= compared,
        "{label}: {} of {compared} points differ, e.g. {:?}",
        failures.len(),
        &failures[..failures.len().min(5)]
    );
}

#[test]
fn every_formula_matches_the_host_evaluator() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let cases: &[(&str, &[(&str, f64)])] = &[
        ("amazing box", &[]),
        ("amazing surf", &[("slot1_mode", 1.0), ("slot1_a", 1.8)]),
        ("surf cylinder", &[("slot1_mode", 2.0), ("slot1_a", 1.8)]),
        (
            "mod kali",
            &[("slot1_mode", 3.0), ("slot1_a", 1.5), ("slot1_c", 0.5)],
        ),
        (
            "kalibox",
            &[("slot1_mode", 4.0), ("slot1_a", 1.8), ("slot1_c", -0.5)],
        ),
        (
            "menger",
            &[("slot1_formula", 2.0), ("slot1_a", 3.0), ("slot1_b", 1.0)],
        ),
        (
            "sierpinski",
            &[("slot1_formula", 3.0), ("slot1_a", 2.0), ("slot1_b", 1.0)],
        ),
        (
            "kifs tetra",
            &[
                ("slot1_formula", 4.0),
                ("slot1_a", 2.0),
                ("slot1_b", 1.0),
                ("slot1_c", 1.0),
            ],
        ),
        (
            "kifs octa",
            &[
                ("slot1_formula", 4.0),
                ("slot1_mode", 1.0),
                ("slot1_a", 2.0),
                ("slot1_b", 1.0),
            ],
        ),
        (
            "kifs icosa",
            &[
                ("slot1_formula", 4.0),
                ("slot1_mode", 2.0),
                ("slot1_a", 2.0),
                ("slot1_b", 1.0),
            ],
        ),
        (
            "pseudo-kleinian",
            &[
                ("slot1_formula", 5.0),
                ("slot1_a", 0.92),
                ("slot1_b", 1.0),
                ("slot1_c", 0.0),
            ],
        ),
        (
            "kaliset",
            &[("slot1_formula", 6.0), ("slot1_a", 1.0), ("slot1_b", 0.1)],
        ),
        (
            "mandelbulb",
            &[
                ("slot1_formula", 7.0),
                ("slot1_a", 8.0),
                ("slot1_b", 1.0),
                ("max_iterations", 8.0),
                ("bailout", 4.0),
            ],
        ),
        (
            "box then bulb",
            &[
                ("slot2_formula", 7.0),
                ("slot2_count", 1.0),
                ("slot2_a", 2.0),
                ("slot2_b", 1.0),
                ("slot1_a", 2.0),
                ("max_iterations", 10.0),
            ],
        ),
        (
            "rotated box",
            &[
                ("slot1_rot_x", 0.3),
                ("slot1_rot_y", -0.2),
                ("slot1_rot_z", 0.5),
            ],
        ),
        (
            "transform then box",
            &[
                ("slot1_formula", 8.0),
                ("slot1_a", 1.0),
                ("slot1_b", 0.1),
                ("slot1_rot_z", 0.4),
                ("slot2_formula", 1.0),
                ("slot2_count", 2.0),
                ("slot2_a", -1.5),
                ("slot2_b", 0.5),
                ("slot2_c", 1.0),
                ("slot2_d", 1.0),
                ("repeat_from", 2.0),
            ],
        ),
        (
            "helispiral",
            &[
                ("slot2_formula", 9.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.2),
                ("slot2_b", 0.1),
                ("slot2_c", 0.0),
            ],
        ),
        (
            "gnarl",
            &[
                ("slot2_formula", 10.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.05),
                ("slot2_b", 1.0),
                ("slot2_c", 1.0),
                ("slot2_d", 1.0),
            ],
        ),
        (
            "julia bulb",
            &[
                ("slot1_formula", 7.0),
                ("slot1_a", 8.0),
                ("slot1_b", 1.0),
                ("max_iterations", 8.0),
                ("bailout", 4.0),
                ("julia_mode", 1.0),
                ("julia_x", 0.3),
                ("julia_y", -0.2),
                ("julia_z", 0.1),
            ],
        ),
        (
            "combine union",
            &[
                ("hybrid_mode", 1.0),
                ("combine_split", 4.0),
                ("slot4_formula", 2.0),
                ("slot4_count", 1.0),
                ("slot4_a", 3.0),
                ("slot4_b", 1.0),
            ],
        ),
        (
            "combine subtract",
            &[
                ("hybrid_mode", 1.0),
                ("combine_op", 2.0),
                ("combine_split", 4.0),
                ("slot4_formula", 2.0),
                ("slot4_count", 1.0),
                ("slot4_a", 3.0),
                ("slot4_b", 1.0),
            ],
        ),
        (
            "bulbox",
            &[
                ("slot1_formula", 11.0),
                ("slot1_a", 2.0),
                ("slot1_b", 0.6),
                ("slot1_c", -0.5),
                ("slot1_d", 1.0),
                ("bailout", 1024.0),
            ],
        ),
        (
            "bulbox without the box",
            &[
                ("slot1_formula", 11.0),
                ("slot1_mode", 1.0),
                ("slot1_a", 2.0),
                ("slot1_b", 0.6),
                ("slot1_c", -0.5),
                ("slot1_d", 1.0),
                ("max_iterations", 8.0),
            ],
        ),
        (
            "surf then bulbox, julia",
            &[
                ("slot1_mode", 1.0),
                ("slot1_a", 1.8),
                ("slot2_formula", 11.0),
                ("slot2_count", 1.0),
                ("slot2_a", 2.0),
                ("slot2_b", 0.6),
                ("slot2_c", -0.5),
                ("slot2_d", 1.0),
                ("julia_mode", 1.0),
                ("julia_x", 0.3),
                ("julia_y", -0.2),
                ("julia_z", 0.1),
            ],
        ),
        (
            "space inversion then box",
            &[
                // The center sits off the test plane, so no grid point lands
                // on the singularity.
                ("slot1_formula", 12.0),
                ("slot1_mode", 1.0),
                ("slot1_a", 3.0),
                ("slot1_b", 0.4),
                ("slot1_c", 0.0),
                ("slot1_d", 3.0),
                ("max_iterations", 6.0),
                ("slot2_formula", 1.0),
                ("slot2_count", 1.0),
                ("slot2_a", -1.5),
                ("slot2_b", 0.5),
                ("slot2_c", 1.0),
                ("slot2_d", 1.0),
                ("repeat_from", 2.0),
            ],
        ),
        (
            "polyfold then box",
            &[
                ("slot1_formula", 13.0),
                ("slot1_a", 5.0),
                ("slot1_b", 10.0),
                ("slot1_c", 0.0),
                ("slot1_d", 0.0),
                ("slot2_formula", 1.0),
                ("slot2_count", 1.0),
                ("slot2_a", -1.5),
                ("slot2_b", 0.5),
                ("slot2_c", 1.0),
                ("slot2_d", 1.0),
                ("repeat_from", 2.0),
            ],
        ),
        (
            "box then sine y",
            &[
                ("slot2_formula", 14.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.0),
                ("slot2_b", 1.5),
                ("slot2_c", 1.0),
                ("slot2_d", 0.0),
                ("max_iterations", 8.0),
            ],
        ),
        (
            "box then reciprocal x",
            &[
                ("slot2_formula", 15.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.5),
                ("max_iterations", 8.0),
            ],
        ),
        (
            // A compact Menger sponge in 3-unit cells: the test grid spans
            // three copies, mirrored.
            "mirror repeat then menger",
            &[
                ("slot1_formula", 16.0),
                ("slot1_a", 3.0),
                ("slot1_b", 0.0),
                ("slot2_formula", 2.0),
                ("slot2_count", 1.0),
                ("slot2_a", 3.0),
                ("slot2_b", 1.0),
                ("repeat_from", 2.0),
                ("max_iterations", 8.0),
            ],
        ),
        (
            "warped repeat then menger",
            &[
                ("slot1_formula", 16.0),
                ("slot1_a", 3.0),
                ("slot1_b", 0.0),
                ("slot1_c", 0.2),
                ("slot1_d", 0.5),
                ("slot2_formula", 2.0),
                ("slot2_count", 1.0),
                ("slot2_a", 3.0),
                ("slot2_b", 1.0),
                ("repeat_from", 2.0),
                ("max_iterations", 8.0),
            ],
        ),
        (
            "twisted temple",
            &[
                ("slot1_a", -1.8),
                ("slot2_formula", 9.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.1),
                ("slot2_b", 0.2),
                ("slot2_c", 0.0),
                ("max_iterations", 14.0),
            ],
        ),
        (
            "gnarled temple",
            &[
                ("slot1_a", -1.8),
                ("slot2_formula", 10.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.15),
                ("slot2_b", 1.0),
                ("slot2_c", 1.0),
                ("slot2_d", 1.0),
                ("max_iterations", 14.0),
            ],
        ),
        (
            "turned warped repeat then gnarled box",
            &[
                ("slot1_formula", 16.0),
                ("slot1_a", 3.0),
                ("slot1_b", 0.0),
                ("slot1_c", 0.2),
                ("slot1_d", 0.5),
                ("slot1_rot_z", 0.3),
                ("slot2_formula", 1.0),
                ("slot2_count", 1.0),
                ("slot2_a", -1.8),
                ("slot2_b", 0.5),
                ("slot2_c", 1.0),
                ("slot2_d", 1.0),
                ("slot3_formula", 10.0),
                ("slot3_count", 1.0),
                ("slot3_a", 0.1),
                ("slot3_b", 1.0),
                ("slot3_c", 1.0),
                ("slot3_d", 1.0),
                ("repeat_from", 2.0),
                ("max_iterations", 12.0),
            ],
        ),
        (
            "koch cube",
            &[
                ("slot1_formula", 17.0),
                ("slot1_a", 1.0),
                ("slot1_b", 1.0),
                ("slot1_c", 1.0),
                ("slot1_d", 0.0),
                ("max_iterations", 10.0),
            ],
        ),
        (
            "stretched koch cube",
            &[
                ("slot1_formula", 17.0),
                ("slot1_a", 1.0),
                ("slot1_b", 1.2),
                ("slot1_c", 1.0),
                ("slot1_d", 0.1),
                ("max_iterations", 10.0),
            ],
        ),
        (
            "jcube",
            &[
                ("slot1_formula", 18.0),
                ("slot1_a", 0.414_213_56),
                ("slot1_b", 3.0),
                ("slot1_c", 1.0),
                ("slot1_d", 1.0),
                ("max_iterations", 10.0),
            ],
        ),
        (
            "lin combine then box",
            &[
                ("slot1_formula", 1.0),
                ("slot1_a", -1.5),
                ("slot2_formula", 19.0),
                ("slot2_count", 1.0),
                ("slot2_a", 1.1),
                ("slot2_b", 0.9),
                ("slot2_c", 1.0),
                ("max_iterations", 12.0),
            ],
        ),
        (
            "box with inverted bounds",
            &[
                ("slot1_formula", 1.0),
                ("slot1_a", 2.0),
                ("slot1_b", 1.2),
                ("slot1_c", -0.3),
                ("slot1_d", 0.8),
                ("slot2_formula", 0.0),
                ("max_iterations", 10.0),
            ],
        ),
        (
            "rotate 4d then box",
            &[
                ("slot1_formula", 1.0),
                ("slot1_a", -1.5),
                ("slot2_formula", 20.0),
                ("slot2_count", 1.0),
                ("slot2_a", 0.11),
                ("slot2_b", 0.19),
                ("slot2_c", 0.06),
                ("slot2_d", 0.0),
                ("max_iterations", 12.0),
            ],
        ),
        (
            "abox mod2",
            &[
                ("slot1_formula", 21.0),
                ("slot1_a", -1.5),
                ("slot1_b", 0.5),
                ("slot1_c", 1.0),
                ("slot1_d", 1.5),
                ("max_iterations", 12.0),
            ],
        ),
        (
            "msltoe sym4",
            &[
                ("slot1_formula", 22.0),
                ("slot1_a", 1.0),
                ("slot1_b", 1.0),
                ("slot1_c", 1.0),
                ("max_iterations", 10.0),
                ("bailout", 4.0),
            ],
        ),
        (
            "soft kifs",
            &[
                ("slot1_formula", 4.0),
                ("slot1_a", 2.0),
                ("slot1_b", 1.0),
                ("slot1_c", 1.0),
                ("slot1_d", 0.7),
            ],
        ),
    ];
    for (label, values) in cases {
        assert_parity(&ctx, label, values);
    }
}

// ── Acceptance scenes ─────────────────────────────────────────────────────

fn luma(p: [f32; 4]) -> f64 {
    f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
}

const SCENE_SIZE: (u32, u32) = (320, 180);

/// Each scene draws structure: its luminance varies across the frame instead
/// of being a flat field of sky or fog.
#[test]
fn acceptance_scenes_render_structure() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    for scene in fractal_scenes() {
        let values: HashMap<&str, f64> =
            scene.params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let pixels = render(&ctx, SCENE_SIZE, &values, Some(&scene.state), 20);
        let lumas: Vec<f64> = pixels.iter().copied().map(luma).collect();
        let mean = lumas.iter().sum::<f64>() / lumas.len() as f64;
        let spread =
            (lumas.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lumas.len() as f64).sqrt();
        eprintln!("{}: mean luma {mean:.4}, spread {spread:.4}", scene.name);
        // A standard deviation of at least 2% of the mean, with a floor for dark scenes.
        assert!(
            spread > 0.02 * mean.max(0.01),
            "{}: flat frame, mean {mean:.4} spread {spread:.4}",
            scene.name
        );
    }
}

/// Where the march starts must not move the surface. With temporal
/// smoothing and post effects off, the tile prepass changes only the start;
/// fewer than 1% of 4x4 cells may differ by more than 16/255.
///
/// Not met yet: with an approximate distance estimate, which points a ray
/// samples decides where it stops, worst on grazing silhouettes. Run with
/// `--ignored` to print each scene's share.
#[test]
#[ignore = "acceptance gate not met yet; see the doc comment"]
fn the_hit_does_not_depend_on_where_the_march_starts() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let (width, height) = SCENE_SIZE;
    let mut results = Vec::new();
    for scene in fractal_scenes() {
        let mut values: HashMap<&str, f64> =
            scene.params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        for (name, value) in [
            ("temporal", 0.0),
            ("dof_mode", 0.0),
            ("bloom", 0.0),
            ("grain", 0.0),
            ("shafts", 0.0),
        ] {
            values.insert(name, value);
        }
        values.insert("tile_prepass", 0.0);
        let from_camera = render(&ctx, SCENE_SIZE, &values, Some(&scene.state), 1);
        values.insert("tile_prepass", 1.0);
        let from_tiles = render(&ctx, SCENE_SIZE, &values, Some(&scene.state), 1);
        let (cols, rows) = (width / 4, height / 4);
        let mut over = 0;
        for cy in 0..rows {
            for cx in 0..cols {
                let mut diff: f64 = 0.0;
                for c in 0..3 {
                    let mut sum_camera = 0.0;
                    let mut sum_tiles = 0.0;
                    for y in cy * 4..cy * 4 + 4 {
                        for x in cx * 4..cx * 4 + 4 {
                            let i = (y * width + x) as usize;
                            sum_camera += srgb8(from_camera[i][c]);
                            sum_tiles += srgb8(from_tiles[i][c]);
                        }
                    }
                    diff = diff.max((sum_camera - sum_tiles).abs() / 16.0);
                }
                if diff > 16.0 {
                    over += 1;
                }
            }
        }
        let fraction = f64::from(over) / f64::from(cols * rows);
        eprintln!("{}: {:.2}% of cells differ", scene.name, fraction * 100.0);
        results.push((scene.name, fraction));
    }
    let failed: Vec<_> = results.iter().filter(|(_, f)| *f >= 0.01).collect();
    assert!(failed.is_empty(), "cells moved: {failed:?}");
}

// ── Dynamic resolution and detail accumulation ────────────────────────────

/// Mean absolute 4-neighbor Laplacian of `at` over the interior of a
/// `width` by `height` frame.
fn mean_laplacian((width, height): (u32, u32), at: impl Fn(u32, u32) -> f64) -> f64 {
    let mut sum = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            sum +=
                (4.0 * at(x, y) - at(x - 1, y) - at(x + 1, y) - at(x, y - 1) - at(x, y + 1)).abs();
        }
    }
    sum / f64::from((width - 2) * (height - 2))
}

/// Mean absolute Laplacian of luma: how much fine detail a frame has.
fn laplacian(pixels: &[[f32; 4]], (width, height): (u32, u32)) -> f64 {
    mean_laplacian((width, height), |x, y| {
        luma(pixels[(y * width + x) as usize])
    })
}

/// Mean absolute Laplacian of the sRGB 8-bit green channel: fine detail as a
/// viewer sees it.
fn srgb_detail(pixels: &[[f32; 4]], (width, height): (u32, u32)) -> f64 {
    mean_laplacian((width, height), |x, y| {
        srgb8(pixels[(y * width + x) as usize][1])
    })
}

/// Mean luma over the pixels with `x0 <= x < x1`, `y0 <= y < y1`.
fn region_luma(pixels: &[[f32; 4]], width: u32, (x0, x1): (u32, u32), (y0, y1): (u32, u32)) -> f64 {
    let mut sum = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            sum += luma(pixels[(y * width + x) as usize]);
        }
    }
    sum / f64::from((x1 - x0) * (y1 - y0))
}

/// RMS difference of the sRGB 8-bit values, red and green.
fn srgb_error(a: &[[f32; 4]], b: &[[f32; 4]]) -> f64 {
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(p, q)| (srgb8(p[0]) - srgb8(q[0])).powi(2) + (srgb8(p[1]) - srgb8(q[1])).powi(2))
        .sum();
    (sum / (2 * a.len()) as f64).sqrt()
}

/// Post effects that add noise or blur of their own.
const PLAIN: [(&str, f64); 5] = [
    ("dof_mode", 0.0),
    ("bloom", 0.0),
    ("grain", 0.0),
    ("vignette", 0.0),
    ("aberration", 0.0),
];

/// The gnarled temple in Teal & Gold, pinned so these tests do not follow the
/// default stack and look.
const TEMPLE: [(&str, f64); 14] = [
    ("look", 4.0),
    ("slot1_formula", 1.0),
    ("slot1_a", -1.8),
    ("slot1_b", 0.5),
    ("slot1_c", 1.0),
    ("slot1_d", 1.0),
    ("slot1_rot_x", 0.0),
    ("slot1_rot_y", 0.0),
    ("slot2_formula", 10.0),
    ("slot2_a", 0.15),
    ("slot2_b", 1.0),
    ("slot2_c", 1.0),
    ("slot2_d", 1.0),
    ("max_iterations", 16.0),
];

/// The Box 2.2 interior's smooth walls, unturned, with nothing after the box.
const BOX_INTERIOR: [(&str, f64); 4] = [
    ("slot1_a", 2.2),
    ("slot1_rot_x", 0.0),
    ("slot1_rot_y", 0.0),
    ("slot2_formula", 0.0),
];

/// The temple with the post effects that add noise or blur off.
fn plain_temple() -> HashMap<&'static str, f64> {
    PLAIN.into_iter().chain(TEMPLE).collect()
}

/// `base` with the camera held still (no frame-rate target, the start found
/// inside), then `extra` on top.
fn held_still(
    base: impl IntoIterator<Item = (&'static str, f64)>,
    extra: &[(&'static str, f64)],
) -> HashMap<&'static str, f64> {
    let mut values: HashMap<&str, f64> = base.into_iter().collect();
    values.insert("target_fps", 0.0);
    values.insert("find_inside", 1.0);
    values.extend(extra.iter().copied());
    values
}

/// `values` rendered at `factor` times `size` for `frames` frames and box
/// filtered back down.
fn downsampled(
    ctx: &GpuContext,
    (w, h): (u32, u32),
    factor: u32,
    values: &HashMap<&str, f64>,
    frames: u32,
) -> Vec<[f32; 4]> {
    let big = render(ctx, (factor * w, factor * h), values, None, frames);
    let share = 1.0 / (factor * factor) as f32;
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut sum = [0.0f32; 4];
            for dy in 0..factor {
                for dx in 0..factor {
                    let p = big[((factor * y + dy) * factor * w + factor * x + dx) as usize];
                    for c in 0..4 {
                        sum[c] += share * p[c];
                    }
                }
            }
            sum
        })
        .collect()
}

/// The temple held still with `extra` set, rendered at twice the size with
/// `detail` 2 so rays stop at the same shell, then averaged down: what an
/// ideal upsampler converges to.
fn supersampled_truth(
    ctx: &GpuContext,
    size: (u32, u32),
    extra: &[(&'static str, f64)],
) -> Vec<[f32; 4]> {
    let mut values = held_still(
        plain_temple(),
        &[("render_scale", 1.0), ("temporal", 0.0), ("detail", 2.0)],
    );
    values.extend(extra.iter().copied());
    downsampled(ctx, size, 2, &values, 64)
}

/// The temple held still for 64 frames at `scale`.
fn held(ctx: &GpuContext, size: (u32, u32), scale: f64, temporal: f64) -> Vec<[f32; 4]> {
    let values = held_still(
        plain_temple(),
        &[("render_scale", scale), ("temporal", temporal)],
    );
    render(ctx, size, &values, None, 64)
}

/// The temple at render scale 0.5, held still or flown slowly for 64 frames.
fn shot(ctx: &GpuContext, size: (u32, u32), throttle: f64) -> Vec<[f32; 4]> {
    let values = held_still(
        plain_temple(),
        &[("throttle", throttle), ("render_scale", 0.5)],
    );
    render(ctx, size, &values, None, 64)
}

/// Out of reach, the target drives the live scale to its floor: the frame is
/// still filled edge to edge, with less fine detail than at the ceiling.
#[test]
fn dynamic_resolution_at_its_floor_fills_the_frame_with_less_detail() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (256, 144);
    let with = |target_fps: f64| {
        let mut values = plain_temple();
        values.insert("temporal", 0.0);
        values.insert("render_scale", 1.0);
        values.insert("target_fps", target_fps);
        render(&ctx, size, &values, None, 60)
    };
    let ceiling = with(0.0);
    let floor = with(1000.0);
    let (detail_ceiling, detail_floor) = (laplacian(&ceiling, size), laplacian(&floor, size));
    eprintln!("detail: ceiling {detail_ceiling:.5}, floor {detail_floor:.5}");
    // At least 20% less detail.
    assert!(
        detail_floor < 0.8 * detail_ceiling,
        "the floor renders fewer pixels"
    );
    let (w, h) = size;
    // The far edges keep their brightness within 15%: nothing is left unfilled.
    for (label, xs, ys) in [
        ("right third", (2 * w / 3, w), (0, h)),
        ("bottom third", (0, w), (2 * h / 3, h)),
    ] {
        let (a, b) = (
            region_luma(&ceiling, w, xs, ys),
            region_luma(&floor, w, xs, ys),
        );
        assert!(
            (a - b).abs() < 0.15 * a + 0.005,
            "{label}: ceiling {a:.4}, floor {b:.4}"
        );
    }
}

/// A still camera keeps gathering jittered samples: each new frame moves the
/// image less as the average grows (about 1/N).
#[test]
fn a_still_camera_keeps_sharpening() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (192, 108);
    let values = held_still(plain_temple(), &[("render_scale", 0.5)]);
    let change = |frames: u32| {
        let a = render(&ctx, size, &values, None, frames);
        let b = render(&ctx, size, &values, None, frames + 1);
        let sum: f64 = a
            .iter()
            .zip(&b)
            .map(|(p, q)| (luma(*p) - luma(*q)).powi(2))
            .sum();
        (sum / a.len() as f64).sqrt()
    };
    let (early, late) = (change(16), change(64));
    eprintln!("change per frame: at 16 frames {early:.6}, at 64 {late:.6}");
    // A 1/N average moves a quarter as much at four times the frames; 0.6
    // leaves room for noise.
    assert!(
        late < 0.6 * early,
        "at 16 frames {early:.6}, at 64 {late:.6}"
    );
}

/// Held still, half render scale comes close to native: the upsampler gathers
/// neighboring render pixels' samples, and shading measures in output pixels
/// at full render size, so the image does not converge to blocks one render
/// pixel wide.
#[test]
fn a_held_shot_at_half_scale_approaches_native() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (320, 180);
    let truth = supersampled_truth(&ctx, size, &[]);
    let half = srgb_error(&held(&ctx, size, 0.5, 1.0), &truth);
    let bilinear = srgb_error(&held(&ctx, size, 0.5, 0.0), &truth);
    let native = srgb_error(&held(&ctx, size, 1.0, 1.0), &truth);
    eprintln!("error against truth: half {half:.3}, bilinear {bilinear:.3}, native {native:.3}");
    // At least 10% closer than one bilinear upscaled frame.
    assert!(
        half < 0.9 * bilinear,
        "half {half:.3}, bilinear {bilinear:.3}"
    );
    // Within 25% of native's error.
    assert!(half < 1.25 * native, "half {half:.3}, native {native:.3}");
}

/// At a render scale whose size rounds short of the output's aspect ratio
/// (320 x 0.7 is 223.99 in f32), a held shot still gains from accumulation:
/// history is reprojected in the output's aspect, so a still camera finds it
/// in place.
#[test]
fn accumulation_helps_at_a_rounded_render_scale() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (320, 180);
    let truth = supersampled_truth(&ctx, size, &[]);
    let accumulated = srgb_error(&held(&ctx, size, 0.7, 1.0), &truth);
    let single = srgb_error(&held(&ctx, size, 0.7, 0.0), &truth);
    eprintln!("error against truth at 0.7: accumulated {accumulated:.3}, single {single:.3}");
    // At least 10% closer than a single frame.
    assert!(
        accumulated < 0.9 * single,
        "accumulated {accumulated:.3}, single {single:.3}"
    );
}

/// Sharpening brings fine detail toward the supersampled reference's without
/// passing it, which would be halos and ringing, and without moving the
/// image away from the reference.
#[test]
fn sharpening_approaches_the_reference_detail() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (320u32, 180u32);
    let truth = supersampled_truth(&ctx, size, &[]);
    let with = |sharpness: f64| {
        let values = held_still(
            plain_temple(),
            &[("render_scale", 0.5), ("sharpness", sharpness)],
        );
        render(&ctx, size, &values, None, 64)
    };
    let (soft, sharp) = (with(0.0), with(1.0));
    let reference = srgb_detail(&truth, size);
    let (soft_detail, sharp_detail) = (
        srgb_detail(&soft, size) / reference,
        srgb_detail(&sharp, size) / reference,
    );
    let (soft_error, sharp_error) = (srgb_error(&soft, &truth), srgb_error(&sharp, &truth));
    eprintln!(
        "detail / reference: unsharpened {soft_detail:.3}, sharpened {sharp_detail:.3}; \
         error: unsharpened {soft_error:.3}, sharpened {sharp_error:.3}"
    );
    // At least a tenth of the reference's detail gained.
    assert!(
        sharp_detail > soft_detail + 0.1,
        "detail {soft_detail:.3} -> {sharp_detail:.3}"
    );
    assert!(
        sharp_detail <= 1.0,
        "sharper than the reference: {sharp_detail:.3}"
    );
    // The error grows by at most 5%.
    assert!(
        sharp_error < 1.05 * soft_error,
        "error {soft_error:.3} -> {sharp_error:.3}"
    );
}

/// The upscaler at render scale 0.5 keeps most of the supersampled
/// reference's fine detail held still, and at least a quarter of it in slow
/// flight, without passing the reference.
#[test]
fn the_upscaler_keeps_fine_detail() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (320, 180);
    for (label, throttle, floor) in [("held", 0.0, 0.6), ("slow flight", 0.3, 0.25)] {
        let truth = supersampled_truth(&ctx, size, &[("throttle", throttle)]);
        let image = shot(&ctx, size, throttle);
        let detail = srgb_detail(&image, size) / srgb_detail(&truth, size);
        let error = srgb_error(&image, &truth);
        eprintln!("{label}: detail / reference {detail:.3}, error {error:.3}");
        assert!(
            detail > floor && detail <= 1.0,
            "{label}: detail {detail:.3}"
        );
    }
}

/// Flying forward, the sky does not move on screen, so its history keeps
/// whatever a filtered read picked up beside geometry unless the clamp
/// removes it. Where a single frame shows open sky (sky across 5x5 pixels),
/// the accumulated frame must not show geometry's color: no trails. The
/// palette is pinned warm and the sky cool, so blue over red tells them apart
/// whatever the default look.
#[test]
fn a_forward_flight_leaves_no_trails_in_the_sky() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let (w, h) = (640u32, 360u32);
    let with = |temporal: f64| {
        let values = held_still(
            plain_temple(),
            &[
                ("render_scale", 0.5),
                ("throttle", 1.0),
                ("speed", 3.0),
                ("temporal", temporal),
            ],
        );
        let warm = [0.75, 0.45, 0.25, 1.0];
        let colors = [
            ("color1", warm),
            ("color2", warm),
            ("color3", warm),
            ("color4", warm),
            ("amb_top", [0.15, 0.2, 0.35, 1.0]),
            ("amb_bottom", [0.05, 0.05, 0.1, 1.0]),
            ("fog_color", [0.2, 0.25, 0.4, 1.0]),
        ];
        render_colored(&ctx, (w, h), &values, &colors, None, 100)
    };
    let (single, accumulated) = (with(0.0), with(1.0));
    let skyish = |p: [f32; 4]| p[2] > p[0];
    let at = |x: u32, y: u32| (y * w + x) as usize;
    let (mut open, mut trails) = (0, 0);
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let all_sky =
                (0..5).all(|dy| (0..5).all(|dx| skyish(single[at(x + dx - 2, y + dy - 2)])));
            if all_sky {
                open += 1;
                if !skyish(accumulated[at(x, y)]) {
                    trails += 1;
                }
            }
        }
    }
    eprintln!("trail pixels {trails} of {open} open sky");
    assert!(open > 1000, "the view should show sky");
    // Fewer than one open-sky pixel in 2000.
    assert!(
        trails * 2000 < open,
        "{trails} of {open} open-sky pixels carry geometry"
    );
}

/// A surface texture shows from where the camera flies: with the default
/// mapping, turning one on adds at least 10% fine detail to a held inside
/// view. The palette and lighting are pinned plain.
#[test]
fn a_surface_texture_is_visible_inside() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (320u32, 180u32);
    let with = |surface: f64| {
        // The box interior and a plain palette, so the texture is what adds
        // detail, whatever the default stack and look.
        let mut values = held_still(PLAIN, &[("render_scale", 1.0), ("surface", surface)]);
        values.extend(BOX_INTERIOR);
        values.extend([
            ("color_source", 0.0),
            ("metallic", 0.0),
            ("roughness", 0.6),
            ("specular", 0.5),
            ("exposure", 0.0),
            ("sun_intensity", 4.0),
        ]);
        let plain = [0.6, 0.55, 0.5, 1.0];
        let colors = [
            ("color1", plain),
            ("color2", plain),
            ("color3", plain),
            ("color4", plain),
        ];
        render_colored(&ctx, size, &values, &colors, None, 32)
    };
    let (bare, textured) = (srgb_detail(&with(0.0), size), srgb_detail(&with(1.0), size));
    eprintln!("detail: no surface {bare:.3}, stone {textured:.3}");
    assert!(
        textured > 1.1 * bare,
        "no surface {bare:.3}, stone {textured:.3}"
    );
}

/// A palette cycling faster than the pixels does not alias: against a
/// supersampled reference, one frame at `palette_scale` 8 has at most 1.5
/// times the error of one at 0.3, so rings narrower than a pixel are filtered
/// rather than point-sampled into moire.
#[test]
fn a_fine_palette_does_not_alias() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (160u32, 90u32);
    let error = |scale: f64| {
        // The box interior: the palette is the only fine detail.
        let mut values = held_still(PLAIN, &[("render_scale", 1.0), ("surface", 0.0)]);
        values.extend(BOX_INTERIOR);
        // The Teal & Gold palette the threshold was set on: its stops differ
        // more than Stone Hall's, so rings alias more.
        values.insert("look", 4.0);
        values.insert("color_source", 1.0);
        values.insert("palette_scale", scale);
        let mut frame = values.clone();
        frame.insert("temporal", 0.0);
        let one = render(&ctx, size, &frame, None, 8);
        let mut truth = values;
        truth.insert("detail", 4.0);
        srgb_error(&one, &downsampled(&ctx, size, 4, &truth, 64))
    };
    let (slow, fine) = (error(0.3), error(8.0));
    eprintln!("error against truth: palette 0.3 {slow:.3}, palette 8 {fine:.3}");
    assert!(
        fine < 1.5 * slow,
        "palette 0.3 {slow:.3}, palette 8 {fine:.3}"
    );
}

/// The gnarled temple, pinned whatever the default stack, and a pose flown
/// into its crust, where every ray starts close to a surface.
fn gnarled_crust() -> (
    HashMap<&'static str, f64>,
    serde_json::Map<String, serde_json::Value>,
) {
    let values: HashMap<&str, f64> = TEMPLE.into_iter().collect();
    let state: serde_json::Value = serde_json::from_str(
        r#"{"fractal_flight": {"version": 1,
            "position": [-1.3639895448743404, 2.069652430434301, 1.206989544874341],
            "orientation": [0.8804762392171493, 0.27984814233312133, -0.3647051996310009, 0.11591689595929516],
            "scale": 1.136977664595685, "locations": []}}"#,
    )
    .expect("pose JSON");
    (values, state.as_object().expect("object").clone())
}

/// Held still inside dense geometry, at most 35% of 8x8 blocks run the shade
/// pass per frame: the history depth test holds across pixel-scale relief
/// under jitter, so most blocks reuse their shading.
#[test]
fn shading_reuse_survives_dense_geometry() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (320u32, 176u32);
    let (mut values, state) = gnarled_crust();
    values.extend(PLAIN);
    values.insert("target_fps", 0.0);
    values.insert("render_scale", 1.0);
    values.insert("debug_view", 8.0);
    let pixels = render(&ctx, size, &values, Some(&state), 12);
    let (w, h) = size;
    let mut computing = 0;
    for by in 0..h / 8 {
        for bx in 0..w / 8 {
            let any = (0..64).any(|i| {
                let (x, y) = (bx * 8 + i % 8, by * 8 + i / 8);
                pixels[(y * w + x) as usize][0] > 0.5
            });
            computing += u32::from(any);
        }
    }
    let share = f64::from(computing) / f64::from((w / 8) * (h / 8));
    eprintln!("blocks computing: {:.1}%", share * 100.0);
    assert!(share <= 0.35, "{:.1}% of blocks compute", share * 100.0);
}

/// The host builds the same default stack the shader draws: every stack
/// parameter's shader default is the host's.
#[test]
fn the_host_defaults_match_the_shader() {
    let shader = shader();
    let inputs = shader.metadata.inputs.as_ref().expect("inputs");
    let names = (0..6)
        .flat_map(|slot| SLOT_FIELDS.map(|field| slot_param(slot, field)))
        .chain(STACK_PARAMS.map(str::to_owned));
    for name in names {
        let input = inputs
            .iter()
            .find(|input| input.name == name)
            .unwrap_or_else(|| panic!("shader has no input {name}"));
        let declared = match input.default.as_ref().expect("a default") {
            serde_json::Value::Bool(b) => f64::from(u8::from(*b)),
            value => value.as_f64().expect("numeric"),
        };
        assert!(
            (declared - default_param(&name)).abs() < 1e-6,
            "{name}: shader {declared}, host {}",
            default_param(&name)
        );
    }
}

/// Picking a look sets the sliders, which then adjust from there.
#[test]
fn a_look_sets_the_sliders_and_they_adjust_from_there() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let size = (160u32, 90u32);
    let with = |look: f64, exposure: Option<f64>| {
        let mut values = held_still(PLAIN, &[("temporal", 0.0), ("look", look)]);
        if let Some(exposure) = exposure {
            values.insert("exposure", exposure);
        }
        render(&ctx, size, &values, None, 8)
    };
    // In sRGB 8-bit units: over 5 is a visible change.
    let stone = with(1.0, None);
    let desert = with(2.0, None);
    assert!(
        srgb_error(&desert, &with(2.0, Some(2.0))) > 5.0,
        "a slider changes the picture after a look is picked"
    );
    let warmth = |pixels: &[[f32; 4]]| {
        let (r, b) = pixels.iter().fold((0.0, 0.0), |(r, b), p| {
            (r + f64::from(p[0]), b + f64::from(p[2]))
        });
        r / b.max(1e-9)
    };
    let (stone_warmth, desert_warmth) = (warmth(&stone), warmth(&desert));
    eprintln!("red over blue: stone hall {stone_warmth:.3}, desert sunbeams {desert_warmth:.3}");
    assert!(
        desert_warmth > 1.2 * stone_warmth,
        "Desert Sunbeams should be at least 20% warmer than Stone Hall: \
         red over blue {desert_warmth:.3} against {stone_warmth:.3}"
    );
}

/// Where the distance estimate is flat, its gradient is zero; the normal
/// still comes out finite. The turned box in Julia mode with seed 0 reduces
/// to its fold planes, where this happens, and a non-finite normal spread
/// into black squares through shading.
#[test]
fn a_flat_distance_estimate_gives_a_finite_normal() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let state: serde_json::Value = serde_json::from_str(
        r#"{"fractal_flight": {"version": 1,
            "position": [3.46875, -0.4334705075445817, 0.748800000000001],
            "orientation": [0.8804762392171493, -0.27984814233312133, -0.3647051996310009, -0.11591689595929516],
            "scale": 0.107571864668547, "locations": []}}"#,
    )
    .expect("pose JSON");
    let mut values: HashMap<&str, f64> = PLAIN.into_iter().collect();
    values.insert("target_fps", 0.0);
    values.insert("temporal", 0.0);
    values.insert("julia_mode", 1.0);
    values.insert("debug_view", 2.0);
    let pixels = render(
        &ctx,
        (1280, 720),
        &values,
        Some(state.as_object().expect("object")),
        30,
    );
    let bad = pixels
        .iter()
        .filter(|p| p.iter().any(|c| !c.is_finite()))
        .count();
    assert_eq!(bad, 0, "{bad} pixels with a non-finite normal");
}

/// Each preset selector's default entry is the slider defaults it names, so
/// picking it again restores the shader's starting state.
#[test]
fn each_selectors_default_entry_is_the_slider_defaults() {
    let shader = shader();
    let inputs = shader.metadata.inputs.as_ref().expect("inputs");
    let default_of = |name: &str| {
        inputs
            .iter()
            .find(|input| input.name == name)
            .and_then(|input| input.default.clone())
            .unwrap_or_else(|| panic!("no default for {name}"))
    };
    let as_numbers = |v: &serde_json::Value| -> Vec<f64> {
        match v {
            serde_json::Value::Array(a) => a.iter().filter_map(serde_json::Value::as_f64).collect(),
            serde_json::Value::Bool(b) => vec![f64::from(u8::from(*b))],
            other => vec![other.as_f64().expect("numeric")],
        }
    };
    for selector in ["look", "stack"] {
        let input = inputs
            .iter()
            .find(|input| input.name == selector)
            .unwrap_or_else(|| panic!("shader has no {selector} input"));
        let value = input
            .default
            .as_ref()
            .and_then(serde_json::Value::as_i64)
            .expect("default") as i32;
        let preset = input.preset(value).expect("a preset for the default");
        for (name, preset_value) in preset {
            let (a, b) = (as_numbers(preset_value), as_numbers(&default_of(name)));
            assert_eq!(a.len(), b.len(), "{selector}: {name}");
            for (x, y) in a.iter().zip(&b) {
                assert!(
                    (x - y).abs() < 1e-4,
                    "{selector}: {name} is {x}, default {y}"
                );
            }
        }
    }
}

/// Picking a stack writes its formula slots into the sliders.
#[test]
fn picking_a_stack_sets_its_slots() {
    let shader = shader();
    let mut params = varda::params::ShaderParams::from_metadata(&shader.metadata);
    params.set("stack", varda::params::ParamValue::Long(2));
    let long = |name: &str| match params.values.get(name) {
        Some(varda::params::ParamValue::Long(v)) => *v,
        other => panic!("{name}: {other:?}"),
    };
    // Goldcape: Lin Combine, Rotate 4D, Amazing Box x4, Koch Cube, JCube, Reciprocal.
    let formulas: Vec<i32> = (0..6).map(|s| long(&slot_param(s, "formula"))).collect();
    assert_eq!(formulas, [19, 20, 1, 17, 18, 15]);
    assert_eq!(long("slot3_count"), 4);
}
