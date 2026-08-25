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

mod common;
use common::headless_gpu;

/// Decode an IEEE-754 half-precision float (as raw bits) to f32.
fn f16_to_f32(bits: u16) -> f32 {
    let sign = (bits >> 15) & 1;
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x3ff;
    let sign_f = if sign == 1 { -1.0 } else { 1.0 };
    let mag = if exp == 0 {
        f32::from(frac) * 2f32.powi(-24) // subnormal
    } else if exp == 0x1f {
        if frac == 0 {
            f32::INFINITY
        } else {
            f32::NAN
        }
    } else {
        (1.0 + f32::from(frac) / 1024.0) * 2f32.powi(i32::from(exp) - 15)
    };
    sign_f * mag
}

/// Advance the mixer by one frame with silent audio and no modulation, on the
/// wall clock.
fn render_once(ctx: &GpuContext, mixer: &mut Mixer) {
    render_frame(ctx, mixer, None);
}

/// One frame at a stated point on the free-running clock.
///
/// Any test that grades how a picture changes *between* frames needs this
/// rather than [`render_once`]: on the wall clock a frame advances `TIME` by
/// however long the last one took to render, so the metric would partly measure
/// the machine. Under a software rasterizer on a shared runner that jitter is
/// larger than the effect being graded.
fn render_at(ctx: &GpuContext, mixer: &mut Mixer, frame: usize) {
    /// The rate a show is authored against, so the steps are the ones a
    /// performer would see.
    const FPS: f32 = 60.0;
    render_frame(ctx, mixer, Some(frame as f32 / FPS));
}

fn render_frame(ctx: &GpuContext, mixer: &mut Mixer, free_run_time: Option<f32>) {
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
    };
    mixer.render(ctx, &inputs, 60, &[]).expect("render");
}

/// Read back the mixer composite (`Rgba16Float`) as linear-light RGBA f32,
/// row-major, `w*h` pixels. Blocks on `poll(Wait)` — allowed here because this
/// is a test, not the render thread.
fn read_back(ctx: &GpuContext, mixer: &Mixer, width: u32, height: u32) -> Vec<[f32; 4]> {
    read_texture(ctx, mixer.composite_texture(), width, height)
}

/// Read back any `Rgba16Float` target as linear-light RGBA f32.
fn read_texture(ctx: &GpuContext, tex: &wgpu::Texture, width: u32, height: u32) -> Vec<[f32; 4]> {
    let bytes_per_pixel = 8u32; // Rgba16Float
    let unpadded = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("render-test readback"),
        size: u64::from(padded * height),
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
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
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

    let mut out = Vec::with_capacity((width * height) as usize);
    {
        let data = buffer
            .slice(..)
            .get_mapped_range()
            .expect("render correctness readback must be mapped");
        let channel = |px: usize, i: usize| {
            f16_to_f32(u16::from_le_bytes([data[px + i * 2], data[px + i * 2 + 1]]))
        };
        for row in 0..height {
            let base = (row * padded) as usize;
            for col in 0..width {
                let px = base + (col as usize) * 8;
                out.push([
                    channel(px, 0),
                    channel(px, 1),
                    channel(px, 2),
                    channel(px, 3),
                ]);
            }
        }
    }
    buffer.unmap();
    out
}

