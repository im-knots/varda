//! Final SDR presentation-pass benchmarks for eight-bit and RGB10A2 outputs.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use varda::engine::value::render::{
    AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
    ResolvedPresentation,
};
use varda::renderer::blit::BlitPipeline;
use varda::renderer::context::GpuContext;

fn presentation(
    depth: PresentationDepth,
    pixel_format: PresentationPixelFormat,
) -> ResolvedPresentation {
    ResolvedPresentation {
        requested: depth,
        resolved: depth,
        pixel_format,
        color_profile: PresentationColorProfile::SrgbFull,
        alpha_mode: AlphaMode::Opaque,
        dither: true,
        fallback_reason: None,
    }
}

fn bench_presentation(c: &mut Criterion) {
    let Ok(context) = GpuContext::new_headless() else {
        eprintln!("no GPU adapter, skipping presentation benchmarks");
        return;
    };
    let cases = [
        (
            "sdr8",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            presentation(PresentationDepth::Sdr8, PresentationPixelFormat::Rgba8),
        ),
        (
            "sdr10",
            wgpu::TextureFormat::Rgb10a2Unorm,
            presentation(PresentationDepth::Sdr10, PresentationPixelFormat::Rgb10A2),
        ),
    ];
    let mut group = c.benchmark_group("sdr_presentation");
    group.sample_size(30);

    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let source = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Presentation Benchmark Source"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());

        for (name, format, resolved) in &cases {
            let Ok(pipeline) = BlitPipeline::new(&context.device, *format) else {
                eprintln!("{format:?} unsupported, skipping {name}");
                continue;
            };
            let target = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Presentation Benchmark Target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: *format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = pipeline.create_bind_group(&context.device, &source_view);
            pipeline.set_presentation(&context.queue, 0, resolved);

            group.bench_with_input(
                BenchmarkId::new(format!("{width}x{height}"), name),
                &(),
                |bencher, ()| {
                    bencher.iter(|| {
                        let mut encoder = context.device.create_command_encoder(
                            &wgpu::CommandEncoderDescriptor {
                                label: Some("Presentation Benchmark Encoder"),
                            },
                        );
                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Presentation Benchmark Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &target_view,
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
                            pipeline.render(&mut pass, &bind_group);
                        }
                        context.queue.submit([encoder.finish()]);
                        let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_presentation);
criterion_main!(benches);
