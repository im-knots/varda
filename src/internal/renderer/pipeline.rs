use anyhow::Result;
use wgpu::util::DeviceExt;

/// ISF shader uniforms (automatic variables)
/// Layout is 16-byte aligned for GPU compatibility
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ISFUniforms {
    pub time: f32,
    pub time_delta: f32,
    pub frame_index: u32,
    pub pass_index: i32,  // PASSINDEX for multi-pass rendering
    pub render_size: [f32; 2],
    // Audio uniforms
    pub audio_level: f32,   // Overall audio level (0.0 to 1.0)
    pub audio_bass: f32,    // Low frequency level
    pub audio_mid: f32,     // Mid frequency level
    pub audio_treble: f32,  // High frequency level
    pub audio_bpm: f32,     // Detected BPM (0.0 if not detected)
    pub audio_beat_phase: f32, // Phase within beat cycle (0.0 to 1.0)
    pub date: [f32; 4],
}

impl Default for ISFUniforms {
    fn default() -> Self {
        Self {
            time: 0.0,
            time_delta: 0.0,
            frame_index: 0,
            pass_index: 0,
            render_size: [800.0, 600.0],
            audio_level: 0.0,
            audio_bass: 0.0,
            audio_mid: 0.0,
            audio_treble: 0.0,
            audio_bpm: 0.0,
            audio_beat_phase: 0.0,
            date: [2026.0, 2.0, 27.0, 0.0],
        }
    }
}

/// Unified shader pipeline — handles generators, filters, single-pass, and multi-pass shaders.
///
/// Binding layout adapts to shader needs:
///   Simple generator:   [0: Uniforms, 1: UserParams]
///   Simple filter:      [0: Uniforms, 1: Sampler, 2: inputImage, 3: UserParams]
///   Multi-pass gen:     [0: Uniforms, 1: Sampler, 2..N: passBuffers, N+1: UserParams]
///   Multi-pass filter:  [0: Uniforms, 1: Sampler, 2: inputImage, 3..N: passBuffers, N+1: UserParams]
pub struct UnifiedPipeline {
    /// Pipeline for the primary target format (surface_format passed at creation)
    pub pipeline: wgpu::RenderPipeline,
    /// Optional pipeline for float (Rgba32Float) targets (multi-pass only)
    pub float_pipeline: Option<wgpu::RenderPipeline>,
    /// Optional pipeline for Rgba8Unorm intermediate pass buffers
    /// (needed when surface_format != Rgba8Unorm, i.e. master effects with passes)
    pub rgba8_pipeline: Option<wgpu::RenderPipeline>,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    /// Sampler — present when shader has textures (input image or pass buffers)
    pub sampler: Option<wgpu::Sampler>,
    /// Whether this shader has an input image binding (i.e. it's a filter)
    pub has_input_image: bool,
    /// Number of pass buffer texture bindings
    pub num_pass_buffers: usize,
    /// Default user params buffer (256 bytes of zeros)
    pub default_user_params_buffer: wgpu::Buffer,
    /// The binding index where user params live
    pub user_params_binding: u32,
    /// The primary format this pipeline was created for
    pub surface_format: wgpu::TextureFormat,
}


impl UnifiedPipeline {
    /// Create a unified pipeline from SPIR-V bytecode.
    ///
    /// - `has_input_image`: true for filters (binding for inputImage texture)
    /// - `num_pass_buffers`: number of persistent/pass buffer textures
    /// - `needs_float_pipeline`: create additional pipeline for Rgba32Float targets
    /// - `surface_format`: target texture format (Rgba8Unorm for decks, surface format for master)
    pub fn new(
        device: &wgpu::Device,
        spirv: &[u32],
        surface_format: wgpu::TextureFormat,
        has_input_image: bool,
        num_pass_buffers: usize,
        needs_float_pipeline: bool,
    ) -> Result<Self> {
        // Convert SPIR-V to WGSL using naga
        let spirv_bytes: Vec<u8> = spirv
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();

        let module = naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)?;

        let wgsl = naga::back::wgsl::write_string(
            &module,
            &info,
            naga::back::wgsl::WriterFlags::empty(),
        )?;

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ISF Unified Shader Module"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });

        // Create uniform buffer
        let uniforms = ISFUniforms::default();
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ISF Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let has_textures = has_input_image || num_pass_buffers > 0;

        // Build bind group layout entries dynamically
        let mut layout_entries = vec![];
        let mut next_binding: u32 = 0;

