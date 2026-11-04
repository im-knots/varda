//! Fixed GPU and CPU costs that every deck pays, apart from its pixels.
//!
//!   `pass_overhead`      — 32 full-frame operations per iteration at 1080p and
//!                          4K, wall time including the GPU. Divide by 32.
//!                          `clear` is a pass that only clears, `draw` a pass
//!                          that reads a full-frame texture into the target,
//!                          `own_encoders` the same draws with one command
//!                          encoder each, `one_pass` all 32 draws in a single
//!                          pass with hardware blending.
//!                          `thumb/clear` clears 32 distinct 256x144 targets,
//!                          a preview's size. `distinct` draws into 32
//!                          distinct full-frame targets, so no pass waits on
//!                          another's target.
//!                          `thumb/clear_cpu` times only encoding and submitting
//!                          those 32 passes; the GPU is drained untimed.
//!   `uniform_upload`     — render-thread time to stage 32 uniform writes of
//!                          256 bytes and submit: one `write_buffer` each, one
//!                          coalesced `write_buffer`, or a `StagingBelt`.
//!   `channel_render_cpu` — render-thread time of a mixer frame with N solid or
//!                          shader decks in one channel. The GPU is drained
//!                          untimed between iterations.
//!   `channel_render_gpu` — 8 mixer frames submitted back to back, then one
//!                          wait, divided by 8. When the GPU is the slower side
//!                          this is GPU time per frame.

use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::{
    audio::AudioData,
    deck::Deck,
    isf::ISFShader,
    mixer::{FrameInputs, Mixer},
    modulation::{AnalyzerValues, AudioValues},
    renderer::context::GpuContext,
};

const OPS: u32 = 32;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SIZES: [(&str, u32, u32); 2] = [("1080p", 1920, 1080), ("4K", 3840, 2160)];
const BARS_SHADER: &str = include_str!("../shaders/bars.fs");

const SHADER: &str = r"
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(source, vec2<i32>(p.xy), 0) * 0.5;
}
";

fn wait(ctx: &GpuContext) {
    let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
}

fn texture(ctx: &GpuContext, width: u32, height: u32) -> wgpu::TextureView {
    ctx.device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("frame overhead"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

struct Draw {
    pipeline: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl Draw {
    fn new(ctx: &GpuContext, source: &wgpu::TextureView) -> Self {
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline = |blend| {
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: None,
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: FORMAT,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        });
        Self {
            pipeline: pipeline(None),
            blended: pipeline(Some(wgpu::BlendState::ALPHA_BLENDING)),
            bind_group,
        }
    }
}

fn pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn encoder(ctx: &GpuContext) -> wgpu::CommandEncoder {
    ctx.device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default())
}

fn pass_overhead(c: &mut Criterion) {
    let Ok(ctx) = GpuContext::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut group = c.benchmark_group("pass_overhead");
    group.sample_size(30);
    for (label, width, height) in SIZES {
        let source = texture(&ctx, width, height);
        let targets = [texture(&ctx, width, height), texture(&ctx, width, height)];
        let draw = Draw::new(&ctx, &source);
        let target = |i: u32| &targets[(i % 2) as usize];

        group.bench_function(BenchmarkId::new(label, "clear"), |b| {
            b.iter(|| {
                let mut e = encoder(&ctx);
                for i in 0..OPS {
                    drop(pass(&mut e, target(i)));
                }
                ctx.queue.submit([e.finish()]);
                wait(&ctx);
            });
        });
        group.bench_function(BenchmarkId::new(label, "draw"), |b| {
            b.iter(|| {
                let mut e = encoder(&ctx);
                for i in 0..OPS {
                    let mut p = pass(&mut e, target(i));
                    p.set_pipeline(&draw.pipeline);
                    p.set_bind_group(0, &draw.bind_group, &[]);
                    p.draw(0..3, 0..1);
                }
                ctx.queue.submit([e.finish()]);
                wait(&ctx);
            });
        });
        group.bench_function(BenchmarkId::new(label, "own_encoders"), |b| {
            b.iter(|| {
                let buffers: Vec<_> = (0..OPS)
                    .map(|i| {
                        let mut e = encoder(&ctx);
                        {
                            let mut p = pass(&mut e, target(i));
                            p.set_pipeline(&draw.pipeline);
                            p.set_bind_group(0, &draw.bind_group, &[]);
                            p.draw(0..3, 0..1);
                        }
                        e.finish()
                    })
                    .collect();
                ctx.queue.submit(buffers);
                wait(&ctx);
            });
        });
        group.bench_function(BenchmarkId::new(label, "one_pass"), |b| {
            b.iter(|| {
                let mut e = encoder(&ctx);
                {
                    let mut p = pass(&mut e, target(0));
                    p.set_pipeline(&draw.blended);
                    p.set_bind_group(0, &draw.bind_group, &[]);
                    for _ in 0..OPS {
                        p.draw(0..3, 0..1);
                    }
                }
                ctx.queue.submit([e.finish()]);
                wait(&ctx);
            });
        });
    }
    let thumbs: Vec<_> = (0..OPS).map(|_| texture(&ctx, 256, 144)).collect();
    group.bench_function(BenchmarkId::new("thumb", "clear"), |b| {
        b.iter(|| {
            let mut e = encoder(&ctx);
            for t in &thumbs {
                drop(pass(&mut e, t));
            }
            ctx.queue.submit([e.finish()]);
            wait(&ctx);
        });
    });
    group.bench_function(BenchmarkId::new("thumb", "clear_cpu"), |b| {
        b.iter_custom(|iterations| {
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                let start = Instant::now();
                let mut e = encoder(&ctx);
                for t in &thumbs {
                    drop(pass(&mut e, t));
                }
                ctx.queue.submit([e.finish()]);
                total += start.elapsed();
                wait(&ctx);
            }
            total
        });
    });
    let source = texture(&ctx, 1920, 1080);
    let draw = Draw::new(&ctx, &source);
    let targets: Vec<_> = (0..OPS).map(|_| texture(&ctx, 1920, 1080)).collect();
    group.bench_function(BenchmarkId::new("1080p", "distinct"), |b| {
        b.iter(|| {
            let mut e = encoder(&ctx);
            for t in &targets {
                let mut p = pass(&mut e, t);
                p.set_pipeline(&draw.pipeline);
                p.set_bind_group(0, &draw.bind_group, &[]);
                p.draw(0..3, 0..1);
            }
            ctx.queue.submit([e.finish()]);
            wait(&ctx);
        });
    });
    group.finish();
}

