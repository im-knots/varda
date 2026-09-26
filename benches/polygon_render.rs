//! Polygon surface render hot path (issue #42).
//!
//! `PolygonBlitPipeline` runs once per surface, per output, per frame — and the
//! default fullscreen output is itself modeled as a single quad surface, so this
//! path is always live. Each draw allocated a fresh uniform params buffer
//! (`create_bind_group`) and a fresh vertex buffer (`triangulate`). On low-VRAM
//! Metal devices these transient buffers accumulate faster than the driver
//! reclaims them, producing the 60→3 FPS cliff reported on Intel Macs.
//!
//! Three groups isolate the cost:
//!   `polygon_surface_prepare`   — per-surface triangulate + `create_bind_group` only.
//!                               This is where the per-frame allocations live, so
//!                               the ring-buffer fix should move the needle here.
//!   `polygon_surface_render`    — full encode → submit → poll for N surfaces, with
//!                               a poll(Wait) every frame. Because it drains the
//!                               GPU each iteration, frames never pipeline, so this
//!                               group structurally CANNOT see a cross-frame
//!                               write-after-read (WAR) hazard on the persistent
//!                               vertex/param pools.
//!   `polygon_surface_pipelined` — submits several frames back-to-back and polls
//!                               ONCE at the end. This reproduces the real
//!                               renderer's CPU-ahead-of-GPU pipelining, so a
//!                               persistent single-buffer pool that is rewritten
//!                               at offset 0 every frame exposes its WAR stall
//!                               here (the regression triple-buffering fixes).
//!
//! Two more groups measure what /spec/performance-hot-paths.md item E would
//! cache, per surface per output per frame:
//!   `surface_geometry`          — the CPU geometry each warp mode rebuilds:
//!                               a 12-point polygon's ear clip, a corner pin's
//!                               homography and clip, and a Bézier cage's
//!                               tessellation at the default six steps.
//!   `bind_group_create`         — one per-frame bind group, the composite
//!                               ring's shape.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use varda::renderer::{
    blit::{PolygonBlitPipeline, PolygonDrawDesc},
    context::GpuContext,
    edge_blend::SurfaceOverlapZones,
};

const W: u32 = 1920;
const H: u32 = 1080;

/// Surface counts: 1 is the fullscreen/default path; higher counts model
/// multi-projector / multi-surface stages where the leak compounds.
const SURFACE_COUNTS: [usize; 4] = [1, 4, 16, 64];

/// Unit quad in normalized canvas space [0..1] — the fullscreen default surface.
const QUAD: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

fn make_context() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

fn poll(ctx: &GpuContext) {
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
}

fn bench_prepare(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let content = ctx.create_render_texture(W, H);
    let content_view = content.create_view(&wgpu::TextureViewDescriptor::default());
    let pipeline = PolygonBlitPipeline::new(&ctx.device, ctx.texture_format).expect("pipeline");
    let zones = SurfaceOverlapZones::default();

    let mut group = c.benchmark_group("polygon_surface_prepare");
    group.sample_size(50);
    for n in SURFACE_COUNTS {
        group.bench_with_input(BenchmarkId::new("surfaces", n), &n, |b, &n| {
            b.iter(|| {
                let draws: Vec<PolygonDrawDesc<'_>> = (0..n)
                    .map(|_| PolygonDrawDesc {
                        content_view: &content_view,
                        uv_scale: [1.0, 1.0],
                        uv_offset: [0.0, 0.0],
                        homography: None,
                        overlap_zones: &zones,
                        vertices: PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 1.0, 1.0),
                        mask_uuid: "",
                        mask_uv_contours: Vec::new(),
                    })
                    .collect();
                let prepared = pipeline.prepare(&ctx.device, &ctx.queue, &draws);
                criterion::black_box(&prepared);
            });
        });
    }
    group.finish();
}

fn bench_render(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let content = ctx.create_render_texture(W, H);
    let content_view = content.create_view(&wgpu::TextureViewDescriptor::default());
    let target = ctx.create_render_texture(W, H);
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let pipeline = PolygonBlitPipeline::new(&ctx.device, ctx.texture_format).expect("pipeline");
    let zones = SurfaceOverlapZones::default();

    let mut group = c.benchmark_group("polygon_surface_render");
    group.sample_size(50);
    for n in SURFACE_COUNTS {
        group.bench_with_input(BenchmarkId::new("surfaces", n), &n, |b, &n| {
            b.iter(|| {
                let mut encoder =
                    ctx.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("polygon bench encoder"),
                        });
                let draws: Vec<PolygonDrawDesc<'_>> = (0..n)
                    .map(|_| PolygonDrawDesc {
                        content_view: &content_view,
                        uv_scale: [1.0, 1.0],
                        uv_offset: [0.0, 0.0],
                        homography: None,
                        overlap_zones: &zones,
                        vertices: PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 1.0, 1.0),
                        mask_uuid: "",
                        mask_uv_contours: Vec::new(),
                    })
                    .collect();
                let (prepared, vertex_pool) = pipeline.prepare(&ctx.device, &ctx.queue, &draws);
                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("polygon bench pass"),
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
                    pipeline.draw(&mut rp, &prepared, &vertex_pool);
                }
                ctx.queue.submit(std::iter::once(encoder.finish()));
                poll(&ctx);
            });
        });
    }
    group.finish();
}

