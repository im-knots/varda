/// TransitionPipeline — renders an ISF transition shader with two input textures.
///
/// Binding layout (fixed):
///   [0: ISFUniforms, 1: Sampler, 2: startImage, 3: endImage, 4: UserParams]
///
/// The `progress` uniform is the first float in the UserParams block (set by the mixer
/// from the crossfader position). Additional user params follow.

use anyhow::Result;
use wgpu::util::DeviceExt;
use super::ISFUniforms;

pub struct TransitionPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub sampler: wgpu::Sampler,
    pub default_user_params_buffer: wgpu::Buffer,
}

impl TransitionPipeline {
    /// Create from SPIR-V bytecode of a transition shader.
    pub fn new(
        device: &wgpu::Device,
        spirv: &[u32],
        target_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        // SPIR-V → WGSL via naga
        let spirv_bytes: Vec<u8> = spirv.iter().flat_map(|w| w.to_le_bytes()).collect();
        let module = naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        ).validate(&module)?;
        let wgsl = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Transition Shader Module"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });

        // Bind group layout: uniforms, sampler, startImage, endImage, userParams
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Transition Bind Group Layout"),
            entries: &[
                // 0: ISFUniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<ISFUniforms>() as u64,
                        ),
                    },
                    count: None,
                },
                // 1: Sampler (filtering — channel textures are Rgba8Unorm)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // 2: startImage
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 3: endImage
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 4: UserParams (progress + additional params)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
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

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Transition Uniform Buffer"),
            contents: bytemuck::cast_slice(&[ISFUniforms::default()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let default_user_params = [0u8; 256];
        let default_user_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Transition Default UserParams"),
            contents: &default_user_params,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Transition Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Transition Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Transition Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Transition Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self { pipeline, bind_group_layout, uniform_buffer, sampler, default_user_params_buffer })
    }

    /// Create a bind group for rendering a transition between two textures.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        start_view: &wgpu::TextureView,
        end_view: &wgpu::TextureView,
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        let params_buf = user_params_buffer.unwrap_or(&self.default_user_params_buffer);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Transition Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(start_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(end_view) },
                wgpu::BindGroupEntry { binding: 4, resource: params_buf.as_entire_binding() },
            ],
        })
    }

    /// Update ISF uniforms.
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &ISFUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[*uniforms]));
    }

    /// Render the transition to a target view, returning a command buffer for batched submission.
    pub fn render_to_cmd(
        &self,
        context: &super::GpuContext,
        start_view: &wgpu::TextureView,
        end_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::CommandBuffer {
        self.update_uniforms(&context.queue, uniforms);
        let bind_group = self.create_bind_group(&context.device, start_view, end_view, user_params_buffer);

        let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Transition Render Encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Transition Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
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
            pass.draw(0..3, 0..1);
        }

        encoder.finish()
    }

    /// Render the transition to a target view (submits immediately).
    pub fn render_to(
        &self,
        context: &super::GpuContext,
        start_view: &wgpu::TextureView,
        end_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        user_params_buffer: Option<&wgpu::Buffer>,
    ) {
        let cmd = self.render_to_cmd(context, start_view, end_view, output_view, uniforms, user_params_buffer);
        context.queue.submit(std::iter::once(cmd));
    }
}

