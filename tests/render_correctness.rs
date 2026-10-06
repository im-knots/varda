//! Render-correctness tests.
//!
//! Render the mixer headless, read back the linear-light composite, and check
//! pixel values with a closed-form answer (opacity, crossfader, zero-opacity
//! culling, blend-mode algebra, passthrough).
//!
//! Tonemap is `Bypass` so the `Rgba16Float` composite holds raw linear values.
//! Colors use only 0.0 / 1.0 channels (gamma-invariant), except
//! crossfader-at-0.5, a true linear midpoint.
//!
//! Skips without a GPU adapter.

use varda::{
    BlendMode,
    audio::AudioData,
    deck::Deck,
    mixer::Mixer,
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
    renderer::tonemap::TonemapMode,
};

/// Small target: solid-color compositing is uniform per pixel. The readback
/// helper pads rows to 256 bytes.
const W: u32 = 16;
const H: u32 = 16;

mod common;
use common::headless_gpu;
use varda::testing::read_rgba16f;

/// Advance the mixer by one frame with silent audio and no modulation, on the
/// wall clock.
fn render_once(ctx: &GpuContext, mixer: &mut Mixer) {
    render_frame(ctx, mixer, None);
}

/// One frame at a given point on the free-running clock.
///
/// Tests that grade change between frames use this rather than
/// [`render_once`]: on the wall clock `TIME` advances by the last frame's
/// render time, which under a software rasterizer jitters more than the
/// effect being measured.
fn render_at(ctx: &GpuContext, mixer: &mut Mixer, frame: usize) {
    /// The authoring frame rate.
    const FPS: f32 = 60.0;
    render_frame(ctx, mixer, Some(frame as f32 / FPS));
}

/// Disable frame skipping for every measurement in this file.
///
/// Decks default to `DeckRenderFps::Auto`, which skips a deck whose render
/// cost is over budget and repeats its previous picture. Under a software
/// rasterizer that is most frames, so any test that cares which frame
/// something happened on would measure the scheduler instead.
fn disable_frame_skipping(mixer: &mut Mixer) {
    for channel in mixer.channels_mut() {
        for slot in &mut channel.decks {
            slot.render_fps = varda::channel::DeckRenderFps::Fixed(0);
        }
    }
}

fn render_frame(ctx: &GpuContext, mixer: &mut Mixer, free_run_time: Option<f32>) {
    disable_frame_skipping(mixer);
    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();
    let inputs = varda::mixer::FrameInputs {
        audio_data: &audio,
        audio_values: &audio_values,
        analyzer_values: &analyzer_values,
        beat_time: None,
        transport: None,
        free_run_time,
        write_param: varda::param_router::write_macro_target,
    };
    mixer.render(ctx, &inputs, 60, &[]).expect("render");
}

/// Render one frame and read it back at the default test size.
fn render_and_read(ctx: &GpuContext, mixer: &mut Mixer) -> Vec<[f32; 4]> {
    render_once(ctx, mixer);
    read_rgba16f(ctx, mixer.composite_texture(), W, H)
}

/// Center pixel of the readback, representative for uniform solid composites.
fn center(pixels: &[[f32; 4]]) -> [f32; 4] {
    pixels[((H / 2) * W + W / 2) as usize]
}

/// A tap deck built through its provider, as the engine builds one.
fn tap_deck(ctx: &GpuContext, point: &varda::tap::TapPoint) -> Deck {
    use varda::source::DeckSourceProvider;
    let shaders = varda::registry::ShaderRegistry::new();
    let mut services = varda::source::Services::new();
    let mut env = varda::source::SourceEnv {
        gpu: ctx,
        width: W,
        height: H,
        services: &mut services,
        shaders: &shaders,
        channels: &[],
    };
    let source = varda::tap::TapProvider
        .create(&varda::tap::Tap::config_for(point), &mut env)
        .expect("tap deck");
    Deck::from_source(ctx, source, W, H)
}

fn new_mixer(ctx: &GpuContext) -> Mixer {
    let mut mixer = Mixer::new(ctx, W, H).expect("mixer");
    // Bypass tonemap so the composite holds raw linear values.
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    mixer
}

fn assert_hi(v: f32, label: &str) {
    assert!(v > 0.85, "{label}: expected ~1.0, got {v}");
}
fn assert_lo(v: f32, label: &str) {
    assert!(v < 0.15, "{label}: expected ~0.0, got {v}");
}
fn assert_near(v: f32, target: f32, tol: f32, label: &str) {
    assert!(
        (v - target).abs() <= tol,
        "{label}: expected ~{target}, got {v}"
    );
}

// ── Compositing capacity ─────────────────────────────────────────────

/// The channel compositor writes one params ring slot per compositing deck,
/// and the ring has a fixed `MAX_DRAW_SLOTS` = 16 (`renderer/blit.rs`) with no
/// growth, unlike `PolygonBlitPipeline::ensure_ring_slots`. More decks than
/// slots must still render correctly.
///
/// Stacked opaque Normal decks show only the top one, so a correct run shows
/// the last deck's color and raises no GPU fault.
#[test]
fn channel_composites_more_decks_than_ring_slots() {
    const DECKS: usize = 20;

    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch = mixer.channel_mut(0).unwrap();
    for i in 0..DECKS {
        // Every deck below the top is red and the top is green, so the check
        // tells "deck 20 drew" from "deck 16 drew".
        let color = if i == DECKS - 1 {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        };
        let deck = Deck::solid_color(&ctx, color, W, H);
        ch.add_deck(deck);
    }

    let faults_before = ctx.errors.fault_count();
    let px = center(&render_and_read(&ctx, &mut mixer));
    let faults = ctx.errors.take_faults();

    assert_eq!(
        ctx.errors.fault_count(),
        faults_before,
        "compositing {DECKS} decks raised GPU faults (ring buffer holds 16 slots): {:?}",
        faults.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    assert_hi(px[1], "top-deck.G");
    assert_lo(px[0], "top-deck.R");
}

// ── Passthrough / opacity ────────────────────────────────────────────

#[test]
fn full_opacity_solid_deck_renders_its_color() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(deck);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_hi(px[0], "red.R");
    assert_lo(px[1], "red.G");
    assert_lo(px[2], "red.B");
}

#[test]
fn zero_opacity_deck_is_culled_from_output() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H);
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(deck);
    ch.set_deck_opacity(0, 0.0);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_lo(px[0], "culled.R");
    assert_lo(px[1], "culled.G");
    assert_lo(px[2], "culled.B");
}

/// Opacity is linear over black: a white deck at opacity `o` composites to
/// brightness `o` (premultiplied alpha, linear light). Re-blending a
/// premultiplied composite as straight alpha would give opacity² (0.25 at
/// half).
#[test]
fn opacity_is_linear_over_black() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let brightness_at = |opacity: f32| {
        let mut mixer = new_mixer(&ctx);
        let deck = Deck::solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H);
        let ch = mixer.channel_mut(0).unwrap();
        ch.add_deck(deck);
        ch.set_deck_opacity(0, opacity);
        center(&render_and_read(&ctx, &mut mixer))[0]
    };

    assert_lo(brightness_at(0.0), "opacity0");
    assert_near(brightness_at(0.25), 0.25, 0.05, "opacity0.25");
    assert_near(brightness_at(0.5), 0.5, 0.05, "opacity0.5");
    assert_hi(brightness_at(1.0), "opacity1");
}

/// The subsequent-channel composite path (composite.wgsl, premultiplied
/// source) also avoids double-darkening: at crossfader=1 a half-opacity white
/// deck in B composites to ~0.5, not 0.25.
#[test]
fn subsequent_channel_partial_opacity_is_linear() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    // Channel A opaque black so B is the visible partial layer at crossfader=1.
    let a = Deck::solid_color(&ctx, [0.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(a);
    let b = Deck::solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H);
    let ch_b = mixer.channel_mut(1).unwrap();
    ch_b.add_deck(b);
    ch_b.set_deck_opacity(0, 0.5);
    mixer.set_crossfader(1.0);

    let px = center(&render_and_read(&ctx, &mut mixer));
    // Half-opacity white B over opaque-black A → ~0.5 linear (not 0.25).
    assert_near(px[0], 0.5, 0.1, "chB.R");
    assert_near(px[1], 0.5, 0.1, "chB.G");
    assert_near(px[2], 0.5, 0.1, "chB.B");
}

// ── Crossfader ───────────────────────────────────────────────────────

fn crossfade_mixer(ctx: &GpuContext) -> Mixer {
    let mut mixer = new_mixer(ctx);
    let a = Deck::solid_color(ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    let b = Deck::solid_color(ctx, [0.0, 0.0, 1.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(a); // channel A = red
    mixer.channel_mut(1).unwrap().add_deck(b); // channel B = blue
    mixer
}

#[test]
fn crossfader_at_zero_shows_channel_a() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = crossfade_mixer(&ctx);
    mixer.set_crossfader(0.0);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_hi(px[0], "xf0.R");
    assert_lo(px[2], "xf0.B");
}

#[test]
fn crossfader_at_one_shows_channel_b() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = crossfade_mixer(&ctx);
    mixer.set_crossfader(1.0);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_lo(px[0], "xf1.R");
    assert_hi(px[2], "xf1.B");
}

#[test]
fn crossfader_at_half_blends_both_channels() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = crossfade_mixer(&ctx);
    mixer.set_crossfader(0.5);
    let px = center(&render_and_read(&ctx, &mut mixer));
    // Linear midpoint of red and blue on the pre-tonemap target.
    assert_near(px[0], 0.5, 0.2, "xf.5.R");
    assert_near(px[2], 0.5, 0.2, "xf.5.B");
}

// ── Blend-mode algebra (GPU only) ───────────────────────────────────

/// base (red, Normal) with a top deck (green) in the given blend mode.
fn blend_mixer(ctx: &GpuContext, top_mode: BlendMode) -> Mixer {
    let mut mixer = new_mixer(ctx);
    let base = Deck::solid_color(ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    let top = Deck::solid_color(ctx, [0.0, 1.0, 0.0, 1.0], W, H);
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(base);
    ch.add_deck(top);
    ch.set_deck_blend_mode(1, top_mode);
    mixer
}

#[test]
fn blend_normal_top_covers_base() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = blend_mixer(&ctx, BlendMode::Normal);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_lo(px[0], "normal.R"); // red base hidden
    assert_hi(px[1], "normal.G"); // green top visible
}

#[test]
fn blend_add_sums_base_and_top() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = blend_mixer(&ctx, BlendMode::Add);
    let px = center(&render_and_read(&ctx, &mut mixer));
    // red + green → yellow: both channels high.
    assert_hi(px[0], "add.R");
    assert_hi(px[1], "add.G");
    assert_lo(px[2], "add.B");
}

