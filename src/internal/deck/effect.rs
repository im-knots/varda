//! Effect (ISF filter) implementation and PassBuffer for multi-pass effects.

use crate::isf::{ISFShader, ISFPass, compile_glsl_to_spirv};
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, UnifiedPipeline, ISFUniforms};
use anyhow::{Context, Result};
use std::collections::HashMap;
use super::{Effect, Deck, PassBuffer};

impl Effect {
    /// Create a new effect from an ISF filter shader
    pub fn new(context: &GpuContext, shader: ISFShader) -> Result<Self> {
        Self::new_with_format(context, shader, wgpu::TextureFormat::Rgba8Unorm)
    }

    /// Create a new effect with a specific target format
    pub fn new_with_format(context: &GpuContext, shader: ISFShader, target_format: wgpu::TextureFormat) -> Result<Self> {
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile filter shader to SPIR-V")?;

        let passes: Vec<ISFPass> = shader.metadata.passes.clone().unwrap_or_default();
        let num_passes = passes.iter().filter(|p| p.target.is_some()).count();
        let uses_float = passes.iter().any(|p| p.float.unwrap_or(false));

        let pipeline = UnifiedPipeline::new(
            &context.device,
            &spirv,
            target_format,
            true,  // has_input_image — it's a filter
            num_passes,
            uses_float,
        ).context("Failed to create effect pipeline")?;

        // Create pass buffers for multi-pass effects
        let width = 1920u32;  // Internal resolution
        let height = 1080u32;
        let mut pass_buffers = HashMap::new();

        for pass in &passes {
            let target_name = match &pass.target {
                Some(name) => name.clone(),
                None => continue,
            };

            let pass_width = Deck::parse_size_expression(&pass.width, width);
            let pass_height = Deck::parse_size_expression(&pass.height, height);
            let is_persistent = pass.persistent.unwrap_or(false);

            let format = if pass.float.unwrap_or(false) {
                wgpu::TextureFormat::Rgba32Float
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            };

            let tex_a = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Effect Pass Buffer A: {}", target_name)),
                size: wgpu::Extent3d { width: pass_width, height: pass_height, depth_or_array_layers: 1 },
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

            let (tex_b, view_b) = if is_persistent {
                let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Effect Pass Buffer B: {}", target_name)),
                    size: wgpu::Extent3d { width: pass_width, height: pass_height, depth_or_array_layers: 1 },
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
            } else {
                (None, None)
            };

            pass_buffers.insert(target_name.clone(), PassBuffer {
                name: target_name,
                texture_a: tex_a,
                view_a,
                texture_b: tex_b,
                view_b,
                persistent: is_persistent,
                read_idx: 0,
            });
        }

        // Initialize parameters from shader inputs
        let inputs = shader.metadata.inputs.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
        let params = ShaderParams::from_inputs(inputs);

        Ok(Self {
            shader,
            pipeline,
            enabled: true,
            params,
            pass_buffers,
            passes,
            target_format,
        })
    }

    /// Apply this effect to an input texture, outputting to target texture
    /// Optionally applies modulation to effect parameters using the given prefix
    pub fn apply(
        &mut self,
        context: &GpuContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        self.apply_with_modulation(context, input_view, output_view, uniforms, None, None, cmd_buffers)
    }

    /// Apply this effect with modulation support
    pub fn apply_with_modulation(
        &mut self,
        context: &GpuContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        modulation: Option<&crate::modulation::ModulationEngine>,
        mod_prefix: Option<&str>,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Ensure user params buffer exists and update it (with modulation if available)
        self.params.ensure_buffer(&context.device);
        if let Some(mod_engine) = modulation {
            self.params.update_buffer_with_modulation(&context.queue, mod_engine, mod_prefix);
        } else {
            self.params.update_buffer(&context.queue);
        }
        let user_params_buffer = self.params.buffer().expect("Buffer should exist after ensure_buffer");

        let has_targeted_passes = self.passes.iter().any(|p| p.target.is_some());

        if has_targeted_passes {
            // Multi-pass effect: run targeted passes first, then final pass to output
            for pass_idx in 0..self.passes.len() {
                let pass = &self.passes[pass_idx];

                let target_name = match &pass.target {
                    Some(name) => name.clone(),
                    None => continue, // Final pass handled below
                };

                let format = if pass.float.unwrap_or(false) {
                    wgpu::TextureFormat::Rgba32Float
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                };

                let iterations = 1;

                for _iter in 0..iterations {
                    let mut pass_uniforms = *uniforms;
                    pass_uniforms.pass_index = pass_idx as i32;
                    self.pipeline.update_uniforms(&context.queue, &pass_uniforms);

                    let pass_buffer_views: Vec<&wgpu::TextureView> = self.passes
                        .iter()
                        .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
                        .map(|pb| pb.read_view())
                        .collect();

                    let bind_group = self.pipeline.create_bind_group(
                        &context.device,
                        Some(input_view),
                        &pass_buffer_views,
                        Some(user_params_buffer),
                    );

                    let target_view = self.pass_buffers.get(&target_name)
                        .map(|pb| pb.write_view())
                        .unwrap_or(output_view);

                    let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some(&format!("Effect Pass {} Encoder", pass_idx)),
                    });

                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some(&format!("Effect Pass {} Render", pass_idx)),
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
                        });

                        render_pass.set_pipeline(self.pipeline.pipeline_for_format(format));
                        render_pass.set_bind_group(0, &bind_group, &[]);
                        render_pass.draw(0..3, 0..1);
                    }

                    context.queue.submit(std::iter::once(encoder.finish()));

                    if let Some(pb) = self.pass_buffers.get_mut(&target_name) {
                        pb.swap();
                    }
                }
            }

            // Final pass: render to output_view using pass buffer results + input
            let mut final_uniforms = *uniforms;
            final_uniforms.pass_index = self.passes.len() as i32;
            self.pipeline.update_uniforms(&context.queue, &final_uniforms);

            let pass_buffer_views: Vec<&wgpu::TextureView> = self.passes
                .iter()
                .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
                .map(|pb| pb.read_view())
                .collect();

            let bind_group = self.pipeline.create_bind_group(
                &context.device,
                Some(input_view),
                &pass_buffer_views,
                Some(user_params_buffer),
            );

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                });

                render_pass.set_pipeline(self.pipeline.pipeline_for_format(self.target_format));
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
                Some(user_params_buffer),
            );

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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

    /// Get the current write texture view (opposite of read for persistent)
    pub fn write_view(&self) -> &wgpu::TextureView {
        if !self.persistent {
            &self.view_a
        } else if self.read_idx == 0 {
            self.view_b.as_ref().unwrap_or(&self.view_a)
        } else {
            &self.view_a
        }
    }

    /// Swap read/write buffers (call after rendering for persistent buffers)
    pub fn swap(&mut self) {
        if self.persistent {
            self.read_idx = 1 - self.read_idx;
        }
    }
}