/// Frames submitted back-to-back before a single poll. Models the renderer's
/// CPU running ahead of the GPU; deep enough that a persistent single-buffer
/// pool's write-after-read stall compounds across frames.
const PIPELINE_DEPTH: usize = 4;

/// Pipelined surface render: submit `PIPELINE_DEPTH` frames before one poll.
///
/// Unlike `bench_render` (which polls every frame and so serializes CPU↔GPU),
/// this keeps several frames in flight. A persistent vertex/param pool written
/// at offset 0 every frame forces frame N+1's `queue.write_buffer` to wait on
/// frame N's draw still reading the same bytes — the WAR hazard that surfaces as
/// periodic stutter in the live renderer. Triple-buffering the pools removes it.
fn bench_render_pipelined(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let content = ctx.create_render_texture(W, H);
    let content_view = content.create_view(&wgpu::TextureViewDescriptor::default());
    let target = ctx.create_render_texture(W, H);
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let pipeline = PolygonBlitPipeline::new(&ctx.device, ctx.texture_format).expect("pipeline");
    let zones = SurfaceOverlapZones::default();

    let mut group = c.benchmark_group("polygon_surface_pipelined");
    group.sample_size(50);
    for n in SURFACE_COUNTS {
        group.bench_with_input(BenchmarkId::new("surfaces", n), &n, |b, &n| {
            b.iter(|| {
                // Submit several frames before polling, so frames overlap and
                // the cross-frame buffer hazard (if any) is exercised.
                for _ in 0..PIPELINE_DEPTH {
                    let mut encoder =
                        ctx.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("polygon pipelined encoder"),
                            });
                    let draws: Vec<PolygonDrawDesc<'_>> = (0..n)
                        .map(|_| PolygonDrawDesc {
                            content_view: &content_view,
                            uv_scale: [1.0, 1.0],
                            uv_offset: [0.0, 0.0],
                            homography: None,
                            overlap_zones: &zones,
                            vertices: PolygonBlitPipeline::triangulate_verts(
                                &QUAD, 0.0, 0.0, 1.0, 1.0,
                            ),
                            mask_uuid: "",
                            mask_uv_contours: Vec::new(),
                        })
                        .collect();
                    let (prepared, vertex_pool) = pipeline.prepare(&ctx.device, &ctx.queue, &draws);
                    {
                        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("polygon pipelined pass"),
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
                        pipeline.draw(&mut rp, &prepared, &vertex_pool);
                    }
                    ctx.queue.submit(std::iter::once(encoder.finish()));
                }
                poll(&ctx);
            });
        });
    }
    group.finish();
}

/// A 12-point star outline in canvas space, so ear clipping has real work.
fn star() -> Vec<[f32; 2]> {
    (0..12)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 12.0;
            let r = if i % 2 == 0 { 0.45 } else { 0.25 };
            [0.5 + r * a.cos(), 0.5 + r * a.sin()]
        })
        .collect()
}

fn bench_surface_geometry(c: &mut Criterion) {
    use varda::surface::warp::{
        BezierWarp, DEFAULT_BEZIER_TESS, WarpMesh, compute_forward_homography,
    };

    let star = star();
    let pinned = [[0.05, 0.02], [0.97, 0.0], [1.0, 0.95], [0.0, 1.0]];
    let mut group = c.benchmark_group("surface_geometry");
    group.bench_function("polygon_12", |b| {
        b.iter(|| {
            PolygonBlitPipeline::triangulate_verts(criterion::black_box(&star), 0.0, 0.0, 1.0, 1.0)
        });
    });
    group.bench_function("corner_pin", |b| {
        b.iter(|| {
            let h = compute_forward_homography(criterion::black_box(&QUAD), &pinned);
            (
                h,
                PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 1.0, 1.0),
            )
        });
    });
    for anchors in [4u32, 8] {
        let cage =
            BezierWarp::from_mesh(&WarpMesh::identity(anchors, anchors), DEFAULT_BEZIER_TESS);
        group.bench_with_input(BenchmarkId::new("bezier", anchors), &cage, |b, cage| {
            b.iter(|| PolygonBlitPipeline::mesh_verts(&criterion::black_box(cage).tessellate()));
        });
    }
    group.finish();
}

fn bench_bind_group(c: &mut Criterion) {
    let Some(ctx) = make_context() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let layer = ctx.create_render_texture(W, H);
    let layer_view = layer.create_view(&wgpu::TextureViewDescriptor::default());
    let below = ctx.create_render_texture(W, H);
    let below_view = below.create_view(&wgpu::TextureViewDescriptor::default());
    let composite =
        varda::renderer::blit::CompositeBlitPipeline::new(&ctx.device, ctx.compositing_format)
            .expect("pipeline");
    let mut group = c.benchmark_group("bind_group_create");
    group.bench_function("composite_ring", |b| {
        b.iter(|| composite.create_ring_bind_group(&ctx.device, &layer_view, &below_view, 0));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_prepare,
    bench_render,
    bench_render_pipelined,
    bench_surface_geometry,
    bench_bind_group
);
criterion_main!(benches);
