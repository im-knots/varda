//! Tier 1 render-correctness tests — see /spec/render-testing.md.
//!
//! Render the mixer headless, read back the linear-light composite texture,
//! and assert on pixel values whose correct result is known in closed form
//! (opacity, crossfader, zero-opacity culling, blend-mode algebra, passthrough).
//!
//! Tonemap is forced to `Bypass` so the `Rgba16Float` composite holds the raw
//! linear compositing result, isolating the math from the tonemap curve. Colours
//! use only 0.0 / 1.0 channels (gamma-invariant) except crossfader-at-0.5, which
//! is a genuine linear midpoint on the pre-tonemap target.
//!
//! Skips cleanly when no GPU adapter is present (same idiom as `benches/`).

use std::sync::mpsc;

use varda::{
    audio::AudioData,
    deck::Deck,
    mixer::Mixer,
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
    renderer::tonemap::TonemapMode,
    BlendMode,
};

/// Small target — solid-colour compositing is per-pixel uniform, so a tiny
/// texture is sufficient and fast. Padded to the 256-byte row alignment in the
/// readback helper.
const W: u32 = 16;
const H: u32 = 16;

fn headless_gpu() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

/// Decode an IEEE-754 half-precision float (as raw bits) to f32.
fn f16_to_f32(bits: u16) -> f32 {
    let sign = (bits >> 15) & 1;
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x3ff;
    let sign_f = if sign == 1 { -1.0 } else { 1.0 };
    let mag = if exp == 0 {
        (frac as f32) * 2f32.powi(-24) // subnormal
    } else if exp == 0x1f {
        if frac == 0 {
            f32::INFINITY
        } else {
            f32::NAN
        }
    } else {
        (1.0 + (frac as f32) / 1024.0) * 2f32.powi(exp as i32 - 15)
    };
    sign_f * mag
}

/// Render one frame, then read back the mixer composite (`Rgba16Float`) as
/// linear-light RGBA f32, row-major, `W*H` pixels. Blocks on `poll(Wait)` —
/// allowed here because this is a test, not the render thread.
fn render_and_read(ctx: &GpuContext, mixer: &mut Mixer) -> Vec<[f32; 4]> {
    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: Default::default(),
    };
    let analyzer_values = AnalyzerValues::default();
    mixer
        .render(ctx, &audio, &audio_values, &analyzer_values, 60, &[])
        .expect("render");

    let tex = mixer.composite_texture();
    let bytes_per_pixel = 8u32; // Rgba16Float
    let unpadded = W * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("render-test readback"),
        size: (padded * H) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
    rx.recv().expect("map channel").expect("map ok");

    let mut out = Vec::with_capacity((W * H) as usize);
    {
        let data = buffer.slice(..).get_mapped_range();
        for row in 0..H {
            let base = (row * padded) as usize;
            for col in 0..W {
                let px = base + (col as usize) * 8;
                let r = f16_to_f32(u16::from_le_bytes([data[px], data[px + 1]]));
                let g = f16_to_f32(u16::from_le_bytes([data[px + 2], data[px + 3]]));
                let b = f16_to_f32(u16::from_le_bytes([data[px + 4], data[px + 5]]));
                let a = f16_to_f32(u16::from_le_bytes([data[px + 6], data[px + 7]]));
                out.push([r, g, b, a]);
            }
        }
    }
    buffer.unmap();
    out
}

/// Centre pixel of the readback — representative for uniform solid composites.
fn center(pixels: &[[f32; 4]]) -> [f32; 4] {
    pixels[((H / 2) * W + W / 2) as usize]
}

fn new_mixer(ctx: &GpuContext) -> Mixer {
    let mut mixer = Mixer::new(ctx, W, H).expect("mixer");
    // Bypass tonemap → composite holds raw linear values.
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

// ── Passthrough / opacity ────────────────────────────────────────────

#[test]
fn full_opacity_solid_deck_renders_its_color() {
    let Some(ctx) = headless_gpu() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let deck = Deck::new_solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("deck");
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
    let deck = Deck::new_solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H).expect("deck");
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(deck);
    ch.set_deck_opacity(0, 0.0);

    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_lo(px[0], "culled.R");
    assert_lo(px[1], "culled.G");
    assert_lo(px[2], "culled.B");
}

/// Opacity is linear over black: a white deck at opacity `o` composites to
/// brightness `o` (premultiplied-alpha, linear-light). This is the regression
/// test for the double-darkening bug where a channel's premultiplied composite
/// was re-blended with straight-alpha in the mixer, yielding opacity² (0.25 at
/// half). See /spec/linear-light-compositing.md and /spec/render-testing.md.
#[test]
fn opacity_is_linear_over_black() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let brightness_at = |opacity: f32| {
        let mut mixer = new_mixer(&ctx);
        let deck = Deck::new_solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H).expect("deck");
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