#[test]
fn blend_multiply_of_disjoint_primaries_is_black() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = blend_mixer(&ctx, BlendMode::Multiply);
    let px = center(&render_and_read(&ctx, &mut mixer));
    // red (1,0,0) * green (0,1,0) → (0,0,0).
    assert_lo(px[0], "mul.R");
    assert_lo(px[1], "mul.G");
    assert_lo(px[2], "mul.B");
}

// ── Blend space: pivot modes ────────────────────────────────────────
//
// Overlay, Hard Light, and Soft Light pin a branch (or, for Pegtop Soft Light,
// an identity point) to 0.5, perceptual middle gray in a gamma-encoded space.
// Middle gray is linear 0.214, so evaluating them on linear operands moves the
// pivot to sRGB 0.735. The 0.0/1.0 blend tests above are gamma-invariant and
// can't see this, so these use mid-tones.

/// Linear value of sRGB 0.5 (perceptual middle gray).
const MID_GREY_LINEAR: f32 = 0.214_041_14;

/// `Rgba16Float` carries ~10-11 mantissa bits; 5e-3 is above its resolution and
/// far below the ~0.1 differences these tests check.
const BLEND_TOL: f32 = 0.005;

/// Base deck at `dst` gray, top deck at `src` gray in `mode`. Grays keep R=G=B
/// so any channel witnesses the result.
fn grey_blend_mixer(ctx: &GpuContext, mode: BlendMode, src: f32, dst: f32) -> Mixer {
    let mut mixer = new_mixer(ctx);
    let base = Deck::solid_color(ctx, [dst, dst, dst, 1.0], W, H);
    let top = Deck::solid_color(ctx, [src, src, src, 1.0], W, H);
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(base);
    ch.add_deck(top);
    ch.set_deck_blend_mode(1, mode);
    mixer
}

fn assert_grey(px: [f32; 4], target: f32, label: &str) {
    assert_near(px[0], target, BLEND_TOL, &format!("{label}.R"));
    assert_near(px[1], target, BLEND_TOL, &format!("{label}.G"));
    assert_near(px[2], target, BLEND_TOL, &format!("{label}.B"));
}

/// Overlay of middle gray over middle gray is identity.
///
/// Both Overlay branches agree at the pivot, so the result is exact whichever
/// side `dst < 0.5` picks. Linear-operand evaluation gives 2·0.214·0.214 = 0.092.
#[test]
fn blend_overlay_of_middle_grey_is_identity() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = grey_blend_mixer(&ctx, BlendMode::Overlay, MID_GREY_LINEAR, MID_GREY_LINEAR);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_grey(px, MID_GREY_LINEAR, "overlay_mid");
}

/// Hard Light of middle gray over middle gray is identity (same pivot, roles
/// swapped). Linear-operand evaluation yields 0.092.
#[test]
fn blend_hard_light_of_middle_grey_is_identity() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = grey_blend_mixer(&ctx, BlendMode::HardLight, MID_GREY_LINEAR, MID_GREY_LINEAR);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_grey(px, MID_GREY_LINEAR, "hard_light_mid");
}

/// Pegtop Soft Light is identity when the source is middle gray: the
/// `(1-2s)` term vanishes at s = 0.5, leaving `2·0.5·d = d`. Linear-operand
/// evaluation yields 0.118.
#[test]
fn blend_soft_light_of_middle_grey_is_identity() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = grey_blend_mixer(&ctx, BlendMode::SoftLight, MID_GREY_LINEAR, MID_GREY_LINEAR);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_grey(px, MID_GREY_LINEAR, "soft_light_mid");
}

/// Off-pivot reference from the sRGB-operand formula: src sRGB 0.25 over dst
/// sRGB 0.75 takes Overlay's screen branch, `1 - 2·(1-0.25)·(1-0.75) = 0.625`
/// → linear 0.34851. Linear-operand evaluation gives 0.0936.
#[test]
fn blend_overlay_off_pivot_matches_perceptual_reference() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    // sRGB 0.25 and 0.75 expressed as the linear values the deck stores.
    let mut mixer = grey_blend_mixer(&ctx, BlendMode::Overlay, 0.050_875_9, 0.522_522_2);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_grey(px, 0.348_51, "overlay_off_pivot");
}

/// Screen is a physical mode and stays on linear operands.
///
/// Linear (correct): `1 - (1-0.214)² = 0.3823`. Encoded operands would give
/// `1 - 0.5² = 0.75` → linear 0.5225.
#[test]
fn blend_screen_stays_linear() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = grey_blend_mixer(&ctx, BlendMode::Screen, MID_GREY_LINEAR, MID_GREY_LINEAR);
    let px = center(&render_and_read(&ctx, &mut mixer));
    let expected = 1.0 - (1.0 - MID_GREY_LINEAR) * (1.0 - MID_GREY_LINEAR);
    assert_grey(px, expected, "screen_linear");
}

// ── Unified color path ──────────────────────────────────────────────
//
// Numeric checks on the deck stage: shadow precision, HDR headroom, and effect
// precision.

/// Deck targets preserve shadow gradation.
///
/// 8-bit linear storage collapses the bottom 41 sRGB levels into 6 codes: 0.002
/// and 0.004 both round to code 1. On a float target they stay distinct. Every
/// input in a dark ramp must come out distinct.
#[test]
fn deck_stage_preserves_shadow_gradation() {
    let Some(ctx) = headless_gpu() else {
        return;
    };

    // Deep-shadow ramp, every step below linear 0.02 (~5.7 stops down), where
    // 8-bit linear has ~6 codes.
    let ramp = [0.001_f32, 0.002, 0.004, 0.006, 0.008, 0.012, 0.016];

    let mut observed = Vec::new();
    for v in ramp {
        let mut mixer = new_mixer(&ctx);
        let deck = Deck::solid_color(&ctx, [v, v, v, 1.0], W, H);
        mixer.channel_mut(0).unwrap().add_deck(deck);
        observed.push(center(&render_and_read(&ctx, &mut mixer))[0]);
    }

    // Every distinct input yields a distinct output. Pairwise, so a failure
    // names which steps collapsed.
    for i in 0..observed.len() {
        for j in (i + 1)..observed.len() {
            assert!(
                (observed[i] - observed[j]).abs() > 1e-5,
                "shadow gradation collapsed: input {} and {} both rendered as {} \
                 (full ramp {:?}) — the deck stage is quantizing linear light",
                ramp[i],
                ramp[j],
                observed[i],
                observed
            );
        }
    }

    // And the ramp stays monotonic.
    for w in observed.windows(2) {
        assert!(
            w[1] > w[0],
            "shadow ramp not monotonic: {w:?} (full ramp {observed:?})"
        );
    }
}

/// A deck can hand values above 1.0 to the compositor, so tonemap operators
/// see scene-referred highlights.
#[test]
fn deck_headroom_above_one_survives_to_the_composite() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    // A generator-style value well above display white.
    let deck = Deck::solid_color(&ctx, [4.0, 2.0, 1.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(deck);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_near(px[0], 4.0, 0.05, "headroom.R");
    assert_near(px[1], 2.0, 0.05, "headroom.G");
    assert_near(px[2], 1.0, 0.05, "headroom.B");
    assert!(
        px[0] > 1.0 && px[1] > 1.0,
        "deck output was clamped to display range: {px:?}"
    );
}

/// Deck effects run at the compositing format, like channel and master
/// effects, so precision doesn't depend on which chain an effect is in.
///
/// Checks the format; `deck_effect_transforms_pixels` checks pixels. For
/// mid-tones 8-bit and float differ by only ~0.001, so a format regression
/// would pass a pixel test.
#[test]
fn deck_effects_run_at_composite_precision() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    // Repo path, not `get_bundled_shader_path()`, which finds shaders only in a
    // packaged .app/tarball and returns None under `cargo test`. A missing
    // shader fails; only a missing GPU skips.
    let invert = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/invert.fs");
    assert!(
        invert.exists(),
        "shaders/invert.fs missing from the repo at {}",
        invert.display()
    );
    let load = || varda::isf::ISFShader::from_file(&invert).expect("parse invert.fs");

    let deck_fx = varda::deck::Effect::new(&ctx, load()).expect("deck effect");
    let channel_fx = varda::deck::Effect::new_with_format(&ctx, load(), ctx.compositing_format)
        .expect("channel effect");

    assert_eq!(
        deck_fx.target_format, ctx.compositing_format,
        "deck effects must target the color-path format, not an 8-bit intermediate"
    );
    assert_eq!(
        deck_fx.target_format, channel_fx.target_format,
        "deck and channel effects must agree on precision"
    );
}

/// Every bundled shader produces a valid render pipeline.
///
/// A `Filtering` sampler needs texture entries declared filterable; wgpu
/// rejects a mismatch at `create_render_pipeline`, which aborts on loading a
/// multipass shader. Other GPU tests here use solid decks or single-pass
/// filters (`num_pass_buffers == 0`), so this builds every bundled shader with
/// the app's constructors: `Deck::new` for generators, `Effect::new` for
/// filters.
#[test]
fn every_bundled_shader_builds_a_pipeline() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    assert!(dir.is_dir(), "shaders/ missing at {}", dir.display());

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read shaders/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("fs"))
        .collect();
    entries.sort();
    assert!(
        entries.len() > 100,
        "expected the full bundled library, found {} .fs files",
        entries.len()
    );

    let mut multipass = 0usize;
    let mut filters = 0usize;
    let mut generators = 0usize;
    let mut transitions = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for path in &entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let shader = match varda::isf::ISFShader::from_file(path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{name}: parse failed: {e}"));
                continue;
            }
        };
        if shader
            .metadata
            .passes
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|p| p.target.is_some())
        {
            multipass += 1;
        }

        // Classify by declared image inputs and use the production
        // constructors, so binding counts come from production code.
        let has_input = |want: &str| {
            shader
                .metadata
                .inputs
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .any(|i| i.input_type.eq_ignore_ascii_case("image") && i.name == want)
        };
        let is_transition = has_input("startImage") && has_input("endImage");
        let is_filter = !is_transition && has_input("inputImage");

        let kind = if is_transition {
            transitions += 1;
            "transition"
        } else if is_filter {
            filters += 1;
            "filter"
        } else {
            generators += 1;
            "generator"
        };

        let built = if is_transition {
            // Transitions have their own pipeline and layout.
            match varda::isf::compile_glsl_to_spirv(&shader.fragment_source, &name) {
                Ok(spirv) => varda::renderer::TransitionPipeline::new(
                    &ctx.device,
                    &spirv,
                    ctx.compositing_format,
                )
                .map(|_| ()),
                Err(e) => Err(e),
            }
        } else if is_filter {
            varda::deck::Effect::new(&ctx, shader).map(|_| ())
        } else {
            Deck::from_shader(&ctx, shader, W, H).map(|_| ())
        };
        if let Err(e) = built {
            failures.push(format!("{name} ({kind}): {e:#}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} bundled shaders failed to build:\n  {}",
        failures.len(),
        entries.len(),
        failures.join("\n  ")
    );
    // Fail if classification collapses and the test covers nothing.
    assert!(
        multipass >= 8,
        "expected at least 8 multipass shaders (the case that regressed), saw {multipass}"
    );
    assert!(
        generators > 0 && filters > 0 && transitions > 0,
        "expected all three shader kinds covered \
         (generators={generators} filters={filters} transitions={transitions})"
    );
}

/// A deck effect transforms the deck's pixels.
///
/// `ParamValue::Bool` is written as a `u32`; a shader declaring the input as
/// `float` reads `1.4e-45` and the toggle never turns on, so the effect runs
/// and does nothing.
///
/// `invert` with default params is a full inversion, so black comes out white.
/// Two effects cancel back to black, which also checks deck ping-pong parity.
#[test]
fn deck_effect_transforms_pixels() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let invert = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/invert.fs");
    assert!(invert.exists(), "shaders/invert.fs missing");

    let run = |n_effects: usize| -> [f32; 4] {
        let mut mixer = new_mixer(&ctx);
        let deck = Deck::solid_color(&ctx, [0.0, 0.0, 0.0, 1.0], W, H);
        let ch = mixer.channel_mut(0).unwrap();
        ch.add_deck(deck);
        for _ in 0..n_effects {
            let shader = varda::isf::ISFShader::from_file(&invert).expect("parse invert.fs");
            let fx = varda::deck::Effect::new(&ctx, shader).expect("deck effect");
            ch.decks[0].deck.add_effect(fx);
        }
        center(&render_and_read(&ctx, &mut mixer))
    };

    let none = run(0);
    assert_lo(none[0], "no-effect.R");

    let once = run(1);
    assert_hi(once[0], "inverted-once.R");
    assert_hi(once[1], "inverted-once.G");
    assert_hi(once[2], "inverted-once.B");

    let twice = run(2);
    assert_lo(twice[0], "inverted-twice.R");
    assert_lo(twice[1], "inverted-twice.G");
    assert_lo(twice[2], "inverted-twice.B");
}

