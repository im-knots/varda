//! ISF generators as a deck source: fragment shaders (single and multi-pass)
//! and GLSL compute shaders.
//!
//! The shader's `INPUTS` become the deck's generator parameters, owned by the
//! deck, which provides MIDI, OSC, modulation, presets and exploration.

pub mod pass;

pub use pass::{
    PassBuffer, create_pass_buffers, create_preprocessor_slots, get_current_date,
    load_imported_textures, parse_size_expression, preprocessor_filterability,
};

use crate::isf::{
    ISFMetadata, ISFPass, ISFShader, compile_glsl_compute_to_spirv, compile_glsl_to_spirv,
};
use crate::renderer::{ComputePipeline, DispatchMode, GpuContext, UnifiedPipeline};
use crate::source::{
    DeckSourceInstance, DeckSourceProvider, LibraryEntry, LibrarySection, PreprocessorSlot,
    SourceConfig, SourceEnv, SourceFrame, SourceLoader, SourceQuery,
};
use anyhow::{Context, Result};
use std::collections::HashMap;
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
        pass_buffers: HashMap<String, PassBuffer>,
        passes: Vec<ISFPass>,
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
        // All pass buffers use the color-path format; ISF `"FLOAT": true` does
        // not change it.
        let pass_buffers = create_pass_buffers(
            gpu,
            &passes,
            width,
            height,
            gpu.compositing_format,
            "Pass Buffer",
        );
        let imported_textures =
            load_imported_textures(&shader.metadata, shader.file_path.as_deref(), gpu);
        let preprocessor_slots = create_preprocessor_slots(gpu, &shader.metadata);
        let pipeline = UnifiedPipeline::new(
            &gpu.device,
            &spirv,
            gpu.compositing_format,
            false,
            pass_buffers.len(),
            imported_textures.len(),
            &preprocessor_filterability(&preprocessor_slots),
        )
        .context("Failed to create shader pipeline")?;
        Ok(Program::Fragment {
            pipeline,
            pass_buffers,
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
        pipeline: &UnifiedPipeline,
        pass_buffers: &mut HashMap<String, PassBuffer>,
        passes: &[ISFPass],
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
        let Some(user_params) = frame.params.buffer() else {
            return;
        };
        let size = [frame.width as f32, frame.height as f32];

        if pipeline.num_pass_buffers == 0 {
            let uniforms = pass::uniforms(
                frame.audio,
                frame.time,
                frame.time_delta,
                frame.frame_index,
                0,
                size,
                frame.phase_times,
            );
            pipeline.update_uniforms(&frame.gpu.queue, &uniforms);
            let bind_group = pipeline.create_bind_group(
                &frame.gpu.device,
                None,
                &[],
                imported,
                preprocessors,
                Some(user_params),
            );
            let mut encoder =
                frame
                    .gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Deck Source Render Encoder"),
                    });
            draw_pass(
                &mut encoder,
                pipeline,
                &bind_group,
                frame.target,
                "Deck Source Render Pass",
            );
            frame.cmd_buffers.push(encoder.finish());
            return;
        }

        let iterations_of = |pass: &ISFPass| {
            if pass.persistent.unwrap_or(false) {
                SIMULATION_ITERATIONS
            } else {
                1
            }
        };
        // One uniform slot per pass iteration plus one for the final pass. The
        // targeted passes share one command buffer, submitted now so the GPU
        // starts early; the final pass joins the frame's batch.
        let slots = passes
            .iter()
            .filter(|p| p.target.is_some())
            .map(iterations_of)
            .sum::<usize>()
            + 1;
        pipeline.ensure_pass_slots(&frame.gpu.device, slots);
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Multi-pass Encoder"),
                });
        let mut slot = 0;
        for (pass_idx, pass) in passes.iter().enumerate() {
            let Some(target_name) = &pass.target else {
                continue;
            };
            // The pass buffer's own size is RENDERSIZE, so shaders that store
            // per-pixel state can address their own texels.
            let pass_size = pass_buffers.get(target_name).map_or(size, |pb| {
                let sz = pb.texture_a.size();
                [sz.width as f32, sz.height as f32]
            });
            for iter in 0..iterations_of(pass) {
                let uniforms = pass::uniforms(
                    frame.audio,
                    frame.time,
                    frame.time_delta / SIMULATION_ITERATIONS as f32,
                    frame.frame_index * SIMULATION_ITERATIONS as u32 + iter as u32,
                    pass_idx,
                    pass_size,
                    frame.phase_times,
                );
                pipeline.write_pass_uniforms(&frame.gpu.queue, slot, &uniforms);
                let target = pass_buffers
                    .get(target_name)
                    .map_or(frame.target, PassBuffer::write_view);
                encode_multi_pass(
                    &frame.gpu.device,
                    pipeline,
                    &mut encoder,
                    slot,
                    passes,
                    pass_buffers,
                    imported,
                    preprocessors,
                    user_params,
                    target,
                );
                slot += 1;
                if let Some(pb) = pass_buffers.get_mut(target_name) {
                    pb.swap();
                }
            }
        }
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
            frame.audio,
            frame.time,
            frame.time_delta,
            frame.frame_index,
            passes.len(),
            size,
            frame.phase_times,
        );
        pipeline.write_pass_uniforms(&frame.gpu.queue, slot, &uniforms);
        encode_multi_pass(
            &frame.gpu.device,
            pipeline,
            &mut encoder,
            slot,
            passes,
            pass_buffers,
            imported,
            preprocessors,
            user_params,
            frame.target,
        );
        frame.cmd_buffers.push(encoder.finish());
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

fn draw_pass(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &UnifiedPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
    label: &str,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
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
    pass.set_pipeline(&pipeline.pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// Encode one pass of a multi-pass generator into `target`, reading its
/// uniforms from `slot` and every pass buffer's current contents.
#[allow(clippy::too_many_arguments)] // the pass's bindings, all borrowed from different owners
fn encode_multi_pass(
    device: &wgpu::Device,
    pipeline: &UnifiedPipeline,
    encoder: &mut wgpu::CommandEncoder,
    slot: usize,
    passes: &[ISFPass],
    pass_buffers: &HashMap<String, PassBuffer>,
    imported: &[&wgpu::TextureView],
    preprocessors: &[&wgpu::TextureView],
    user_params: &wgpu::Buffer,
    target: &wgpu::TextureView,
) {
    let pass_views: Vec<&wgpu::TextureView> = passes
        .iter()
        .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
        .map(PassBuffer::read_view)
        .collect();
    let bind_group = pipeline.create_pass_bind_group(
        device,
        slot,
        None,
        &pass_views,
        imported,
        preprocessors,
        Some(user_params),
    );
    draw_pass(encoder, pipeline, &bind_group, target, "Multi-pass Render");
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
                pass_buffers,
                passes,
                imported_textures,
                preprocessor_slots,
            } => {
                let imported: Vec<&wgpu::TextureView> =
                    imported_textures.iter().map(|(_, _, v)| v).collect();
                let preprocessors: Vec<&wgpu::TextureView> =
                    preprocessor_slots.iter().map(|s| &s.view).collect();
                Self::render_fragment(
                    frame,
                    pipeline,
                    pass_buffers,
                    passes,
                    &imported,
                    &preprocessors,
                );
            }
            Program::Compute { pipeline } => Self::render_compute(frame, pipeline),
        }
        Ok(())
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