/// Render one frame and read it back at the default test size.
fn render_and_read(ctx: &GpuContext, mixer: &mut Mixer) -> Vec<[f32; 4]> {
    render_once(ctx, mixer);
    read_back(ctx, mixer, W, H)
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

// ── Compositing capacity ─────────────────────────────────────────────

/// The channel compositor writes one params ring-buffer slot per compositing
/// deck (`write_params_slot(.., i, ..)` in `channel/mod.rs`), and the ring is
/// allocated at a fixed `MAX_DRAW_SLOTS` = 16 in `renderer/blit.rs`. Nothing
/// clamps or grows it — `PolygonBlitPipeline` has `ensure_ring_slots`, the blit
/// and composite pipelines do not — so deck 17 addresses past the end of the
/// buffer.
///
/// Stacked opaque Normal decks mean the topmost is the only visible one, so a
/// correct run shows the last deck's colour and raises no GPU fault.
#[test]
fn channel_composites_more_decks_than_ring_slots() {
    const DECKS: usize = 20;

    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch = mixer.channel_mut(0).unwrap();
    for i in 0..DECKS {
        // Every deck below the top is red; the top deck is green, so the
        // assertion below distinguishes "deck 20 drew" from "deck 16 drew".
        let color = if i == DECKS - 1 {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        };
        let deck = Deck::new_solid_color(&ctx, color, W, H).expect("deck");
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

// ── Blend space: pivot modes (spec/blend-modes.md § Blend Space) ─────
//
// Overlay, Hard Light, and Soft Light pin a branch (or, for Pegtop Soft Light,
// an identity point) to the constant 0.5 — perceptual middle grey in a
// gamma-encoded space. Middle grey is linear 0.214, so evaluating these three
// on linear operands relocates the pivot to sRGB 0.735.
//
// The existing blend tests above use only 0.0/1.0 channels, which are
// gamma-invariant — that is precisely why this shipped undetected. These tests
// use mid-tones, the only place the defect is observable.

/// Linear value of sRGB 0.5 — perceptual middle grey.
const MID_GREY_LINEAR: f32 = 0.214_041_14;

/// `Rgba16Float` carries ~10-11 mantissa bits; 5e-3 is comfortably above the
/// format's resolution and far below the ~0.1 deltas these tests discriminate.
const BLEND_TOL: f32 = 0.005;

/// Base deck at `dst` grey, top deck at `src` grey in `mode`. Greys keep R=G=B
/// so any channel witnesses the result.
fn grey_blend_mixer(ctx: &GpuContext, mode: BlendMode, src: f32, dst: f32) -> Mixer {
    let mut mixer = new_mixer(ctx);
    let base = Deck::new_solid_color(ctx, [dst, dst, dst, 1.0], W, H).expect("base");
    let top = Deck::new_solid_color(ctx, [src, src, src, 1.0], W, H).expect("top");
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

/// Overlay of middle grey over middle grey is identity.
///
/// Both branches of Overlay agree at the pivot, so this is exact regardless of
/// which side the comparison lands on — no knife-edge on `dst < 0.5`.
/// Linear-operand evaluation instead yields 2·0.214·0.214 = 0.092.
#[test]
fn blend_overlay_of_middle_grey_is_identity() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = grey_blend_mixer(&ctx, BlendMode::Overlay, MID_GREY_LINEAR, MID_GREY_LINEAR);
    let px = center(&render_and_read(&ctx, &mut mixer));
    assert_grey(px, MID_GREY_LINEAR, "overlay_mid");
}

/// Hard Light of middle grey over middle grey is identity (same pivot, roles
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

/// Pegtop Soft Light is identity when the source is middle grey: the
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

/// Off-pivot reference value, computed from the sRGB-operand formula:
/// src sRGB 0.25 over dst sRGB 0.75 takes Overlay's screen branch —
/// `1 - 2·(1-0.25)·(1-0.75) = 0.625` → linear 0.34851.
/// Linear-operand evaluation yields 0.0936, a 0.255 error.
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

/// Guard: the fix must stay scoped to the pivot modes. Screen is a physical
/// mode and must keep evaluating on linear operands.
///
/// Linear (correct): `1 - (1-0.214)² = 0.3823`.
/// If Screen were wrongly encoded too it would give `1 - 0.5² = 0.75` → linear
/// 0.5225 — far outside tolerance.
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
            "shadow ramp not monotonic: {w:?} (full ramp {observed:?})"
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

// ── Generator framing ────────────────────────────────────────────────

/// `dull_skull` must keep its subject on screen for the whole animation.
///
/// Its camera used to orbit the world origin without bound (`rotAngle =
/// PHASE_TIME_1`). Past roughly 68° it crossed behind the backdrop half-space
/// the skull melts into, and from there every frame was solid black until the
/// orbit came back around — the shader went dark for a third of its cycle. The
/// swing is now `sin(PHASE_TIME_1) * sway_range` about the skull's mean
/// position, which bounds it and keeps the camera in front of the backdrop.
///
/// Driven at maximum `speed` and `rot_speed` so a few hundred frames stand in
/// for minutes of a set. Measured over these samples at this resolution:
///
/// | metric                | fixed            | buggy                  |
/// |-----------------------|------------------|------------------------|
/// | dark-pixel fraction   | 0.000 throughout | 0.49–1.00 on 12/30     |
/// | mean horizontal edge  | 0.0034–0.0062    | 0.0000 while blacked   |
///
/// The dark fraction is the decisive one. The edge floor additionally catches
/// the frames where the drifting backdrop swallowed the skull into a
/// featureless blob, which measured 0.0013–0.0024.
#[test]
fn dull_skull_stays_in_frame_for_the_whole_sway() {
    const SW: u32 = 320;
    const SH: u32 = 180;
    const FRAMES: usize = 400;
    const SAMPLE_EVERY: usize = 20;
    /// Fixed measures 0.000; buggy reaches 1.000.
    const MAX_DARK_FRACTION: f32 = 0.25;
    /// Fixed measures 0.0034 at worst; a swallowed skull measures 0.0024.
    const MIN_EDGE: f32 = 0.0020;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/dull_skull.fs");
    let shader = varda::isf::ISFShader::from_file(&path).expect("parse dull_skull.fs");

    let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let mut deck = Deck::new(&ctx, shader, SW, SH).expect("deck");
    deck.generator_params.set_float("speed", 3.0);
    deck.generator_params.set_float("rot_speed", 2.0);
    mixer.channel_mut(0).unwrap().add_deck(deck);

    // Thresholds above were measured on gamma-encoded values — the space the
    // audience sees, and the only one in which "is anything visible" means
    // anything. The composite is linear, so encode before measuring.
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

        let lum: Vec<f32> = read_back(&ctx, &mixer, SW, SH)
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
/// Passing `no_shader_scales_accumulated_phase_by_a_parameter` is necessary but
/// not sufficient. A parameter can be perfectly continuous and still be an
/// *amplitude* — something that sets where the image is sampled rather than how
/// fast it moves — and an LFO on an amplitude drives the picture out and back at
/// the LFO's rate. That reads as sloshing, and it is what an operator means when
/// they say an automated fader looks stuttery.
///
/// `liquid_light.fs`'s Agitation was built both ways, so the two are measured
/// against each other here. Per-frame luminance change under a 1 Hz triangle
/// LFO sweeping Agitation 0.1..0.9, against the same shader with the fader
/// parked at the midpoint:
///
/// | Agitation implemented as | parked  | automated | ratio  |
/// |--------------------------|---------|-----------|--------|
/// | domain-warp gain         | 0.00135 | 0.01814   | 13.5x  |
/// | mixing rate (slot 1)     | 0.00475 | 0.00459   | 0.97x  |
///
/// A rate scores at or below 1.0 because the LFO spends half its time below the
/// midpoint, slowing the churn. Anything much above 1.0 means the fader is
/// moving the fluid rather than stirring it.
#[test]
fn liquid_light_agitation_survives_being_automated() {
    const SW: u32 = 256;
    const SH: u32 = 144;
    const WARMUP: usize = 300;
    const MEASURE: usize = 120;
    /// Triangle period in frames — a brisk but ordinary LFO.
    const PERIOD: usize = 60;
    const LOW: f32 = 0.1;
    const HIGH: f32 = 0.9;
    /// Rate scores 0.97x, the amplitude version it replaced scored 13.5x.
    const MAX_RATIO: f32 = 3.0;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/liquid_light.fs");
    let shader = varda::isf::ISFShader::from_file(&path).expect("parse liquid_light.fs");

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_back(ctx, mixer, SW, SH)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    // Median per-frame change over MEASURE frames, with Agitation set by
    // `automate` each frame. Median rather than mean so one outlier cannot
    // carry the result either way.
    let run = |automate: &dyn Fn(usize) -> f32| -> f32 {
        let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
        mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
        let mut deck = Deck::new(&ctx, shader.clone(), SW, SH).expect("deck");
        deck.generator_params.set_float("flow_speed", 0.5);
        deck.generator_params.set_float("agitation", automate(0));
        mixer.channel_mut(0).unwrap().add_deck(deck);

        for _ in 0..WARMUP {
            render_once(&ctx, &mut mixer);
        }
        let mut prev = luminance(&ctx, &mixer);
        let mut deltas = Vec::with_capacity(MEASURE);
        for frame in 0..MEASURE {
            mixer.channel_mut(0).unwrap().decks[0]
                .deck
                .generator_params
                .set_float("agitation", automate(frame));
            render_once(&ctx, &mut mixer);
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

    // Guard the guard: a frozen dish makes any automation look smooth.
    assert!(
        parked > 1e-4,
        "the dish is not moving with the fader parked ({parked:.6}), so this proves nothing"
    );

    let ratio = automated / parked;
    assert!(
        ratio <= MAX_RATIO,
        "automating Agitation makes the fluid slosh: {automated:.5} per frame under a \
         {PERIOD}-frame LFO against {parked:.5} parked ({ratio:.1}x — allowed {MAX_RATIO:.1}x). \
         Agitation is setting a position rather than a rate; see spec/phase-accumulators.md \
         § Authoring Rules."
    );
}

/// Moving a rate fader must change the speed of the motion, not cut to a
/// different frame of it.
///
/// This is the behavioural half of the phase-accumulator contract; the source
/// half is `no_shader_scales_accumulated_phase_by_a_parameter` in
/// `tests/shader_param_contract_guard.rs`. Both exist because the guard reads
/// source and so cannot see a value that crosses a function boundary, while
/// this measures pixels and so catches the mistake however it is spelled.
///
/// The subject is `liquid_light`'s Dish Rotation, chosen because it is a *pure*
/// rate: it drives accumulator slot 2 and has no other effect on the image. An
/// earlier version of this test used Agitation, which was where the bug
/// actually shipped, and had to be retargeted when Agitation gained a
/// legitimate instantaneous effect — it now opens the domain warp as well as
/// speeding the flow. Once a fader changes the picture for a good reason, a
/// pixel metric can no longer tell that change apart from a teleport: measured
/// against the restored bug, correct code scored 1.26x and buggy code 1.90x,
/// which is not a gap a threshold can live in. A pure rate keeps the
/// measurement clean.
///
/// Mean per-pixel luminance change between consecutive frames, 30 s of
/// accumulated phase in, with Dish Rotation nudged from 0.40 to 0.45:
///
/// | metric                       | fixed  | buggy  |
/// |------------------------------|--------|--------|
/// | steady-state delta per frame | 0.0031 | 0.0021 |
/// | delta across the fader move  | 0.0034 | 0.0369 |
/// | ratio                        | 1.1x   | 17.6x  |
///
/// Buggy numbers taken by writing `float dish = PHASE_TIME_2 * swirl;`, which
/// is the shape the guard and this test both exist to reject.
#[test]
fn liquid_light_dish_rotation_changes_speed_rather_than_position() {
    const SW: u32 = 256;
    const SH: u32 = 144;
    /// 30 s at 60 fps. The jump grows with accumulated phase, so a short run
    /// would let the bug through; a set lasts far longer than this.
    const WARMUP: usize = 1800;
    /// Frames of steady state to average the baseline over.
    const BASELINE_FRAMES: usize = 10;
    /// Fixed measures 1.1x, buggy 17.6x, so the threshold sits between them
    /// with room for run-to-run variation on either side.
    const MAX_RATIO: f32 = 5.0;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/liquid_light.fs");
    let shader = varda::isf::ISFShader::from_file(&path).expect("parse liquid_light.fs");

    let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let mut deck = Deck::new(&ctx, shader, SW, SH).expect("deck");
    deck.generator_params.set_float("flow_speed", 1.0);
    deck.generator_params.set_float("swirl", 0.4);
    mixer.channel_mut(0).unwrap().add_deck(deck);

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_back(ctx, mixer, SW, SH)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    for _ in 0..WARMUP {
        render_once(&ctx, &mut mixer);
    }

    let mut prev = luminance(&ctx, &mixer);
    let mut baseline = 0.0f32;
    for _ in 0..BASELINE_FRAMES {
        render_once(&ctx, &mut mixer);
        let cur = luminance(&ctx, &mixer);
        baseline += mean_delta(&prev, &cur);
        prev = cur;
    }
    baseline /= BASELINE_FRAMES as f32;

    // Guard the guard: a static image would make any jump look proportional.
    assert!(
        baseline > 1e-4,
        "the dish is not turning (baseline {baseline:.6}), so this proves nothing"
    );

    mixer.channel_mut(0).unwrap().decks[0]
        .deck
        .generator_params
        .set_float("swirl", 0.45);
    render_once(&ctx, &mut mixer);
    let jump = mean_delta(&prev, &luminance(&ctx, &mixer));

    assert!(
        jump <= baseline * MAX_RATIO,
        "moving Dish Rotation spun the dish: {jump:.4} against a steady-state {baseline:.4} \
         ({:.1}x — allowed {MAX_RATIO:.1}x). Accumulated phase is being scaled by a live \
         parameter again; see spec/phase-accumulators.md.",
        jump / baseline
    );
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

/// An effect fed smoothly changing input must change smoothly.
///
/// `chroma_flow` grades every pixel against a palette of anchor colours and
/// gives each colour group its own flow direction, so the anchors decide how the
/// whole frame moves. In auto mode those anchors were re-derived every frame by
/// greedy farthest-point selection over a grid of samples. Selection is
/// discontinuous: when two candidates are near-tied, a change in the picture too
/// small to see swaps which one wins, and that slot's anchor jumps to an
/// unrelated colour. Because every pixel is graded against it, one jump
/// re-groups the entire frame at once and the image lurches — the "jitters every
/// so often" report, against a source that is itself perfectly smooth.
///
/// Measured as the worst single-frame change over the median one, which is what
/// separates an occasional lurch from ordinary motion. A fixed palette is the
/// control: it holds the anchors still, so whatever it scores is the harness and
/// the content rather than the effect. Auto has to stay near it.
///
/// Persisting the palette fixed most of it. Two further faults surfaced later,
/// both of them in the machinery meant to keep the palette calm. Slots were
/// matched to fresh anchors in slot-index order, so slot 0 took whichever anchor
/// it liked and the last slot was left with whatever remained — possibly nothing
/// like where it sat. And the settle was easing the anchors so far behind the
/// picture that a lagging anchor would eventually sweep a whole flat region
/// across a decision boundary in one frame, which meant turning Palette
/// Stability *up* made the picture measurably less stable. Matching the closest
/// available pair each round and shortening the settle fixed both.
///
/// | source          | original | palette persisted | + ordered matching, short settle |
/// |-----------------|----------|-------------------|----------------------------------|
/// | `dull_skull`    | 7.44x    | 5.77x             | 1.16x                            |
/// | `liquid_light`  | 5.84x    | 1.10x             | 1.10x                            |
///
/// Auto now scores at or below the fixed-palette control. The middle column was
/// read against a different transport model, so it is indicative rather than
/// directly comparable; the outer two were measured on the current one.
#[test]
fn chroma_flow_auto_palette_does_not_lurch_on_smooth_input() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const WARMUP: usize = 30;
    const MEASURE: usize = 90;
    /// How much worse than the fixed-palette control the automatic one is
    /// allowed to be.
    ///
    /// Graded against the control rather than against an absolute number,
    /// because the absolute figure measures the effect and the source as much as
    /// the palette: a warp with a hard grade on the way out produces spiky frame
    /// deltas by design, and a fixed threshold set against one transport model
    /// silently becomes a different test under the next. The control holds the
    /// anchors still and is otherwise identical, so it isolates the one thing
    /// this is about. Auto currently runs at or just under it.
    const MAX_SPIKE_RATIO: f32 = 1.4;

    let Some(ctx) = headless_gpu() else {
        return;
    };

    let fx_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/chroma_flow.fs");
    let fx_shader = varda::isf::ISFShader::from_file(&fx_path).expect("parse chroma_flow.fs");

    let luminance = |ctx: &GpuContext, mixer: &Mixer| -> Vec<f32> {
        read_back(ctx, mixer, SW, SH)
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect()
    };
    let mean_delta = |a: &[f32], b: &[f32]| -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    };

    // Worst-frame-over-median change for one configuration.
    let spike_ratio = |src_name: &str, manual: bool, stability: f32| -> f32 {
        let src =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("shaders/{src_name}"));
        let src_shader = varda::isf::ISFShader::from_file(&src).expect("parse source");
        let mut mixer = Mixer::new(&ctx, SW, SH).expect("mixer");
        mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
        let deck = Deck::new(&ctx, src_shader, SW, SH).expect("deck");
        let ch = mixer.channel_mut(0).unwrap();
        ch.add_deck(deck);
        let mut fx = varda::deck::Effect::new(&ctx, fx_shader.clone()).expect("deck effect");
        fx.params.set_bool("palette_mode", manual);
        fx.params.set_float("palette_stability", stability);
        ch.decks[0].deck.add_effect(fx);
        // Decks default to adaptive skipping keyed on wall-clock render cost. A
        // skipped frame repeats the previous picture, which lands in the metric
        // as a near-zero delta or a doubled one, so the measurement would partly
        // grade the scheduler and vary with machine load.
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
        deltas.last().expect("measured frames") / median
    };

    for src in ["dull_skull.fs", "liquid_light.fs", "taste_of_noise.fs"] {
        // Across the whole of Palette Stability, not just its default. Turning
        // that control up used to make the picture measurably *less* steady,
        // which is the opposite of what it promises, and a single reading at the
        // default would not have caught it.
        let auto = [0.0f32, 0.5, 1.0]
            .into_iter()
            .map(|stability| spike_ratio(src, false, stability))
            .fold(0.0f32, f32::max);
        let manual = spike_ratio(src, true, 0.5);
        assert!(
            auto < manual * MAX_SPIKE_RATIO,
            "{src}: auto palette lurched — worst frame changed {auto:.2}x the \
             median, against {manual:.2}x with the palette held fixed. An anchor jumped \
             and regraded the whole frame at once."
        );
    }
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
    let deck = Deck::new(ctx, src_shader, w, h).expect("deck");
    {
        let ch = mixer.channel_mut(0).expect("channel 0");
        ch.add_deck(deck);
        if let Some(configure) = configure {
            let fx_shader = varda::isf::ISFShader::from_file(&fx_path).expect("parse chroma_flow");
            let mut fx = varda::deck::Effect::new(ctx, fx_shader).expect("deck effect");
            configure(&mut fx.params);
            ch.decks[0].deck.add_effect(fx);
        }
        // Adaptive skipping keys on wall-clock render cost, so the number of
        // rendered frames would otherwise vary between runs that must line up.
        ch.decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(0);
    }

    let last = capture_at.iter().copied().max().unwrap_or(0);
    let mut out = Vec::with_capacity(capture_at.len());
    for frame in 1..=last {
        render_at(ctx, &mut mixer, frame);
        if capture_at.contains(&frame) {
            out.push(
                read_back(ctx, &mixer, w, h)
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

/// Colour must never cross into darkness.
///
/// A layer travels over whatever it is sitting on, and dark ground is not
/// something to travel over — it is floor. Without the rule every region expands
/// into whatever is next to it, the frame fills in, and the result reads as
/// smoke; with it, shapes are held to the lit parts of the picture and crawl
/// around the dark ones.
///
/// Dark ground is also kept distinct from ground a layer has *vacated*. Vacated
/// ground is repainted from the refill cycle, and repainting the shadows would
/// turn the darkest parts of the frame into the loudest. Barrier ground shows
/// the brightness it seeded with instead, and is never repainted.
///
/// The comparison is against the same source with no effect, so "dark" means
/// dark in the real picture rather than wherever the test guessed. It takes the
/// brightest each pixel ever gets across the whole run: a pixel dark only at the
/// final frame may legitimately have seeded as a layer while it was lit, and
/// carries that colour until its memory runs out. Only a pixel that is dark for
/// the entire run can never have been anything but barrier.
#[test]
fn chroma_flow_never_flows_into_darkness() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 90;
    /// Readback is f16 and the barrier path passes its seeded brightness
    /// through, so anything above rounding is colour that genuinely arrived
    /// where it was forbidden.
    const TOLERANCE: f32 = 4e-3;
    /// Needs a source with genuine shadow in it: the skull and liquid shaders
    /// used elsewhere never fall below the barrier at all, so they would prove
    /// nothing.
    const SOURCE: &str = "taste_of_noise.fs";
    /// Raised well above its default. The shader classifies against the deck's
    /// own output while this test reads the composited frame, and the two do not
    /// agree closely enough to call a pixel sitting near the threshold. Lifting
    /// the barrier far above anything the test calls dark makes the subset
    /// unambiguous in either space, at no cost to what is being checked — the
    /// rule under test is that layers cannot enter barrier ground, not where the
    /// barrier happens to sit.
    const BARRIER: f32 = 0.6;
    /// Comfortably below `BARRIER` however the two spaces relate.
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
            // Push hard: fast warp and a large palette give the motion every
            // opportunity to spill somewhere it should not reach.
            p.set_float("zoom", 1.6);
            p.set_float("rotate", 90.0);
            p.set_float("warp_amount", 1.5);
            p.set_float("palette_size", 6.0);
            p.set_float("barrier_level", BARRIER);
            // Softening happens on the way out and would blur a lit pixel a
            // fraction of a texel into the dark. Off, so this grades transport.
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

/// Hardness must be able to open the boundary.
///
/// The guard above proves the barrier holds; on its own that is also satisfied
/// by a barrier welded shut. This is the other half: wound down, colour is
/// supposed to burst its banks and run into the dark, which is the whole point
/// of having the control on a modulator for a drop.
///
/// Graded against the same effect with hardness closed, not against the source.
/// Measuring the open run against the source directly does not work: the
/// reference is the brightest each pixel ever gets across the run while the
/// reading is the final frame, so on an animated source the two differ by a
/// wide margin whatever the barrier does. That version passed with hardness
/// wired out of the calculation entirely. Two runs of the same effect at the
/// same frame share all of that, and differ only in the one control.
#[test]
fn chroma_flow_barrier_hardness_opens_the_boundary() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 90;
    const SOURCE: &str = "taste_of_noise.fs";
    const BARRIER: f32 = 0.6;
    const CALLED_DARK: f32 = 0.15;
    /// Softened all the way, the flow should be well into the dark rather than
    /// just brushing the boundary.
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
/// Feeding a picture back through a warp and blending, however lightly, is a
/// diffusion: run it back through itself sixty times a second and every boundary
/// in the frame washes out within seconds. That is what turns this kind of
/// effect into smoke, and it is why the field is stored as flat stack heights
/// and moved by a hard hand-off rather than a blend.
///
/// Edge energy is the direct measure. Diffusion drives it toward zero; transport
/// leaves it alone, because a region that moves as a body keeps the boundary it
/// started with. Measured against the untouched source, so a quiet passage in
/// the source does not read as a failure.
#[test]
fn chroma_flow_regions_keep_their_edges() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    /// Long enough that a per-frame blend would have compounded away. A 5% blend
    /// leaves under a thousandth of any boundary after this many frames.
    const FRAMES: usize = 180;
    /// Grouping flattens the interior of each region, so some loss against the
    /// source is expected and correct. Total collapse is not.
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

/// The picture has to actually travel.
///
/// This is the failure that produced three rebuilds. Each one moved its material
/// correctly and none of it showed, because the vacated space was refilled from
/// the live frame — so what was carried away was instantly replaced by the
/// picture that had been there all along. All that appeared was churn in the
/// thin band where the moved material and the source disagreed, which reads as
/// flicker rather than flow.
///
/// A still camera is the control. It exercises the identical path — same warp
/// pass, same regeneration, same grading — with only the motion removed, so
/// anything the two runs share is not travel. A build that merely re-grades its
/// source in place scores near zero here no matter how busy it looks.
#[test]
fn chroma_flow_actually_travels() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 120;
    /// Well clear of the boundary-band churn a static build produces, and far
    /// under what real travel gives.
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

/// The mask must hold its circle still and let the rest move.
///
/// The hold is applied twice, in the warp buffer and again on the way out, and
/// only the second is checked here. Fading back to the source at the end is what
/// makes held ground read as untouched picture rather than as a posterised,
/// frozen version of the effect, and that is what this measures. Holding it in
/// the buffer as well matters for a reason a still mask cannot show: without it
/// the warp keeps running underneath the held area, and the moment the mask is
/// moved — which is the whole point of putting it on a modulator — everything
/// that accumulated under there surfaces at once.
///
/// Measured against the bare deck. An earlier version of this used the effect
/// itself in a hold-everything configuration, which looked tidier and was very
/// nearly vacuous: any break shared by both runs cancels out, and removing
/// either half of the mask still passed.
#[test]
fn chroma_flow_mask_holds_its_circle_still() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 120;
    const RADIUS: f32 = 0.3;
    /// Held ground is a fade back to the source, so it should match to rounding.
    const MAX_HELD_DRIFT: f32 = 6e-3;
    /// Outside has a warp and a hard grade on it, so it should differ by far
    /// more than this. Set low enough to stay clear of quiet passages in the
    /// source.
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
        // Skip an annulus around the boundary: the edge lands between texels and
        // grading either side of it is a coin toss.
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

/// The source must keep bleeding back into the field.
///
/// A warp can only move and stretch what is already in the buffer, so a field
/// left to warp alone drifts away from its input and eventually shows nothing
/// but its own smeared history. Letting the source reassert itself at a
/// controlled rate is Deforum's strength schedule, and it is what keeps the
/// picture tied to the material it is supposed to be transforming.
///
/// Guarded as "Regen changes the picture", because the failure it catches is
/// exact: if the source is never mixed back in, the setting is inert and runs at
/// different values come back bit for bit identical.
#[test]
fn chroma_flow_regenerates_from_the_source() {
    const SW: u32 = 128;
    const SH: u32 = 128;
    const FRAMES: usize = 300;
    /// The failure this catches is exact equality, so the bar only has to sit
    /// clear of readback rounding. It is set well above that anyway, since two
    /// genuinely different regeneration rates diverge across the whole frame.
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
// See /spec/program-tap.md. The contract these pin is that a tap shows the
// *previous* frame, uniformly, regardless of where the tapping deck sits in
// the channel order. That is what makes feedback loops terminate, so it must
// fail loudly if someone later "optimizes" a tap into reading the live target.

/// Advance the mixer the way the app does: resolve taps, then render.
fn render_frame_with_taps(ctx: &GpuContext, mixer: &mut Mixer) {
    mixer.prepare_taps(ctx);
    render_once(ctx, mixer);
}

/// A tap in a *later* channel than the one it reads. This is the direction a
/// naive implementation gets wrong: channel 0 has already composited by the
/// time channel 1's decks render, so binding it directly would show the
/// current frame's red on frame one.
#[test]
fn tap_shows_the_previous_frame_not_the_current_one() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch0_uuid = mixer.channel(0).unwrap().uuid().to_string();

    let red = Deck::new_solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("solid deck");
    mixer.channel_mut(0).unwrap().add_deck(red);
    let tap = Deck::new_from_tap(&ctx, varda::deck::TapSource::Channel(ch0_uuid), "tap", W, H)
        .expect("tap deck");
    mixer.channel_mut(1).unwrap().add_deck(tap);
    mixer.set_crossfader(1.0);

    render_frame_with_taps(&ctx, &mut mixer);
    let first = center(&read_back(&ctx, &mixer, W, H));
    assert_lo(
        first[0],
        "frame 1 tap must be black — channel 0 composited earlier in this same \
         frame, and showing its red would mean the tap read the live target",
    );

    render_frame_with_taps(&ctx, &mut mixer);
    let second = center(&read_back(&ctx, &mixer, W, H));
    assert_hi(second[0], "frame 2 tap must show frame 1's red");
}

/// The same assertion with the tap in an *earlier* channel than its source.
/// Both directions agreeing is what proves the double buffer removed the
/// ordering dependence rather than merely reversing it.
#[test]
fn tap_latency_does_not_depend_on_channel_order() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch1_uuid = mixer.channel(1).unwrap().uuid().to_string();

    let tap = Deck::new_from_tap(&ctx, varda::deck::TapSource::Channel(ch1_uuid), "tap", W, H)
        .expect("tap deck");
    mixer.channel_mut(0).unwrap().add_deck(tap);
    let red = Deck::new_solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("solid deck");
    mixer.channel_mut(1).unwrap().add_deck(red);
    mixer.set_crossfader(0.0);

    render_frame_with_taps(&ctx, &mut mixer);
    assert_lo(
        center(&read_back(&ctx, &mixer, W, H))[0],
        "frame 1 tap must be black in the earlier-channel direction too",
    );

    render_frame_with_taps(&ctx, &mut mixer);
    assert_hi(
        center(&read_back(&ctx, &mixer, W, H))[0],
        "frame 2 tap must show frame 1's red, exactly as in the other direction",
    );
}

/// A master tap is uniformly one frame behind for the same reason: every deck
/// renders before the master composite, so there is no ordering to depend on.
#[test]
fn master_tap_shows_the_previous_frame() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);

    // The tap deck sits *under* the red deck in the same channel: an opaque
    // Normal blend covers it, so the program is always exactly red, while the
    // tap deck still renders every frame. Parking it in the far channel instead
    // would fade that channel out, and a fully faded channel is culled — the
    // deck would never render and this would measure the cull, not the tap.
    let tap = Deck::new_from_tap(&ctx, varda::deck::TapSource::MasterProgram, "tap", W, H)
        .expect("tap deck");
    mixer.channel_mut(0).unwrap().add_deck(tap);
    let red = Deck::new_solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("solid deck");
    mixer.channel_mut(0).unwrap().add_deck(red);
    mixer.set_crossfader(0.0);

    let tap_texture = |m: &Mixer| -> Vec<[f32; 4]> {
        read_texture(&ctx, &m.channel(0).unwrap().decks[0].deck.texture, W, H)
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

/// A deck tapping the channel it lives in is a legitimate and commonly wanted
/// feedback configuration. At partial opacity it must converge rather than
/// producing NaN or infinity in the `Rgba16Float` target.
#[test]
fn self_tapping_deck_converges_rather_than_diverging() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let ch0_uuid = mixer.channel(0).unwrap().uuid().to_string();

    let red = Deck::new_solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("solid deck");
    mixer.channel_mut(0).unwrap().add_deck(red);
    let tap = Deck::new_from_tap(&ctx, varda::deck::TapSource::Channel(ch0_uuid), "tap", W, H)
        .expect("tap deck");
    let tap_idx = mixer.channel_mut(0).unwrap().add_deck(tap);
    mixer.channel_mut(0).unwrap().decks[tap_idx].opacity = 0.5;
    mixer.set_crossfader(0.0);

    for frame in 0..12 {
        render_frame_with_taps(&ctx, &mut mixer);
        let px = center(&read_back(&ctx, &mixer, W, H));
        for (i, v) in px.iter().enumerate() {
            assert!(
                v.is_finite(),
                "frame {frame} channel {i} went non-finite ({v}) — a self-tap at \
                 50% opacity is a difference equation and must settle"
            );
        }
    }
}

/// The feature has to be free when unused: no tap deck, no tap target.
#[test]
fn a_scene_without_taps_allocates_no_tap_targets() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let mut mixer = new_mixer(&ctx);
    let red = Deck::new_solid_color(&ctx, [1.0, 0.0, 0.0, 1.0], W, H).expect("solid deck");
    mixer.channel_mut(0).unwrap().add_deck(red);

    render_frame_with_taps(&ctx, &mut mixer);
    assert!(
        mixer.has_no_tap_targets(),
        "a scene with no tap decks must not allocate any tap render target"
    );

    // ...and adding one, then removing it, gives the memory back.
    let ch0_uuid = mixer.channel(0).unwrap().uuid().to_string();
    let tap = Deck::new_from_tap(&ctx, varda::deck::TapSource::Channel(ch0_uuid), "tap", W, H)
        .expect("tap deck");
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
