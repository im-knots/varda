//! Effect (ISF filter) implementation.

use super::Effect;
use crate::generator::{
    create_pass_buffers, create_preprocessor_slots, load_imported_textures,
    preprocessor_filterability,
};
use crate::isf::{ISFPass, ISFShader, compile_glsl_to_spirv};
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, ISFUniforms, UnifiedPipeline};
use anyhow::{Context, Result};

impl Effect {
    /// Create a new effect from an ISF filter shader.
    ///
    /// Targets `compositing_format`, the same as channel and master effects, so
    /// an effect behaves identically at any of the three tiers. See
    /// spec/unified-color-pipeline.md.
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
        let num_passes = passes.iter().filter(|p| p.target.is_some()).count();

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
            true, // has_input_image — it's a filter
            num_passes,
            imported_textures.len(),
            &preprocessor_filterable,
        )
        .context("Failed to create effect pipeline")?;

        // Pass buffers follow the effect's own target format so a deck, channel,
        // and master instance of the same shader behave identically. Sized at
        // the internal resolution.
        let pass_buffers = create_pass_buffers(
            context,
            &passes,
            1920,
            1080,
            target_format,
            "Effect Pass Buffer",
        );

        // Initialize parameters from shader inputs
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
            pass_buffers,
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

    /// Set the UUID, used during scene restore to preserve identity.
    ///
    /// Rebuilds the modulation prefix in step. Assignments are stored as
    /// `effect/<uuid>/param/<param>`, so an effect that comes back under a different
    /// prefix than it was saved with loses every modulation routed at it.
    pub fn set_uuid(&mut self, uuid: String) {
        self.param_prefix = crate::engine::value::param::effect_param_prefix(&uuid);
        self.uuid = uuid;
    }

    /// Apply this effect to an input texture, outputting to target texture
    /// Optionally applies modulation to effect parameters using the given prefix
    ///
    /// # Errors
    ///
    /// Propagates any error from [`Effect::apply_with_modulation`], which is
    /// currently infallible.
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

    /// Encode one pass of a multi-pass effect into `target`, reading its
    /// uniforms from `slot`, the input, and every pass buffer's current contents.
    #[allow(clippy::too_many_arguments)] // the pass's bindings, borrowed from different owners
    fn encode_pass(
        &self,
        context: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        input_view: &wgpu::TextureView,
        imported_views: &[&wgpu::TextureView],
        preprocessor_views: &[&wgpu::TextureView],
        user_params_buffer: &wgpu::Buffer,
        target: &wgpu::TextureView,
    ) {
        let pass_buffer_views: Vec<&wgpu::TextureView> = self
            .passes
            .iter()
            .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
            .map(super::PassBuffer::read_view)
            .collect();
        let bind_group = self.pipeline.create_pass_bind_group(
            &context.device,
            slot,
            Some(input_view),
            &pass_buffer_views,
            imported_views,
            preprocessor_views,
            Some(user_params_buffer),
        );
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Effect Pass Render"),
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
        render_pass.set_pipeline(&self.pipeline.pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Apply this effect with modulation support
    ///
    /// # Errors
    ///
    /// Never fails today — recording the render passes is infallible. The
    /// `Result` is kept so callers stay source-compatible if a fallible step
    /// (pipeline recreation, pass-buffer reallocation) is added later.
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

        // Ensure user params buffer exists and update it (with modulation if available)
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

        let has_targeted_passes = self.passes.iter().any(|p| p.target.is_some());

        if has_targeted_passes {
            // Multi-pass effect: targeted passes, then the final pass to the
            // output. Each pass has its own uniform slot, so all of them are
            // written up front and encoded into one command buffer.
            let targeted = self.passes.iter().filter(|p| p.target.is_some()).count();
            self.pipeline
                .ensure_pass_slots(&context.device, targeted + 1);
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Effect Multi-pass Encoder"),
                    });
            let mut slot = 0;
            for (pass_idx, pass) in self.passes.iter().enumerate() {
                let Some(target_name) = &pass.target else {
                    continue; // Final pass handled below
                };
                let mut pass_uniforms = *uniforms;
                pass_uniforms.pass_index = i32::try_from(pass_idx).unwrap_or(i32::MAX);
                self.pipeline
                    .write_pass_uniforms(&context.queue, slot, &pass_uniforms);
                let target_view = self
                    .pass_buffers
                    .get(target_name)
                    .map_or(output_view, super::PassBuffer::write_view);
                self.encode_pass(
                    context,
                    &mut encoder,
                    slot,
                    input_view,
                    &imported_views,
                    &preprocessor_views,
                    user_params_buffer,
                    target_view,
                );
                slot += 1;
                if let Some(pb) = self.pass_buffers.get_mut(target_name) {
                    pb.swap();
                }
            }

            let mut final_uniforms = *uniforms;
            final_uniforms.pass_index = i32::try_from(self.passes.len()).unwrap_or(i32::MAX);
            self.pipeline
                .write_pass_uniforms(&context.queue, slot, &final_uniforms);
            self.encode_pass(
                context,
                &mut encoder,
                slot,
                input_view,
                &imported_views,
                &preprocessor_views,
                user_params_buffer,
                output_view,
            );
            cmd_buffers.push(encoder.finish());
        } else {
            // Simple single-pass effect
            self.pipeline.update_uniforms(&context.queue, uniforms);

            let bind_group = self.pipeline.create_bind_group(
                &context.device,
                Some(input_view),
                &[],
                &imported_views,
                &preprocessor_views,
                Some(user_params_buffer),
            );

            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Effect Render Encoder"),
                    });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Effect Render Pass"),
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

                render_pass.set_pipeline(&self.pipeline.pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            cmd_buffers.push(encoder.finish());
        }

        Ok(())
    }
}
