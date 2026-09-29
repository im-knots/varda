//! One output: its picture settings, its sink, and the single path every
//! output composes through.

use super::{Delivered, FramePath, OutputSinkInstance, SinkFrame, SinkQuery, storage_formats};
use crate::engine::value::render::{
    AlphaMode, CalibrationMode, EdgeBlendConfig, EdgeBlendMode, ModeAvailability, OutputRotation,
    PresentationPixelFormat, PresentationRequest, ResolvedPresentation, TonemapMode, Unassigned,
};
use crate::renderer::blit::{BlitPipeline, PolygonBlitPipeline, PolygonDrawDesc};
use crate::renderer::context::{GpuContext, SurfaceAssignment, SurfaceRenderInfo};
use crate::renderer::edge_blend::EdgeBlendPipeline;
use crate::renderer::measure::{ContentLightLevels, ContentLightMeter};
use crate::renderer::{ReadbackBuffer, ReadbackFormat};
use crate::source::Services;
use anyhow::Result;

/// What an output shows this frame.
#[derive(Clone, Copy)]
pub enum Content<'a> {
    /// Surfaces at their canvas positions, each through its warp.
    Surfaces(&'a [SurfaceRenderInfo<'a>]),
    /// One picture over the whole output. `fit_aspect` letterboxes it to that
    /// width-to-height ratio on a window; other sinks stretch it.
    Picture {
        view: &'a wgpu::TextureView,
        fit_aspect: Option<f32>,
    },
}

/// What rendering a frame came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderedFrame {
    /// Nothing was drawn: the sink is not ready or has nothing to draw into.
    Skipped,
    Done,
    /// The sink failed; the output stops with this message.
    Stop(String),
    /// The sink ended its session and must be started again.
    Restart,
}

/// A texture and its default view.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        (width, height): (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

/// GPU resources bound to one presentation and size.
struct Resources {
    /// Surfaces compose here when edge blending runs.
    blend_source: Target,
    /// The composed picture, before the final pass. The UI preview reads it.
    composed: Target,
    /// The delivered picture, for every sink but a window.
    delivered: Option<Target>,
    readback: Option<ReadbackBuffer>,
    polygon: PolygonBlitPipeline,
    edge_blend: EdgeBlendPipeline,
    /// The final pass: rotation, transfer encode, quantization, dither.
    finish: BlitPipeline,
    target_format: wgpu::TextureFormat,
    intermediate_format: wgpu::TextureFormat,
}

/// An output: where one picture of the show goes.
pub struct Output {
    pub uuid: String,
    pub name: String,
    /// Which surfaces this output shows. Empty shows what `unassigned` says.
    pub surface_assignments: Vec<SurfaceAssignment>,
    /// What to show with nothing assigned; `None` takes the sink's default.
    pub unassigned: Option<Unassigned>,
    pub calibration_mode: CalibrationMode,
    pub edge_blend_mode: EdgeBlendMode,
    pub edge_blend: EdgeBlendConfig,
    pub rotation: OutputRotation,
    /// Output transform overriding the mixer's show-wide default when set.
    pub tonemap_override: Option<TonemapMode>,
    presentation_request: PresentationRequest,
    resolved_presentation: ResolvedPresentation,
    mode_availability: Vec<ModeAvailability>,
    /// Whether a startable sink is delivering.
    pub active: bool,
    pub started_at: Option<std::time::Instant>,
    /// The audio passthrough the running session holds.
    pub audio: Option<crate::delivery::AudioPassthrough>,
    /// Measures an HDR delivery while it runs. Boxed because it is rarely
    /// present and large (a pipeline and a reduction chain).
    light_meter: Option<Box<ContentLightMeter>>,
    /// Running `MaxCLL` and `MaxFALL` across the current session.
    pub light_levels: ContentLightLevels,
    sink: Box<dyn OutputSinkInstance>,
    size: (u32, u32),
    resources: Resources,
    /// Whether the last frame went through `composed`, so a preview of it is
    /// meaningful.
    composed_last_frame: bool,
}

