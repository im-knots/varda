//! Depth-sensor shader preprocessor GPU cost (spec/depth-sensor-preprocessor.md).
//!
//! Three fullscreen passes at the sensor's native resolution convert the shared
//! `R16Uint` depth stream into the `depth`/`mask`/`motion`/`rgb` fields an ISF
//! shader consumes. This is new per-frame GPU work on every deck that declares
//! the preprocessor, so it needs a number rather than an assurance.
//!
//! Two groups:
//!   `depth_preprocess_passes` — one full conversion at VGA and at QVGA, drained
//!                             each iteration. This is the per-sensor-frame cost.
//!   `depth_preprocess_decks`  — N decks each running their own conversion, since
//!                             the spec chose per-deck pipelines (so each deck
//!                             gets its own near/far framing) over one shared
//!                             conversion per device. This group is what would
//!                             justify revisiting that call.
//!
//! Note the real per-*render*-frame cost is lower than these numbers suggest:
//! the passes are gated on the sensor's upload counter, so a 30 Hz Kinect
//! driving a 60 Hz deck runs them on half the frames.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::depth::preprocess::{DepthPreprocessParams, DepthPreprocessPipeline};
use varda::renderer::context::GpuContext;

/// Kinect v1 native resolution, and a half-size case for LIDAR-class sensors.
const RESOLUTIONS: [(u32, u32); 2] = [(640, 480), (320, 240)];

/// Deck counts: 1 is the common single-shader case; higher counts model a set
/// where several decks each frame the same sensor differently.
const DECK_COUNTS: [usize; 3] = [1, 2, 4];

fn make_context() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

/// A depth texture shaped like the one `DepthSensorManager` owns, filled with a
/// ramp so hole-fill and gradient work are exercised rather than short-circuited
/// on a uniform image.
fn make_depth_source(gpu: &GpuContext, width: u32, height: u32) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench depth src"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R16Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // Ramp with a scatter of unresolved texels, matching real Kinect output:
    // roughly one in eight samples is a hole the fill pass must search around.
    let data: Vec<u16> = (0..width * height)
        .map(|i| {
            if i % 8 == 0 {
                0
            } else {
                1000 + (i % 3000) as u16
            }
        })
        .collect();
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 2),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn drain(gpu: &GpuContext) {
    let _ = gpu.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(5)),
    });
}

fn bench_passes(c: &mut Criterion) {
    let Some(gpu) = make_context() else {
        eprintln!("skipping depth_preprocess bench: no headless GPU adapter");
        return;
    };
    let params = DepthPreprocessParams::default();
    let mut group = c.benchmark_group("depth_preprocess_passes");

    for (width, height) in RESOLUTIONS {
        let src = make_depth_source(&gpu, width, height);
        let mut pipeline = DepthPreprocessPipeline::new(&gpu.device, width, height);
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{width}x{height}")),
            &(width, height),
            |b, _| {
                b.iter(|| {
                    pipeline.update_uniform(&gpu.queue, &params, 1.0 / 30.0);
                    let mut cmds = Vec::new();
                    pipeline.run(&gpu.device, &src, None, &mut cmds);
                    gpu.queue.submit(cmds);
                    drain(&gpu);
                });
            },
        );
    }
    group.finish();
}

fn bench_decks(c: &mut Criterion) {
    let Some(gpu) = make_context() else {
        return;
    };
    let params = DepthPreprocessParams::default();
    let (width, height) = RESOLUTIONS[0];
    let src = make_depth_source(&gpu, width, height);
    let mut group = c.benchmark_group("depth_preprocess_decks");

    for count in DECK_COUNTS {
        let mut pipelines: Vec<_> = (0..count)
            .map(|_| DepthPreprocessPipeline::new(&gpu.device, width, height))
            .collect();
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, _| {
            b.iter(|| {
                let mut cmds = Vec::new();
                for pipeline in &mut pipelines {
                    pipeline.update_uniform(&gpu.queue, &params, 1.0 / 30.0);
                    pipeline.run(&gpu.device, &src, None, &mut cmds);
                }
                gpu.queue.submit(cmds);
                drain(&gpu);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_passes, bench_decks);
criterion_main!(benches);
