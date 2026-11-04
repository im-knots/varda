//! Draws a linear texture into the current viewport of a UI render pass, with
//! the blend state egui uses for its images.

use anyhow::Result;
use std::num::NonZeroU64;

/// Size of `corner_radius` in `preview.wgsl`.
const SHAPE_SIZE: u64 = size_of::<f32>() as u64;

/// Corners of a drawn preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewCorners {
    Square,
    /// The radius set by [`PreviewPipeline::set_corner_radius`].
    Rounded,
}

pub struct PreviewPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Two corner radii, square then rounded.
    shape_buffer: wgpu::Buffer,
    /// Byte stride between radii, aligned to the device minimum.
    shape_stride: u32,
    corner_radius: f32,
}

impl PreviewPipeline {
    /// Create the pipeline for a render pass on `target_format`.
    ///
    /// # Errors
    ///
    /// Never returns `Err`; validation failures surface on the device's error
    /// scope. The `Result` matches the other pipeline constructors.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Preview Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(SHAPE_SIZE),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Preview Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Preview Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });
        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Preview Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/preview.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Preview Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: Some(if target_format.is_srgb() {
                    "fs_srgb_target"
                } else {
                    "fs_gamma_target"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // egui's blend state.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
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
            label: Some("Preview Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let shape_stride = (SHAPE_SIZE as u32)
            .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment);
        let shape_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview Shape Buffer"),
            size: u64::from(shape_stride) * 2,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            shape_buffer,
            shape_stride,
            corner_radius: 0.0,
        })
    }

    /// Set the radius of [`PreviewCorners::Rounded`], in pixels.
    pub fn set_corner_radius(&mut self, queue: &wgpu::Queue, radius: f32) {
        if radius.to_bits() == self.corner_radius.to_bits() {
            return;
        }
        self.corner_radius = radius;
        queue.write_buffer(
            &self.shape_buffer,
            u64::from(self.shape_stride),
            bytemuck::bytes_of(&radius),
        );
    }

    /// A bind group for drawing `source`.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Preview Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.shape_buffer,
                        offset: 0,
                        size: NonZeroU64::new(SHAPE_SIZE),
                    }),
                },
            ],
        })
    }

    /// Draw a bind group's source into the pass's viewport.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
        corners: PreviewCorners,
    ) {
        let offset = match corners {
            PreviewCorners::Square => 0,
            PreviewCorners::Rounded => self.shape_stride,
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[offset]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewCorners, PreviewPipeline};
    use crate::renderer::context::GpuContext;

    const SIZE: u32 = 32;
    /// The background the previews draw over, as display bytes.
    const BACKGROUND: [u8; 4] = [40, 80, 160, 255];

    fn source(
        context: &GpuContext,
        w: u32,
        h: u32,
        texel: impl Fn(u32, u32) -> [f32; 4],
    ) -> wgpu::TextureView {
        let texture = context.create_compositing_texture(w, h);
        let data: Vec<u8> = (0..w * h)
            .flat_map(|i| texel(i % w, i / w))
            .flat_map(|channel| half::f16::from_f32(channel).to_le_bytes())
            .collect();
        context.queue.write_texture(
            texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(h),
            },
            texture.size(),
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Draw `view` over [`BACKGROUND`] on a `format` target and return its display bytes.
    fn draw(
        context: &GpuContext,
        format: wgpu::TextureFormat,
        view: &wgpu::TextureView,
        corners: PreviewCorners,
        corner_radius: f32,
    ) -> Vec<[u8; 4]> {
        let mut pipeline = PreviewPipeline::new(&context.device, format).unwrap();
        pipeline.set_corner_radius(&context.queue, corner_radius);
        let bind_group = pipeline.create_bind_group(&context.device, view);
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview test target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let channel = |byte: u8| {
            let display = f64::from(byte) / 255.0;
            if format.is_srgb() {
                if display <= 0.04045 {
                    display / 12.92
                } else {
                    ((display + 0.055) / 1.055).powf(2.4)
                }
            } else {
                display
            }
        };
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: channel(BACKGROUND[0]),
                            g: channel(BACKGROUND[1]),
                            b: channel(BACKGROUND[2]),
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
            pipeline.draw(&mut pass, &bind_group, corners);
        }
        let row = (SIZE * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview readback"),
            size: u64::from(row * SIZE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(SIZE),
                },
            },
            texture.size(),
        );
        context.submit(std::iter::once(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        (0..SIZE * SIZE)
            .map(|i| {
                let at = ((i / SIZE) * row + (i % SIZE) * 4) as usize;
                [mapped[at], mapped[at + 1], mapped[at + 2], mapped[at + 3]]
            })
            .collect()
    }

    fn display_byte(linear: f32) -> u8 {
        let linear = linear.clamp(0.0, 1.0);
        let gamma = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (gamma * 255.0).round() as u8
    }

    fn assert_near(actual: u8, expected: u8, tolerance: u8, what: &str) {
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "{what}: {actual} is not within {tolerance} of {expected}"
        );
    }

    /// A linear value above 1 clamps, and both target kinds show the same bytes.
    #[test]
    fn linear_sources_show_the_same_on_srgb_and_gamma_targets() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        for value in [0.0, 0.002, 0.05, 0.214, 0.5, 1.0, 3.0] {
            let view = source(&context, SIZE, SIZE, |_, _| {
                [value, value * 0.5, value * 0.25, 1.0]
            });
            for format in [
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureFormat::Rgba8Unorm,
            ] {
                let center = draw(&context, format, &view, PreviewCorners::Square, 0.0)
                    [(SIZE * SIZE / 2 + SIZE / 2) as usize];
                for (channel, scale) in [1.0, 0.5, 0.25].into_iter().enumerate() {
                    assert_near(
                        center[channel],
                        display_byte(value * scale),
                        1,
                        &format!("{value} on {format:?}"),
                    );
                }
            }
        }
    }

    /// Alpha acts as on an egui image: the color adds to the background the
    /// alpha leaves.
    #[test]
    fn partly_transparent_sources_blend_as_premultiplied_images() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let view = source(&context, SIZE, SIZE, |_, _| [0.0, 0.0, 0.0, 0.5]);
        let center = draw(
            &context,
            wgpu::TextureFormat::Rgba8Unorm,
            &view,
            PreviewCorners::Square,
            0.0,
        )[(SIZE * SIZE / 2 + SIZE / 2) as usize];
        for channel in 0..3 {
            assert_near(center[channel], BACKGROUND[channel] / 2, 1, "half alpha");
        }
    }

    /// Fine detail far below the preview's resolution reads as its average.
    #[test]
    fn stripes_much_finer_than_a_pixel_average_out() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let view = source(&context, SIZE * 10, SIZE * 10, |x, _| {
            let v = (x % 2) as f32;
            [v, v, v, 1.0]
        });
        let drawn = draw(
            &context,
            wgpu::TextureFormat::Rgba8Unorm,
            &view,
            PreviewCorners::Square,
            0.0,
        );
        let gray = display_byte(0.5);
        for pixel in drawn {
            assert_near(pixel[0], gray, 2, "stripes");
        }
    }

    #[test]
    fn rounded_corners_leave_the_background_and_square_ones_do_not() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let view = source(&context, SIZE, SIZE, |_, _| [1.0, 1.0, 1.0, 1.0]);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let white = [255, 255, 255, 255];

        let rounded = draw(&context, format, &view, PreviewCorners::Rounded, 8.0);
        let last = (SIZE - 1) as usize;
        for corner in [0, last, last * SIZE as usize, (last + 1) * (last + 1) - 1] {
            assert_eq!(rounded[corner], BACKGROUND, "corner {corner}");
        }
        assert_eq!(rounded[SIZE as usize / 2], white, "top edge");
        assert_eq!(
            rounded[(SIZE * SIZE / 2 + SIZE / 2) as usize],
            white,
            "center"
        );

        let square = draw(&context, format, &view, PreviewCorners::Square, 8.0);
        assert!(square.iter().all(|pixel| *pixel == white));
    }
}
