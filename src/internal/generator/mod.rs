//! ISF generators as a deck source: fragment shaders (single and multi-pass)
//! and GLSL compute shaders.
//!
//! The shader's `INPUTS` become the deck's generator parameters, owned by the
//! deck, which provides MIDI, OSC, modulation, presets and exploration.

pub mod pass;

pub use pass::{
    PassBindings, PassBuffer, PassSet, create_preprocessor_slots, get_current_date,
    load_imported_textures, parse_size_expression, preprocessor_filterability,
};

use crate::isf::{ISFMetadata, ISFShader, compile_glsl_compute_to_spirv, compile_glsl_to_spirv};
use crate::renderer::{ComputePipeline, DispatchMode, GpuContext, UnifiedPipeline};
use crate::source::{
    DeckSourceInstance, DeckSourceProvider, LibraryEntry, LibrarySection, PreprocessorSlot,
    SourceConfig, SourceEnv, SourceFrame, SourceLoader, SourceQuery,
};
use anyhow::{Context, Result};
use std::path::Path;

pub const SOURCE_TYPE: &str = "Shader";

/// ISF generator shaders.
pub struct ShaderProvider;

impl ShaderProvider {
    /// The shader a config names: a file path, or a library shader's name.
    fn resolve(config: &SourceConfig, query: &SourceQuery) -> Result<ISFShader> {
        let key = config
            .str("path")
            .or_else(|| config.str("name"))
            .context("a shader source needs a `path` or a `name`")?;
        if Path::new(key).is_file() {
            return ISFShader::from_file(key)
                .with_context(|| format!("Failed to load shader: {key}"));
        }
        query
            .shaders
            .generators()
            .into_iter()
            .find(|s| s.name() == key)
            .cloned()
            .with_context(|| format!("Shader not found: {key}"))
    }

    /// Refuse a shader whose required preprocessors this engine cannot run,
    /// or whose depth sensor is missing, before anything is built.
    ///
    /// Unknown and optional types are allowed and fall back to default outputs.
    fn preflight(metadata: &ISFMetadata, name: &str, query: &SourceQuery) -> Result<()> {
        let analyzers = crate::depth::preprocess::register(crate::analyzer::default_registry());
        for pp in &metadata.preprocessors {
            let ty = pp.preprocessor_type.as_str();
            let Some(category) = analyzers.category_for(ty) else {
                log::warn!(
                    "Shader '{name}' declares unknown preprocessor '{ty}'; its outputs will be blank"
                );
                continue;
            };
            if category.is_required() && ty != crate::depth::preprocess::PREPROCESSOR_TYPE {
                anyhow::bail!(
                    "Shader '{name}' requires preprocessor '{ty}', which this build cannot provide."
                );
            }
        }
        if crate::depth::preprocess::requested_device(metadata).is_some() {
            let depth = query
                .services
                .get::<crate::depth::DepthSensorManager>()
                .with_context(|| format!("Shader '{name}' needs a depth sensor: none detected"))?;
            crate::depth::preprocess::preflight_for_shader(depth, metadata, name)?;
        }
        Ok(())
    }
}

impl DeckSourceProvider for ShaderProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Generators"
    }

    fn icon(&self) -> &'static str {
        "🎨"
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        LibrarySection {
            entries: query
                .shaders
                .generators()
                .into_iter()
                .map(|s| LibraryEntry::new(s.name(), Shader::config_for(s)))
                .collect(),
            ..LibrarySection::default()
        }
    }

    fn loader(&self, config: &SourceConfig, query: &SourceQuery) -> Option<Result<SourceLoader>> {
        Some(Self::resolve(config, query).and_then(|shader| {
            Self::preflight(&shader.metadata, &shader.name(), query)?;
            let loader: SourceLoader = Box::new(move |gpu, width, height| {
                Ok(Box::new(Shader::new(gpu, shader, width, height)?)
                    as Box<dyn DeckSourceInstance>)
            });
            Ok(loader)
        }))
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let loader = self
            .loader(config, &env.query())
            .context("shader source has a loader")??;
        loader(env.gpu, env.width, env.height)
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config
            .get("path")
            .or_else(|| config.get("name"))
            .cloned()
            .unwrap_or_default()
    }
}

enum Program {
    Fragment {
        pipeline: UnifiedPipeline,
        passes: PassSet,
        /// Loaded from ISF `IMPORTED`, sorted by name for deterministic binding.
        imported_textures: Vec<(String, wgpu::Texture, wgpu::TextureView)>,
        preprocessor_slots: Vec<PreprocessorSlot>,
    },
    Compute {
        pipeline: ComputePipeline,
    },
}

/// One generator deck.
pub struct Shader {
    shader: ISFShader,
    program: Program,
}

