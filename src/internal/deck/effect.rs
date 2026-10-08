//! Effect (ISF filter) implementation.

use super::Effect;
use crate::generator::{
    PassBindings, PassSet, create_preprocessor_slots, load_imported_textures,
    preprocessor_filterability,
};
use crate::isf::{ISFPass, ISFShader, compile_glsl_to_spirv};
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, ISFUniforms, UnifiedPipeline};
use anyhow::{Context, Result};

/// An effect's compiled shader and GPU resources.
pub(super) struct Program {
    pipeline: UnifiedPipeline,
    passes: PassSet,
    /// Textures loaded from ISF `IMPORTED` images, sorted by name.
    imported_textures: Vec<(String, wgpu::Texture, wgpu::TextureView)>,
}

/// Where an effect's program is.
pub(super) enum ProgramState {
    Building(std::sync::mpsc::Receiver<Result<Program>>),
    Ready(Box<Program>),
    Failed(String),
}

/// Compile `shader` and build its pipeline and resources.
fn build_program(
    context: &GpuContext,
    shader: &ISFShader,
    target_format: wgpu::TextureFormat,
    preprocessor_filterable: &[bool],
) -> Result<Program> {
    let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
        .context("Failed to compile filter shader to SPIR-V")?;
    let passes: Vec<ISFPass> = shader.metadata.passes.clone().unwrap_or_default();
    let imported_textures =
        load_imported_textures(&shader.metadata, shader.file_path.as_deref(), context);
    let pipeline = UnifiedPipeline::new(
        &context.device,
        &spirv,
        target_format,
        true, // has_input_image, since it's a filter
        &PassSet::binding_filterability(&passes, target_format),
        &PassSet::plan(&passes, target_format, shader.metadata.specialize_passes),
        imported_textures.len(),
        preprocessor_filterable,
        &crate::generator::specialization_defaults(shader),
    )
    .context("Failed to create effect pipeline")?;
    // Pass buffers use the effect's target format, sized at the internal resolution.
    let passes = PassSet::new(
        context,
        passes,
        1920,
        1080,
        target_format,
        "Effect Pass Buffer",
    );
    Ok(Program {
        pipeline,
        passes,
        imported_textures,
    })
}

impl Effect {
    /// Build an effect from an ISF filter shader, targeting `compositing_format` like channel and
    /// master effects, before returning. For tests and benches; the engine uses
    /// [`Self::pending`].
    ///
    /// # Errors
    ///
    /// Returns an error if the ISF fragment source fails to compile to SPIR-V,
    /// or if the render pipeline cannot be created for the target format.
    pub fn new(context: &GpuContext, shader: ISFShader) -> Result<Self> {
        Self::new_with_format(context, shader, context.compositing_format)
    }