impl Output {
    /// A new output around `sink`, sized for `render` and configured for the
    /// default presentation.
    ///
    /// # Errors
    ///
    /// Fails when the sink can deliver nothing or a pipeline cannot be built.
    pub fn new(
        gpu: &GpuContext,
        services: &Services,
        uuid: String,
        name: String,
        mut sink: Box<dyn OutputSinkInstance>,
        render: (u32, u32),
    ) -> Result<Self> {
        let request = PresentationRequest::default();
        let presentation = sink.configure(gpu, &SinkQuery { services }, request)?;
        let size = sink.size(render);
        let resources = Self::build(
            gpu,
            sink.as_ref(),
            &presentation.resolved,
            size,
            OutputRotation::default(),
        )?;
        Ok(Self {
            uuid,
            name,
            surface_assignments: Vec::new(),
            unassigned: None,
            calibration_mode: CalibrationMode::Off,
            edge_blend_mode: EdgeBlendMode::default(),
            edge_blend: EdgeBlendConfig::default(),
            rotation: OutputRotation::default(),
            tonemap_override: None,
            presentation_request: request,
            resolved_presentation: presentation.resolved,
            mode_availability: presentation.modes,
            active: false,
            started_at: None,
            audio: None,
            light_meter: None,
            light_levels: ContentLightLevels::default(),
            sink,
            size,
            resources,
            composed_last_frame: false,
        })
    }

    fn readback_alpha_mode(presentation: &ResolvedPresentation) -> AlphaMode {
        if presentation.pixel_format == PresentationPixelFormat::Rgba16 {
            AlphaMode::Premultiplied
        } else {
            presentation.alpha_mode
        }
    }

    fn build(
        gpu: &GpuContext,
        sink: &dyn OutputSinkInstance,
        resolved: &ResolvedPresentation,
        size: (u32, u32),
        rotation: OutputRotation,
    ) -> Result<Resources> {
        let device = &gpu.device;
        let target_format = sink.target_format(resolved);
        let intermediate_format = sink.intermediate_format(resolved);
        let composed_size = rotation.effective_dimensions(size.0, size.1);
        let intermediate_usage =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let path = sink.frame_path();
        let delivered = (path != FramePath::Present).then(|| {
            Target::new(
                device,
                size,
                target_format,
                intermediate_usage | wgpu::TextureUsages::COPY_SRC,
                "Output Delivered",
            )
        });
        let readback = (path == FramePath::Readback).then(|| {
            ReadbackBuffer::new_with_contract(
                device,
                size.0,
                size.1,
                storage_formats(resolved).1,
                resolved.color_profile,
                Self::readback_alpha_mode(resolved),
            )
        });
        Ok(Resources {
            blend_source: Target::new(
                device,
                composed_size,
                intermediate_format,
                intermediate_usage,
                "Output Blend Source",
            ),
            composed: Target::new(
                device,
                composed_size,
                intermediate_format,
                intermediate_usage,
                "Output Composed",
            ),
            delivered,
            readback,
            polygon: PolygonBlitPipeline::new(device, intermediate_format)?,
            edge_blend: EdgeBlendPipeline::new(device, intermediate_format)?,
            finish: BlitPipeline::new(device, target_format)?,
            target_format,
            intermediate_format,
        })
    }

    /// Rebuild what depends on the presentation, the size or the rotation.
    fn rebuild(&mut self, gpu: &GpuContext) -> Result<()> {
        self.resources = Self::build(
            gpu,
            self.sink.as_ref(),
            &self.resolved_presentation,
            self.size,
            self.rotation,
        )?;
        Ok(())
    }

    pub fn sink(&self) -> &dyn OutputSinkInstance {
        self.sink.as_ref()
    }

    pub fn sink_mut(&mut self) -> &mut dyn OutputSinkInstance {
        self.sink.as_mut()
    }

    /// Swap in a new sink, keeping every picture setting, and re-resolve the
    /// presentation against it. The caller stops the old sink first.
    ///
    /// # Errors
    ///
    /// Fails when the new sink can deliver nothing.
    pub fn replace_sink(
        &mut self,
        gpu: &GpuContext,
        services: &Services,
        sink: Box<dyn OutputSinkInstance>,
        render: (u32, u32),
    ) -> Result<()> {
        self.sink = sink;
        self.size = self.sink.size(render);
        self.set_presentation_request(gpu, services, self.presentation_request)
    }

    /// Whether this output draws this frame: a startable sink while it runs,
    /// any other while it is ready.
    pub fn is_live(&self) -> bool {
        self.sink.is_ready() && (self.active || !self.sink.startable())
    }

