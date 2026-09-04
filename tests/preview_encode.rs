//! The preview encode path: linear-light engine textures → gamma-encoded
//! thumbnails for egui.
//!
//! egui assumes any texture it is handed is already gamma-encoded ("We expect
//! 'normal' textures that are NOT sRGB-aware" — egui-wgpu's `egui.wgsl`) and
//! applies the inverse transfer function before writing to its sRGB
//! framebuffer. Handing it a linear texture round-trips to raw linear light on
//! screen, so previews read far darker than the output window showing the same
//! frame. `BlitPipeline::set_srgb_encode` is the fix; this pins its numerics.
//!
//! Skips cleanly with no GPU adapter.

use std::sync::mpsc;
use varda::renderer::{BlitPipeline, COLOR_PATH_FORMAT, context::GpuContext};

const W: u32 = 16;
const H: u32 = 16;

mod common;
use common::headless_gpu;

/// Reference linear → sRGB (IEC 61966-2-1), to check the shader against.
fn srgb_from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Blit a solid linear colour through the encoder and return the target's
/// centre pixel as 0–255 bytes.
fn encode_solid(ctx: &GpuContext, linear: [f64; 3], encode: bool) -> [u8; 4] {
    // Source: linear-light, same format the color path uses.
    let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("src"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_PATH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());

    // Target: plain Rgba8Unorm. Deliberately NOT *UnormSrgb — the hardware would
    // decode on sample and cancel the encode out.
    let dst = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dst"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let dst_view = dst.create_view(&wgpu::TextureViewDescriptor::default());

    let pipeline = BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgba8Unorm).expect("blit");
    pipeline.set_srgb_encode(&ctx.queue, encode);
    let bg = pipeline.create_bind_group(&ctx.device, &src_view);

    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    // Fill the source by clearing to the linear value.
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("fill src"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &src_view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: linear[0],
                    g: linear[1],
                    b: linear[2],
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("encode"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dst_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pipeline.render(&mut pass, &bg);
    }

    let padded =
        (W * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * H),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &dst,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
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
    ctx.queue.submit(std::iter::once(enc.finish()));

    let (tx, rx) = mpsc::channel();
    buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
    rx.recv().expect("map channel").expect("map ok");
    let out;
    {
        let d = buf
            .slice(..)
            .get_mapped_range()
            .expect("preview readback must be mapped");
        let b = ((H / 2) * padded) as usize + (W / 2) as usize * 4;
        out = [d[b], d[b + 1], d[b + 2], d[b + 3]];
    }
    buf.unmap();
    out
}

/// With the encode on, a linear value must come out sRGB-encoded — which is what
/// the output window shows and therefore what previews must show too.
#[test]
fn encode_applies_the_srgb_transfer_function() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    // Mid and deep-shadow values: the shadows are where the mismatch was worst.
    for linear in [0.2_f32, 0.05, 0.5] {
        let px = encode_solid(&ctx, [f64::from(linear); 3], true);
        let want = (srgb_from_linear(linear) * 255.0).round();
        for (i, ch) in px[..3].iter().enumerate() {
            assert!(
                (f32::from(*ch) - want).abs() <= 2.0,
                "linear {linear}: channel {i} encoded to {ch}, expected ~{want}"
            );
        }
    }
}

/// With the encode off the blit is a passthrough — this is what previews used to
/// get, and it is visibly darker. Pins the difference so a regression that drops
/// the flag cannot pass silently.
#[test]
fn without_encode_the_raw_linear_value_passes_through() {
    let Some(ctx) = headless_gpu() else {
        return;
    };
    let linear = 0.2_f32;
    let raw = encode_solid(&ctx, [f64::from(linear); 3], false);
    let encoded = encode_solid(&ctx, [f64::from(linear); 3], true);

    assert!(
        (f32::from(raw[0]) - linear * 255.0).abs() <= 2.0,
        "passthrough should be the raw linear value, got {}",
        raw[0]
    );
    // linear 0.2 → 51/255 raw vs ~124/255 encoded. If these ever converge the
    // encode has stopped doing anything.
    assert!(
        i32::from(encoded[0]) - i32::from(raw[0]) > 40,
        "encoded ({}) should be much brighter than raw ({})",
        encoded[0],
        raw[0]
    );
}
