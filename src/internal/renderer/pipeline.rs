use anyhow::Result;
use wgpu::util::DeviceExt;

/// ISF automatic uniforms. 16-byte aligned for the GPU.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ISFUniforms {
    pub time: f32,
    pub time_delta: f32,
    pub frame_index: u32,
    pub pass_index: i32, // PASSINDEX for multi-pass rendering
    pub render_size: [f32; 2],
    pub audio_level: f32,      // Overall audio level (0.0 to 1.0)
    pub audio_bass: f32,       // Low frequency level
    pub audio_mid: f32,        // Mid frequency level
    pub audio_treble: f32,     // High frequency level
    pub audio_bpm: f32,        // Detected BPM (0.0 if not detected)
    pub audio_beat_phase: f32, // Phase within beat cycle (0.0 to 1.0)
    pub date: [f32; 4],
    /// Engine-side phase accumulators (sum of dt * param * scale per frame).
    pub phase_times: [f32; 4],
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
            phase_times: [0.0; 4],
        }
    }
}

/// Shader pipeline for generators, filters, single-pass and multi-pass shaders.
///
/// Binding layout by shader kind:
///   Simple generator:   [0: Uniforms, 1: `UserParams`]
///   Simple filter:      [0: Uniforms, 1: Sampler, 2: inputImage, 3: `UserParams`]
///   Multi-pass gen:     [0: Uniforms, 1: Sampler, 2..N: passBuffers, N+1..M: imported, M+1..P: preprocessor, P+1: `UserParams`]
///   Multi-pass filter:  [0: Uniforms, 1: Sampler, 2: inputImage, 3..N: passBuffers, N+1..M: imported, M+1..P: preprocessor, P+1: `UserParams`]
///   With imported:      [0: Uniforms, 1: Sampler, ..., N+1..M: imported, M+1..P: preprocessor, P+1: `UserParams`]
pub struct UnifiedPipeline {
    /// Every target (final and pass buffers) is `COLOR_PATH_FORMAT`, so one
    /// pipeline covers all of them.
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Uniforms, one slot per pass.
    uniforms: super::pass_uniforms::PassUniforms,
    /// Present when the shader has textures (input image, pass buffers, or imported).
    pub sampler: Option<wgpu::Sampler>,
    /// Whether this shader has an input image binding (a filter).
    pub has_input_image: bool,
    pub num_pass_buffers: usize,
    pub num_imported_textures: usize,
    pub num_preprocessor_textures: usize,
    /// 256 bytes of zeros.
    pub default_user_params_buffer: wgpu::Buffer,
    pub user_params_binding: u32,
    pub surface_format: wgpu::TextureFormat,
}

