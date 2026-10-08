//! A deck frame reduced and display-encoded on the GPU for analyzers.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::renderer::{ReadbackBuffer, ReadbackFormat, ReadbackFrame};

/// Most bilinear taps per axis. At 8 a footprint of up to 16 texels is fully
/// covered; beyond that taps spread out.
const MAX_TAPS: u32 = 8;

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    footprint: [f32; 2],
    taps: [u32; 2],
}

/// The size a `width` x `height` frame is read at for an analyzer that wants
/// `long_side`: scaled to fit, aspect kept, never larger than the frame.
pub(crate) fn capture_size((width, height): (u32, u32), long_side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= long_side {
        return (width, height);
    }
    let scale = f64::from(long_side) / f64::from(longest);
    let fit = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    (fit(width), fit(height))
}

/// One deck's reduction pass, target and readback, rebuilt when the size changes.
pub(crate) struct FrameCapture {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    readback: ReadbackBuffer,
    source_size: (u32, u32),
    size: (u32, u32),
}

impl FrameCapture {
    pub(crate) fn new(device: &wgpu::Device, source_size: (u32, u32), size: (u32, u32)) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Analyzer capture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Analyzer capture"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Analyzer capture vertex"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../renderer/shaders/fullscreen.wgsl").into(),
            ),
        });
        let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Analyzer capture fragment"),
            source: wgpu::ShaderSource::Wgsl(include_str!("capture.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Analyzer capture"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Analyzer capture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let footprint = [
            source_size.0 as f32 / size.0 as f32,
            source_size.1 as f32 / size.1 as f32,
        ];
        let taps = footprint.map(|f| ((f / 2.0).ceil() as u32).clamp(1, MAX_TAPS));
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Analyzer capture"),
            contents: bytemuck::bytes_of(&Params { footprint, taps }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Analyzer capture"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            pipeline,
            layout,
            sampler,
            params,
            target,
            target_view,
            readback: ReadbackBuffer::new(device, size.0, size.1, ReadbackFormat::Rgba8),
            source_size,
            size,
        }
    }

    /// Whether this capture reads a `source_size` frame at `size`.
    pub(crate) fn fits(&self, source_size: (u32, u32), size: (u32, u32)) -> bool {
        self.source_size == source_size && self.size == size
    }

    /// A finished earlier capture, if one is ready. Never blocks.
    pub(crate) fn try_read(&mut self, device: &wgpu::Device) -> Option<ReadbackFrame> {
        self.readback.try_read(device)
    }

    /// Record the reduction of `source` and its copy to the readback buffer.
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
    ) {
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Analyzer capture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Analyzer capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.readback.begin_readback(encoder, &self.target);
    }
}

#[cfg(test)]
mod tests {
    use super::capture_size;

    #[test]
    fn the_long_side_fits_and_the_aspect_is_kept() {
        assert_eq!(capture_size((3840, 2160), 1920), (1920, 1080));
        assert_eq!(capture_size((1080, 1920), 256), (144, 256));
        assert_eq!(capture_size((1920, 1080), 1920), (1920, 1080));
        assert_eq!(capture_size((640, 360), 1920), (640, 360));
        assert_eq!(capture_size((4000, 1), 256), (256, 1));
    }
}