const UNIFORM_BYTES: u64 = 256;

fn uniform_upload(c: &mut Criterion) {
    let Ok(ctx) = GpuContext::new_headless() else {
        return;
    };
    let ring = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniform ring"),
        size: UNIFORM_BYTES * u64::from(OPS),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let data = vec![7u8; (UNIFORM_BYTES * u64::from(OPS)) as usize];
    let mut belt = wgpu::util::StagingBelt::new(ctx.device.clone(), 64 * 1024);
    let mut group = c.benchmark_group("uniform_upload");
    group.sample_size(50);

    let mut timed = |name: &str, f: &mut dyn FnMut()| {
        group.bench_function(name, |b| {
            b.iter_custom(|iterations| {
                let mut total = Duration::ZERO;
                for _ in 0..iterations {
                    let start = Instant::now();
                    f();
                    total += start.elapsed();
                    wait(&ctx);
                }
                total
            });
        });
    };
    timed("write_buffer_each", &mut || {
        for i in 0..u64::from(OPS) {
            let at = (i * UNIFORM_BYTES) as usize;
            ctx.queue.write_buffer(
                &ring,
                i * UNIFORM_BYTES,
                &data[at..at + UNIFORM_BYTES as usize],
            );
        }
        ctx.queue.submit([encoder(&ctx).finish()]);
    });
    timed("write_buffer_coalesced", &mut || {
        ctx.queue.write_buffer(&ring, 0, &data);
        ctx.queue.submit([encoder(&ctx).finish()]);
    });
    timed("staging_belt", &mut || {
        let mut e = encoder(&ctx);
        let size = NonZeroU64::new(UNIFORM_BYTES).expect("nonzero");
        for i in 0..u64::from(OPS) {
            let at = (i * UNIFORM_BYTES) as usize;
            belt.write_buffer(&mut e, &ring, i * UNIFORM_BYTES, size)
                .copy_from_slice(&data[at..at + UNIFORM_BYTES as usize]);
        }
        belt.finish();
        ctx.queue.submit([e.finish()]);
        belt.recall();
    });
    group.finish();
}

fn mixer_with(ctx: &GpuContext, decks: usize, shader: bool) -> Mixer {
    let mut mixer = Mixer::new(ctx, 1920, 1080).expect("mixer");
    let channel = mixer.channel_mut(0).expect("channel 0");
    for i in 0..decks {
        let deck = if shader {
            let shader = ISFShader::from_string(BARS_SHADER).expect("bars shader");
            Deck::from_shader(ctx, shader, 1920, 1080).expect("shader deck")
        } else {
            let t = i as f32 / decks as f32;
            Deck::solid_color(ctx, [t, 0.5, 1.0 - t, 1.0], 1920, 1080)
        };
        channel.add_deck(deck);
    }
    mixer
}

fn render(ctx: &GpuContext, mixer: &mut Mixer) {
    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: std::collections::HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();
    let inputs = FrameInputs {
        audio_data: &audio,
        audio_values: &audio_values,
        analyzer_values: &analyzer_values,
        beat_time: None,
        transport: None,
        free_run_time: None,
        write_param: varda::param_router::write_macro_target,
    };
    mixer.render(ctx, &inputs, 60, &[]).expect("render");
}

const GPU_FRAMES: u32 = 8;

fn channel_render(c: &mut Criterion) {
    let Ok(ctx) = GpuContext::new_headless() else {
        return;
    };
    for (group_name, gpu) in [("channel_render_cpu", false), ("channel_render_gpu", true)] {
        let mut group = c.benchmark_group(group_name);
        group.sample_size(30);
        for (kind, shader) in [("solid", false), ("shader", true)] {
            for decks in [1, 4, 8] {
                let mut mixer = mixer_with(&ctx, decks, shader);
                for _ in 0..3 {
                    render(&ctx, &mut mixer);
                    wait(&ctx);
                }
                group.bench_function(BenchmarkId::new(kind, decks), |b| {
                    b.iter_custom(|iterations| {
                        let mut total = Duration::ZERO;
                        for _ in 0..iterations {
                            if gpu {
                                let start = Instant::now();
                                for _ in 0..GPU_FRAMES {
                                    render(&ctx, &mut mixer);
                                }
                                wait(&ctx);
                                total += start.elapsed() / GPU_FRAMES;
                            } else {
                                let start = Instant::now();
                                render(&ctx, &mut mixer);
                                total += start.elapsed();
                                wait(&ctx);
                            }
                        }
                        total
                    });
                });
            }
        }
        group.finish();
    }
}

criterion_group!(benches, pass_overhead, uniform_upload, channel_render);
criterion_main!(benches);
