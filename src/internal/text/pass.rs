//! The GPU pass that draws a text deck: cached coverage masks as quads over
//! the background. See /spec/text-source.md § Rendering.
//!
//! With an opaque background (the default) the quads blend premultiplied
//! straight onto a background clear, which is exact even where units overlap.
//! With a background that is not opaque, the deck's target holds straight
//! alpha, which blending cannot produce, so the quads go into a layer of the
//! deck's size and a resolve pass composites it over the background.

use super::raster::Raster;
use crate::renderer::GpuContext;
use crate::source::SourceFrame;
use anyhow::Result;

/// Size of one quad's uniform block: three `vec4<f32>`.
const QUAD_BYTES: u64 = 48;

/// A rasterized unit on the GPU.
pub struct Mask {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    /// Font size it was drawn at, in pixels.
    pub px: f32,
    /// Top-left relative to the text origin, in ems.
    pub origin: [f32; 2],
}

/// One quad to draw this frame.
pub struct Draw<'m> {
    pub mask: &'m Mask,
    /// Left, top, right, bottom in deck pixels.
    pub rect: [f32; 4],
    pub color: [f32; 4],
    /// Clip edge across the mask, 0 to 1, and which side is kept.
    pub clip_u: f32,
    pub keep_after: bool,
    pub fade: f32,
}

/// The deck-sized layer the quads draw into when the background is not
/// opaque.
struct Layer {
    view: wgpu::TextureView,
    resolve_group: wgpu::BindGroup,
    size: (u32, u32),
}

pub struct TextPass {
    quads: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    quad_layout: wgpu::BindGroupLayout,
    mask_layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    ring: wgpu::Buffer,
    ring_group: wgpu::BindGroup,
    ring_slots: usize,
    stride: u64,
    /// Quad uniforms packed for one upload, kept across frames.
    staging: Vec<u8>,
    background: wgpu::Buffer,
    layer: Option<Layer>,
}