    /// What the output shows with no surfaces assigned.
    pub fn effective_unassigned(&self) -> Unassigned {
        self.unassigned
            .unwrap_or_else(|| self.sink.default_unassigned())
    }

    pub fn presentation_request(&self) -> PresentationRequest {
        self.presentation_request
    }

    pub fn resolved_presentation(&self) -> &ResolvedPresentation {
        &self.resolved_presentation
    }

    /// Every presentation mode, with the reason this output cannot deliver it.
    pub fn mode_availability(&self) -> &[ModeAvailability] {
        &self.mode_availability
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The format of what this output delivers.
    pub fn readback_format(&self) -> Option<ReadbackFormat> {
        self.resources.readback.as_ref().map(ReadbackBuffer::format)
    }

    /// The composed picture, for a preview, when the last frame produced one.
    pub fn preview_view(&self) -> Option<&wgpu::TextureView> {
        self.composed_last_frame
            .then_some(&self.resources.composed.view)
    }

    /// Size of [`Self::preview_view`].
    pub fn preview_size(&self) -> (u32, u32) {
        (
            self.resources.composed.texture.width(),
            self.resources.composed.texture.height(),
        )
    }

    /// Resolve `request` against the sink and rebuild for the result.
    ///
    /// # Errors
    ///
    /// Fails when the sink can deliver nothing for it.
    pub fn set_presentation_request(
        &mut self,
        gpu: &GpuContext,
        services: &Services,
        request: PresentationRequest,
    ) -> Result<()> {
        let presentation = self.sink.configure(gpu, &SinkQuery { services }, request)?;
        self.presentation_request = request;
        self.mode_availability = presentation.modes;
        self.apply_resolved(gpu, presentation.resolved)
    }

    /// Take a presentation the sink settled on when it started.
    ///
    /// # Errors
    ///
    /// Fails when a pipeline for it cannot be built.
    pub fn apply_resolved(
        &mut self,
        gpu: &GpuContext,
        resolved: ResolvedPresentation,
    ) -> Result<()> {
        let changed = self.sink.target_format(&resolved) != self.resources.target_format
            || self.sink.intermediate_format(&resolved) != self.resources.intermediate_format
            || self.resources.readback.as_ref().is_some_and(|readback| {
                readback.format() != storage_formats(&resolved).1
                    || readback.color_profile() != resolved.color_profile
                    || readback.alpha_mode() != Self::readback_alpha_mode(&resolved)
            });
        self.resolved_presentation = resolved;
        if changed { self.rebuild(gpu) } else { Ok(()) }
    }

    /// Follow the render resolution, or a window's size.
    ///
    /// # Errors
    ///
    /// Fails when resources cannot be rebuilt.
    pub fn resize(&mut self, gpu: &GpuContext, render: (u32, u32)) -> Result<()> {
        let size = self.sink.size(render);
        if size == self.size || size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.size = size;
        self.rebuild(gpu)
    }

    /// Set the rotation applied in the final pass.
    ///
    /// # Errors
    ///
    /// Fails when resources cannot be rebuilt.
    pub fn set_rotation(&mut self, gpu: &GpuContext, rotation: OutputRotation) -> Result<()> {
        if rotation == self.rotation {
            return Ok(());
        }
        self.rotation = rotation;
        self.rebuild(gpu)
    }

    /// Draw `content` and hand the result to the sink.
    pub fn render(
        &mut self,
        gpu: &GpuContext,
        services: &mut Services,
        content: Content<'_>,
        fps: u32,
    ) -> RenderedFrame {
        if !self.sink.is_ready() {
            return RenderedFrame::Skipped;
        }
        let path = self.sink.frame_path();
        let surface = if path == FramePath::Present {
            match self.sink.acquire(gpu) {
                Some(surface) => Some(surface),
                None => return RenderedFrame::Skipped,
            }
        } else {
            None
        };
        let surface_view = surface.as_ref().map(|s| {
            s.texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        });

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Output Encoder"),
            });
        let use_edge_blend =
            self.edge_blend_mode == EdgeBlendMode::Manual && self.edge_blend.any_enabled();
        let res = &self.resources;
        let target_view = match (&surface_view, &res.delivered) {
            (Some(view), _) => view,
            (None, Some(delivered)) => &delivered.view,
            (None, None) => return RenderedFrame::Skipped,
        };

