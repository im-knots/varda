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

impl Effect {
    /// Create a new effect from an ISF filter shader, targeting `compositing_format` like channel
    /// and master effects.
    ///
    /// # Errors
    ///
    /// Returns an error if the ISF fragment source fails to compile to SPIR-V,
    /// or if the render pipeline cannot be created for the target format.
    pub fn new(context: &GpuContext, shader: ISFShader) -> Result<Self> {
        Self::new_with_format(context, shader, context.compositing_format)
    }

    /// Create a new effect with a specific target format
    ///
    /// # Errors
    ///
    /// Returns an error if the ISF fragment source fails to compile to SPIR-V,
    /// or if the render pipeline cannot be created for `target_format`.
    pub fn new_with_format(
        context: &GpuContext,
        shader: ISFShader,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile filter shader to SPIR-V")?;

        let passes: Vec<ISFPass> = shader.metadata.passes.clone().unwrap_or_default();

        // Load ISF IMPORTED images
        let imported_textures =
            load_imported_textures(&shader.metadata, shader.file_path.as_deref(), context);

        // Create preprocessor texture slots from ISF PREPROCESSORS declarations
        let preprocessor_textures = create_preprocessor_slots(context, &shader.metadata);
        let preprocessor_filterable = preprocessor_filterability(&preprocessor_textures);
        let pipeline = UnifiedPipeline::new(
            &context.device,
            &spirv,
            target_format,
            true, // has_input_image, since it's a filter
            &PassSet::binding_filterability(&passes, target_format),
            &PassSet::target_formats(&passes, target_format),
            imported_textures.len(),
            &preprocessor_filterable,
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

        let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
        let params = ShaderParams::from_inputs(inputs);
        let phase_inputs_config = shader.metadata.phase_inputs.clone();

        let uuid = crate::ids::generate_short_uuid();
        let param_prefix = crate::engine::value::param::effect_param_prefix(&uuid);

        Ok(Self {
            uuid,
            param_prefix,
            shader,
            pipeline,
            enabled: true,
            params,
            passes,
            target_format,
            imported_textures,
            preprocessor_textures,
            phase_accumulators: [0.0; 4],
            phase_inputs_config,
        })
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

    /// Apply this effect to an input texture, writing to the target texture. Optionally modulates
    /// parameters under the given prefix.
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
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        self.apply_with_modulation(
            context,
            input_view,
            output_view,
            uniforms,
            None,
            cmd_buffers,
        )
    }

    /// Apply this effect with modulation support
    ///
    /// # Errors
    ///
    /// Never fails; the `Result` leaves room for a fallible step such as pipeline recreation.
    ///
    /// # Panics
    ///
    /// Panics if the user parameter buffer is absent immediately after
    /// `ensure_buffer` created it.
    pub fn apply_with_modulation(
        &mut self,
        context: &GpuContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        modulation: Option<&crate::modulation::ModulationEngine>,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

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
        let user_params_buffer = self
            .params
            .buffer()
            .expect("Buffer should exist after ensure_buffer");

        let imported_views: Vec<&wgpu::TextureView> =
            self.imported_textures.iter().map(|(_, _, v)| v).collect();
        let preprocessor_views: Vec<&wgpu::TextureView> = self
            .preprocessor_textures
            .iter()
            .map(|pp| &pp.view)
            .collect();

        // Targeted passes, then the output pass, each with its own uniform
        // slot, encoded into one command buffer. A single-pass effect is the
        // output pass alone. Persistent passes run once per frame here.
        self.pipeline
            .ensure_pass_slots(&context.device, self.passes.uniform_slots(1));
        let bindings = PassBindings {
            device: &context.device,
            queue: &context.queue,
            pipeline: &self.pipeline,
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
        let slot = self
            .passes
            .encode_targeted(&mut encoder, &bindings, 1, |step| {
                let mut pass_uniforms = *uniforms;
                pass_uniforms.pass_index = i32::try_from(step.pass_index).unwrap_or(i32::MAX);
                pass_uniforms
            });
        let mut final_uniforms = *uniforms;
        if self.passes.has_targeted_passes() {
            final_uniforms.pass_index =
                i32::try_from(self.passes.passes().len()).unwrap_or(i32::MAX);
        }
        self.passes
            .encode_output(&mut encoder, &bindings, slot, final_uniforms, output_view);
        cmd_buffers.push(encoder.finish());
        self.passes.finish_frame();

        Ok(())
    }
}