// ── Generator framing ────────────────────────────────────────────────

/// `dull_skull` keeps its subject on screen for the whole animation.
///
/// An unbounded camera orbit crosses behind the backdrop past ~68° and renders
/// black for a third of the cycle. The swing is `sin(PHASE_TIME_1) *
/// sway_range` about the skull's mean position, which keeps the camera in
/// front of the backdrop.
///
/// Driven at maximum `speed` and `rot_speed` so a few hundred frames cover
/// minutes of a set. Measured at this resolution:
///
/// | metric                | bounded          | unbounded              |
/// |-----------------------|------------------|------------------------|
/// | dark-pixel fraction   | 0.000 throughout | 0.49–1.00 on 12/30     |
/// | mean horizontal edge  | 0.0034–0.0062    | 0.0000 while blacked   |
///
/// The dark fraction is the main check. The edge floor also catches frames
/// where the backdrop swallows the skull into a featureless blob
/// (0.0013–0.0024).
#[test]
fn dull_skull_stays_in_frame_for_the_whole_sway() {
    const SW: u32 = 320;
    const SH: u32 = 180;
    const FRAMES: usize = 400;
    const SAMPLE_EVERY: usize = 20;
    /// Bounded measures 0.000; unbounded reaches 1.000.
    const MAX_DARK_FRACTION: f32 = 0.25;
    /// Bounded measures 0.0034 at worst; a swallowed skull measures 0.0024.
    const MIN_EDGE: f32 = 0.0020;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/dull_skull.fs");
    let shader = varda::isf::ISFShader::from_file(&path).expect("parse dull_skull.fs");

    let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let mut deck = Deck::from_shader(&ctx, shader, SW, SH).expect("deck");
    deck.generator_params.set_float("speed", 3.0);
    deck.generator_params.set_float("rot_speed", 2.0);
    mixer.channel_mut(0).unwrap().add_deck(deck);

    // Thresholds were measured on gamma-encoded values, what the audience
    // sees. The composite is linear, so encode before measuring.
    let encode = |v: f32| -> f32 {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055
        }
    };

    let mut failures: Vec<String> = Vec::new();
    for frame in 1..=FRAMES {
        render_once(&ctx, &mut mixer);
        if !frame.is_multiple_of(SAMPLE_EVERY) {
            continue;
        }

        let lum: Vec<f32> = read_rgba16f(&ctx, mixer.composite_texture(), SW, SH)
            .iter()
            .map(|p| encode(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]))
            .collect();

        let dark = lum.iter().filter(|v| **v < 0.05).count() as f32 / lum.len() as f32;
        let mut edge = 0.0f32;
        for row in 0..SH as usize {
            for col in 1..SW as usize {
                edge += (lum[row * SW as usize + col] - lum[row * SW as usize + col - 1]).abs();
            }
        }
        edge /= (SH * (SW - 1)) as f32;

        if dark > MAX_DARK_FRACTION {
            failures.push(format!(
                "frame {frame}: {:.0}% of the frame is black",
                dark * 100.0
            ));
        } else if edge < MIN_EDGE {
            failures.push(format!(
                "frame {frame}: no discernible subject (edge {edge:.5})"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the camera left the skull on {} of {} sampled frames:\n  {}",
        failures.len(),
        FRAMES / SAMPLE_EVERY,
        failures.join("\n  ")
    );
}

/// Automating a control must not make the picture stutter.
///
/// A continuous parameter can still be an amplitude, setting where the image
/// is sampled rather than how fast it moves, and an LFO on an amplitude
/// sloshes the picture at the LFO's rate.
///
/// Per-frame luminance change of `liquid_light.fs` under a 1 Hz triangle LFO
/// sweeping Agitation 0.1..0.9, against the fader parked at the midpoint:
///
/// | Agitation implemented as | parked  | automated | ratio  |
/// |--------------------------|---------|-----------|--------|
/// | domain-warp gain         | 0.00135 | 0.01814   | 13.5x  |
/// | mixing rate (slot 1)     | 0.00475 | 0.00459   | 0.97x  |
///
/// A rate scores at or below 1.0 because the LFO spends half its time below
/// the midpoint. Much above 1.0 means the fader moves the fluid rather than
/// stirring it.
#[test]
fn liquid_light_agitation_survives_being_automated() {
    const SW: u32 = 256;
    const SH: u32 = 144;
    const WARMUP: usize = 300;
    const MEASURE: usize = 120;
    /// Triangle period in frames, a typical LFO.
    const PERIOD: usize = 60;
    const LOW: f32 = 0.1;
    const HIGH: f32 = 0.9;
    /// A rate scores 0.97x, an amplitude 13.5x.
    const MAX_RATIO: f32 = 3.0;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/liquid_light.fs");
    let shader = varda::isf::ISFShader::from_file(&path).expect("parse liquid_light.fs");

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_rgba16f(ctx, mixer.composite_texture(), SW, SH)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    // Median per-frame change over MEASURE frames, with Agitation set by
    // `automate` each frame. Median so one outlier can't decide it.
    let run = |automate: &dyn Fn(usize) -> f32| -> f32 {
        let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
        mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
        let mut deck = Deck::from_shader(&ctx, shader.clone(), SW, SH).expect("deck");
        deck.generator_params.set_float("flow_speed", 0.5);
        deck.generator_params.set_float("agitation", automate(0));
        mixer.channel_mut(0).unwrap().add_deck(deck);
        // No adaptive skipping: a skipped frame repeats the picture, and under a
        // software rasterizer most frames skip, making the median delta zero.
        mixer.channel_mut(0).unwrap().decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(0);

        // `render_at`, not `render_once`: equal clock steps, so slow frames
        // under a software rasterizer don't alias the animation into stillness.
        for frame in 0..WARMUP {
            render_at(&ctx, &mut mixer, frame);
        }
        let mut prev = luminance(&ctx, &mixer);
        let mut deltas = Vec::with_capacity(MEASURE);
        for frame in 0..MEASURE {
            mixer.channel_mut(0).unwrap().decks[0]
                .deck
                .generator_params
                .set_float("agitation", automate(frame));
            // `automate` keys the LFO off the measurement index, while the
            // clock continues from the warmup, so both stay monotonic.
            render_at(&ctx, &mut mixer, WARMUP + frame);
            let cur = luminance(&ctx, &mixer);
            deltas.push(mean_delta(&prev, &cur));
            prev = cur;
        }
        deltas.sort_by(|a, b| a.partial_cmp(b).expect("no NaN frames"));
        deltas[deltas.len() / 2]
    };

    let midpoint = f32::midpoint(LOW, HIGH);
    let parked = run(&|_| midpoint);
    let automated = run(&|frame: usize| {
        let phase = (frame % PERIOD) as f32 / PERIOD as f32;
        let triangle = if phase < 0.5 {
            phase * 2.0
        } else {
            2.0 - phase * 2.0
        };
        LOW + (HIGH - LOW) * triangle
    });

    // A frozen dish would make any automation look smooth.
    assert!(
        parked > 1e-4,
        "the dish is not moving with the fader parked ({parked:.6}), so this proves nothing"
    );

    let ratio = automated / parked;
    assert!(
        ratio <= MAX_RATIO,
        "automating Agitation makes the fluid slosh: {automated:.5} per frame under a \
         {PERIOD}-frame LFO against {parked:.5} parked ({ratio:.1}x — allowed {MAX_RATIO:.1}x). \
         Agitation is setting a position rather than a rate."
    );
}

/// Moving a rate fader changes the speed of the motion, not the frame shown.
///
/// Pixel-side check for the phase-accumulator contract; the source-side check
/// is `no_shader_scales_accumulated_phase_by_a_parameter` in
/// `tests/shader_param_contract_guard.rs`, which can't follow values across
/// function calls.
///
/// Uses `liquid_light`'s Dish Rotation because it is a pure rate (accumulator
/// slot 2, no other effect on the image). A fader with an instantaneous effect
/// too can't be told apart from a jump by a pixel metric.
///
/// Mean per-pixel luminance change between consecutive frames, 30 s of
/// accumulated phase in, with Dish Rotation nudged from 0.40 to 0.45:
///
/// | metric                       | correct | scaled |
/// |------------------------------|---------|--------|
/// | steady-state delta per frame | 0.0031  | 0.0021 |
/// | delta across the fader move  | 0.0034  | 0.0369 |
/// | ratio                        | 1.1x    | 17.6x  |
///
/// "Scaled" is `float dish = PHASE_TIME_2 * swirl;`.
#[test]
fn liquid_light_dish_rotation_changes_speed_rather_than_position() {
    const SW: u32 = 256;
    const SH: u32 = 144;
    /// 30 s at 60 fps. The jump grows with accumulated phase, so a short run
    /// would miss it.
    const WARMUP: usize = 1800;
    /// Frames of steady state to average the baseline over.
    const BASELINE_FRAMES: usize = 10;
    /// Correct measures 1.1x, scaled 17.6x; the threshold sits between with
    /// margin both ways.
    const MAX_RATIO: f32 = 5.0;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/liquid_light.fs");
    let shader = varda::isf::ISFShader::from_file(&path).expect("parse liquid_light.fs");

    let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let mut deck = Deck::from_shader(&ctx, shader, SW, SH).expect("deck");
    deck.generator_params.set_float("flow_speed", 1.0);
    deck.generator_params.set_float("swirl", 0.4);
    mixer.channel_mut(0).unwrap().add_deck(deck);
    // No adaptive skipping, so the metric grades the shader, not the scheduler.
    mixer.channel_mut(0).unwrap().decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(0);

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_rgba16f(ctx, mixer.composite_texture(), SW, SH)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    // Deterministic clock: this grades between-frame change, so wall-clock
    // frame cost must not enter the measurement.
    for frame in 0..WARMUP {
        render_at(&ctx, &mut mixer, frame);
    }

    let mut prev = luminance(&ctx, &mixer);
    let mut baseline = 0.0f32;
    for frame in 0..BASELINE_FRAMES {
        render_at(&ctx, &mut mixer, WARMUP + frame);
        let cur = luminance(&ctx, &mixer);
        baseline += mean_delta(&prev, &cur);
        prev = cur;
    }
    baseline /= BASELINE_FRAMES as f32;

    // A static image would make any jump look proportional.
    assert!(
        baseline > 1e-4,
        "the dish is not turning (baseline {baseline:.6}), so this proves nothing"
    );

    mixer.channel_mut(0).unwrap().decks[0]
        .deck
        .generator_params
        .set_float("swirl", 0.45);
    render_at(&ctx, &mut mixer, WARMUP + BASELINE_FRAMES);
    let jump = mean_delta(&prev, &luminance(&ctx, &mixer));

    assert!(
        jump <= baseline * MAX_RATIO,
        "moving Dish Rotation spun the dish: {jump:.4} against a steady-state {baseline:.4} \
         ({:.1}x — allowed {MAX_RATIO:.1}x). Accumulated phase is being scaled by a live \
         parameter again.",
        jump / baseline
    );
}

/// A shader that emits above display white reaches the compositor unclamped.
///
/// Shaders end with `max(col, 0.0)` rather than `clamp(col, 0.0, 1.0)`, keeping
/// the floor so negatives never reach the blend math. A clamping filter
/// removes headroom produced upstream.
///
/// `glow_bloom` is additive (`result = src.rgb + bloom * amount * color`), so
/// white input plus bloom exceeds 1.0. Params are pinned so the expected value
/// is deterministic.
#[test]
fn additive_filter_emits_above_display_white() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/glow_bloom.fs");
    assert!(path.exists(), "shaders/glow_bloom.fs missing");

    let mut mixer = new_mixer(&ctx);
    let deck = Deck::solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H);
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(deck);

    let shader = varda::isf::ISFShader::from_file(&path).expect("parse glow_bloom.fs");
    let mut fx = varda::deck::Effect::new(&ctx, shader).expect("deck effect");
    // threshold 0: all of a white input counts as bright; full glow amount.
    fx.params.set_float("threshold", 0.0);
    fx.params.set_float("glow_amount", 1.0);
    fx.params.set_color("glow_color", [1.0, 1.0, 1.0, 1.0]);
    ch.decks[0].deck.add_effect(fx);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert!(
        px[0] > 1.3,
        "additive bloom over white should exceed display white, got {px:?} \
         — a terminal clamp has come back somewhere in the chain"
    );
}

