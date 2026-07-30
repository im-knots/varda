//! TEMPORARY DIAGNOSTIC — not part of the permanent suite.
//!
//! Isolates two candidate causes of the reported raymarcher FPS drop:
//!   1. render-target format cost for a fragment-heavy shader (8-bit vs 16F)
//!   2. the per-frame preview gamma-encode cost
//!
//! Both render at 1080p, GPU-timed by submitting and blocking on completion so
//! the number is GPU drain time, not CPU encode time.

use criterion::{criterion_group, criterion_main, Criterion};
use varda::isf::{compile_glsl_to_spirv, ISFShader};
use varda::params::ShaderParams;
use varda::renderer::{context::GpuContext, BlitPipeline, ISFUniforms, UnifiedPipeline};

const W: u32 = 1920;
const H: u32 = 1080;
const RAYMARCH: &str = include_str!("../shaders/apollonian_glow.fs");

fn ctx() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

fn target(
    ctx: &GpuContext,
    fmt: wgpu::TextureFormat,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let t = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}

fn drain(ctx: &GpuContext) {
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
}

/// Render the raymarcher once into `view` and block until the GPU is done.
fn run_shader(
    ctx: &GpuContext,
    pipe: &UnifiedPipeline,
    bg: &wgpu::BindGroup,
    view: &wgpu::TextureView,
) {
    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
        p.set_pipeline(&pipe.pipeline);
        p.set_bind_group(0, bg, &[]);
        p.draw(0..3, 0..1);
    }
    ctx.queue.submit(std::iter::once(enc.finish()));
    drain(ctx);
}

fn bench_target_format(c: &mut Criterion) {
    let Some(ctx) = ctx() else { return };
    let shader = ISFShader::from_string(RAYMARCH).expect("parse");
    let spirv = compile_glsl_to_spirv(&shader.fragment_source, "apollonian_glow").expect("spirv");
    let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);

    let mut group = c.benchmark_group("raymarch_target_format");
    group.sample_size(20);
    for (label, fmt) in [
        ("rgba8unorm", wgpu::TextureFormat::Rgba8Unorm),
        ("rgba16float", wgpu::TextureFormat::Rgba16Float),
    ] {
        let pipe =
            UnifiedPipeline::new(&ctx.device, &spirv, fmt, false, 0, 0, 0).expect("pipeline");
        let mut params = ShaderParams::from_inputs(inputs);
        params.ensure_buffer(&ctx.device);
        params.update_buffer(&ctx.queue);
        let bg = pipe.create_bind_group(&ctx.device, None, &[], &[], &[], params.buffer());
        let (_t, v) = target(&ctx, fmt, W, H);
        let u = ISFUniforms {
            render_size: [W as f32, H as f32],
            ..Default::default()
        };
        pipe.update_uniforms(&ctx.queue, &u);
        // Warm up shader caches before timing.
        run_shader(&ctx, &pipe, &bg, &v);
        group.bench_function(label, |b| b.iter(|| run_shader(&ctx, &pipe, &bg, &v)));
    }
    group.finish();
}

fn bench_preview_encode(c: &mut Criterion) {
    let Some(ctx) = ctx() else { return };
    // Sources are 1080p linear float, as the deck/composite textures are.
    let sources: Vec<_> = (0..14)
        .map(|_| target(&ctx, wgpu::TextureFormat::Rgba16Float, W, H))
        .collect();
    let pipe = BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgba8Unorm).expect("blit");
    pipe.set_srgb_encode(&ctx.queue, true);

    let mut group = c.benchmark_group("preview_encode");
    group.sample_size(20);
    // 960 long edge is what PreviewEncoder::MAX_DIM caps to today.
    for (label, pw, ph) in [("960x540", 960u32, 540u32), ("256x144", 256, 144)] {
        let targets: Vec<_> = (0..14)
            .map(|_| target(&ctx, wgpu::TextureFormat::Rgba8Unorm, pw, ph))
            .collect();
        for n in [1usize, 14] {
            group.bench_function(format!("{label}/{n}_previews"), |b| {
                b.iter(|| {
                    let mut enc = ctx
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                    for i in 0..n {
                        let bg = pipe.create_bind_group(&ctx.device, &sources[i].1);
                        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &targets[i].1,
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
                        pipe.render(&mut p, &bg);
                    }
                    ctx.queue.submit(std::iter::once(enc.finish()));
                    drain(&ctx);
                })
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_target_format, bench_preview_encode);
criterion_main!(benches);