        // A whole-output picture on a non-window sink with nothing to blend is
        // one pass straight into the delivered texture.
        let direct = match &content {
            Content::Picture { view, .. } if path != FramePath::Present && !use_edge_blend => {
                Some(*view)
            }
            _ => None,
        };
        // Vertices for a whole-output picture, drawn like a surface.
        let quad_vertices: [[f32; 2]; 4];
        let finish_source = if let Some(view) = direct {
            view
        } else {
            let quad_info;
            let surfaces: &[SurfaceRenderInfo<'_>] = match &content {
                Content::Surfaces(infos) => infos,
                Content::Picture { view, fit_aspect } => {
                    let (width, height) =
                        self.rotation.effective_dimensions(self.size.0, self.size.1);
                    let rect = match fit_aspect {
                        Some(aspect) if path == FramePath::Present => {
                            crate::renderer::context::aspect_fit_rect(*aspect, width, height)
                        }
                        _ => [0.0, 0.0, 1.0, 1.0],
                    };
                    let [x, y, w, h] = rect;
                    quad_vertices = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
                    quad_info = SurfaceRenderInfo {
                        uuid: "",
                        content_view: view,
                        vertices: &quad_vertices,
                        extra_contours: &[],
                        bounding_box: rect,
                        uv_scale: [1.0, 1.0],
                        uv_offset: [0.0, 0.0],
                        warp_mode: None,
                        overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                        hole_uv_contours: Vec::new(),
                    };
                    std::slice::from_ref(&quad_info)
                }
            };
            let compose_into = if use_edge_blend {
                &res.blend_source.view
            } else {
                &res.composed.view
            };
            compose_surfaces(gpu, &mut encoder, &res.polygon, compose_into, surfaces);
            if use_edge_blend {
                res.edge_blend.render(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &res.blend_source.view,
                    &res.composed.view,
                    &self.edge_blend,
                );
            }
            &res.composed.view
        };
        self.composed_last_frame = direct.is_none();

        res.finish.set_presentation(
            &gpu.queue,
            self.rotation.index(),
            &self.resolved_presentation,
        );
        let bind_group = res.finish.create_bind_group(&gpu.device, finish_source);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Output Finish"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
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
            res.finish.render(&mut pass, &bind_group);
        }

        if let Some(surface) = surface {
            gpu.submit(std::iter::once(encoder.finish()));
            self.sink.present(gpu, surface);
            return RenderedFrame::Done;
        }
        self.deliver(gpu, services, encoder, fps)
    }

    /// Hand the finished texture on, for every sink but a window.
    fn deliver(
        &mut self,
        gpu: &GpuContext,
        services: &mut Services,
        mut encoder: wgpu::CommandEncoder,
        fps: u32,
    ) -> RenderedFrame {
        let path = self.sink.frame_path();
        let Some(delivered) = self.resources.delivered.as_ref() else {
            return RenderedFrame::Skipped;
        };
        let (width, height) = self.size;
        let request = self.presentation_request;

        if path == FramePath::Converted {
            let mut frame = SinkFrame {
                gpu,
                services,
                view: &delivered.view,
                width,
                height,
                request,
                resolved: &self.resolved_presentation,
                fps,
            };
            if let Err(error) = self.sink.encode(&mut frame, &mut encoder) {
                return RenderedFrame::Stop(error);
            }
        }
        if let Some(readback) = self.resources.readback.as_mut() {
            readback.begin_readback(&mut encoder, &delivered.texture);
        }

        // Measure content light levels of the frame about to be delivered while
        // an HDR delivery runs.
        if self.active && self.resolved_presentation.transfer.is_hdr() {
            if self.light_meter.is_none() {
                match ContentLightMeter::new(&gpu.device) {
                    Ok(meter) => self.light_meter = Some(Box::new(meter)),
                    Err(error) => log::warn!(
                        "Output '{}': content light measurement unavailable: {error}",
                        self.name
                    ),
                }
            }
            if let Some(meter) = self.light_meter.as_mut() {
                meter.measure(&gpu.device, &mut encoder, &delivered.view, width, height);
            }
        } else if self.light_meter.is_some() {
            self.light_meter = None;
        }
        gpu.submit(std::iter::once(encoder.finish()));

        if matches!(path, FramePath::Gpu | FramePath::Converted) {
            let mut frame = SinkFrame {
                gpu,
                services,
                view: &delivered.view,
                width,
                height,
                request,
                resolved: &self.resolved_presentation,
                fps,
            };
            if let Err(error) = self.sink.publish(&mut frame) {
                return RenderedFrame::Stop(error);
            }
        }
        if let Some(meter) = self.light_meter.as_mut()
            && let Some(levels) = meter.try_read(&gpu.device)
        {
            self.light_levels.observe(levels);
        }
        if let Some(frame) = self
            .resources
            .readback
            .as_mut()
            .and_then(|readback| readback.try_read(&gpu.device))
        {
            return match self.sink.deliver(frame) {
                Delivered::Ok => RenderedFrame::Done,
                Delivered::Failed(message) => RenderedFrame::Stop(message),
                Delivered::Restart => RenderedFrame::Restart,
            };
        }
        RenderedFrame::Done
    }

    /// End the running session's measurement, returning what it measured.
    pub fn take_light_levels(&mut self) -> ContentLightLevels {
        self.light_meter = None;
        std::mem::take(&mut self.light_levels)
    }
}