impl UnifiedPipeline {
    /// Create a unified pipeline from SPIR-V bytecode.
    ///
    /// - `has_input_image`: true for filters (binding for inputImage texture)
    /// - `num_pass_buffers`: number of persistent/pass buffer textures
    /// - `num_imported_textures`: number of ISF IMPORTED image textures
    /// - `preprocessor_filterable`: one entry per preprocessor texture binding;
    ///   false for `texelFetch`-only float data (`FORMAT: "rgba32float"`)
    /// - `surface_format`: always `COLOR_PATH_FORMAT`
    ///
    /// # Errors
    ///
    /// Returns an error if the SPIR-V fails to parse, fails naga validation, or
    /// cannot be transpiled to WGSL.
    // Takes many distinct GPU descriptors with nothing in common to bundle.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        spirv: &[u32],
        surface_format: wgpu::TextureFormat,
        has_input_image: bool,
        num_pass_buffers: usize,
        num_imported_textures: usize,
        preprocessor_filterable: &[bool],
    ) -> Result<Self> {
        let num_preprocessor_textures = preprocessor_filterable.len();
        // SPIR-V to WGSL via naga.
        let spirv_bytes: Vec<u8> = spirv.iter().flat_map(|word| word.to_le_bytes()).collect();

        let module =
            naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)?;

        let wgsl =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ISF Unified Shader Module"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });

        let uniforms = super::pass_uniforms::PassUniforms::new(device);

        let has_textures = has_input_image
            || num_pass_buffers > 0
            || num_imported_textures > 0
            || num_preprocessor_textures > 0;

        let mut layout_entries = vec![];
        let mut next_binding: u32 = 0;

        // Binding 0: ISFUniforms (always present)
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: next_binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(
                    std::mem::size_of::<ISFUniforms>() as u64
                ),
            },
            count: None,
        });
        next_binding += 1;

        // Sampler, only if the shader uses textures. Always filtering: every
        // sampled texture is Rgba16Float, which is filterable.
        let sampler = if has_textures {
            let sampler_type = wgpu::SamplerBindingType::Filtering;
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(sampler_type),
                count: None,
            });
            next_binding += 1;

            let filter_mode = wgpu::FilterMode::Linear;
            Some(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("ISF Sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter_mode,
                min_filter: filter_mode,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }))
        } else {
            None
        };

        // Input image texture (for filters). Filterable, to match the sampler:
        // wgpu rejects a filtering sampler paired with a non-filterable texture.
        if has_input_image {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            next_binding += 1;
        }

        // Imported image textures (ISF IMPORTED)
        for _ in 0..num_imported_textures {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            next_binding += 1;
        }

        // Preprocessor textures (after imported, before user params). A
        // non-filterable entry is valid next to the filtering sampler as long as
        // the shader reads it only with `texelFetch`, which naga lowers to a
        // sampler-free load.
        for filterable in preprocessor_filterable {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float {
                        filterable: *filterable,
                    },
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

        let default_user_params = [0u8; 256];
        let default_user_params_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Default User Params Buffer"),
                contents: &default_user_params,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ISF Unified Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fullscreen Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let create_pipeline = |format: wgpu::TextureFormat, label: &str| {
            let blend_state = Some(wgpu::BlendState::REPLACE);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader_module,
                    entry_point: Some("main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: blend_state,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
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
                multiview_mask: None,
                cache: None,
            })
        };

        let pipeline = create_pipeline(surface_format, "ISF Unified Render Pipeline");

        Ok(Self {
            pipeline,
            bind_group_layout,
            uniforms,
            sampler,
            has_input_image,
            num_pass_buffers,
            num_imported_textures,
            num_preprocessor_textures,
            default_user_params_buffer,
            user_params_binding,
            surface_format,
        })
    }

    /// Create a bind group for rendering.
    ///
    /// - `input_view`: Required for filters, the input texture to process
    /// - `pass_buffer_views`: Pass buffer textures (empty for non-multi-pass)
    /// - `imported_views`: ISF IMPORTED image texture views (empty if none)
    /// - `preprocessor_views`: Preprocessor texture views (empty if none)
    /// - `user_params_buffer`: User params buffer (uses default if None)
    ///
    /// # Panics
    ///
    /// Panics if the pipeline was built with `has_input_image` but `input_view`
    /// is `None`.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        input_view: Option<&wgpu::TextureView>,
        pass_buffer_views: &[&wgpu::TextureView],
        imported_views: &[&wgpu::TextureView],
        preprocessor_views: &[&wgpu::TextureView],
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        self.create_pass_bind_group(
            device,
            0,
            input_view,
            pass_buffer_views,
            imported_views,
            preprocessor_views,
            user_params_buffer,
        )
    }

    /// [`Self::create_bind_group`] for one pass of a multi-pass shader, reading
    /// the uniforms in `slot`.
    ///
    /// # Panics
    ///
    /// Panics if the pipeline was built with `has_input_image` but `input_view`
    /// is `None`.
    #[allow(clippy::too_many_arguments)] // one per binding group the layout can hold
    pub fn create_pass_bind_group(
        &self,
        device: &wgpu::Device,
        slot: usize,
        input_view: Option<&wgpu::TextureView>,
        pass_buffer_views: &[&wgpu::TextureView],
        imported_views: &[&wgpu::TextureView],
        preprocessor_views: &[&wgpu::TextureView],
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        self.uniforms.with_binding(slot, |uniforms| {
            self.bind_group(
                device,
                uniforms,
                input_view,
                pass_buffer_views,
                imported_views,
                preprocessor_views,
                user_params_buffer,
            )
        })
    }

    #[allow(clippy::too_many_arguments)] // one per binding group the layout can hold
    fn bind_group(
        &self,
        device: &wgpu::Device,
        uniforms: wgpu::BindingResource<'_>,
        input_view: Option<&wgpu::TextureView>,
        pass_buffer_views: &[&wgpu::TextureView],
        imported_views: &[&wgpu::TextureView],
        preprocessor_views: &[&wgpu::TextureView],
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        let mut entries = vec![];
        let mut next_binding: u32 = 0;

        // Binding 0: Uniforms
        entries.push(wgpu::BindGroupEntry {
            binding: next_binding,
            resource: uniforms,
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

        // Imported image textures
        for view in imported_views {
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            next_binding += 1;
        }

        // Preprocessor texture views (after imported)
        for view in preprocessor_views {
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

    /// Bind group for a simple generator (no input, passes, or imports).
    pub fn create_bind_group_with_params(
        &self,
        device: &wgpu::Device,
        user_params_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        self.create_bind_group(device, None, &[], &[], &[], Some(user_params_buffer))
    }

    /// Update a single-pass shader's uniforms.
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &ISFUniforms) {
        self.uniforms.write(queue, 0, uniforms);
    }

    /// Make room for `passes` passes' uniforms this frame.
    pub fn ensure_pass_slots(&self, device: &wgpu::Device, passes: usize) {
        self.uniforms.ensure_slots(device, passes);
    }

    /// Write one pass's uniforms into `slot`.
    pub fn write_pass_uniforms(&self, queue: &wgpu::Queue, slot: usize, uniforms: &ISFUniforms) {
        self.uniforms.write(queue, slot, uniforms);
    }
}