impl TextPass {
    /// # Errors
    ///
    /// Fails when a pipeline cannot be created.
    pub fn new(gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;
        let quad_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Text Quad Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(QUAD_BYTES),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Text Mask Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: texture_binding(true),
                count: None,
            }],
        });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Text Resolve Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: texture_binding(false),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });

        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Text Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text.wgsl").into()),
        });
        let resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Text Resolve Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text_resolve.wgsl").into()),
        });
        let quads = pipeline(
            gpu,
            "Text Quads",
            &[Some(&quad_layout), Some(&mask_layout)],
            &quad_shader,
            ("vs_quad", "fs_quad"),
            wgpu::PrimitiveTopology::TriangleStrip,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let resolve = pipeline(
            gpu,
            "Text Resolve",
            &[Some(&resolve_layout)],
            &resolve_shader,
            ("vs_full", "fs_resolve"),
            wgpu::PrimitiveTopology::TriangleList,
            None,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Text Mask Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let stride = QUAD_BYTES.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));
        let ring_slots = 16;
        let ring = ring_buffer(device, stride, ring_slots);
        let ring_group = ring_bind_group(device, &quad_layout, &ring, &sampler);
        let background = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Text Background"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            quads,
            resolve,
            quad_layout,
            mask_layout,
            resolve_layout,
            sampler,
            ring,
            ring_group,
            ring_slots,
            stride,
            staging: Vec::new(),
            background,
            layer: None,
        })
    }

    /// Upload a raster as a mask.
    pub fn upload(&self, gpu: &GpuContext, raster: &Raster) -> Mask {
        let size = wgpu::Extent3d {
            width: raster.width,
            height: raster.height,
            depth_or_array_layers: 1,
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Text Mask"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &raster.coverage,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster.width),
                rows_per_image: Some(raster.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Text Mask Group"),
            layout: &self.mask_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Mask {
            _texture: texture,
            bind_group,
            width: raster.width,
            height: raster.height,
            px: raster.px,
            origin: raster.origin,
        }
    }

    /// Draw `draws` over `background` into the frame's target.
    pub fn draw(&mut self, frame: &mut SourceFrame, background: [f32; 4], draws: &[Draw]) {
        self.write_quads(frame, draws);
        let opaque = background[3] >= 1.0;
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Text Pass"),
                });
        let (target, clear) = if opaque {
            self.layer = None;
            (frame.target, color(background))
        } else {
            self.ensure_layer(frame);
            let layer = self.layer.as_ref().map(|l| &l.view);
            (layer.unwrap_or(frame.target), wgpu::Color::TRANSPARENT)
        };
        {
            let mut pass = begin(&mut encoder, target, clear, "Text Quads");
            pass.set_pipeline(&self.quads);
            for (i, draw) in draws.iter().enumerate() {
                let offset = (i as u64 * self.stride) as u32;
                pass.set_bind_group(0, &self.ring_group, &[offset]);
                pass.set_bind_group(1, &draw.mask.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        if !opaque && let Some(layer) = &self.layer {
            frame
                .gpu
                .queue
                .write_buffer(&self.background, 0, bytemuck::cast_slice(&background));
            let mut pass = begin(
                &mut encoder,
                frame.target,
                wgpu::Color::TRANSPARENT,
                "Text Resolve",
            );
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &layer.resolve_group, &[]);
            pass.draw(0..3, 0..1);
        }
        frame.cmd_buffers.push(encoder.finish());
    }

    /// Pack and upload every quad's uniforms at once.
    fn write_quads(&mut self, frame: &SourceFrame, draws: &[Draw]) {
        if draws.is_empty() {
            return;
        }
        if draws.len() > self.ring_slots {
            self.ring_slots = draws.len().next_power_of_two();
            self.ring = ring_buffer(&frame.gpu.device, self.stride, self.ring_slots);
            self.ring_group = ring_bind_group(
                &frame.gpu.device,
                &self.quad_layout,
                &self.ring,
                &self.sampler,
            );
        }
        let (w, h) = (frame.width.max(1) as f32, frame.height.max(1) as f32);
        self.staging.clear();
        self.staging.resize(draws.len() * self.stride as usize, 0);
        for (i, draw) in draws.iter().enumerate() {
            let [left, top, right, bottom] = draw.rect;
            let block: [f32; 12] = [
                left / w * 2.0 - 1.0,
                1.0 - top / h * 2.0,
                right / w * 2.0 - 1.0,
                1.0 - bottom / h * 2.0,
                draw.color[0],
                draw.color[1],
                draw.color[2],
                draw.color[3],
                draw.clip_u.min(1.0e9),
                if draw.keep_after { 1.0 } else { 0.0 },
                draw.fade,
                0.0,
            ];
            let start = i * self.stride as usize;
            self.staging[start..start + QUAD_BYTES as usize]
                .copy_from_slice(bytemuck::cast_slice(&block));
        }
        frame.gpu.queue.write_buffer(&self.ring, 0, &self.staging);
    }

    fn ensure_layer(&mut self, frame: &SourceFrame) {
        let size = (frame.width.max(1), frame.height.max(1));
        if self.layer.as_ref().is_some_and(|l| l.size == size) {
            return;
        }
        let texture = frame.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Text Layer"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: frame.gpu.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let resolve_group = frame
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Text Resolve Group"),
                layout: &self.resolve_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.background.as_entire_binding(),
                    },
                ],
            });
        self.layer = Some(Layer {
            view,
            resolve_group,
            size,
        });
    }
}

fn texture_binding(filterable: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn pipeline(
    gpu: &GpuContext,
    label: &str,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    shader: &wgpu::ShaderModule,
    (vertex, fragment): (&str, &str),
    topology: wgpu::PrimitiveTopology,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: layouts,
            immediate_size: 0,
        });
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.compositing_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
}

fn ring_buffer(device: &wgpu::Device, stride: u64, slots: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Text Quad Ring"),
        size: stride * slots as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn ring_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    ring: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Text Quad Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: ring,
                    offset: 0,
                    size: wgpu::BufferSize::new(QUAD_BYTES),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn color([r, g, b, a]: [f32; 4]) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(r),
        g: f64::from(g),
        b: f64::from(b),
        a: f64::from(a),
    }
}

fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    clear: wgpu::Color,
    label: &'static str,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