/// Draw `surfaces` into `target`, cleared to black. Warp is applied per
/// surface: a corner pin as a homography, a mesh or Bezier cage baked into the
/// vertices. A surface combined from several contours is drawn as a
/// bounding-box fill, since one warp mesh cannot span disjoint contours.
fn compose_surfaces(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &PolygonBlitPipeline,
    target: &wgpu::TextureView,
    surfaces: &[SurfaceRenderInfo<'_>],
) {
    use crate::surface::warp::WarpMode;
    let draws: Vec<PolygonDrawDesc<'_>> = surfaces
        .iter()
        .map(|surf| {
            let bb = surf.bounding_box;
            let triangulated = || {
                PolygonBlitPipeline::triangulate_verts(surf.vertices, bb[0], bb[1], bb[2], bb[3])
            };
            let (homography, vertices) = if surf.extra_contours.is_empty() {
                match &surf.warp_mode {
                    Some(WarpMode::CornerPin { corners }) => {
                        let src_corners = [
                            [bb[0], bb[1]],
                            [bb[0] + bb[2], bb[1]],
                            [bb[0] + bb[2], bb[1] + bb[3]],
                            [bb[0], bb[1] + bb[3]],
                        ];
                        (
                            Some(crate::surface::warp::compute_forward_homography(
                                &src_corners,
                                corners,
                            )),
                            triangulated(),
                        )
                    }
                    Some(WarpMode::Mesh(mesh)) => (None, PolygonBlitPipeline::mesh_verts(mesh)),
                    Some(WarpMode::Bezier(b)) => {
                        (None, PolygonBlitPipeline::mesh_verts(&b.tessellate()))
                    }
                    None => (None, triangulated()),
                }
            } else {
                (
                    None,
                    PolygonBlitPipeline::triangulate_multi(
                        surf.vertices,
                        surf.extra_contours,
                        bb[0],
                        bb[1],
                        bb[2],
                        bb[3],
                    ),
                )
            };
            PolygonDrawDesc {
                content_view: surf.content_view,
                uv_scale: surf.uv_scale,
                uv_offset: surf.uv_offset,
                homography,
                overlap_zones: &surf.overlap_zones,
                vertices,
                mask_uuid: surf.uuid,
                mask_uv_contours: surf.hole_uv_contours.clone(),
            }
        })
        .collect();
    let (prepared, vertex_pool) = pipeline.prepare(&gpu.device, &gpu.queue, &draws);
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Output Surfaces"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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
    pipeline.draw(&mut pass, &prepared, &vertex_pool);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::value::render::{ModeAvailability, OutputRotation};
    use crate::output::{Presentation, SinkConfig};
    use crate::renderer::ReadbackFrame;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A sink that keeps every frame it is handed.
    struct Capture(Rc<RefCell<Vec<ReadbackFrame>>>);

    impl OutputSinkInstance for Capture {
        fn sink_type(&self) -> &'static str {
            "capture"
        }
        fn label(&self) -> String {
            "capture".into()
        }
        fn config(&self) -> SinkConfig {
            SinkConfig::new("capture")
        }
        fn frame_path(&self) -> FramePath {
            FramePath::Readback
        }
        fn configure(
            &mut self,
            _gpu: &GpuContext,
            _query: &SinkQuery,
            request: PresentationRequest,
        ) -> Result<Presentation> {
            Ok(Presentation {
                resolved: crate::delivery::presentation::resolve_eight_bit_sdr(request, "test"),
                modes: Vec::<ModeAvailability>::new(),
            })
        }
        fn deliver(&mut self, frame: ReadbackFrame) -> Delivered {
            self.0.borrow_mut().push(frame);
            Delivered::Ok
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    const SIZE: u32 = 4;

    /// A 4×4 picture with a different color in each quadrant, so any rotation
    /// or flip moves colors.
    fn quadrants(gpu: &GpuContext) -> Target {
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let mut bytes = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let quadrant = usize::from(y >= SIZE / 2) * 2 + usize::from(x >= SIZE / 2);
                bytes.extend_from_slice(&colors[quadrant]);
            }
        }
        let target = Target::new(
            &gpu.device,
            (SIZE, SIZE),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            "Test Quadrants",
        );
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * SIZE),
                rows_per_image: Some(SIZE),
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        target
    }

    /// The pixels the output delivers for `content`, read back.
    fn deliver(
        gpu: &GpuContext,
        output: &mut Output,
        frames: &Rc<RefCell<Vec<ReadbackFrame>>>,
        content: impl Fn() -> Content<'static>,
    ) -> Vec<u8> {
        let mut services = Services::new();
        frames.borrow_mut().clear();
        for _ in 0..20 {
            assert_eq!(
                output.render(gpu, &mut services, content(), 60),
                RenderedFrame::Done
            );
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            if let Some(frame) = frames.borrow_mut().pop() {
                let stride = frame.stride() as usize;
                return frame
                    .bytes()
                    .chunks(stride)
                    .take(SIZE as usize)
                    .flat_map(|row| row[..(SIZE * 4) as usize].to_vec())
                    .collect();
            }
        }
        panic!("the output never delivered a frame");
    }

    fn capture_output(gpu: &GpuContext) -> (Output, Rc<RefCell<Vec<ReadbackFrame>>>) {
        let frames = Rc::new(RefCell::new(Vec::new()));
        let mut output = Output::new(
            gpu,
            &Services::new(),
            "o1".into(),
            "Capture".into(),
            Box::new(Capture(Rc::clone(&frames))),
            (SIZE, SIZE),
        )
        .expect("output");
        output.active = true;
        (output, frames)
    }

    /// Surfaces and a whole-output picture must come out the same way, rotation
    /// and encoding included.
    #[test]
    fn surface_routing_honors_rotation_like_a_whole_picture() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let picture = Box::leak(Box::new(quadrants(&gpu)));
        let view: &'static wgpu::TextureView = &picture.view;
        let vertices: &'static [[f32; 2]] =
            Box::leak(Box::new([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]));
        let surfaces: &'static [SurfaceRenderInfo<'static>] =
            Box::leak(Box::new([SurfaceRenderInfo {
                uuid: "s1",
                content_view: view,
                vertices,
                extra_contours: &[],
                bounding_box: [0.0, 0.0, 1.0, 1.0],
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                warp_mode: None,
                overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                hole_uv_contours: Vec::new(),
            }]));
        for rotation in [OutputRotation::Deg0, OutputRotation::Deg90] {
            let (mut output, frames) = capture_output(&gpu);
            output.set_rotation(&gpu, rotation).unwrap();
            let direct = deliver(&gpu, &mut output, &frames, || Content::Picture {
                view,
                fit_aspect: None,
            });
            assert!(
                output.preview_view().is_none(),
                "a direct picture composes nothing"
            );
            let routed = deliver(&gpu, &mut output, &frames, || Content::Surfaces(surfaces));
            assert!(
                output.preview_view().is_some(),
                "surfaces compose a preview"
            );
            assert_eq!(routed, direct, "{rotation:?}");
        }
    }

    /// A startable sink draws only while it runs.
    #[test]
    fn a_stopped_startable_output_is_not_live() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let (mut output, _) = capture_output(&gpu);
        assert!(output.is_live());
        output.active = false;
        assert!(!output.is_live());
        assert_eq!(output.effective_unassigned(), Unassigned::Program);
        output.unassigned = Some(Unassigned::Stage);
        assert_eq!(output.effective_unassigned(), Unassigned::Stage);
    }
}
