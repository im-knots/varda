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