/// An effect fed smoothly changing input changes smoothly.
///
/// `chroma_flow` grades every pixel against a palette of anchor colors, and
/// each color group has its own flow direction. Auto mode derives anchors by
/// greedy farthest-point selection, which is discontinuous: near-tied
/// candidates swap on invisible changes, an anchor jumps, and the whole frame
/// regroups at once.
///
/// Measured as tail frame change over median change, against a fixed palette
/// as control (the harness and content without anchor motion). The palette
/// persists across frames, slots match the closest available anchor pair each
/// round, and the settle is short enough that a lagging anchor can't sweep a
/// flat region across a boundary in one frame.
///
/// | source          | per-frame | persisted | + closest-pair matching, short settle |
/// |-----------------|-----------|-----------|---------------------------------------|
/// | `dull_skull`    | 7.44x     | 5.77x     | 1.16x                                 |
/// | `liquid_light`  | 5.84x     | 1.10x     | 1.10x                                 |
///
/// The middle column used a different transport model, so it is indicative
/// only.
#[test]
fn chroma_flow_auto_palette_does_not_lurch_on_smooth_input() {
    /// One configuration's delta distribution, not only its ratio, so a
    /// failure shows what the frames did.
    struct Spike {
        ratio: f32,
        median: f32,
        p95: f32,
        lo: f32,
        hi: f32,
    }

    impl std::fmt::Display for Spike {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "ratio {:.2} (median {:.6}, p95 {:.6}, min {:.6}, max {:.6})",
                self.ratio, self.median, self.p95, self.lo, self.hi
            )
        }
    }

    const SW: u32 = 128;
    const SH: u32 = 128;
    const WARMUP: usize = 30;
    const MEASURE: usize = 90;
    /// How much worse than the fixed-palette control the auto palette may be.
    ///
    /// Relative to the control because the absolute figure depends on the
    /// effect and source as much as the palette; the control differs only in
    /// holding anchors still.
    ///
    /// Worst observed p95 ratio is 1.08 (`taste_of_noise` at stability 0); 1.3
    /// allows 20%. If this trips on a software rasterizer, find out why the tail
    /// moved rather than widening it.
    const MAX_SPIKE_RATIO: f32 = 1.3;
    /// The control settings graded, lowest to highest.
    const STABILITIES: [f32; 3] = [0.0, 0.5, 1.0];
    /// Slack on "steadier as the control rises". Metal reads 1.93 / 1.94 / 1.93,
    /// flat within noise. A real inversion is far larger (1.81 rising to 4.64).
    const MONOTONIC_TOLERANCE: f32 = 1.15;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let fx_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/chroma_flow.fs");
    let fx_shader = varda::isf::ISFShader::from_file(&fx_path).expect("parse chroma_flow.fs");

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_rgba16f(ctx, mixer.composite_texture(), SW, SH)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    // Tail-frame-over-median change for one configuration.
    //
    // p95 rather than max: the max is one frame of ninety and differs between
    // renderers (for `taste_of_noise` the top deltas run 1.43, 1.45, 1.45,
    // 1.47, 1.49, then 1.66). A lurch that matters lifts the whole tail.
    let spike_ratio = |src_name: &str, manual: bool, stability: f32| -> Spike {
        let src =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("shaders/{src_name}"));
        let src_shader = varda::isf::ISFShader::from_file(&src).expect("parse source");
        let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
        mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
        let deck = Deck::from_shader(&ctx, src_shader, SW, SH).expect("deck");
        let ch = mixer.channel_mut(0).unwrap();
        ch.add_deck(deck);
        let mut fx = varda::deck::Effect::new(&ctx, fx_shader.clone()).expect("deck effect");
        fx.params.set_bool("palette_mode", manual);
        fx.params.set_float("palette_stability", stability);
        ch.decks[0].deck.add_effect(fx);
        // No adaptive skipping: a skipped frame repeats the picture and shows
        // as a near-zero or doubled delta, grading the scheduler instead.
        ch.decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(0);

        for frame in 0..WARMUP {
            render_at(&ctx, &mut mixer, frame);
        }
        let mut prev = luminance(&ctx, &mixer);
        let mut deltas = Vec::with_capacity(MEASURE);
        for frame in WARMUP..WARMUP + MEASURE {
            render_at(&ctx, &mut mixer, frame);
            let cur = luminance(&ctx, &mixer);
            deltas.push(mean_delta(&prev, &cur));
            prev = cur;
        }
        deltas.sort_by(|a, b| a.partial_cmp(b).expect("no NaN frames"));
        let median = deltas[deltas.len() / 2];

        assert!(
            median > 0.0,
            "{src_name} is animated, so frames must differ; got a static image"
        );
        // Return absolute figures too: a low median beside a high p95 means
        // repeated frames, a high median means a picture that never settles,
        // and spread across stability settings points at the palette.
        Spike {
            ratio: deltas[deltas.len() * 95 / 100] / median,
            median,
            p95: deltas[deltas.len() * 95 / 100],
            lo: deltas[0],
            hi: deltas[deltas.len() - 1],
        }
    };

    for src in ["dull_skull.fs", "liquid_light.fs", "taste_of_noise.fs"] {
        // Across the whole Palette Stability range, not only its default.
        let per: Vec<Spike> = STABILITIES
            .into_iter()
            .map(|stability| spike_ratio(src, false, stability))
            .collect();
        let manual_spike = spike_ratio(src, true, 0.5);
        let manual = manual_spike.ratio;
        let mut detail = String::new();
        for (spike, stab) in per.iter().zip(STABILITIES) {
            use std::fmt::Write as _;
            let _ = write!(detail, "\n    stability {stab:.1}: {spike}");
        }
        let report = format!("\n  auto:{detail}\n    manual (fixed palette): {manual_spike}");

        // 1. Steadier as the control goes up.
        //
        // Stability 0.0 asks the palette to react in about five frames, so a
        // real anchor change shows at low settings. Comparing the lowest
        // setting against a fixed palette would be wrong.
        for (i, pair) in per.windows(2).enumerate() {
            assert!(
                pair[1].ratio <= pair[0].ratio * MONOTONIC_TOLERANCE,
                "{src}: Palette Stability is inverted. Raising it from {:.1} to \
                 {:.1} made the picture *less* steady, {:.2}x against {:.2}x. \
                 That is the opposite of what the control promises.{report}",
                STABILITIES[i],
                STABILITIES[i + 1],
                pair[0].ratio,
                pair[1].ratio
            );
        }

        // 2. At the top setting, an auto palette is as steady as a fixed one.
        //    An anchor that hops regardless of stability fails here.
        let steadiest = per.last().expect("STABILITIES is not empty");
        assert!(
            steadiest.ratio < manual * MAX_SPIKE_RATIO,
            "{src}: auto palette lurched even at full stability. The p95 frame \
             changed {:.2}x the median, against {manual:.2}x with the palette held \
             fixed, so an anchor jumped and regraded the whole frame at once.{report}",
            steadiest.ratio
        );
    }
}

