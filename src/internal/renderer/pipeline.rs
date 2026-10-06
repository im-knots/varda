use anyhow::Result;
use std::sync::{Arc, mpsc};
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
    /// Sub-pixel offset in output pixels, in `[-0.5, 0.5)`. `JITTER`.
    pub jitter: [f32; 2],
    /// Index in the jitter cycle. `JITTERINDEX`.
    pub jitter_index: i32,
    /// 0 on the first frame after `HISTORY` targets were cleared, else 1.
    /// `HISTORYVALID`.
    pub history_valid: i32,
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
            jitter: [0.0; 2],
            jitter_index: 0,
            history_valid: 1,
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
    /// The pipelines for the current specialization constants.
    variant: Variant,
    /// Pipelines built for other constants, most recent last, so returning to
    /// an earlier combination does not rebuild.
    other_variants: Vec<Variant>,
    /// The constants [`Self::new`] built with, the inputs' defaults. Their
    /// variant is never evicted.
    pinned: Vec<f64>,
    /// What a variant is built from, shared with the build thread.
    recipe: Arc<Recipe>,
    /// The variant a worker thread is building for live frames.
    building: Option<Build>,
    /// Constants whose worker build failed. Live frames do not retry them.
    failed: Vec<Vec<f64>>,
    /// Write-locked by a test to hold worker builds before they start.
    #[cfg(test)]
    build_gate: Arc<std::sync::RwLock<()>>,
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

/// Variants kept besides the current one, the pinned one included.
const MAX_OTHER_VARIANTS: usize = 8;

/// The passes a shader renders, for choosing its pipelines.
#[derive(Debug, Clone, Default)]
pub struct PassPlan {
    /// Each targeted pass's `PASSINDEX` and target formats, in attachment
    /// order.
    pub targeted: Vec<(i32, Vec<wgpu::TextureFormat>)>,
    /// The output pass's `PASSINDEX`.
    pub output_index: i32,
    /// One pipeline per pass, with `PASSINDEX` as the specialization
    /// constant after the inputs' (ISF `SPECIALIZE_PASSES`). Otherwise one
    /// pipeline per distinct set of target formats.
    pub specialize: bool,
}

/// Render pipelines keyed by `PASSINDEX` when passes are specialized, and by
/// target formats.
type Pipelines = Vec<(Option<i32>, Vec<wgpu::TextureFormat>, wgpu::RenderPipeline)>;

/// The render pipelines of one set of specialization constants.
struct Variant {
    constants: Vec<f64>,
    pipelines: Pipelines,
}

/// What every variant of a shader is built from.
struct Recipe {
    fragment_module: wgpu::ShaderModule,
    vertex_module: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    passes: PassPlan,
    output_format: wgpu::TextureFormat,
}

/// A variant being built on a worker thread.
struct Build {
    constants: Vec<f64>,
    /// The pipelines, or the error that stopped the build. Disconnected if
    /// the thread panicked.
    result: mpsc::Receiver<std::result::Result<Pipelines, String>>,
}