/// The subsequent-channel composite path (composite.wgsl, premultiplied source)
/// must also avoid double-darkening: crossfader=1 shows channel B, and a
/// half-opacity white deck in B composites to ~0.5, not 0.25.
#[test]
fn subsequent_channel_partial_opacity_is_linear() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    // Channel A opaque black so B is the visible partial layer at crossfader=1.
    let a = Deck::new_solid_color(&ctx, [0.0, 0.0, 0.0, 1.0], W, H).expect("A");
    mixer.channel_mut(0).unwrap().add_deck(a);
    let b = Deck::new_solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H).expect("B");
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
    let a = Deck::new_solid_color(ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("A");
    let b = Deck::new_solid_color(ctx, [0.0, 0.0, 1.0, 1.0], W, H).expect("B");
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

// ── Blend-mode algebra (GPU-only; cannot be unit-tested on the CPU) ───

/// base (red, Normal) with a top deck (green) in the given blend mode.
fn blend_mixer(ctx: &GpuContext, top_mode: BlendMode) -> Mixer {
    let mut mixer = new_mixer(ctx);
    let base = Deck::new_solid_color(ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("base");
    let top = Deck::new_solid_color(ctx, [0.0, 1.0, 0.0, 1.0], W, H).expect("top");
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

// ── Unified color path (spec/unified-color-pipeline.md) ──────────────
//
// These three tests replace the unfalsifiable revisit trigger that the
// superseded linear-light decision left behind ("revisit if users report shadow
// banding"). They assert the deck stage's numeric behaviour directly.

/// Deck targets must preserve shadow gradation.
///
/// The deck tier used to be `Rgba8Unorm` holding *linear* values, which is the
/// one combination that loses on both axes: 8-bit precision with linear code
/// distribution. Re-quantizing linear light into 8 bits collapsed the bottom 41
/// sRGB levels into 6, so closely-spaced dark values became indistinguishable.
///
/// Concretely, at 8-bit: 0.002 → round(0.51) = 1 and 0.004 → round(1.02) = 1 —
/// the same stored code. On a float target they stay distinct. This asserts every
/// input in a dark ramp survives as a distinct output.
#[test]
fn deck_stage_preserves_shadow_gradation() {
    let Some(ctx) = headless_gpu() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };

    // Deep-shadow ramp. Every step is below linear 0.02 (~5.7 stops down),
    // which is the region 8-bit linear resolved into ~6 codes.
    let ramp = [0.001_f32, 0.002, 0.004, 0.006, 0.008, 0.012, 0.016];

    let mut observed = Vec::new();
    for v in ramp {
        let mut mixer = new_mixer(&ctx);
        let deck = Deck::new_solid_color(&ctx, [v, v, v, 1.0], W, H).expect("deck");
        mixer.channel_mut(0).unwrap().add_deck(deck);
        observed.push(center(&render_and_read(&ctx, &mut mixer))[0]);
    }

    // Every distinct input must yield a distinct output. Pairwise, because a
    // simple dedup count would not say which steps collapsed.
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

    // And the ramp must stay monotonic — ordering is not merely preserved by
    // accident of rounding.
    for w in observed.windows(2) {
        assert!(
            w[1] > w[0],
            "shadow ramp not monotonic: {:?} (full ramp {:?})",
            w,
            observed
        );
    }
}

/// A deck must be able to hand HDR to the compositor.
///
/// The deck tier's `Rgba8Unorm` target clamped to [0,1], so no value above 1.0
/// from any deck ever reached the `Rgba16Float` composite. That left the nine
/// tonemap operators fed by blend arithmetic alone — a signal bounded around
/// 2.0 that could not exercise curves designed for scene-referred highlights.
#[test]
fn deck_headroom_above_one_survives_to_the_composite() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    // A generator-style value well above display white.
    let deck = Deck::new_solid_color(&ctx, [4.0, 2.0, 1.0, 1.0], W, H).expect("deck");
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

/// Deck effects must run at the same precision as channel and master effects.
///
/// `Effect::new` defaulted to `Rgba8Unorm` while channel and master effects were
/// given `compositing_format`, so an effect's precision silently depended on
/// which chain it was dropped into — and a four-deep deck chain re-quantized
/// once per stage. See spec/unified-color-pipeline.md.
///
/// This asserts the format invariant; `deck_effect_transforms_pixels` covers the
/// behavioural side. Kept separate because they fail for different reasons — a
/// format regression here is silent in pixels for mid-tone content, where 8-bit
/// and float differ by only ~0.001.
#[test]
fn deck_effects_run_at_composite_precision() {
    let Some(ctx) = headless_gpu() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    // Repo path, not `get_bundled_shader_path()` — that resolves relative to the
    // executable and only finds shaders in a packaged .app/tarball, so it returns
    // None under `cargo test` and would make this test silently skip forever.
    // A missing shader here is a failure, not a skip; only a missing GPU skips.
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

/// Every bundled shader must produce a valid render pipeline.
///
/// Regression test for a real break introduced while unifying the color path:
/// the sampler was switched to `Filtering` (correct — `Rgba16Float` is
/// filterable) but the *texture* bind-group entries still declared
/// `Float { filterable: false }` whenever a shader had pass buffers. wgpu
/// rejects that pairing at `create_render_pipeline`, so the app aborted on
/// startup as soon as it loaded a multipass shader.
///
/// Nothing caught it: every other GPU test here uses solid-colour decks or a
/// single-pass filter, so `num_pass_buffers == 0` and the two settings happened
/// to agree. This builds every bundled shader through the same constructors the
/// app uses — `Deck::new` for generators, `Effect::new` for filters — so any
/// future bind-group-layout mismatch fails here instead of at runtime.
#[test]
fn every_bundled_shader_builds_a_pipeline() {
    let Some(ctx) = headless_gpu() else {
        eprintln!("no GPU adapter — skipping");
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

        // Three shader kinds, three production constructors. Classify by the image
        // inputs the shader declares, and use the real constructors so binding
        // counts come from production code rather than being re-derived (and
        // drifting) here.
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
            // Transitions bypass Deck/Effect entirely — own pipeline, own layout.
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
            Deck::new(&ctx, shader, W, H).map(|_| ())
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
    // Guard the guard: if the classification ever collapses this test would still
    // pass while covering nothing interesting.
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

/// A deck effect must actually transform the deck's pixels.
///
/// Regression test for a bug that made every ISF `bool` toggle permanently
/// false: `ParamValue::Bool` is written as a `u32`, but five shaders declared
/// their bool inputs as `float`. The bytes `01 00 00 00` reinterpreted as an
/// IEEE-754 float are `1.4e-45`, so `invert_r > 0.5` was never true and
/// `invert.fs` returned its input unchanged — the effect ran, bound correctly,
/// and did nothing.
///
/// `invert` with default params is a total inversion, so black must come out
/// white. Two effects must cancel back to black, which also pins the deck
/// ping-pong parity (an off-by-one there would show the wrong buffer).
#[test]
fn deck_effect_transforms_pixels() {
    let Some(ctx) = headless_gpu() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let invert = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/invert.fs");
    assert!(invert.exists(), "shaders/invert.fs missing");

    let run = |n_effects: usize| -> [f32; 4] {
        let mut mixer = new_mixer(&ctx);
        let deck = Deck::new_solid_color(&ctx, [0.0, 0.0, 0.0, 1.0], W, H).expect("deck");
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

/// A shader that emits above display white must reach the compositor unclamped.
///
/// The clamp-removal pass (spec/unified-color-pipeline.md step 7) replaced the
/// terminal `clamp(col, 0.0, 1.0)` in 37 shaders with `max(col, 0.0)` — dropping
/// the ceiling while keeping the floor, so negatives never reach the blend math.
/// Before that, generators and filters silently limited to display white and the
/// nine tonemap operators could never see a signal wide enough to exercise them.
///
/// `glow_bloom` is purely additive (`result = src.rgb + bloom * amount * color`),
/// so white input plus bloom must exceed 1.0. Params are pinned rather than left
/// at defaults so the expected value is deterministic.
///
/// This also covers the filter half of the pass, which matters more than the
/// generator half: a filter that clamps destroys headroom produced *upstream*,
/// so every clamping filter was an HDR limiter sitting in the chain.
#[test]
fn additive_filter_emits_above_display_white() {
    let Some(ctx) = headless_gpu() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/glow_bloom.fs");
    assert!(path.exists(), "shaders/glow_bloom.fs missing");

    let mut mixer = new_mixer(&ctx);
    let deck = Deck::new_solid_color(&ctx, [1.0, 1.0, 1.0, 1.0], W, H).expect("deck");
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(deck);

    let shader = varda::isf::ISFShader::from_file(&path).expect("parse glow_bloom.fs");
    let mut fx = varda::deck::Effect::new(&ctx, shader).expect("deck effect");
    // threshold 0 → all of a white input counts as "bright"; full glow amount.
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