/// The auto palette is state carried between frames, not re-derived each
/// frame.
///
/// The lurch test above passes on Metal even with easing and nearest-anchor
/// pairing removed, because its content never produces near-tied candidates.
/// This grades the mechanism directly, which needs the palette to be the only
/// thing moving:
///
/// * A static source: `gradient.fs` with `anim_speed` at its default of zero,
///   which still has three colors.
/// * Chroma Flow's warp, zoom, drift and trail zeroed, since flow otherwise
///   dominates whole-frame luminance.
///
/// Even then luminance barely sees the palette, so the final pass is replaced
/// with one that shows `paletteBuf`; the palette pass runs as shipped.
///
/// The source changes once the palette has settled. Carried and eased, the
/// palette still moves ten frames later (0.695 of its first frame's
/// movement); snapped, it lands in one frame and stops (0.0).
#[test]
fn chroma_flow_auto_palette_is_carried_between_frames() {
    const W: u32 = 64;
    const H: u32 = 64;
    /// Frames for the flow buffer and palette to settle before the change.
    const SETTLE: usize = 60;
    /// `tau` is 0.5s at full stability and a frame is 1/60s, so an easing
    /// palette is far from settled by here.
    const LATE: usize = 10;
    /// Movement at LATE as a fraction of the first frame's after the change:
    /// 0.695 eased, 0.0 snapped. The threshold is midway.
    const MIN_RESIDUAL: f32 = 0.35;
    /// How much a frame delta may exceed the previous one. Easing decays, so
    /// the true value is 1.0; the worst on Metal is 1.11. The allowance covers
    /// readback noise.
    const MAX_RISE: f32 = 1.35;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let fx_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/chroma_flow.fs");
    let src_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/gradient.fs");
    let src_shader = varda::isf::ISFShader::from_file(&src_path).expect("parse gradient.fs");
    let shipped = std::fs::read_to_string(&fx_path).expect("read chroma_flow.fs");
    let display = "    // Composite pass. Display only: nothing below is ever read back.";
    assert!(
        shipped.contains(display),
        "chroma_flow.fs's composite pass moved"
    );
    let showing_palette = shipped.replacen(
        display,
        "    fragColor = vec4(texture(sampler2D(paletteBuf, texSampler), vec2(uv.x, 0.5)).rgb, 1.0);\n    return;",
        1,
    );
    let fx_shader =
        varda::isf::ISFShader::from_string(&showing_palette).expect("parse chroma_flow.fs");

    let mut mixer = Mixer::new(&ctx, W, H).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let deck = Deck::from_shader(&ctx, src_shader, W, H).expect("deck");
    {
        let ch = mixer.channel_mut(0).expect("channel 0");
        ch.add_deck(deck);
        let mut fx = varda::deck::Effect::new(&ctx, fx_shader).expect("deck effect");
        fx.params.set_bool("palette_mode", false);
        fx.params.set_float("palette_stability", 1.0);
        // Everything that moves a pixel other than the palette.
        for still in [
            "zoom",
            "rotate",
            "drift_x",
            "drift_y",
            "warp_amount",
            "flow_speed",
            "group_shear",
            "trail",
        ] {
            fx.params.set_float(still, 0.0);
        }
        ch.decks[0].deck.add_effect(fx);
        ch.decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(0);
    }

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_rgba16f(ctx, mixer.composite_texture(), W, H)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    for frame in 1..=SETTLE {
        render_at(&ctx, &mut mixer, frame);
    }
    mixer.channel_mut(0).expect("channel 0").decks[0]
        .deck
        .generator_params
        .set_color("color_c", [1.0, 0.35, 0.0, 1.0]);

    // Every consecutive frame, since the shape of the sequence matters.
    let mut prev = luminance(&ctx, &mixer);
    let mut deltas = Vec::with_capacity(LATE);
    for frame in (SETTLE + 1)..=(SETTLE + LATE) {
        render_at(&ctx, &mut mixer, frame);
        let cur = luminance(&ctx, &mixer);
        deltas.push(mean_delta(&prev, &cur));
        prev = cur;
    }

    let early = deltas[0];
    let late = deltas[LATE - 1];
    assert!(
        early > 0.0,
        "the palette did not follow the source at all, so this proves nothing\n  sequence: {deltas:.8?}"
    );

    // 1. Still moving late, because state is carried.
    let residual = late / early;
    assert!(
        residual >= MIN_RESIDUAL,
        "the auto palette is not carried between frames: after the source changed, \
         its movement fell from {early:.8} to {late:.8} ({residual:.4} of the start) \
         by frame {LATE}. An eased palette is still seeking here; one re-derived \
         each frame is already final. Check that paletteBuf survives the frame.\
         \n  sequence: {deltas:.8?}"
    );

    // 2. Moving every frame, decaying smoothly.
    //
    // A palette that updates only on some frames shows near-still frames and
    // then one catch-up frame carrying several frames of easing. Easing is a
    // decaying exponential, so each delta should be no larger than the last.
    for (i, pair) in deltas.windows(2).enumerate() {
        assert!(
            pair[1] <= pair[0] * MAX_RISE,
            "the auto palette stutters: frame delta rose from {:.8} to {:.8} \
             ({:.2}x) at step {}. An eased palette decays monotonically, so a \
             rise means the palette sat out a frame and caught up on the next. \
             Check that the palette pass runs, and lands, every frame.\
             \n  sequence: {deltas:.8?}",
            pair[0],
            pair[1],
            pair[1] / pair[0],
            i + 1
        );
    }
}

/// Settle, then jitter the fixture's two rivals back and forth across the point
/// where they swap, returning the median per-frame change.
fn tie_bait_jitter(ctx: &GpuContext, mixer: &mut Mixer, size: (u32, u32)) -> f32 {
    const SETTLE: usize = 30;
    const SWEEP: usize = 40;
    let (w, h) = size;

    let luminance = |mixer: &Mixer| -> Vec<f32> {
        read_rgba16f(ctx, mixer.composite_texture(), w, h)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };
    let set_tie = |mixer: &mut Mixer, v: f32| {
        mixer.channel_mut(0).expect("channel 0").decks[0]
            .deck
            .generator_params
            .set_float("tie", v);
    };

    set_tie(mixer, 0.5);
    for frame in 0..SETTLE {
        render_at(ctx, mixer, frame);
    }
    let mut prev = luminance(mixer);

    let mut deltas = Vec::with_capacity(SWEEP);
    for step in 0..SWEEP {
        #[allow(clippy::cast_precision_loss)]
        let tie = 0.5 + 0.5 * ((step as f32) * std::f32::consts::TAU / 8.0).sin();
        set_tie(mixer, tie);
        render_at(ctx, mixer, SETTLE + step);
        let cur = luminance(mixer);
        deltas.push(mean_delta(&prev, &cur));
        prev = cur;
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).expect("no NaN frames"));
    deltas[deltas.len() / 2]
}

/// An anchor hands over rather than flapping.
///
/// The palette is chosen by greedy farthest-point selection over a grid of
/// samples. A memoryless selection flaps when two candidates sit near the cut:
/// each wobble swaps which is in the palette and the frame regrades.
/// `palettePass` pairing can't absorb it, since the set itself changes.
///
/// Real content hits the cut only by chance (`dull_skull` does on DX12, not on
/// Metal). `tests/shaders/palette_tie_bait.fs` forces it: flat blocks on the
/// extraction grid, with two sliding through the swap point.
///
/// * The swing exceeds the 2% `SELECTION_MARGIN`, which would otherwise
///   suppress the swap.
/// * The content jitters around the cut; a single sweep swaps once and easing
///   hides it.
/// * Flapping lifts the whole distribution rather than adding a tail, so
///   p95-over-median can't see it. This grades how much more the picture
///   moves than its content.
#[test]
fn chroma_flow_palette_hands_over_rather_than_flapping() {
    const W: u32 = 64;
    const H: u32 = 64;
    /// How much more the graded picture may move than the content it grades.
    ///
    /// Measured at the fastest setting: 4.3x with hysteresis, 350x without.
    const MAX_CHASE: f32 = 20.0;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/shaders/palette_tie_bait.fs");
    let fx_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/chroma_flow.fs");

    let build = |graded: bool| -> Mixer {
        let mut mixer = Mixer::new(&ctx, W, H).expect("mixer");
        mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
        let deck = Deck::from_shader(
            &ctx,
            varda::isf::ISFShader::from_file(&src).expect("parse tie-bait fixture"),
            W,
            H,
        )
        .expect("deck");
        let ch = mixer.channel_mut(0).expect("channel 0");
        ch.add_deck(deck);
        if !graded {
            return mixer;
        }
        let fx_shader = varda::isf::ISFShader::from_file(&fx_path).expect("parse chroma_flow.fs");
        let mut fx = varda::deck::Effect::new(&ctx, fx_shader).expect("deck effect");
        fx.params.set_bool("palette_mode", false);
        // Two groups: the center sample, and one slot the two rivals contest.
        // With more slots both are included and nothing can jump.
        fx.params.set_float("palette_size", 2.0);
        // The fastest setting, where easing hides a jump least.
        fx.params.set_float("palette_stability", 0.0);
        for still in [
            "zoom",
            "rotate",
            "drift_x",
            "drift_y",
            "warp_amount",
            "flow_speed",
            "group_shear",
        ] {
            fx.params.set_float(still, 0.0);
        }
        // Show the palette plainly: at defaults `barrier_level` keeps the
        // darker rival from being graded and `color_preservation` blends the
        // source back over the anchor, hiding the effect.
        fx.params.set_float("barrier_level", 0.0);
        fx.params.set_float("color_preservation", 0.0);
        fx.params.set_float("edge_blend_width", 0.0);
        ch.decks[0].deck.add_effect(fx);
        mixer
    };

    let mut plain = build(false);
    let content = tie_bait_jitter(&ctx, &mut plain, (W, H));
    let mut graded = build(true);
    let auto = tie_bait_jitter(&ctx, &mut graded, (W, H));

    assert!(
        content > 0.0,
        "the content never moved at all, so this proves nothing"
    );
    let ratio = auto / content;
    assert!(
        ratio <= MAX_CHASE,
        "the auto palette flaps: while two candidates jitter across the point \
         where they swap, the graded picture moves {auto:.6} per frame while the \
         content itself moves only {content:.6} ({ratio:.1}x). The selection is \
         changing which colour is in the palette every time the ranking crosses, \
         instead of keeping the one already on screen."
    );
}

/// Render `source`, optionally through Chroma Flow, capturing the luminance
/// field after each of `capture_at` frame counts. `configure` receives the
/// effect before it is attached.
fn chroma_flow_frames(
    ctx: &GpuContext,
    source: &str,
    size: (u32, u32),
    capture_at: &[usize],
    configure: Option<&dyn Fn(&mut varda::params::ShaderParams)>,
) -> Vec<Vec<f32>> {
    let (w, h) = size;
    let fx_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/chroma_flow.fs");
    let src_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join(source);
    let src_shader = varda::isf::ISFShader::from_file(&src_path).expect("parse source shader");

    let mut mixer = Mixer::new(ctx, w, h).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let deck = Deck::from_shader(ctx, src_shader, w, h).expect("deck");
    {
        let ch = mixer.channel_mut(0).expect("channel 0");
        ch.add_deck(deck);
        if let Some(configure) = configure {
            let fx_shader = varda::isf::ISFShader::from_file(&fx_path).expect("parse chroma_flow");
            let mut fx = varda::deck::Effect::new(ctx, fx_shader).expect("deck effect");
            configure(&mut fx.params);
            ch.decks[0].deck.add_effect(fx);
        }
        // Adaptive skipping depends on wall-clock cost, so frame counts would
        // vary between runs that must line up.
        ch.decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(0);
    }

    let last = capture_at.iter().copied().max().unwrap_or(0);
    let mut out = Vec::with_capacity(capture_at.len());
    for frame in 1..=last {
        render_at(ctx, &mut mixer, frame);
        if capture_at.contains(&frame) {
            out.push(
                read_rgba16f(ctx, mixer.composite_texture(), w, h)
                    .iter()
                    .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
                    .collect(),
            );
        }
    }
    out
}

