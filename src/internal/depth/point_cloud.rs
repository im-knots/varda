//! GPU point-cloud reprojection pipeline for depth sensors.
//!
//! Renders one splat per depth texel into the deck target texture, deprojecting
//! via intrinsics and orbiting a virtual camera. Colour comes from the RGB
//! stream, a depth ramp, or a solid tint. See point_cloud.wgsl and
//! spec/depth-sensors.md.

use super::backend::DepthIntrinsics;

/// Point colouring mode (fader-bucketed on the param router).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Rgb,
    DepthRamp,
    Solid,
}

impl ColorMode {
    pub fn as_f32(self) -> f32 {
        match self {
            ColorMode::Rgb => 0.0,
            ColorMode::DepthRamp => 1.0,
            ColorMode::Solid => 2.0,
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => ColorMode::Rgb,
            1 => ColorMode::DepthRamp,
            _ => ColorMode::Solid,
        }
    }
}

/// User-facing point-cloud parameters (router-exposed).
#[derive(Debug, Clone, Copy)]
pub struct PointCloudParams {
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub zoom: f32,
    pub point_size: f32,
    pub color_mode: ColorMode,
    pub depth_min_mm: f32,
    pub depth_max_mm: f32,
    pub solid_color: [f32; 3],
    /// Per-point jitter amount (metres of max offset). `0` = rigid texel grid.
    pub seed: f32,
    /// Time-animated drift of the jitter offset (0 = static, higher = faster/larger flow).
    pub drift: f32,
    /// Strength of the shared procedural curl/noise displacement field (0 = off).
    pub disruption: f32,
}

impl Default for PointCloudParams {
    fn default() -> Self {
        Self {
            orbit_yaw: 0.0,
            orbit_pitch: 0.0,
            zoom: 1.0,
            point_size: 1.0,
            color_mode: ColorMode::DepthRamp,
            depth_min_mm: 400.0,
            depth_max_mm: 4000.0,
            solid_color: [0.6, 0.9, 1.0],
            seed: 0.0,
            drift: 0.0,
            disruption: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuParams {
    intrinsics: [f32; 4],
    dims_range: [f32; 4],
    view: [f32; 4],
    misc: [f32; 4],
    solid: [f32; 4],
    // time, seed, drift, disruption
    anim: [f32; 4],
}

/// Point-cloud render pipeline. Target format is the deck texture format.
pub struct PointCloudPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
}

impl PointCloudPipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Point Cloud Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("point_cloud.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Point Cloud BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Point Cloud Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Point Cloud Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Point Cloud Uniform"),
            size: std::mem::size_of::<GpuParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            layout,
            uniform,
        }
    }

    /// Upload params + intrinsics for this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn update_uniform(
        &self,
        queue: &wgpu::Queue,
        intr: DepthIntrinsics,
        src_w: u32,
        src_h: u32,
        target_w: u32,
        target_h: u32,
        time: f32,
        params: &PointCloudParams,
    ) {
        let gpu = GpuParams {
            intrinsics: [intr.fx, intr.fy, intr.cx, intr.cy],
            dims_range: [
                src_w as f32,
                src_h as f32,
                params.depth_min_mm,
                params.depth_max_mm,
            ],
            view: [
                params.orbit_yaw,
                params.orbit_pitch,
                params.zoom.max(0.01),
                params.point_size.max(0.0),
            ],
            misc: [
                params.color_mode.as_f32(),
                intr.depth_scale_m,
                target_w as f32,
                target_h as f32,
            ],
            solid: [
                params.solid_color[0],
                params.solid_color[1],
                params.solid_color[2],
                0.0,
            ],
            anim: [time, params.seed, params.drift, params.disruption],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&[gpu]));
    }

    /// Render the point cloud into `target`. Clears to black first.
    /// `point_count` = src_w * src_h (one splat per depth texel).
    pub fn render(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        rgb_view: &wgpu::TextureView,
        target: &wgpu::TextureView,
        point_count: u32,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Point Cloud Bind Group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(rgb_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Point Cloud Encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Point Cloud Pass"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..(point_count * 6), 0..1);
        }
        cmd_buffers.push(encoder.finish());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mode_encoding_is_stable() {
        assert_eq!(ColorMode::Rgb.as_f32(), 0.0);
        assert_eq!(ColorMode::DepthRamp.as_f32(), 1.0);
        assert_eq!(ColorMode::Solid.as_f32(), 2.0);
        // from_u8 is the persistence inverse of as_f32.
        assert_eq!(ColorMode::from_u8(0), ColorMode::Rgb);
        assert_eq!(ColorMode::from_u8(1), ColorMode::DepthRamp);
        assert_eq!(ColorMode::from_u8(2), ColorMode::Solid);
        assert_eq!(ColorMode::from_u8(99), ColorMode::Solid);
    }

    #[test]
    fn default_params_are_sane() {
        let p = PointCloudParams::default();
        assert!(p.zoom > 0.0);
        assert!(p.point_size > 0.0);
        assert!(p.depth_min_mm < p.depth_max_mm);
        // New animation params default to off so legacy behaviour is unchanged.
        assert_eq!(p.seed, 0.0);
        assert_eq!(p.drift, 0.0);
        assert_eq!(p.disruption, 0.0);
    }

    #[test]
    fn pipeline_builds_on_headless() {
        let Ok(gpu) = crate::renderer::GpuContext::new_headless() else {
            return;
        };
        let _pipe = PointCloudPipeline::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
    }
}
