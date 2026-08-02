//! Scratch harness: render `warped_grid.fs` frames to PNG for visual comparison.
//!
//! Renders the working-tree shader alongside the committed one from HEAD and
//! writes both plus an amplified absolute difference, so a change to the
//! massing or the facade can be eyeballed and its speckle measured.

use std::sync::mpsc;

use varda::{
    audio::AudioData,
    deck::Deck,
    mixer::Mixer,
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
    renderer::tonemap::TonemapMode,
};

const W: u32 = 960;
const H: u32 = 540;
const SPEED: f32 = 3.0;
/// Frames to render before capturing, so the camera is well into the field.
const CAPTURE_AT: &[usize] = &[30, 200, 500, 900];

fn read_back(ctx: &GpuContext, mixer: &Mixer) -> Vec<[f32; 4]> {
    let bytes_per_pixel = 8u32;
    let padded = (W * bytes_per_pixel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frames readback"),
        size: u64::from(padded * H),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: mixer.composite_texture(),
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
    rx.recv().unwrap().unwrap();
    let data = buffer.slice(..).get_mapped_range();
    let mut out = Vec::with_capacity((W * H) as usize);
    for row in 0..H {
        let base = (row * padded) as usize;
        for col in 0..W {
            let px = base + (col * bytes_per_pixel) as usize;
            let ch = |i: usize| {
                half::f16::from_bits(u16::from_le_bytes([data[px + i * 2], data[px + i * 2 + 1]]))
                    .to_f32()
            };
            out.push([ch(0), ch(1), ch(2), ch(3)]);
        }
    }
    out
}

fn encode(v: f32) -> u8 {
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055
    };
    (s.clamp(0.0, 1.0) * 255.0) as u8
}

fn render_series(ctx: &GpuContext, source: &str, overrides: &[(&str, f32)]) -> Vec<Vec<[f32; 4]>> {
    let shader = varda::isf::ISFShader::from_string(source).expect("parse");
    let mut mixer = Mixer::new(ctx, W, H).expect("mixer");
    mixer.set_tonemap_mode(&ctx.queue, TonemapMode::Bypass);
    let mut deck = Deck::new(ctx, shader, W, H).expect("deck");
    deck.generator_params.set_float("speed", SPEED);
    for (name, value) in overrides {
        deck.generator_params.set_float(name, *value);
    }
    let ch = mixer.channel_mut(0).unwrap();
    ch.add_deck(deck);
    // Pin the deck to the target rate. Left on Auto, the channel skips frames
    // for a deck that overruns its budget share, which both hides the true cost
    // and makes the phase accumulator advance by a load-dependent amount.
    ch.decks[0].render_fps = varda::channel::DeckRenderFps::Fixed(60);

    let mut frames = Vec::new();
    let last = *CAPTURE_AT.last().unwrap();
    for frame in 1..=last {
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
        if CAPTURE_AT.contains(&frame) {
            frames.push(read_back(ctx, &mixer));
        }
    }
    frames
}

fn save(path: &str, rgb: &[u8]) {
    image::save_buffer(path, rgb, W, H, image::ColorType::Rgb8).expect("write png");
}

/// Fraction of pixels that are salt-and-pepper outliers: luminance far from the
/// mean of their four neighbours. Over-stepping a raymarch tunnels individual
/// rays through geometry, which shows up as isolated wrong pixels rather than
/// as a smooth shift, so this is the metric that decides whether a larger
/// stride is safe.
fn speckle(frame: &[[f32; 4]]) -> f32 {
    let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let at = |x: usize, y: usize| lum(&frame[y * W as usize + x]);
    let mut count = 0usize;
    let mut total = 0usize;
    for y in 1..(H as usize - 1) {
        for x in 1..(W as usize - 1) {
            let c = at(x, y);
            let n = (at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1)) * 0.25;
            // Relative, so bright glow regions are not counted as speckle just
            // for being bright.
            if (c - n).abs() > 0.35 * (n.abs() + 0.05) {
                count += 1;
            }
            total += 1;
        }
    }
    count as f32 / total as f32
}

fn main() {
    let ctx = GpuContext::new_headless().expect("headless GPU");
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/warped_grid.fs"),
    )
    .expect("read warped_grid.fs");

    let reference = String::from_utf8(
        std::process::Command::new("git")
            .args(["show", "HEAD:shaders/warped_grid.fs"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("git show")
            .stdout,
    )
    .expect("HEAD shader is UTF-8");
    assert!(!reference.is_empty(), "HEAD has no warped_grid.fs");

    // Control: the same source twice must be bit-identical, or a diff against
    // the reference means nothing.
    let control_a = render_series(&ctx, &src, &[]);
    let control_b = render_series(&ctx, &src, &[]);
    let control: f64 = control_a
        .iter()
        .flatten()
        .zip(control_b.iter().flatten())
        .map(|(a, b)| f64::from((a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()))
        .sum();
    println!("control (same source twice), total abs diff: {control:.6}");

    let now = control_a;
    let before = render_series(&ctx, &reference, &[]);
    // Windows off, so the silhouette can be judged on its own. Spires off too:
    // a mast a tenth of a cell wide speckles in any raymarcher simply for being
    // thinner than the step, and that has to be separated from speckle caused
    // by the massing field overestimating distance.
    let bare = render_series(
        &ctx,
        &src,
        &[
            ("window_amount", 0.0),
            ("window_glow", 0.0),
            ("spire_amount", 0.0),
        ],
    );

    std::fs::create_dir_all("/tmp/warped_grid").unwrap();
    for (i, frame) in CAPTURE_AT.iter().enumerate() {
        let mut a = Vec::with_capacity((W * H * 3) as usize);
        let mut b = Vec::with_capacity((W * H * 3) as usize);
        let mut diff = Vec::with_capacity((W * H * 3) as usize);
        let mut worst = 0.0f32;
        let mut sum = 0.0f64;
        for (pa, pb) in now[i].iter().zip(before[i].iter()) {
            for c in 0..3 {
                a.push(encode(pa[c]));
                b.push(encode(pb[c]));
                let d = (pa[c] - pb[c]).abs();
                worst = worst.max(d);
                sum += f64::from(d);
                diff.push(encode(d * 8.0));
            }
        }
        let mut c = Vec::with_capacity((W * H * 3) as usize);
        for px in &bare[i] {
            for ch in px.iter().take(3) {
                c.push(encode(*ch));
            }
        }
        save(&format!("/tmp/warped_grid/f{frame}_new.png"), &a);
        save(&format!("/tmp/warped_grid/f{frame}_old.png"), &b);
        save(&format!("/tmp/warped_grid/f{frame}_bare.png"), &c);
        save(&format!("/tmp/warped_grid/f{frame}_diff.png"), &diff);
        // Bare speckle is the one that matters for stride safety: window panes
        // are legitimate high-frequency detail and score as speckle themselves,
        // so only the windows-off render can say whether rays are tunnelling.
        println!(
            "frame {frame}: mean abs diff {:.5}, worst {:.4} | speckle new {:.4}% bare {:.4}% old {:.4}%",
            sum / f64::from(W * H * 3),
            worst,
            speckle(&now[i]) * 100.0,
            speckle(&bare[i]) * 100.0,
            speckle(&before[i]) * 100.0,
        );
    }
    println!("wrote /tmp/warped_grid/");
}