/// The luminance field after `frames` frames.
fn chroma_flow_run(
    ctx: &GpuContext,
    source: &str,
    size: (u32, u32),
    frames: usize,
    configure: Option<&dyn Fn(&mut varda::params::ShaderParams)>,
) -> Vec<f32> {
    chroma_flow_frames(ctx, source, size, &[frames], configure)
        .pop()
        .expect("one capture")
}

/// Mean absolute difference between horizontally adjacent pixels.
fn edge_energy(img: &[f32], width: usize) -> f32 {
    let mut sum = 0.0;
    let mut n = 0usize;
    for row in img.chunks_exact(width) {
        for x in 1..width {
            sum += (row[x] - row[x - 1]).abs();
            n += 1;
        }
    }
    sum / n as f32
}

/// Color never crosses into darkness.
///
/// Dark ground is a barrier: without it every region expands into its
/// neighbors and the frame fills in like smoke. Barrier ground is also kept
/// apart from vacated ground: vacated ground is repainted from the refill
/// cycle, barrier ground keeps its seeded brightness and is never repainted.
///
/// "Dark" is taken from the same source with no effect, using the brightest
/// each pixel gets across the run: a pixel lit earlier may have seeded a layer
/// and carry its color. Only pixels dark for the whole run are checked.
#[test]
fn chroma_flow_never_flows_into_darkness() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 90;
    /// Readback is f16 and barrier pixels pass their seeded brightness
    /// through, so anything above rounding is color where it isn't allowed.
    const TOLERANCE: f32 = 4e-3;
    /// Needs real shadow: the skull and liquid shaders never fall below the
    /// barrier.
    const SOURCE: &str = "taste_of_noise.fs";
    /// Well above the default. The shader classifies against the deck output
    /// while this test reads the composite, and the two differ near the
    /// threshold. A high barrier makes the dark subset unambiguous in both.
    const BARRIER: f32 = 0.6;
    /// Well below `BARRIER` in either space.
    const CALLED_DARK: f32 = 0.15;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let every_frame: Vec<usize> = (1..=FRAMES).collect();
    let plain = chroma_flow_frames(&ctx, SOURCE, (SW, SH), &every_frame, None);
    let mut brightest = vec![0.0f32; (SW * SH) as usize];
    for frame in &plain {
        for (peak, v) in brightest.iter_mut().zip(frame) {
            *peak = peak.max(*v);
        }
    }

    let flowed = chroma_flow_run(
        &ctx,
        SOURCE,
        (SW, SH),
        FRAMES,
        Some(&|p: &mut varda::params::ShaderParams| {
            // Fast warp and a large palette, to give color every chance to
            // spill.
            p.set_float("zoom", 1.6);
            p.set_float("rotate", 90.0);
            p.set_float("warp_amount", 1.5);
            p.set_float("palette_size", 6.0);
            p.set_float("barrier_level", BARRIER);
            // Edge softening would blur a lit pixel slightly into the dark.
            // Off, so this grades transport only.
            p.set_float("edge_blend_width", 0.0);
        }),
    );

    let dark: Vec<f32> = brightest
        .iter()
        .zip(&flowed)
        .filter(|(peak, _)| **peak < CALLED_DARK)
        .map(|(peak, f)| f - peak)
        .collect();

    assert!(
        dark.len() > 500,
        "the source has almost no permanently dark area ({} of {} pixels), so \
         this proves nothing — pick a source with real shadow in it",
        dark.len(),
        brightest.len()
    );

    let worst = dark.iter().copied().fold(f32::MIN, f32::max);
    let leaked = dark.iter().filter(|d| **d > TOLERANCE).count();
    assert!(
        leaked == 0,
        "colour flowed into darkness: {leaked} of {} barrier pixels brightened, \
         the worst by {worst:.3}. Dark ground is floor — a layer must not be able \
         to enter it, nor to crawl out of it, nor to have it repainted.",
        dark.len()
    );
}

/// Low hardness opens the barrier.
///
/// The barrier test above would also pass with a barrier that never opens.
/// Wound down, color must run into the dark.
///
/// Compared against the same effect with hardness closed, not the source: the
/// source reference is each pixel's brightest across the run while the reading
/// is the final frame, which differ widely on animated content regardless of
/// the barrier.
#[test]
fn chroma_flow_barrier_hardness_opens_the_boundary() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 90;
    const SOURCE: &str = "taste_of_noise.fs";
    const BARRIER: f32 = 0.6;
    const CALLED_DARK: f32 = 0.15;
    /// Fully softened, the flow should reach well into the dark.
    const MIN_SPILL: f32 = 0.02;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let every_frame: Vec<usize> = (1..=FRAMES).collect();
    let plain = chroma_flow_frames(&ctx, SOURCE, (SW, SH), &every_frame, None);
    let mut brightest = vec![0.0f32; (SW * SH) as usize];
    for frame in &plain {
        for (peak, v) in brightest.iter_mut().zip(frame) {
            *peak = peak.max(*v);
        }
    }

    let run = |hardness: f32| -> Vec<f32> {
        chroma_flow_run(
            &ctx,
            SOURCE,
            (SW, SH),
            FRAMES,
            Some(&move |p: &mut varda::params::ShaderParams| {
                p.set_float("zoom", 1.6);
                p.set_float("rotate", 90.0);
                p.set_float("warp_amount", 1.5);
                p.set_float("palette_size", 6.0);
                p.set_float("barrier_level", BARRIER);
                p.set_float("barrier_hardness", hardness);
            }),
        )
    };

    let closed = run(1.0);
    let open = run(0.0);

    let spill: Vec<f32> = brightest
        .iter()
        .zip(closed.iter().zip(&open))
        .filter(|(peak, _)| **peak < CALLED_DARK)
        .map(|(_, (shut, wide))| (wide - shut).abs())
        .collect();

    assert!(
        spill.len() > 500,
        "the source has almost no permanently dark area ({} of {} pixels), so \
         this proves nothing",
        spill.len(),
        brightest.len()
    );

    let mean = spill.iter().sum::<f32>() / spill.len() as f32;
    assert!(
        mean > MIN_SPILL,
        "the barrier will not open: with hardness at zero the dark ground still \
         only moved by {mean:.4}. Nothing can flow past the boundary, so the \
         control is inert and there is no drop to play."
    );
}

/// The picture must not dissolve into itself.
///
/// Feeding a picture back through a warp with any blend is diffusion: at 60
/// fps every boundary washes out in seconds. The field is stored as flat
/// stack heights and moved by a hard hand-off to avoid this.
///
/// Edge energy measures it: diffusion drives it toward zero, while a region
/// moving as a body keeps its boundary. Compared against the source so a quiet
/// passage doesn't read as failure.
#[test]
fn chroma_flow_regions_keep_their_edges() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    /// A 5% per-frame blend leaves under a thousandth of any boundary after
    /// this many frames.
    const FRAMES: usize = 180;
    /// Grouping flattens region interiors, so some loss against the source is
    /// expected. Total collapse is not.
    const MIN_EDGE_FRACTION: f32 = 0.35;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let plain = chroma_flow_run(&ctx, "dull_skull.fs", (SW, SH), FRAMES, None);
    let flowed = chroma_flow_run(&ctx, "dull_skull.fs", (SW, SH), FRAMES, Some(&|_| {}));

    let source_edges = edge_energy(&plain, SW as usize);
    let flowed_edges = edge_energy(&flowed, SW as usize);
    assert!(
        source_edges > 0.0,
        "the source is a flat field, so there are no edges to preserve"
    );

    let fraction = flowed_edges / source_edges;
    assert!(
        fraction > MIN_EDGE_FRACTION,
        "the picture dissolved: after {FRAMES} frames its edge energy is \
         {flowed_edges:.4} against the source's {source_edges:.4}, a fraction of \
         {fraction:.2}. The field is being blended somewhere instead of handed \
         over whole, and the boundaries are washing out."
    );
}

/// The picture actually travels.
///
/// If vacated space is refilled from the live frame, carried material is
/// replaced by the picture already there, leaving only flicker at the band
/// where they disagree.
///
/// A still camera is the control: same warp, regeneration, and grading with
/// motion removed. A build that only re-grades its source in place scores near
/// zero.
#[test]
fn chroma_flow_actually_travels() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 120;
    /// Above the boundary-band churn of a static build and well below real
    /// travel.
    const MIN_TRAVEL: f32 = 0.02;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let run = |moving: bool| -> Vec<f32> {
        chroma_flow_run(
            &ctx,
            "dull_skull.fs",
            (SW, SH),
            FRAMES,
            Some(&move |p: &mut varda::params::ShaderParams| {
                p.set_float("zoom", if moving { 1.3 } else { 1.0 });
                p.set_float("rotate", if moving { 30.0 } else { 0.0 });
                p.set_float("warp_amount", if moving { 0.8 } else { 0.0 });
            }),
        )
    };

    let still = run(false);
    let moving = run(true);
    let travel = still
        .iter()
        .zip(&moving)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / still.len() as f32;

    assert!(
        travel > MIN_TRAVEL,
        "the picture is not going anywhere: after {FRAMES} frames, a zooming, \
         rotating, warping camera differs from a still one by only {travel:.4} \
         mean luminance. The effect is re-grading its source in place instead of \
         carrying it."
    );
}