impl UnifiedPipeline {
    /// Create a unified pipeline from SPIR-V bytecode.
    ///
    /// - `has_input_image`: true for filters (binding for inputImage texture)
    /// - `pass_buffer_filterable`: one entry per pass buffer binding; false for
    ///   32-bit float targets, which the shader must read with `texelFetch`
    /// - `passes`: the targeted passes and the output pass
    /// - `num_imported_textures`: number of ISF IMPORTED image textures
    /// - `preprocessor_filterable`: one entry per preprocessor texture binding;
    ///   false for `texelFetch`-only float data (`FORMAT: "rgba32float"`)
    /// - `surface_format`: always `COLOR_PATH_FORMAT`
    /// - `constants`: the specialization constants to build with, by
    ///   `constant_id`, the `SPECIALIZE` inputs' defaults; their pipelines stay
    ///   cached for the pipeline's life. See [`Self::specialize`]
    ///
    /// # Errors
    ///
    /// Returns an error if the SPIR-V fails to parse, fails naga validation, or
    /// cannot be transpiled to WGSL, if the shader binds more sampled textures
    /// than the device allows, or if its specialization constants are not
    /// numbered `0..constants.len()`.
    // Takes many distinct GPU descriptors with nothing in common to bundle.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        spirv: &[u32],
        surface_format: wgpu::TextureFormat,
        has_input_image: bool,
        pass_buffer_filterable: &[bool],
        passes: &PassPlan,
        num_imported_textures: usize,
        preprocessor_filterable: &[bool],
        constants: &[f64],
    ) -> Result<Self> {
        let num_pass_buffers = pass_buffer_filterable.len();
        let num_preprocessor_textures = preprocessor_filterable.len();
        let sampled_textures = usize::from(has_input_image)
            + num_pass_buffers
            + num_imported_textures
            + num_preprocessor_textures;
        let limit = device.limits().max_sampled_textures_per_shader_stage as usize;
        if sampled_textures > limit {
            anyhow::bail!(
                "shader binds {sampled_textures} textures (input, pass buffers, IMPORTED and \
                 PREPROCESSORS); this GPU allows {limit}"
            );
        }
        // SPIR-V to WGSL via naga.
        let spirv_bytes: Vec<u8> = spirv.iter().flat_map(|word| word.to_le_bytes()).collect();

        let module =
            naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())
                .map_err(|e| {
                let hint = if e.to_string().contains("SpecConstantOp") {
                    "; an expression on a specialization constant cannot be translated: \
                         pass the constant through a function and use the result"
                } else {
                    ""
                };
                anyhow::anyhow!("{e}{hint}")
            })?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)?;
        check_constant_ids(&module, constants.len() + usize::from(passes.specialize))?;

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

        // Pass buffer textures. A non-filterable one is valid next to the
        // filtering sampler as long as the shader reads it with `texelFetch`.
        for filterable in pass_buffer_filterable {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float {
                        filterable: *filterable,
                    },
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

        let vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fullscreen Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let recipe = Recipe {
            fragment_module: shader_module,
            vertex_module,
            pipeline_layout,
            passes: passes.clone(),
            output_format: surface_format,
        };
        Ok(Self {
            variant: Variant {
                constants: constants.to_vec(),
                pipelines: recipe.build(device, constants),
            },
            other_variants: Vec::new(),
            pinned: constants.to_vec(),
            recipe: Arc::new(recipe),
            building: None,
            failed: Vec::new(),
            #[cfg(test)]
            build_gate: Arc::default(),
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

    /// Use the pipelines built with `constants`, the values of the shader's
    /// specialization constants by `constant_id`. The first time a
    /// combination is seen its pipelines are built, which compiles the shader
    /// again.
    ///
    /// On a `live` frame the build runs on a worker thread and the current
    /// pipelines stay in use until a later call finds it done. While a build
    /// runs, the constants of the latest call are built next. Offline frames
    /// build in place.
    pub fn specialize(
        &mut self,
        device: &wgpu::Device,
        constants: impl Iterator<Item = f64>,
        live: bool,
    ) {
        let mut constants = constants.peekable();
        if constants.peek().is_none() && self.variant.constants.is_empty() {
            return;
        }
        let constants: Vec<f64> = constants.collect();
        self.finish_build();
        if constants == self.variant.constants {
            return;
        }
        if let Some(index) = self
            .other_variants
            .iter()
            .position(|v| v.constants == constants)
        {
            let variant = self.other_variants.remove(index);
            self.install(variant);
        } else if !live {
            let started = std::time::Instant::now();
            let pipelines = self.recipe.build(device, &constants);
            log::info!("specialized shader pipeline in {:.0?}", started.elapsed());
            self.install(Variant {
                constants,
                pipelines,
            });
        } else if self.building.is_none() && !self.failed.contains(&constants) {
            self.start_build(device, constants);
        }
    }

    /// Make `variant` current, keeping the previous one. Past the limit the
    /// oldest variant that is not pinned is dropped.
    fn install(&mut self, variant: Variant) {
        let previous = std::mem::replace(&mut self.variant, variant);
        self.other_variants.push(previous);
        if self.other_variants.len() > MAX_OTHER_VARIANTS
            && let Some(oldest) = self
                .other_variants
                .iter()
                .position(|v| v.constants != self.pinned)
        {
            self.other_variants.remove(oldest);
        }
    }

    /// Build the pipelines for `constants` on a worker thread.
    fn start_build(&mut self, device: &wgpu::Device, constants: Vec<f64>) {
        let (sender, result) = mpsc::channel();
        let recipe = Arc::clone(&self.recipe);
        let device = device.clone();
        let job = constants.clone();
        #[cfg(test)]
        let gate = Arc::clone(&self.build_gate);
        let spawned = std::thread::Builder::new()
            .name("shader specialize".to_owned())
            .spawn(move || {
                #[cfg(test)]
                let _open = gate.read();
                let started = std::time::Instant::now();
                let outcome = recipe.build_checked(&device, &job);
                if outcome.is_ok() {
                    log::info!("specialized shader pipeline in {:.0?}", started.elapsed());
                }
                // Fails only when the pipeline was dropped, which discards the result.
                let _ = sender.send(outcome);
            });
        match spawned {
            Ok(_) => self.building = Some(Build { constants, result }),
            Err(e) => self.fail(constants, &e.to_string()),
        }
    }

    /// Use the worker's pipelines if its build has finished. Never blocks.
    fn finish_build(&mut self) {
        let Some(build) = &self.building else {
            return;
        };
        let outcome = match build.result.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("the build thread panicked".to_owned()),
            Ok(outcome) => outcome,
        };
        let Some(Build { constants, .. }) = self.building.take() else {
            return;
        };
        match outcome {
            // An offline frame may have built the same constants meanwhile.
            Ok(_) if self.constants() == constants.as_slice() => {}
            Ok(_) if self.other_variants.iter().any(|v| v.constants == constants) => {}
            Ok(pipelines) => self.install(Variant {
                constants,
                pipelines,
            }),
            Err(message) => self.fail(constants, &message),
        }
    }

    fn fail(&mut self, constants: Vec<f64>, message: &str) {
        log::warn!(
            "specializing shader pipeline for {constants:?} failed, keeping the current one: \
             {message}"
        );
        self.failed.push(constants);
    }

    /// The specialization constants the current pipelines were built with.
    pub fn constants(&self) -> &[f64] {
        &self.variant.constants
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

    /// The pipeline for pass `pass_index` writing `formats`, in attachment
    /// order.
    ///
    /// # Panics
    ///
    /// Panics if the pass was not in the plan the pipeline was built with,
    /// which is a caller bug.
    pub fn pipeline_for(
        &self,
        pass_index: i32,
        formats: &[wgpu::TextureFormat],
    ) -> &wgpu::RenderPipeline {
        let key = self.recipe.passes.specialize.then_some(pass_index);
        self.variant
            .pipelines
            .iter()
            .find(|(k, f, _)| *k == key && f == formats)
            .map(|(_, _, pipeline)| pipeline)
            .expect("the pass was planned when the pipeline was built")
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

impl Recipe {
    /// Every pipeline the passes need, for one set of input constants.
    fn build(&self, device: &wgpu::Device, constants: &[f64]) -> Pipelines {
        let output = (self.passes.output_index, vec![self.output_format]);
        let mut pipelines: Pipelines = Vec::new();
        for (index, formats) in self.passes.targeted.iter().chain(std::iter::once(&output)) {
            let key = self.passes.specialize.then_some(*index);
            if pipelines.iter().any(|(k, f, _)| *k == key && f == formats) {
                continue;
            }
            let mut values = constants.to_vec();
            if self.passes.specialize {
                values.push(f64::from(*index));
            }
            let pipeline = create_pipeline(
                device,
                &self.pipeline_layout,
                &self.vertex_module,
                &self.fragment_module,
                formats,
                &values,
                "ISF Unified Render Pipeline",
            );
            pipelines.push((key, formats.clone(), pipeline));
        }
        pipelines
    }

    /// [`Self::build`], returning the first GPU error it raised. Error scopes
    /// are per thread, so this catches only this build's errors.
    fn build_checked(
        &self,
        device: &wgpu::Device,
        constants: &[f64],
    ) -> std::result::Result<Pipelines, String> {
        let scopes = [
            wgpu::ErrorFilter::OutOfMemory,
            wgpu::ErrorFilter::Internal,
            wgpu::ErrorFilter::Validation,
        ]
        .map(|filter| device.push_error_scope(filter));
        let pipelines = self.build(device, constants);
        let errors: Vec<wgpu::Error> = scopes
            .into_iter()
            .rev()
            .filter_map(|scope| pollster::block_on(scope.pop()))
            .collect();
        match errors.first() {
            Some(error) => Err(error.to_string()),
            None => Ok(pipelines),
        }
    }
}

/// Fails unless the shader's specialization constants are numbered
/// `0..count`, one per `SPECIALIZE` input.
fn check_constant_ids(module: &naga::Module, count: usize) -> Result<()> {
    let mut ids: Vec<u16> = module.overrides.iter().filter_map(|(_, o)| o.id).collect();
    ids.sort_unstable();
    let expected: Vec<u16> = (0..count).filter_map(|i| u16::try_from(i).ok()).collect();
    if ids != expected {
        anyhow::bail!(
            "shader declares specialization constants {ids:?}; its {count} SPECIALIZE inputs \
             need constant_id 0 to {}",
            count.saturating_sub(1)
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // one per part of the pipeline descriptor
fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vertex: &wgpu::ShaderModule,
    fragment: &wgpu::ShaderModule,
    formats: &[wgpu::TextureFormat],
    constants: &[f64],
    label: &str,
) -> wgpu::RenderPipeline {
    // wgpu names a numbered override by its id in decimal.
    let names: Vec<String> = (0..constants.len()).map(|id| id.to_string()).collect();
    let constants: Vec<(&str, f64)> = names
        .iter()
        .map(String::as_str)
        .zip(constants.iter().copied())
        .collect();
    // 32-bit float targets are not blendable; no blend state means replace
    // for them too.
    let targets: Vec<Option<wgpu::ColorTargetState>> = formats
        .iter()
        .map(|&format| {
            let blendable = !matches!(
                format,
                wgpu::TextureFormat::Rgba32Float
                    | wgpu::TextureFormat::R32Float
                    | wgpu::TextureFormat::Rg32Float
            );
            Some(wgpu::ColorTargetState {
                format,
                blend: blendable.then_some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: vertex,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: fragment,
            entry_point: Some("main"),
            targets: &targets,
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A single-pass generator with one specialization constant.
    fn specialized_pipeline(gpu: &crate::renderer::context::GpuContext) -> UnifiedPipeline {
        let spirv = crate::isf::compile_glsl_to_spirv(
            "#version 450
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 fragColor;
layout(constant_id = 0) const int MODE = 1;
void main() {
    fragColor = vec4(float(MODE) * 0.25, 0.0, 0.0, 1.0);
}
",
            "specialized",
        )
        .expect("compiles");
        UnifiedPipeline::new(
            &gpu.device,
            &spirv,
            gpu.compositing_format,
            false,
            &[],
            &PassPlan::default(),
            0,
            &[],
            &[1.0],
        )
        .expect("pipeline")
    }

    /// Call `specialize` on live frames until `pipeline` uses `want`.
    fn live_until(pipeline: &mut UnifiedPipeline, device: &wgpu::Device, want: &[f64]) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while pipeline.constants() != want {
            assert!(Instant::now() < deadline, "{want:?} never built");
            std::thread::sleep(Duration::from_millis(5));
            pipeline.specialize(device, want.iter().copied(), true);
        }
    }

    /// Call `specialize` on live frames until no build runs.
    fn live_until_idle(pipeline: &mut UnifiedPipeline, device: &wgpu::Device, request: &[f64]) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while pipeline.building.is_some() {
            assert!(Instant::now() < deadline, "the build never finished");
            std::thread::sleep(Duration::from_millis(5));
            pipeline.specialize(device, request.iter().copied(), true);
        }
    }

    #[test]
    fn offline_frames_switch_on_the_call() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut pipeline = specialized_pipeline(&gpu);
        pipeline.specialize(&gpu.device, [3.0].into_iter(), false);
        assert_eq!(pipeline.constants(), [3.0]);
        pipeline.specialize(&gpu.device, [1.0].into_iter(), false);
        assert_eq!(pipeline.constants(), [1.0]);
    }

    #[test]
    fn live_frames_keep_the_current_pipelines_until_the_build_is_done() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut pipeline = specialized_pipeline(&gpu);
        let gate = Arc::clone(&pipeline.build_gate);
        let held = gate.write().expect("gate");
        for _ in 0..3 {
            pipeline.specialize(&gpu.device, [3.0].into_iter(), true);
            assert_eq!(pipeline.constants(), [1.0]);
        }
        drop(held);
        live_until(&mut pipeline, &gpu.device, &[3.0]);

        // A cached combination switches on the call.
        pipeline.specialize(&gpu.device, [1.0].into_iter(), true);
        assert_eq!(pipeline.constants(), [1.0]);
    }

    #[test]
    fn a_newer_request_during_a_build_supersedes_the_waiting_one() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut pipeline = specialized_pipeline(&gpu);
        let gate = Arc::clone(&pipeline.build_gate);
        let held = gate.write().expect("gate");
        pipeline.specialize(&gpu.device, [2.0].into_iter(), true);
        pipeline.specialize(&gpu.device, [3.0].into_iter(), true);
        pipeline.specialize(&gpu.device, [4.0].into_iter(), true);
        drop(held);
        live_until(&mut pipeline, &gpu.device, &[4.0]);

        // The running build was kept; the superseded request was never built.
        let built: Vec<&[f64]> = pipeline
            .other_variants
            .iter()
            .map(|v| v.constants.as_slice())
            .collect();
        assert_eq!(built, [[1.0].as_slice(), &[2.0]]);
    }

    #[test]
    fn the_default_variant_is_never_evicted() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut pipeline = specialized_pipeline(&gpu);
        for value in 2..(MAX_OTHER_VARIANTS + 4) {
            pipeline.specialize(&gpu.device, [value as f64].into_iter(), false);
        }
        assert!(pipeline.other_variants.len() <= MAX_OTHER_VARIANTS);
        assert!(
            pipeline.other_variants.iter().any(|v| v.constants == [1.0]),
            "the defaults stay cached"
        );
        assert!(
            !pipeline.other_variants.iter().any(|v| v.constants == [2.0]),
            "the oldest other variant was evicted"
        );

        // Switching back is instant on a live frame.
        pipeline.specialize(&gpu.device, [1.0].into_iter(), true);
        assert_eq!(pipeline.constants(), [1.0]);
        assert!(pipeline.building.is_none());
    }

    #[test]
    fn a_failed_build_keeps_the_current_pipelines() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut pipeline = specialized_pipeline(&gpu);
        // The shader declares one constant; a second one fails validation.
        let bad = [3.0, 7.0];
        pipeline.specialize(&gpu.device, bad.into_iter(), true);
        assert!(pipeline.building.is_some());
        live_until_idle(&mut pipeline, &gpu.device, &bad);
        assert_eq!(pipeline.constants(), [1.0]);

        // Not retried.
        pipeline.specialize(&gpu.device, bad.into_iter(), true);
        assert!(pipeline.building.is_none());
        assert_eq!(gpu.errors.fault_count(), 0);
    }
}