        // Binding 0: ISFUniforms (always present)
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: next_binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<ISFUniforms>() as u64),
            },
            count: None,
        });
        next_binding += 1;

        // Sampler (only if shader uses textures)
        let sampler = if has_textures {
            // Use NonFiltering for float texture compatibility when pass buffers are present
            let sampler_type = if num_pass_buffers > 0 {
                wgpu::SamplerBindingType::NonFiltering
            } else {
                wgpu::SamplerBindingType::Filtering
            };
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(sampler_type),
                count: None,
            });
            next_binding += 1;

            let filter_mode = if num_pass_buffers > 0 {
                wgpu::FilterMode::Nearest
            } else {
                wgpu::FilterMode::Linear
            };
            Some(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("ISF Sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter_mode,
                min_filter: filter_mode,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            }))
        } else {
            None
        };

        // Input image texture (for filters)
        if has_input_image {
            let filterable = num_pass_buffers == 0; // float textures aren't filterable
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            next_binding += 1;
        }

        // Pass buffer textures
        for _ in 0..num_pass_buffers {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            next_binding += 1;
        }

        // User params (always last)
        let user_params_binding = next_binding;
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: user_params_binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ISF Unified Bind Group Layout"),
            entries: &layout_entries,
        });

        // Default user params buffer
        let default_user_params = [0u8; 256];
        let default_user_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Default User Params Buffer"),
            contents: &default_user_params,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ISF Unified Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fullscreen Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        // Helper to create a render pipeline for a specific format
        let create_pipeline = |format: wgpu::TextureFormat, label: &str| {
            let blend_state = if format == wgpu::TextureFormat::Rgba32Float {
                None
            } else {
                Some(wgpu::BlendState::REPLACE)
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                        format,
                        blend: blend_state,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
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
            })
        };

        let pipeline = create_pipeline(surface_format, "ISF Unified Render Pipeline");
        let float_pipeline = if needs_float_pipeline {
            Some(create_pipeline(wgpu::TextureFormat::Rgba32Float, "ISF Unified Float Pipeline"))
        } else {
            None
        };
        // Create Rgba8Unorm pipeline for intermediate pass buffers when the primary
        // format is not Rgba8Unorm (e.g. master effects use Bgra8UnormSrgb but pass
        // buffers use Rgba8Unorm)
        let rgba8_pipeline = if num_pass_buffers > 0 && surface_format != wgpu::TextureFormat::Rgba8Unorm {
            Some(create_pipeline(wgpu::TextureFormat::Rgba8Unorm, "ISF Unified Rgba8 Pipeline"))
        } else {
            None
        };

        Ok(Self {
            pipeline,
            float_pipeline,
            rgba8_pipeline,
            bind_group_layout,
            uniform_buffer,
            sampler,
            has_input_image,
            num_pass_buffers,
            default_user_params_buffer,
            user_params_binding,
            surface_format,
        })
    }

    /// Create a bind group for rendering.
    ///
    /// - `input_view`: Required for filters, the input texture to process
    /// - `pass_buffer_views`: Pass buffer textures (empty for non-multi-pass)
    /// - `user_params_buffer`: User params buffer (uses default if None)
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        input_view: Option<&wgpu::TextureView>,
        pass_buffer_views: &[&wgpu::TextureView],
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        let mut entries = vec![];
        let mut next_binding: u32 = 0;

        // Binding 0: Uniforms
        entries.push(wgpu::BindGroupEntry {
            binding: next_binding,
            resource: self.uniform_buffer.as_entire_binding(),
        });
        next_binding += 1;

        // Sampler (if present)
        if let Some(sampler) = &self.sampler {
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
            next_binding += 1;
        }

        // Input image (for filters)
        if self.has_input_image {
            let view = input_view.expect("Filter pipeline requires an input texture view");
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            next_binding += 1;
        }

        // Pass buffer textures
        for view in pass_buffer_views {
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            next_binding += 1;
        }

        // User params (always last)
        let params_buf = user_params_buffer.unwrap_or(&self.default_user_params_buffer);
        entries.push(wgpu::BindGroupEntry {
            binding: self.user_params_binding,
            resource: params_buf.as_entire_binding(),
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ISF Unified Bind Group"),
            layout: &self.bind_group_layout,
            entries: &entries,
        })
    }

    /// Convenience: create bind group for simple generator (no input, no passes)
    pub fn create_bind_group_with_params(
        &self,
        device: &wgpu::Device,
        user_params_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        self.create_bind_group(device, None, &[], Some(user_params_buffer))
    }

    /// Update uniforms
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &ISFUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[*uniforms]));
    }

    /// Get the pipeline for a specific target format
    pub fn pipeline_for_format(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        if format == wgpu::TextureFormat::Rgba32Float {
            self.float_pipeline.as_ref().unwrap_or(&self.pipeline)
        } else if format == wgpu::TextureFormat::Rgba8Unorm && format != self.surface_format {
            self.rgba8_pipeline.as_ref().unwrap_or(&self.pipeline)
        } else {
            &self.pipeline
        }
    }
}