/// The mask holds its circle still and lets the rest move.
///
/// The hold applies in the warp buffer and on output; only the output fade
/// back to the source is checked here. The buffer hold keeps the warp from
/// running under the held area, which would surface all at once when the mask
/// moves.
///
/// Compared against the bare deck, so a break shared by the effect's own
/// configurations can't cancel out.
#[test]
fn chroma_flow_mask_holds_its_circle_still() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 120;
    const RADIUS: f32 = 0.3;
    /// Held ground is a fade back to the source, so it should match to rounding.
    const MAX_HELD_DRIFT: f32 = 6e-3;
    /// Outside has a warp and a hard grade, so it differs by far more. Low
    /// enough to clear quiet passages in the source.
    const MIN_FLOW: f32 = 0.02;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let plain = chroma_flow_run(&ctx, "dull_skull.fs", (SW, SH), FRAMES, None);
    let masked = chroma_flow_run(
        &ctx,
        "dull_skull.fs",
        (SW, SH),
        FRAMES,
        Some(&|p: &mut varda::params::ShaderParams| {
            p.set_float("mask_radius", RADIUS);
            p.set_float("mask_softness", 0.0);
            p.set_float("zoom", 1.3);
            p.set_float("rotate", 40.0);
            p.set_float("warp_amount", 1.0);
            p.set_float("edge_blend_width", 0.0);
        }),
    );

    let mut held = Vec::new();
    let mut flowing = Vec::new();
    for (i, (p, m)) in plain.iter().zip(&masked).enumerate() {
        let x = (i % SW as usize) as f32 / SW as f32 - 0.5;
        let y = (i / SW as usize) as f32 / SH as f32 - 0.5;
        let r = (x * x + y * y).sqrt();
        // Skip an annulus around the boundary, which falls between texels.
        if r < RADIUS * 0.8 {
            held.push((p - m).abs());
        } else if r > RADIUS * 1.25 {
            flowing.push((p - m).abs());
        }
    }

    assert!(
        held.len() > 500 && flowing.len() > 500,
        "the sample is too small to mean anything: {} held, {} flowing",
        held.len(),
        flowing.len()
    );

    let worst_held = held.iter().copied().fold(0.0f32, f32::max);
    assert!(
        worst_held < MAX_HELD_DRIFT,
        "the mask is not holding: inside a radius of {RADIUS} the picture moved \
         by up to {worst_held:.4} against the untouched source. Held ground is \
         supposed to be the source, unwarped and ungraded."
    );

    let mean_flow = flowing.iter().sum::<f32>() / flowing.len() as f32;
    assert!(
        mean_flow > MIN_FLOW,
        "the mask is holding everything: outside a radius of {RADIUS} the picture \
         differs from the untouched source by only {mean_flow:.4}, so the effect \
         is suppressed where it should be running at full strength."
    );
}

/// The source keeps bleeding back into the field.
///
/// A warp only moves what is in the buffer, so without the source mixed back
/// in, the field drifts into its own smeared history. Regen sets that rate,
/// like Deforum's strength schedule. If regen is inert, runs at different
/// values are bit-identical.
#[test]
fn chroma_flow_regenerates_from_the_source() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 300;
    /// The failure is exact equality, so the bar only needs to clear readback
    /// rounding; different rates diverge across the whole frame.
    const MIN_DIFFERENCE: f32 = 5e-3;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let run = |regen: f32| -> Vec<f32> {
        chroma_flow_run(
            &ctx,
            "dull_skull.fs",
            (SW, SH),
            FRAMES,
            Some(&move |p: &mut varda::params::ShaderParams| {
                p.set_float("regen_seconds", regen);
                p.set_float("zoom", 1.2);
            }),
        )
    };

    let brief = run(0.05);
    let long = run(4.0);
    let difference = brief
        .iter()
        .zip(&long)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / brief.len() as f32;

    assert!(
        difference > MIN_DIFFERENCE,
        "Regen is inert: after {FRAMES} frames, letting the source back in every \
         0.05s differs from every 4s by only {difference:.5} mean luminance. \
         Nothing is being drawn back from the source, so the field is warping \
         alone and will smear itself away."
    );
}

// ── Program / channel tap ────────────────────────────────────────────
//
// A tap shows the previous frame, whatever the tapping deck's position in the
// channel order. That is what lets feedback loops terminate, so a tap must
// never read the live target.

/// Advance the mixer as the app does: resolve taps, then render.
fn render_frame_with_taps(ctx: &GpuContext, mixer: &mut Mixer) {
    mixer.prepare_taps(ctx);
    render_once(ctx, mixer);
}

/// A tap in a later channel than the one it reads. Channel 0 has already
/// composited when channel 1 renders, so binding it directly would show this
/// frame's red on frame one.
#[test]
fn tap_shows_the_previous_frame_not_the_current_one() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch0_uuid = mixer.channel(0).unwrap().uuid().to_string();

    let red = Deck::solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(red);
    let tap = tap_deck(&ctx, &varda::tap::TapPoint::Channel { uuid: ch0_uuid });
    mixer.channel_mut(1).unwrap().add_deck(tap);
    mixer.set_crossfader(1.0);

    render_frame_with_taps(&ctx, &mut mixer);
    let first = center(&read_rgba16f(&ctx, mixer.composite_texture(), W, H));
    assert_lo(
        first[0],
        "frame 1 tap must be black — channel 0 composited earlier in this same \
         frame, and showing its red would mean the tap read the live target",
    );

    render_frame_with_taps(&ctx, &mut mixer);
    let second = center(&read_rgba16f(&ctx, mixer.composite_texture(), W, H));
    assert_hi(second[0], "frame 2 tap must show frame 1's red");
}

/// The same check with the tap in an earlier channel than its source. Both
/// directions agreeing shows there is no ordering dependence.
#[test]
fn tap_latency_does_not_depend_on_channel_order() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch1_uuid = mixer.channel(1).unwrap().uuid().to_string();

    let tap = tap_deck(&ctx, &varda::tap::TapPoint::Channel { uuid: ch1_uuid });
    mixer.channel_mut(0).unwrap().add_deck(tap);
    let red = Deck::solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(1).unwrap().add_deck(red);
    mixer.set_crossfader(0.0);

    render_frame_with_taps(&ctx, &mut mixer);
    assert_lo(
        center(&read_rgba16f(&ctx, mixer.composite_texture(), W, H))[0],
        "frame 1 tap must be black in the earlier-channel direction too",
    );

    render_frame_with_taps(&ctx, &mut mixer);
    assert_hi(
        center(&read_rgba16f(&ctx, mixer.composite_texture(), W, H))[0],
        "frame 2 tap must show frame 1's red, exactly as in the other direction",
    );
}

/// A master tap is one frame behind too: every deck renders before the master
/// composite.
#[test]
fn master_tap_shows_the_previous_frame() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);

    // The tap deck sits under the red deck in the same channel: opaque Normal
    // covers it, so the program is always red while the tap deck still renders.
    // In the far channel it would be faded out and culled, never rendering.
    let tap = tap_deck(&ctx, &varda::tap::TapPoint::MasterProgram);
    mixer.channel_mut(0).unwrap().add_deck(tap);
    let red = Deck::solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(red);
    mixer.set_crossfader(0.0);

    let tap_texture = |m: &Mixer| -> Vec<[f32; 4]> {
        read_rgba16f(&ctx, &m.channel(0).unwrap().decks[0].deck.texture, W, H)
    };

    render_frame_with_taps(&ctx, &mut mixer);
    assert_lo(
        center(&tap_texture(&mixer))[0],
        "frame 1 master tap must be black — the program has not been composited yet",
    );

    render_frame_with_taps(&ctx, &mut mixer);
    assert_hi(
        center(&tap_texture(&mixer))[0],
        "frame 2 master tap must show frame 1's program",
    );
}

/// A deck tapping its own channel is valid feedback. At partial opacity it
/// converges, with no NaN or infinity in the `Rgba16Float` target.
#[test]
fn self_tapping_deck_converges_rather_than_diverging() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch0_uuid = mixer.channel(0).unwrap().uuid().to_string();

    let red = Deck::solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(red);
    let tap = tap_deck(&ctx, &varda::tap::TapPoint::Channel { uuid: ch0_uuid });
    let tap_idx = mixer.channel_mut(0).unwrap().add_deck(tap);
    mixer.channel_mut(0).unwrap().decks[tap_idx].opacity = 0.5;
    mixer.set_crossfader(0.0);

    for frame in 0..12 {
        render_frame_with_taps(&ctx, &mut mixer);
        let px = center(&read_rgba16f(&ctx, mixer.composite_texture(), W, H));
        for (i, v) in px.iter().enumerate() {
            assert!(
                v.is_finite(),
                "frame {frame} channel {i} went non-finite ({v}) — a self-tap at \
                 50% opacity is a difference equation and must settle"
            );
        }
    }
}

/// No tap deck means no tap target is allocated.
#[test]
fn a_scene_without_taps_allocates_no_tap_targets() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let red = Deck::solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H);
    mixer.channel_mut(0).unwrap().add_deck(red);

    render_frame_with_taps(&ctx, &mut mixer);
    assert!(
        mixer.has_no_tap_targets(),
        "a scene with no tap decks must not allocate any tap render target"
    );

    // ...and adding one then removing it frees the memory.
    let ch0_uuid = mixer.channel(0).unwrap().uuid().to_string();
    let tap = tap_deck(&ctx, &varda::tap::TapPoint::Channel { uuid: ch0_uuid });
    let idx = mixer.channel_mut(1).unwrap().add_deck(tap);
    render_frame_with_taps(&ctx, &mut mixer);
    assert!(
        !mixer.has_no_tap_targets(),
        "a tapped channel must have a tap target"
    );

    mixer.channel_mut(1).unwrap().remove_deck_slot(idx);
    render_frame_with_taps(&ctx, &mut mixer);
    assert!(
        mixer.has_no_tap_targets(),
        "removing the last tap deck must release the tap target"
    );
}

// ── Multi-pass ordering ──────────────────────────────────────────────

/// Pass 0 writes red only when it sees its own `PASSINDEX`; the final pass
/// shows what pass 0 wrote and adds green.
const PASS_ORDER_SHADER: &str = r#"/*{
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [{"NAME": "unused", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}],
    "PASSES": [{"TARGET": "first"}, {}]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D first;

layout(set = 0, binding = 3) uniform UserParams {
    float unused;
};

void main() {
    if (PASSINDEX == 0) {
        fragColor = vec4(1.0, 0.0, 0.0, 1.0);
    } else {
        fragColor = vec4(texture(sampler2D(first, texSampler), uv).rgb + vec3(0.0, 1.0, 0.0), 1.0);
    }
}
"#;

/// All passes of a multi-pass shader encode into one command buffer. Each pass
/// reads its own uniforms, and a later pass sees what an earlier one wrote in
/// the same frame.
#[test]
fn a_later_pass_reads_what_an_earlier_pass_wrote_this_frame() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let shader = varda::isf::ISFShader::from_string(PASS_ORDER_SHADER).expect("parse");
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_hi(
        px[0],
        "pass 0 ran with its own PASSINDEX, before the final pass",
    );
    assert_hi(px[1], "the final pass ran");
    assert_lo(px[2], "blue");
}

// ── Pass extensions: HISTORY, FORMAT, TARGETS ────────────────────────

/// The uniform block with the fields appended for pass extensions.
const ISF_UNIFORMS_WITH_HISTORY: &str = r"
layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
    vec2 JITTER;
    int JITTERINDEX;
    int HISTORYVALID;
};
";

fn pass_extension_shader(passes: &str, bindings: &str, body: &str) -> varda::isf::ISFShader {
    let source = format!(
        r#"/*{{
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "PASSES": {passes}
}}*/

#version 450