impl Shader {
    /// Compile `shader` for a `width × height` deck.
    ///
    /// # Errors
    ///
    /// Fails when the GLSL does not compile to SPIR-V or the pipeline cannot
    /// be created, or a compute shader has no `COMPUTE` block.
    pub fn new(gpu: &GpuContext, shader: ISFShader, width: u32, height: u32) -> Result<Self> {
        let program = if shader.metadata.is_compute() {
            Self::compute(gpu, &shader, width, height)?
        } else {
            Self::fragment(gpu, &shader, width, height)?
        };
        Ok(Self { shader, program })
    }

    fn fragment(gpu: &GpuContext, shader: &ISFShader, width: u32, height: u32) -> Result<Program> {
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile shader to SPIR-V")?;
        let passes = shader.metadata.passes.clone().unwrap_or_default();
        // Targets without a `FORMAT` use the color-path format; ISF
        // `"FLOAT": true` does not change it.
        let default_format = gpu.compositing_format;
        let imported_textures =
            load_imported_textures(&shader.metadata, shader.file_path.as_deref(), gpu);
        let preprocessor_slots = create_preprocessor_slots(gpu, &shader.metadata);
        let pipeline = UnifiedPipeline::new(
            &gpu.device,
            &spirv,
            gpu.compositing_format,
            false,
            &PassSet::binding_filterability(&passes, default_format),
            &PassSet::plan(&passes, default_format, shader.metadata.specialize_passes),
            imported_textures.len(),
            &preprocessor_filterability(&preprocessor_slots),
            &specialization_defaults(shader),
        )
        .context("Failed to create shader pipeline")?;
        let passes = PassSet::new(gpu, passes, width, height, default_format, "Pass Buffer");
        Ok(Program::Fragment {
            pipeline,
            passes,
            imported_textures,
            preprocessor_slots,
        })
    }

    fn compute(gpu: &GpuContext, shader: &ISFShader, width: u32, height: u32) -> Result<Program> {
        let compute = shader
            .metadata
            .compute
            .as_ref()
            .context("Compute shader missing COMPUTE configuration")?;
        let spirv = compile_glsl_compute_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile compute shader to SPIR-V")?;
        if compute.dispatch == "custom" {
            log::warn!("Custom dispatch mode not yet implemented, using resolution");
        }
        let pipeline = ComputePipeline::new(
            &gpu.device,
            &spirv,
            width,
            height,
            &shader.metadata.buffers,
            compute.workgroup_size,
            DispatchMode::Resolution,
            compute.num_passes,
        )
        .context("Failed to create compute pipeline")?;
        Ok(Program::Compute { pipeline })
    }

    /// A config for a deck of `shader`.
    pub fn config_for(shader: &ISFShader) -> SourceConfig {
        let config = SourceConfig::new(SOURCE_TYPE);
        match &shader.file_path {
            Some(path) => config.with("path", path),
            None => config.with("name", shader.name()),
        }
    }