    /// [`Self::new`] for a specific target format.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn new_with_format(
        context: &GpuContext,
        shader: ISFShader,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let preprocessor_textures = create_preprocessor_slots(context, &shader.metadata);
        let program = build_program(
            context,
            &shader,
            target_format,
            &preprocessor_filterability(&preprocessor_textures),
        )?;
        Ok(Self::with_program(
            shader,
            target_format,
            preprocessor_textures,
            ProgramState::Ready(Box::new(program)),
        ))
    }

    /// An effect whose program builds on a worker thread. It is in its chain at once, and every
    /// command can address it; it draws from the first frame after [`Self::poll_build`] finds it
    /// ready.
    pub fn pending(
        context: &GpuContext,
        shader: ISFShader,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let preprocessor_textures = create_preprocessor_slots(context, &shader.metadata);
        let filterable = preprocessor_filterability(&preprocessor_textures);
        let build = {
            let context = context.clone();
            let shader = shader.clone();
            crate::renderer::builds::spawn(move || {
                build_program(&context, &shader, target_format, &filterable)
            })
        };
        Self::with_program(
            shader,
            target_format,
            preprocessor_textures,
            ProgramState::Building(build),
        )
    }

    fn with_program(
        shader: ISFShader,
        target_format: wgpu::TextureFormat,
        preprocessor_textures: Vec<super::PreprocessorSlot>,
        program: ProgramState,
    ) -> Self {
        let params = ShaderParams::from_metadata(&shader.metadata);
        let phase_inputs_config = shader.metadata.phase_inputs.clone();
        let uuid = crate::ids::generate_short_uuid();
        let param_prefix = crate::engine::value::param::effect_param_prefix(&uuid);
        Self {
            uuid,
            param_prefix,
            shader,
            program,
            enabled: true,
            params,
            target_format,
            preprocessor_textures,
            phase_accumulators: [0.0; 4],
            phase_inputs_config,
        }
    }

    /// Take a finished build, if one arrived. Call once per frame before drawing.
    pub fn poll_build(&mut self) {
        let ProgramState::Building(build) = &self.program else {
            return;
        };
        self.program = match build.try_recv() {
            Ok(Ok(program)) => ProgramState::Ready(Box::new(program)),
            Ok(Err(e)) => {
                log::warn!("Effect '{}' failed to build: {e:#}", self.shader.name());
                ProgramState::Failed(format!("{e:#}"))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                ProgramState::Failed("the build stopped".to_string())
            }
        };
    }

    /// Where the effect's build is, for the snapshot.
    pub fn status(&self) -> crate::engine::value::effect::EffectStatus {
        use crate::engine::value::effect::EffectStatus;
        match &self.program {
            ProgramState::Building(_) => EffectStatus::Building,
            ProgramState::Ready(_) => EffectStatus::Ready,
            ProgramState::Failed(message) => EffectStatus::Failed {
                message: message.clone(),
            },
        }
    }

    /// Whether the effect draws this frame: enabled and built.
    pub fn is_active(&self) -> bool {
        self.enabled && matches!(self.program, ProgramState::Ready(_))
    }

    /// This effect's stable UUID.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// The cached `effect/<uuid>/param` prefix modulation targets are keyed under.
    pub fn param_prefix(&self) -> &str {
        &self.param_prefix
    }

    /// Set the UUID during scene restore. Also rebuilds the modulation prefix, since assignments
    /// are keyed by `effect/<uuid>/param/<param>`.
    pub fn set_uuid(&mut self, uuid: String) {
        self.param_prefix = crate::engine::value::param::effect_param_prefix(&uuid);
        self.uuid = uuid;
    }

    /// Apply this effect to an input texture, writing to the target texture. `live` is whether
    /// the wall clock paces the frame.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Effect::apply_with_modulation`], which currently never fails.
    pub fn apply(
        &mut self,
        context: &GpuContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        live: bool,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        self.apply_with_modulation(
            context,
            input_view,
            output_view,
            uniforms,
            None,
            live,
            cmd_buffers,
        )
    }

    /// Apply this effect with modulation support. Its events reset to false afterwards. A `live`
    /// frame builds new specialized pipelines in the background (see
    /// [`UnifiedPipeline::specialize`]).
    ///
    /// # Errors
    ///
    /// Never fails; the `Result` leaves room for a fallible step such as pipeline recreation.
    ///
    /// # Panics
    ///
    /// Panics if the user parameter buffer is absent immediately after
    /// `ensure_buffer` created it.
    // One per frame input the effect reads.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_modulation(
        &mut self,
        context: &GpuContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        modulation: Option<&crate::modulation::ModulationEngine>,
        live: bool,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let ProgramState::Ready(program) = &mut self.program else {
            return Ok(());
        };

        // Upload user params, modulated when available.
        self.params.ensure_buffer(&context.device);
        if let Some(mod_engine) = modulation {
            self.params.update_buffer_with_modulation(
                &context.queue,
                mod_engine,
                Some(&self.param_prefix),
            );
        } else {
            self.params.update_buffer(&context.queue);
        }
        program.pipeline.specialize(
            &context.device,
            self.params.specialization_constants(),
            live,
        );
        let params = &self.params;
        program
            .passes
            .update_sizes(context, &|name| params.base_number(name));
        let user_params_buffer = self
            .params
            .buffer()
            .expect("Buffer should exist after ensure_buffer");

        let imported_views: Vec<&wgpu::TextureView> = program
            .imported_textures
            .iter()
            .map(|(_, _, v)| v)
            .collect();
        let preprocessor_views: Vec<&wgpu::TextureView> = self
            .preprocessor_textures
            .iter()
            .map(|pp| &pp.view)
            .collect();

        // Targeted passes, then the output pass, each with its own uniform
        // slot, encoded into one command buffer. A single-pass effect is the
        // output pass alone. Persistent passes run once per frame here.
        program
            .pipeline
            .ensure_pass_slots(&context.device, program.passes.uniform_slots(1));
        let bindings = PassBindings {
            device: &context.device,
            queue: &context.queue,
            pipeline: &program.pipeline,
            input: Some(input_view),
            imported: &imported_views,
            preprocessors: &preprocessor_views,
            user_params: user_params_buffer,
        };
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Effect Encoder"),
            });
        let slot = program
            .passes
            .encode_targeted(&mut encoder, &bindings, 1, |step| {
                let mut pass_uniforms = *uniforms;
                pass_uniforms.pass_index = i32::try_from(step.pass_index).unwrap_or(i32::MAX);
                pass_uniforms
            });
        let mut final_uniforms = *uniforms;
        if program.passes.has_targeted_passes() {
            final_uniforms.pass_index =
                i32::try_from(program.passes.output_index()).unwrap_or(i32::MAX);
        }
        program
            .passes
            .encode_output(&mut encoder, &bindings, slot, final_uniforms, output_view);
        cmd_buffers.push(encoder.finish());
        program.passes.finish_frame();
        self.params.clear_events();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shader whose GLSL does not compile builds into a failed effect that
    /// stays in the chain and is skipped.
    #[test]
    fn a_failed_build_is_reported_and_skipped() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let shader = ISFShader::from_string(
            r#"/*{"ISFVSN": "2", "CATEGORIES": ["Test"], "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}]}*/
void main() { gl_FragColor = not_a_function(); }"#,
        )
        .expect("parses");
        let mut effect = Effect::pending(&gpu, shader, gpu.compositing_format);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while effect.status() == crate::engine::value::effect::EffectStatus::Building {
            assert!(
                std::time::Instant::now() < deadline,
                "the build never finished"
            );
            effect.poll_build();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(
            effect.status(),
            crate::engine::value::effect::EffectStatus::Failed { .. }
        ));
        assert!(!effect.is_active());
    }
}