layout(location = 0) in vec2 uv;
{ISF_UNIFORMS_WITH_HISTORY}
layout(set = 0, binding = 1) uniform sampler texSampler;
{bindings}

{body}
"#
    );
    varda::isf::ISFShader::from_string(&source).expect("parse")
}

/// A `HISTORY` pass counts frames in a 32-bit float target: it adds one to
/// what it wrote last frame, and starts from one when `HISTORYVALID` is 0.
fn frame_counter_shader() -> varda::isf::ISFShader {
    pass_extension_shader(
        r#"[{"TARGET": "count", "HISTORY": true, "FORMAT": "rgba32float"}, {}]"#,
        "layout(set = 0, binding = 2) uniform texture2D count;",
        r"
layout(location = 0) out vec4 fragColor;
void main() {
    float previous = texelFetch(sampler2D(count, texSampler), ivec2(gl_FragCoord.xy), 0).r;
    if (PASSINDEX == 0) {
        fragColor = vec4(HISTORYVALID == 1 ? previous + 1.0 : 1.0, 0.0, 0.0, 1.0);
    } else {
        fragColor = vec4(previous * 0.1, 0.0, 0.0, 1.0);
    }
}",
    )
}

#[test]
fn a_history_pass_reads_its_own_previous_frame_and_resets_on_resize() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, frame_counter_shader(), W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);

    let mut counts = Vec::new();
    for _ in 0..3 {
        counts.push(center(&render_and_read(&ctx, &mut mixer))[0]);
    }
    assert_near(
        counts[0],
        0.1,
        0.002,
        "first frame: history invalid, count 1",
    );
    assert_near(counts[1], 0.2, 0.002, "second frame reads the first");
    assert_near(counts[2], 0.3, 0.002, "third frame reads the second");

    mixer.channel_mut(0).unwrap().decks[0]
        .deck
        .resize(&ctx, W, H);
    let after_resize = center(&render_and_read(&ctx, &mut mixer))[0];
    assert_near(after_resize, 0.1, 0.002, "a resize starts history over");
}

#[test]
fn one_pass_writes_several_targets_in_their_own_formats() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    // `fine` holds 1 + 2^-16, which an f16 target would round to 1.0.
    let shader = pass_extension_shader(
        r#"[{"TARGETS": ["fine", "coarse"], "FORMATS": ["rgba32float", "rgba16float"]}, {}]"#,
        "layout(set = 0, binding = 2) uniform texture2D fine;\n\
         layout(set = 0, binding = 3) uniform texture2D coarse;",
        r"
layout(location = 0) out vec4 out0;
layout(location = 1) out vec4 out1;
void main() {
    if (PASSINDEX == 0) {
        out0 = vec4(1.0 + 1.0 / 65536.0, 0.0, 0.0, 1.0);
        out1 = vec4(0.0, 0.0, 0.5, 1.0);
    } else {
        ivec2 texel = ivec2(gl_FragCoord.xy);
        float excess = texelFetch(sampler2D(fine, texSampler), texel, 0).r - 1.0;
        float blue = texelFetch(sampler2D(coarse, texSampler), texel, 0).b;
        out0 = vec4(excess * 65536.0 * 0.25, 0.0, blue, 1.0);
    }
}",
    );
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_near(
        px[0],
        0.25,
        0.01,
        "the rgba32float target kept a 2^-16 step f16 cannot hold",
    );
    assert_near(
        px[2],
        0.5,
        0.01,
        "the second target was written in the same pass",
    );
}

#[test]
fn a_shader_binding_more_textures_than_the_device_allows_fails_by_name() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let shader = frame_counter_shader();
    let spirv = varda::isf::compile_glsl_to_spirv(&shader.fragment_source, "limit").unwrap();
    let limit = ctx.device.limits().max_sampled_textures_per_shader_stage as usize;
    let too_many = vec![true; limit + 1];
    let err = varda::renderer::UnifiedPipeline::new(
        &ctx.device,
        &spirv,
        ctx.compositing_format,
        false,
        &too_many,
        &varda::renderer::PassPlan::default(),
        0,
        &[],
        &[],
    )
    .err()
    .expect("over the limit");
    assert!(
        format!("{err:#}").contains(&format!("this GPU allows {limit}")),
        "{err:#}"
    );
}

/// A generator whose one `SPECIALIZE` input picks its color through a
/// specialization constant, or declares `constant` in place of it.
fn specialized_shader(constant: &str) -> varda::isf::ISFShader {
    let source = format!(
        r#"/*{{
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {{"NAME": "mode", "TYPE": "long", "DEFAULT": 1, "VALUES": [1, 2, 3], "SPECIALIZE": true}}
    ]
}}*/

#version 450

layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 fragColor;
{constant}
void main() {{
    fragColor = vec4(float(MODE) * 0.25, 0.0, 0.0, 1.0);
}}
"#
    );
    varda::isf::ISFShader::from_string(&source).expect("parse")
}

#[test]
fn a_specialized_input_rebuilds_the_pipeline_with_its_value() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let shader = specialized_shader("layout(constant_id = 0) const int MODE = 1;");
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);
    // Offline frames build the new pipelines before drawing.
    let set_mode = |mixer: &mut varda::mixer::Mixer, mode: i32| {
        set_long(mixer, "mode", mode);
        render_at(&ctx, mixer, 0);
        center(&read_rgba16f(&ctx, mixer.composite_texture(), W, H))[0]
    };
    assert_near(set_mode(&mut mixer, 1), 0.25, 0.002, "the default");
    assert_near(set_mode(&mut mixer, 3), 0.75, 0.002, "mode 3 rebuilds");
    assert_near(
        set_mode(&mut mixer, 1),
        0.25,
        0.002,
        "back to the cached default",
    );
}

#[test]
fn a_live_specialized_input_keeps_drawing_until_its_pipeline_is_built() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let shader = specialized_shader("layout(constant_id = 0) const int MODE = 1;");
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);
    assert_near(
        center(&render_and_read(&ctx, &mut mixer))[0],
        0.25,
        0.002,
        "the default",
    );

    // The frame that asks for mode 3 starts the build and draws mode 1.
    set_long(&mut mixer, "mode", 3);
    assert_near(
        center(&render_and_read(&ctx, &mut mixer))[0],
        0.25,
        0.002,
        "the current pipelines while mode 3 builds",
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let red = center(&render_and_read(&ctx, &mut mixer))[0];
        if (red - 0.75).abs() < 0.002 {
            break;
        }
        assert_near(red, 0.25, 0.002, "mode 1 until mode 3 is ready");
        assert!(std::time::Instant::now() < deadline, "mode 3 never built");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // A cached combination switches on the frame that asks for it.
    set_long(&mut mixer, "mode", 1);
    assert_near(
        center(&render_and_read(&ctx, &mut mixer))[0],
        0.25,
        0.002,
        "back to the cached default",
    );
}

fn set_long(mixer: &mut Mixer, name: &str, value: i32) {
    mixer.channel_mut(0).unwrap().decks[0]
        .deck
        .generator_params
        .set_long(name, value);
}

#[test]
fn a_specialized_input_without_its_constant_fails_to_load() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let shader = specialized_shader("const int MODE = 1;");
    let err = Deck::from_shader(&ctx, shader, W, H)
        .err()
        .expect("no constant_id 0");
    assert!(format!("{err:#}").contains("constant_id"), "{err:#}");
}

#[test]
fn specialized_passes_each_see_their_own_pass_index() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    // Branches on the constant only: with a wrong constant both pipelines
    // would run the same branch.
    let source = format!(
        r#"/*{{
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "SPECIALIZE_PASSES": true,
    "PASSES": [{{"TARGET": "first"}}, {{}}]
}}*/

#version 450

layout(location = 0) in vec2 uv;
{ISF_UNIFORMS_WITH_HISTORY}
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D first;
layout(location = 0) out vec4 fragColor;
layout(constant_id = 0) const int PASS_CONSTANT = -1;

// A comparison on the constant itself would not translate.
int specialized(int value) {{
    return value;
}}

void main() {{
    if (specialized(PASS_CONSTANT) == 0) {{
        fragColor = vec4(0.25, 0.0, 0.0, 1.0);
    }} else {{
        float previous = texelFetch(sampler2D(first, texSampler), ivec2(gl_FragCoord.xy), 0).r;
        fragColor = vec4(previous + 0.5, 0.0, 0.0, 1.0);
    }}
}}
"#
    );
    let shader = varda::isf::ISFShader::from_string(&source).expect("parse");
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);
    let value = center(&render_and_read(&ctx, &mut mixer))[0];
    assert_near(
        value,
        0.75,
        0.002,
        "pass 0 wrote 0.25, the output pass added 0.5",
    );
}

#[test]
fn a_missing_imported_image_binds_a_placeholder_instead_of_shifting_the_layout() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let source = r#"/*{
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "IMPORTED": {
        "missing": {"PATH": "no_such_image_anywhere.png"}
    }
}*/

#version 450

layout(location = 0) in vec2 uv;
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D missing;
layout(location = 0) out vec4 fragColor;

void main() {
    fragColor = vec4(texture(sampler2D(missing, texSampler), vec2(0.5)).rgb, 1.0);
}
"#;
    let shader = varda::isf::ISFShader::from_string(source).expect("parse");
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);
    let px = center(&render_and_read(&ctx, &mut mixer));
    // Magenta: the placeholder, visibly a missing texture.
    assert_near(px[0], 1.0, 0.01, "red");
    assert_near(px[1], 0.0, 0.01, "green");
    assert_near(px[2], 1.0, 0.01, "blue");
}

#[test]
fn a_pass_sized_by_an_input_follows_the_input() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    // The output pass reports the scaled pass's width as a fraction of W.
    let source = format!(
        r#"/*{{
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [{{"NAME": "scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.25, "MAX": 1.0}}],
    "PASSES": [{{"TARGET": "scaled", "WIDTH": "$WIDTH*$scale", "HEIGHT": "$HEIGHT*$scale"}}, {{}}]
}}*/

#version 450

layout(location = 0) in vec2 uv;
{ISF_UNIFORMS_WITH_HISTORY}
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D scaled;
layout(location = 0) out vec4 fragColor;

void main() {{
    float width = float(textureSize(sampler2D(scaled, texSampler), 0).x);
    fragColor = vec4(width / {W}.0, 0.0, 0.0, 1.0);
}}
"#
    );
    let shader = varda::isf::ISFShader::from_string(&source).expect("parse");
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::from_shader(&ctx, shader, W, H).expect("deck");
    mixer.channel_mut(0).unwrap().add_deck(deck);
    assert_near(
        center(&render_and_read(&ctx, &mut mixer))[0],
        1.0,
        0.002,
        "full size at scale 1",
    );
    mixer.channel_mut(0).unwrap().decks[0]
        .deck
        .generator_params
        .set_float("scale", 0.5);
    assert_near(
        center(&render_and_read(&ctx, &mut mixer))[0],
        0.5,
        0.002,
        "half size once the input changes",
    );
}
