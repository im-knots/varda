//! Effect (ISF filter) implementation and `PassBuffer` for multi-pass effects.

use super::source::load_imported_textures;
use super::{Deck, Effect, PassBuffer};
use crate::isf::{compile_glsl_to_spirv, ISFPass, ISFShader};
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, ISFUniforms, UnifiedPipeline};
use anyhow::{Context, Result};
use std::collections::HashMap;

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
        let preprocessor_textures: Vec<crate::deck::PreprocessorSlot> = shader
            .metadata
            .preprocessors
            .iter()
            .map(|pp| {
                // Create a 1×1 placeholder texture (will be resized when analyzer provides data)
                let texture = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Preprocessor: {}", pp.name)),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    // Data texture (packed analyzer output) — NOT part of the
                    // color path. Format is the encoding; do not make this float.
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                crate::deck::PreprocessorSlot {
                    name: pp.name.clone(),
                    analyzer_type: pp.preprocessor_type.clone(),
                    options: pp.options.clone(),
                    texture,
                    view,
                }
            })
            .collect();

        let pipeline = UnifiedPipeline::new(
            &context.device,
            &spirv,
            target_format,
            true, // has_input_image — it's a filter
            num_passes,
            imported_textures.len(),
            preprocessor_textures.len(),
        )
        .context("Failed to create effect pipeline")?;

        // Create pass buffers for multi-pass effects
        let width = 1920u32; // Internal resolution
        let height = 1080u32;
        let mut pass_buffers = HashMap::new();

        for pass in &passes {
            let target_name = match &pass.target {
                Some(name) => name.clone(),
                None => continue,
            };

            let pass_width = Deck::parse_size_expression(pass.width.as_deref(), width);
            let pass_height = Deck::parse_size_expression(pass.height.as_deref(), height);
            let is_persistent = pass.persistent.unwrap_or(false);

            // Pass buffers follow the effect's own target format so a deck,
            // channel, and master instance of the same shader behave identically.
            let format = target_format;

            let tex_a = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Effect Pass Buffer A: {target_name}")),
                size: wgpu::Extent3d {
                    width: pass_width,
                    height: pass_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view_a = tex_a.create_view(&wgpu::TextureViewDescriptor::default());

            // Double-buffered whether persistent or not — see the note in
            // source.rs: a single-textured pass target aliases COLOR_TARGET with
            // RESOURCE in its own pass and wgpu rejects it.
            let (tex_b, view_b) = {
                let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Effect Pass Buffer B: {target_name}")),
                    size: wgpu::Extent3d {
                        width: pass_width,
                        height: pass_height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                (Some(tex), Some(view))
            };

            pass_buffers.insert(
                target_name.clone(),
                PassBuffer {
                    name: target_name,
                    texture_a: tex_a,
                    view_a,
                    texture_b: tex_b,
                    view_b,
                    persistent: is_persistent,
                    read_idx: 0,
                },
            );
        }

        // Initialize parameters from shader inputs
        let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
        let params = ShaderParams::from_inputs(inputs);
        let phase_inputs_config = shader.metadata.phase_inputs.clone();

        let uuid = crate::deck::generate_short_uuid();
        let param_prefix = format!("fx_{uuid}");

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
            // Multi-pass effect: run targeted passes first, then final pass to output
            for pass_idx in 0..self.passes.len() {
                let pass = &self.passes[pass_idx];

                let target_name = match &pass.target {
                    Some(name) => name.clone(),
                    None => continue, // Final pass handled below
                };

                let iterations = 1;

                for _iter in 0..iterations {
                    let mut pass_uniforms = *uniforms;
                    pass_uniforms.pass_index = i32::try_from(pass_idx).unwrap_or(i32::MAX);
                    self.pipeline
                        .update_uniforms(&context.queue, &pass_uniforms);

                    let pass_buffer_views: Vec<&wgpu::TextureView> = self
                        .passes
                        .iter()
                        .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
                        .map(super::PassBuffer::read_view)
                        .collect();

                    let bind_group = self.pipeline.create_bind_group(
                        &context.device,
                        Some(input_view),
                        &pass_buffer_views,
                        &imported_views,
                        &preprocessor_views,
                        Some(user_params_buffer),
                    );

                    let target_view = self
                        .pass_buffers
                        .get(&target_name)
                        .map_or(output_view, super::PassBuffer::write_view);

                    let mut encoder =
                        context
                            .device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some(&format!("Effect Pass {pass_idx} Encoder")),
                            });

                    {
                        let mut render_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some(&format!("Effect Pass {pass_idx} Render")),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
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

                    // Multipass intermediate passes MUST submit immediately —
                    // update_uniforms() overwrites the same buffer each iteration,
                    // so batching would cause all passes to see the last pass's data.
                    context.queue.submit(std::iter::once(encoder.finish()));

                    if let Some(pb) = self.pass_buffers.get_mut(&target_name) {
                        pb.swap();
                    }
                }
            }

            // Final pass: render to output_view using pass buffer results + input
            let mut final_uniforms = *uniforms;
            final_uniforms.pass_index = i32::try_from(self.passes.len()).unwrap_or(i32::MAX);
            self.pipeline
                .update_uniforms(&context.queue, &final_uniforms);

            let pass_buffer_views: Vec<&wgpu::TextureView> = self
                .passes
                .iter()
                .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
                .map(super::PassBuffer::read_view)
                .collect();

            let bind_group = self.pipeline.create_bind_group(
                &context.device,
                Some(input_view),
                &pass_buffer_views,
                &imported_views,
                &preprocessor_views,
                Some(user_params_buffer),
            );

            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Effect Final Pass Encoder"),
                    });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Effect Final Pass Render"),
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

impl PassBuffer {
    /// Get the current read texture view
    pub fn read_view(&self) -> &wgpu::TextureView {
        if self.read_idx == 0 {
            &self.view_a
        } else {
            self.view_b.as_ref().unwrap_or(&self.view_a)
        }
    }

    /// Get the current write texture view — always the one not being read.
    pub fn write_view(&self) -> &wgpu::TextureView {
        if self.read_idx == 0 {
            self.view_b.as_ref().unwrap_or(&self.view_a)
        } else {
            &self.view_a
        }
    }

    /// Swap read/write buffers. Call after rendering the pass that targets this
    /// buffer, so later passes (and the next frame) read what was just written.
    ///
    /// Unconditional, including for non-persistent buffers: the read and write
    /// views must never be the same texture, or the pass that targets it binds
    /// it as both a colour attachment and a sampled resource. `persistent` now
    /// governs only whether the contents carry meaning across frames — which is
    /// what the ISF key actually means — not the buffering strategy.
    pub fn swap(&mut self) {
        self.read_idx = 1 - self.read_idx;
    }
}
