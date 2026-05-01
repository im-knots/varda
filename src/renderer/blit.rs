/// Simple blit pipeline for copying textures to the screen
use anyhow::Result;
use wgpu::util::DeviceExt;

/// Uniform buffer for blit parameters - 32 bytes (8 x f32)
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    opacity: f32,
    _pad: f32,
    /// UV scale factor (default 1.0, 1.0 = no scaling)
    uv_scale: [f32; 2],
    /// UV offset (default 0.0, 0.0 = no offset)
    uv_offset: [f32; 2],
    _pad2: [f32; 2],
}

pub struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
}

impl BlitPipeline {
    /// Create a new blit pipeline with REPLACE blend (for final screen output)
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        Self::with_blend(device, target_format, wgpu::BlendState::REPLACE)
    }

    /// Create a blit pipeline with a specific blend state
    pub fn with_blend(device: &wgpu::Device, target_format: wgpu::TextureFormat, blend_state: wgpu::BlendState) -> Result<Self> {
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blit Bind Group Layout"),
            entries: &[
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Texture
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
                // Params uniform buffer (opacity)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create params buffer with default opacity of 1.0
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blit Params Buffer"),
            contents: bytemuck::cast_slice(&[BlitParams {
                opacity: 1.0,
                _pad: 0.0,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                _pad2: [0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blit Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Load shaders
        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blit Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(blend_state),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blit Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            params_buffer,
        })
    }

    /// Create a bind group for a texture view
    pub fn create_bind_group(&self, device: &wgpu::Device, texture_view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blit Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Update the opacity value (call before render)
    pub fn set_opacity(&self, queue: &wgpu::Queue, opacity: f32) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[BlitParams {
            opacity,
            _pad: 0.0,
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            _pad2: [0.0, 0.0],
        }]));
    }

    /// Set UV transform parameters for scaling modes
    pub fn set_uv_transform(&self, queue: &wgpu::Queue, opacity: f32, uv_scale: [f32; 2], uv_offset: [f32; 2]) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[BlitParams {
            opacity,
            _pad: 0.0,
            uv_scale,
            uv_offset,
            _pad2: [0.0, 0.0],
        }]));
    }

    /// Render a texture to a render pass
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, bind_group: &'a wgpu::BindGroup) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Render with specific opacity (updates buffer and renders)
    pub fn render_with_opacity<'a>(&'a self, queue: &wgpu::Queue, render_pass: &mut wgpu::RenderPass<'a>, bind_group: &'a wgpu::BindGroup, opacity: f32) {
        self.set_opacity(queue, opacity);
        self.render(render_pass, bind_group);
    }
}