    fn render_fragment(
        frame: &mut SourceFrame,
        pipeline: &mut UnifiedPipeline,
        passes: &mut PassSet,
        imported: &[&wgpu::TextureView],
        preprocessors: &[&wgpu::TextureView],
    ) {
        const SIMULATION_ITERATIONS: usize = 4;
        frame.params.ensure_buffer(&frame.gpu.device);
        frame.params.update_buffer_with_modulation(
            &frame.gpu.queue,
            frame.modulation,
            Some(frame.param_prefix),
        );
        pipeline.specialize(
            &frame.gpu.device,
            frame.params.specialization_constants(),
            frame.live,
        );
        passes.update_sizes(frame.gpu, &|name| frame.params.base_number(name));
        let pipeline = &*pipeline;
        let Some(user_params) = frame.params.buffer() else {
            return;
        };
        let size = [frame.width as f32, frame.height as f32];

        // One uniform slot per pass iteration plus one for the final pass. The
        // targeted passes share one command buffer, submitted now so the GPU
        // starts early; the final pass joins the frame's batch. A single-pass
        // shader is the final pass alone.
        pipeline.ensure_pass_slots(
            &frame.gpu.device,
            passes.uniform_slots(SIMULATION_ITERATIONS),
        );
        let bindings = PassBindings {
            device: &frame.gpu.device,
            queue: &frame.gpu.queue,
            pipeline,
            input: None,
            imported,
            preprocessors,
            user_params,
        };
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Multi-pass Encoder"),
                });
        let (audio, time, time_delta, frame_index, phase_times) = (
            frame.audio,
            frame.time,
            frame.time_delta,
            frame.frame_index,
            frame.phase_times,
        );
        // A persistent pass substeps with a fraction of the frame's delta and
        // its own frame index. RENDERSIZE is the pass buffer's own size, so
        // shaders that store per-pixel state can address their own texels.
        let slot = passes.encode_targeted(&mut encoder, &bindings, SIMULATION_ITERATIONS, |step| {
            let (delta, index) = if step.substeps > 1 {
                (
                    time_delta / step.substeps as f32,
                    frame_index * step.substeps as u32 + step.substep as u32,
                )
            } else {
                (time_delta, frame_index)
            };
            let mut uniforms = pass::uniforms(
                audio,
                time,
                delta,
                index,
                step.pass_index,
                step.size,
                phase_times,
            );
            // Jitter follows the rendered frame, not the substep.
            uniforms.jitter = pass::jitter(frame_index);
            uniforms.jitter_index = i32::try_from(frame_index % pass::JITTER_CYCLE).unwrap_or(0);
            uniforms
        });
        if slot > 0 {
            frame.gpu.submit(std::iter::once(encoder.finish()));
            encoder = frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Multi-pass Final Encoder"),
                });
        }
        let uniforms = pass::uniforms(
            audio,
            time,
            time_delta,
            frame_index,
            passes.output_index(),
            size,
            phase_times,
        );
        passes.encode_output(&mut encoder, &bindings, slot, uniforms, frame.target);
        frame.cmd_buffers.push(encoder.finish());
        passes.finish_frame();
    }

    fn render_compute(frame: &mut SourceFrame, pipeline: &ComputePipeline) {
        frame.params.ensure_buffer(&frame.gpu.device);
        frame.params.update_buffer_with_modulation(
            &frame.gpu.queue,
            frame.modulation,
            Some(frame.param_prefix),
        );
        let Some(user_params) = frame.params.buffer() else {
            return;
        };
        let (dx, dy, dz) = pipeline.dispatch_counts(frame.width, frame.height);
        // All passes encode into one command buffer, each in its own compute
        // pass with its own uniform slot; wgpu orders their storage accesses.
        pipeline.ensure_pass_slots(&frame.gpu.device);
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Compute Shader Dispatch Encoder"),
                });
        pipeline.clear_non_persistent_buffers(&mut encoder);
        for pass_idx in 0..pipeline.num_passes {
            let slot = pass_idx as usize;
            let uniforms = pass::uniforms(
                frame.audio,
                frame.time,
                frame.time_delta,
                frame.frame_index,
                slot,
                [frame.width as f32, frame.height as f32],
                frame.phase_times,
            );
            pipeline.write_pass_uniforms(&frame.gpu.queue, slot, &uniforms);
            let bind_group =
                pipeline.create_pass_bind_group(&frame.gpu.device, slot, Some(user_params));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Compute Shader Pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.compute_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(dx, dy, dz);
        }
        // Submitted now, not with the frame's batch, so the GPU starts early.
        frame.gpu.submit(std::iter::once(encoder.finish()));

        let mut copy = frame
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Compute Output Copy Encoder"),
            });
        copy.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &pipeline.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: frame.target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        frame.cmd_buffers.push(copy.finish());
    }
}

impl DeckSourceInstance for Shader {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        self.shader.name()
    }

    fn config(&self) -> SourceConfig {
        Self::config_for(&self.shader)
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        match &mut self.program {
            Program::Fragment {
                pipeline,
                passes,
                imported_textures,
                preprocessor_slots,
            } => {
                let imported: Vec<&wgpu::TextureView> =
                    imported_textures.iter().map(|(_, _, v)| v).collect();
                let preprocessors: Vec<&wgpu::TextureView> =
                    preprocessor_slots.iter().map(|s| &s.view).collect();
                Self::render_fragment(frame, pipeline, passes, &imported, &preprocessors);
            }
            Program::Compute { pipeline } => Self::render_compute(frame, pipeline),
        }
        Ok(())
    }

    /// Pass buffers follow the deck's size. `HISTORY` targets start over.
    fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if let Program::Fragment { passes, .. } = &mut self.program {
            passes.resize(gpu, width, height);
        }
    }

    fn shader(&self) -> Option<&ISFShader> {
        Some(&self.shader)
    }

    fn preprocessor_slots(&self) -> &[PreprocessorSlot] {
        match &self.program {
            Program::Fragment {
                preprocessor_slots, ..
            } => preprocessor_slots,
            Program::Compute { .. } => &[],
        }
    }

    fn preprocessor_slots_mut(&mut self) -> Option<&mut Vec<PreprocessorSlot>> {
        match &mut self.program {
            Program::Fragment {
                preprocessor_slots, ..
            } => Some(preprocessor_slots),
            Program::Compute { .. } => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// The specialization constants of `shader`'s `SPECIALIZE` inputs at their
/// defaults.
pub fn specialization_defaults(shader: &ISFShader) -> Vec<f64> {
    let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
    crate::params::ShaderParams::from_inputs(inputs)
        .specialization_constants()
        .collect()
}
