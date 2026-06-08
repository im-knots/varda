//! Deck rendering — source rendering, effect chain, video frame updates, and resize.

use super::{Deck, DeckSource, PassBuffer, PreprocessorSlot, ScalingMode};
use crate::analyzer::traits::TextureData;
use crate::analyzer::{AnalyzerRegistry, DeckAnalyzers};
use crate::audio::AudioData;
use crate::isf::{ISFPass, PhaseInput};
use crate::modulation::ModulationEngine;
use crate::params::ShaderParams;
use crate::renderer::BlitPipeline;
use crate::renderer::{GpuContext, ISFUniforms, UnifiedPipeline};
use anyhow::Result;
use std::collections::HashMap;
use std::time::Instant;

/// Upload analyzer texture data to a preprocessor slot's GPU texture.
///
/// If dimensions changed, recreates the texture and view. Otherwise writes data in place.
fn upload_texture_to_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut PreprocessorSlot,
    tex_data: &TextureData,
) {
    if tex_data.width == 0 || tex_data.height == 0 || tex_data.data.is_empty() {
        return;
    }

    let current_size = slot.texture.size();
    if current_size.width != tex_data.width || current_size.height != tex_data.height {
        // Dimensions changed — recreate texture
        let new_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Preprocessor: {}", slot.name)),
            size: wgpu::Extent3d {
                width: tex_data.width,
                height: tex_data.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        slot.view = new_texture.create_view(&wgpu::TextureViewDescriptor::default());
        slot.texture = new_texture;
    }

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &slot.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &tex_data.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * tex_data.width),
            rows_per_image: Some(tex_data.height),
        },
        wgpu::Extent3d {
            width: tex_data.width,
            height: tex_data.height,
            depth_or_array_layers: 1,
        },
    );
}

/// Accumulate phase times: for each PhaseInput, adds `dt * param_value * scale` to the accumulator.
fn accumulate_phase_times(
    accumulators: &mut [f32; 4],
    dt: f32,
    phase_inputs: Option<&[PhaseInput]>,
    params: &ShaderParams,
) {
    if let Some(inputs) = phase_inputs {
        for pi in inputs {
            if pi.index < 4 {
                let param_val = params.get_float(&pi.param).unwrap_or(1.0);
                accumulators[pi.index] += dt * param_val * pi.scale;
            }
        }
    }
}

impl Deck {
    /// Update video frame using double-buffered staging uploads.
    /// Takes the latest decoded frame from the background decode thread
    /// and uploads it to the GPU texture via a pre-allocated mapped buffer.
    pub fn update_video_frame(&mut self, encoder: &mut wgpu::CommandEncoder) -> Result<()> {
        match &mut self.source {
            DeckSource::Video {
                ref handle,
                ref texture,
                ref mut staging,
                ..
            } => {
                if let Some(frame) = handle.take_frame() {
                    let width = handle.width;
                    let height = handle.height;
                    staging.upload(&frame.color_data, texture, width, height, encoder);
                }
            }
            DeckSource::HapVideo {
                ref handle,
                ref texture,
                ref alpha_texture,
                ref mut staging,
                ref mut alpha_staging,
                ..
            } => {
                if let Some(frame) = handle.take_frame() {
                    let width = handle.width;
                    let height = handle.height;
                    staging.upload(&frame.color_data, texture, width, height, encoder);

                    if let (Some(alpha_data), Some(_alpha_fmt), Some(alpha_tex)) = (
                        frame.alpha_data.as_ref(),
                        frame.alpha_format,
                        alpha_texture.as_ref(),
                    ) {
                        if let Some(ref mut a_staging) = alpha_staging {
                            a_staging.upload(alpha_data, alpha_tex, width, height, encoder);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Request re-mapping of staging buffers after queue.submit().
    pub fn request_video_remap(&mut self) {
        match &mut self.source {
            DeckSource::Video {
                ref mut staging, ..
            } => staging.request_remap(),
            DeckSource::HapVideo {
                ref mut staging,
                ref mut alpha_staging,
                ..
            } => {
                staging.request_remap();
                if let Some(ref mut a) = alpha_staging {
                    a.request_remap();
                }
            }
            _ => {}
        }
    }

    /// Render the deck to its texture (source + effect chain)
    pub fn render(
        &mut self,
        context: &GpuContext,
        audio_data: &AudioData,
        modulation: &ModulationEngine,
        deck_idx: usize,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        let prefix = format!("deck{}", deck_idx);
        self.render_with_prefix(context, audio_data, modulation, &prefix, cmd_buffers)
    }

    /// Ensure analyzers are running for all preprocessor slots that need them.
    ///
    /// Called once at deck creation or when effects change. Automatically
    /// requests analyzer types declared in PREPROCESSORS blocks.
    pub(crate) fn ensure_preprocessor_analyzers(&mut self, registry: &AnalyzerRegistry) {
        // Collect all (analyzer_type, options) needed by preprocessor slots
        let mut needed: Vec<(String, serde_json::Value)> = Vec::new();
        if let DeckSource::Shader {
            preprocessor_textures,
            ..
        } = &self.source
        {
            for slot in preprocessor_textures {
                needed.push((slot.analyzer_type.clone(), slot.options.clone()));
            }
        }
        for effect in &self.effects {
            for slot in &effect.preprocessor_textures {
                needed.push((slot.analyzer_type.clone(), slot.options.clone()));
            }
        }

        // Deduplicate by analyzer_type and request each
        let mut seen = std::collections::HashSet::new();
        for (analyzer_type, options) in &needed {
            if seen.insert(analyzer_type.clone())
                && self.analyzers.latest_snapshot(analyzer_type).is_none()
            {
                if self
                    .analyzers
                    .request(analyzer_type, registry, options)
                    .is_some()
                {
                    log::info!(
                        "Deck '{}': auto-started analyzer '{}'",
                        self.uuid,
                        analyzer_type
                    );
                } else {
                    log::warn!(
                        "Deck '{}': failed to start analyzer '{}'",
                        self.uuid,
                        analyzer_type
                    );
                }
            }
        }
    }

    /// Render the deck with a custom param prefix for modulation key lookup
    pub fn render_with_prefix(
        &mut self,
        context: &GpuContext,
        audio_data: &AudioData,
        modulation: &ModulationEngine,
        param_prefix: &str,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        // Update preprocessor textures from analyzer snapshots before rendering
        if self.analyzers.has_active_instances() {
            Self::update_preprocessor_textures(
                &self.analyzers,
                &context.device,
                &context.queue,
                &mut self.source,
                &mut self.effects,
            );
        }

        // Advance render_time by a fixed dt so skipped frames don't cause
        // animation jumps. The shader sees smooth, consistent time steps
        // regardless of how many frames were skipped.
        let time_delta = self.render_dt;
        self.render_time += time_delta;
        let time = self.render_time;
        self.frame_count += 1;

        // Derive per-deck FPS from wall-clock render interval (for UI display only)
        let now = Instant::now();
        let wall_dt = (now - self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        if wall_dt > 0.0 && wall_dt < 1.0 {
            let instant_fps = 1.0 / wall_dt;
            self.fps_smoothed = 0.1 * instant_fps + 0.9 * self.fps_smoothed;
        }

        // Accumulate generator phase times using the fixed dt
        accumulate_phase_times(
            &mut self.phase_accumulators,
            time_delta,
            self.generator_phase_inputs.as_deref(),
            &self.generator_params,
        );
        let generator_phase_times = self.phase_accumulators;

        let enabled_effects: Vec<usize> = self
            .effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.enabled)
            .map(|(i, _)| i)
            .collect();

        let source_to_b = enabled_effects.len() % 2 == 1;
        let generator_target = if source_to_b {
            &self.texture_b_view
        } else {
            &self.texture_view
        };

        match &mut self.source {
            DeckSource::Shader {
                pipeline,
                pass_buffers,
                passes,
                imported_textures,
                preprocessor_textures,
                ..
            } => {
                let imported_views: Vec<&wgpu::TextureView> =
                    imported_textures.iter().map(|(_, _, v)| v).collect();
                let preprocessor_views: Vec<&wgpu::TextureView> =
                    preprocessor_textures.iter().map(|pp| &pp.view).collect();
                if pipeline.num_pass_buffers > 0 {
                    Self::render_multi_pass_static(
                        context,
                        pipeline,
                        passes,
                        pass_buffers,
                        time,
                        time_delta,
                        self.frame_count,
                        self.texture.width(),
                        self.texture.height(),
                        generator_target,
                        audio_data,
                        &mut self.generator_params,
                        modulation,
                        &param_prefix,
                        &imported_views,
                        &preprocessor_views,
                        generator_phase_times,
                        cmd_buffers,
                    )?;
                } else {
                    Self::render_simple_static(
                        context,
                        pipeline,
                        &self.texture,
                        time,
                        time_delta,
                        self.frame_count,
                        generator_target,
                        audio_data,
                        &mut self.generator_params,
                        modulation,
                        &param_prefix,
                        &imported_views,
                        &preprocessor_views,
                        generator_phase_times,
                        cmd_buffers,
                    )?;
                }
            }

            DeckSource::Video {
                ref texture_view,
                ref blit_pipeline,
                source_width,
                source_height,
                scaling_mode,
                ..
            } => {
                let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                    *source_width,
                    *source_height,
                    self.texture.width(),
                    self.texture.height(),
                );
                blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

                let bind_group = blit_pipeline.create_bind_group(&context.device, texture_view);
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Video Blit Encoder"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Video Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
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
                    blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::HapVideo {
                ref texture_view,
                ref alpha_texture_view,
                ref dummy_alpha_view,
                ref convert_pipeline,
                ref hap_format,
                ref handle,
                source_width,
                source_height,
                scaling_mode,
                ..
            } => {
                let needs_ycocg = hap_format.needs_ycocg_convert();
                let has_alpha = handle.is_dual_plane && alpha_texture_view.is_some();
                let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                    *source_width,
                    *source_height,
                    self.texture.width(),
                    self.texture.height(),
                );
                convert_pipeline.set_params_with_uv(
                    &context.queue,
                    1.0,
                    needs_ycocg,
                    has_alpha,
                    uv_scale,
                    uv_offset,
                );

                let alpha_view = if let Some(ref av) = alpha_texture_view {
                    av
                } else {
                    dummy_alpha_view
                };
                let bind_group =
                    convert_pipeline.create_bind_group(&context.device, texture_view, alpha_view);
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("HAP Convert Encoder"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("HAP Convert Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
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
                    convert_pipeline.draw(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::Image {
                texture_view,
                blit_pipeline,
                source_width,
                source_height,
                scaling_mode,
                ..
            } => {
                let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                    *source_width,
                    *source_height,
                    self.texture.width(),
                    self.texture.height(),
                );
                blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

                let bind_group = blit_pipeline.create_bind_group(&context.device, texture_view);
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Image Blit Encoder"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Image Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
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
                    blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::SolidColor { color } => {
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("SolidColor Clear Encoder"),
                        });
                {
                    let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("SolidColor Clear Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: color[0],
                                    g: color[1],
                                    b: color[2],
                                    a: color[3],
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::ExternalSource {
                kind,
                blit_pipeline,
                source_width,
                source_height,
                scaling_mode,
            } => {
                if let Some(ext_view) = &self.external_source_view {
                    Self::blit_external_source(
                        context,
                        blit_pipeline,
                        ext_view,
                        *source_width,
                        *source_height,
                        self.texture.width(),
                        self.texture.height(),
                        *scaling_mode,
                        generator_target,
                        kind.label(),
                        cmd_buffers,
                    );
                }
            }
        }

        // Apply effect chain (ping-pong between textures)
        let mut read_from_b = source_to_b;
        for &effect_idx in &enabled_effects {
            // Accumulate phase times for this effect
            let effect = &mut self.effects[effect_idx];
            accumulate_phase_times(
                &mut effect.phase_accumulators,
                time_delta,
                effect.phase_inputs_config.as_deref(),
                &effect.params,
            );
            let effect_phase_times = effect.phase_accumulators;

            let uniforms = ISFUniforms {
                time,
                time_delta,
                frame_index: self.frame_count,
                pass_index: 0,
                render_size: [self.texture.width() as f32, self.texture.height() as f32],
                audio_level: audio_data.level,
                audio_bass: audio_data.bass(),
                audio_mid: audio_data.mid(),
                audio_treble: audio_data.treble(),
                audio_bpm: audio_data.bpm.unwrap_or(0.0),
                audio_beat_phase: audio_data.beat_phase(),
                date: get_current_date(),
                phase_times: effect_phase_times,
            };
            let (input_view, output_view) = if read_from_b {
                (&self.texture_b_view, &self.texture_view)
            } else {
                (&self.texture_view, &self.texture_b_view)
            };
            self.effects[effect_idx].apply_with_modulation(
                context,
                input_view,
                output_view,
                &uniforms,
                Some(modulation),
                cmd_buffers,
            )?;
            read_from_b = !read_from_b;
        }

        // Capture frame for analyzer pipeline (non-blocking, one-frame latency)
        if let Some(readback_cmd) = self.analyzers.capture_frame(&context.device, &self.texture) {
            cmd_buffers.push(readback_cmd);
        }

        Ok(())
    }

    /// Upload analyzer texture data into preprocessor slots.
    ///
    /// For each preprocessor slot (on source and effects), looks up the matching
    /// analyzer snapshot and uploads texture data via `queue.write_texture()`.
    /// If the texture dimensions changed, recreates the GPU texture.
    fn update_preprocessor_textures(
        analyzers: &DeckAnalyzers,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &mut DeckSource,
        effects: &mut [super::Effect],
    ) {
        if let DeckSource::Shader {
            preprocessor_textures,
            ..
        } = source
        {
            for slot in preprocessor_textures.iter_mut() {
                if let Some(snapshot) = analyzers.latest_snapshot(&slot.analyzer_type) {
                    if let Some(tex_data) = snapshot.textures.get(&slot.name) {
                        upload_texture_to_slot(device, queue, slot, tex_data);
                    }
                }
            }
        }

        for effect in effects.iter_mut() {
            for slot in effect.preprocessor_textures.iter_mut() {
                if let Some(snapshot) = analyzers.latest_snapshot(&slot.analyzer_type) {
                    if let Some(tex_data) = snapshot.textures.get(&slot.name) {
                        upload_texture_to_slot(device, queue, slot, tex_data);
                    }
                }
            }
        }
    }

    /// Render simple (non-multi-pass) shader (static version)
    fn render_simple_static(
        context: &GpuContext,
        pipeline: &UnifiedPipeline,
        texture: &wgpu::Texture,
        time: f32,
        time_delta: f32,
        frame_count: u32,
        target_view: &wgpu::TextureView,
        audio_data: &AudioData,
        generator_params: &mut ShaderParams,
        modulation: &ModulationEngine,
        param_prefix: &str,
        imported_views: &[&wgpu::TextureView],
        preprocessor_views: &[&wgpu::TextureView],
        phase_times: [f32; 4],
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        let uniforms = ISFUniforms {
            time,
            time_delta,
            frame_index: frame_count,
            pass_index: 0,
            render_size: [texture.width() as f32, texture.height() as f32],
            audio_level: audio_data.level,
            audio_bass: audio_data.bass(),
            audio_mid: audio_data.mid(),
            audio_treble: audio_data.treble(),
            audio_bpm: audio_data.bpm.unwrap_or(0.0),
            audio_beat_phase: audio_data.beat_phase(),
            date: get_current_date(),
            phase_times,
        };

        pipeline.update_uniforms(&context.queue, &uniforms);

        generator_params.ensure_buffer(&context.device);
        generator_params.update_buffer_with_modulation(
            &context.queue,
            modulation,
            Some(param_prefix),
        );

        let user_params_buffer = generator_params
            .buffer()
            .expect("Buffer should exist after ensure_buffer");
        let bind_group = pipeline.create_bind_group(
            &context.device,
            None,
            &[],
            imported_views,
            preprocessor_views,
            Some(user_params_buffer),
        );

        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Deck Source Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Deck Source Render Pass"),
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

            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        cmd_buffers.push(encoder.finish());
        Ok(())
    }

    /// Render multi-pass shader with proper ping-pong buffers
    fn render_multi_pass_static(
        context: &GpuContext,
        multi_pass: &UnifiedPipeline,
        passes: &[ISFPass],
        pass_buffers: &mut HashMap<String, PassBuffer>,
        time: f32,
        time_delta: f32,
        frame_count: u32,
        render_width: u32,
        render_height: u32,
        final_target: &wgpu::TextureView,
        audio_data: &AudioData,
        generator_params: &mut ShaderParams,
        modulation: &ModulationEngine,
        param_prefix: &str,
        imported_views: &[&wgpu::TextureView],
        preprocessor_views: &[&wgpu::TextureView],
        phase_times: [f32; 4],
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        generator_params.ensure_buffer(&context.device);
        generator_params.update_buffer_with_modulation(
            &context.queue,
            modulation,
            Some(param_prefix),
        );
        let user_params_buffer = generator_params
            .buffer()
            .expect("Buffer should exist after ensure_buffer");

        const SIMULATION_ITERATIONS: usize = 4;

        for pass_idx in 0..passes.len() {
            let pass = &passes[pass_idx];

            let iterations = if pass.persistent.unwrap_or(false) {
                SIMULATION_ITERATIONS
            } else {
                1
            };

            let target_name = match &pass.target {
                Some(name) => name,
                None => continue,
            };

            let format = if pass.float.unwrap_or(false) {
                wgpu::TextureFormat::Rgba32Float
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            };

            // Use the pass buffer's actual dimensions as RENDERSIZE so
            // shaders that store per-pixel state (e.g. particle buffers)
            // can address their own texels correctly.
            let pass_render_size = pass_buffers
                .get(target_name)
                .map(|pb| {
                    let sz = pb.texture_a.size();
                    [sz.width as f32, sz.height as f32]
                })
                .unwrap_or([render_width as f32, render_height as f32]);

            for iter in 0..iterations {
                let effective_frame = frame_count * SIMULATION_ITERATIONS as u32 + iter as u32;

                let uniforms = ISFUniforms {
                    time,
                    time_delta: time_delta / SIMULATION_ITERATIONS as f32,
                    frame_index: effective_frame,
                    pass_index: pass_idx as i32,
                    render_size: pass_render_size,
                    audio_level: audio_data.level,
                    audio_bass: audio_data.bass(),
                    audio_mid: audio_data.mid(),
                    audio_treble: audio_data.treble(),
                    audio_bpm: audio_data.bpm.unwrap_or(0.0),
                    audio_beat_phase: audio_data.beat_phase(),
                    date: get_current_date(),
                    phase_times,
                };

                multi_pass.update_uniforms(&context.queue, &uniforms);

                let pass_buffer_views: Vec<&wgpu::TextureView> = passes
                    .iter()
                    .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
                    .map(|pb| pb.read_view())
                    .collect();

                let bind_group = multi_pass.create_bind_group(
                    &context.device,
                    None,
                    &pass_buffer_views,
                    imported_views,
                    preprocessor_views,
                    Some(user_params_buffer),
                );

                let target_view = pass_buffers
                    .get(target_name)
                    .map(|pb| pb.write_view())
                    .unwrap_or(final_target);

                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Sim Pass Encoder"),
                        });

                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Sim Pass Render"),
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

                    render_pass.set_pipeline(multi_pass.pipeline_for_format(format));
                    render_pass.set_bind_group(0, &bind_group, &[]);
                    render_pass.draw(0..3, 0..1);
                }

                // Multipass intermediate passes MUST submit immediately —
                // update_uniforms() overwrites the same buffer each iteration,
                // so batching would cause all passes to see the last pass's data.
                context.queue.submit(std::iter::once(encoder.finish()));

                if let Some(pb) = pass_buffers.get_mut(target_name) {
                    pb.swap();
                }
            }
        }

        // Final render pass to screen
        {
            let uniforms = ISFUniforms {
                time,
                time_delta,
                frame_index: frame_count,
                pass_index: passes.len() as i32,
                render_size: [render_width as f32, render_height as f32],
                audio_level: audio_data.level,
                audio_bass: audio_data.bass(),
                audio_mid: audio_data.mid(),
                audio_treble: audio_data.treble(),
                audio_bpm: audio_data.bpm.unwrap_or(0.0),
                audio_beat_phase: audio_data.beat_phase(),
                date: get_current_date(),
                phase_times,
            };

            multi_pass.update_uniforms(&context.queue, &uniforms);

            let pass_buffer_views: Vec<&wgpu::TextureView> = passes
                .iter()
                .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
                .map(|pb| pb.read_view())
                .collect();

            let bind_group = multi_pass.create_bind_group(
                &context.device,
                None,
                &pass_buffer_views,
                imported_views,
                preprocessor_views,
                Some(user_params_buffer),
            );

            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Final Pass Encoder"),
                    });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Final Pass Render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: final_target,
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

                render_pass
                    .set_pipeline(multi_pass.pipeline_for_format(wgpu::TextureFormat::Rgba8Unorm));
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            cmd_buffers.push(encoder.finish());
        }

        Ok(())
    }

    /// Blit an external source (Camera, NDI, Syphon) with scaling to the generator target.
    fn blit_external_source(
        context: &GpuContext,
        blit_pipeline: &BlitPipeline,
        source_view: &wgpu::TextureView,
        source_width: u32,
        source_height: u32,
        target_width: u32,
        target_height: u32,
        scaling_mode: ScalingMode,
        generator_target: &wgpu::TextureView,
        label: &str,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) {
        let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
            source_width,
            source_height,
            target_width,
            target_height,
        );
        blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

        let bind_group = blit_pipeline.create_bind_group(&context.device, source_view);
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(&format!("{} Blit Encoder", label)),
            });
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("{} Blit Pass", label)),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: generator_target,
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
            blit_pipeline.render(&mut render_pass, &bind_group);
        }
        cmd_buffers.push(encoder.finish());
    }

    /// Resize the deck's render targets
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture (Linear)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.texture_view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture B (Linear)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.texture_b_view = self
            .texture_b
            .create_view(&wgpu::TextureViewDescriptor::default());
    }

    /// Get the final output texture view (after effect chain)
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.texture_view
    }
}

/// Get current date as [year, month, day, seconds_in_day]
pub fn get_current_date() -> [f32; 4] {
    use std::time::SystemTime;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let total_seconds = now.as_secs();
    let seconds_in_day = (total_seconds % 86400) as f32;

    let days_since_epoch = total_seconds / 86400;
    let year = 1970.0 + (days_since_epoch as f32 / 365.25);
    let day_of_year = (days_since_epoch % 365) as f32;
    let month = (day_of_year / 30.0).floor() + 1.0;
    let day = (day_of_year % 30.0) + 1.0;

    [year, month, day, seconds_in_day]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isf::ISFInput;
    use crate::isf::PhaseInput;
    use crate::params::ShaderParams;

    #[test]
    fn isf_uniforms_size_is_80_bytes() {
        assert_eq!(
            std::mem::size_of::<ISFUniforms>(),
            80,
            "ISFUniforms should be 80 bytes (64 original + 16 for phase_times)"
        );
    }

    #[test]
    fn accumulate_phase_times_basic() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![PhaseInput {
            param: "speed".into(),
            index: 0,
            scale: 1.0,
        }];
        let isf_inputs = vec![ISFInput {
            name: "speed".into(),
            input_type: "float".into(),
            default: Some(serde_json::json!(2.0)),
            min: Some(0.0),
            max: Some(5.0),
            label: None,
            values: None,
            labels: None,
            identity: None,
        }];
        let params = ShaderParams::from_inputs(&isf_inputs);

        // dt=0.1, speed=2.0, scale=1.0 → accumulate 0.2
        accumulate_phase_times(&mut accum, 0.1, Some(&inputs), &params);
        assert!((accum[0] - 0.2).abs() < 1e-5);
        assert_eq!(accum[1], 0.0);

        // Accumulate again: 0.2 + 0.2 = 0.4
        accumulate_phase_times(&mut accum, 0.1, Some(&inputs), &params);
        assert!((accum[0] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn accumulate_phase_times_with_scale() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![PhaseInput {
            param: "speed".into(),
            index: 0,
            scale: 0.3,
        }];
        let isf_inputs = vec![ISFInput {
            name: "speed".into(),
            input_type: "float".into(),
            default: Some(serde_json::json!(1.0)),
            min: Some(0.0),
            max: Some(5.0),
            label: None,
            values: None,
            labels: None,
            identity: None,
        }];
        let params = ShaderParams::from_inputs(&isf_inputs);

        // dt=0.5, speed=1.0, scale=0.3 → 0.15
        accumulate_phase_times(&mut accum, 0.5, Some(&inputs), &params);
        assert!((accum[0] - 0.15).abs() < 1e-5);
    }

    #[test]
    fn accumulate_phase_times_speed_change_is_continuous() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![PhaseInput {
            param: "speed".into(),
            index: 0,
            scale: 1.0,
        }];
        let isf_inputs = vec![ISFInput {
            name: "speed".into(),
            input_type: "float".into(),
            default: Some(serde_json::json!(1.0)),
            min: Some(0.0),
            max: Some(5.0),
            label: None,
            values: None,
            labels: None,
            identity: None,
        }];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        // Run 10 frames at speed=1.0, dt=0.016
        for _ in 0..10 {
            accumulate_phase_times(&mut accum, 0.016, Some(&inputs), &params);
        }
        let before_change = accum[0];

        // Change speed to 3.0 — no jump should occur
        params.set_float("speed", 3.0);
        accumulate_phase_times(&mut accum, 0.016, Some(&inputs), &params);
        let after_change = accum[0];

        // Value should increase by dt*3.0, not jump to TIME*3.0
        let expected_delta = 0.016 * 3.0;
        assert!(
            (after_change - before_change - expected_delta).abs() < 1e-5,
            "Phase time should be continuous: before={}, after={}, expected delta={}",
            before_change,
            after_change,
            expected_delta
        );
    }

    #[test]
    fn accumulate_phase_times_multi_index() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![
            PhaseInput {
                param: "speed".into(),
                index: 0,
                scale: 1.0,
            },
            PhaseInput {
                param: "rot_x".into(),
                index: 1,
                scale: 1.0,
            },
            PhaseInput {
                param: "rot_y".into(),
                index: 2,
                scale: 1.0,
            },
            PhaseInput {
                param: "rot_z".into(),
                index: 3,
                scale: 1.0,
            },
        ];
        let isf_inputs = vec![
            ISFInput {
                name: "speed".into(),
                input_type: "float".into(),
                default: Some(serde_json::json!(1.0)),
                min: Some(0.0),
                max: Some(5.0),
                label: None,
                values: None,
                labels: None,
                identity: None,
            },
            ISFInput {
                name: "rot_x".into(),
                input_type: "float".into(),
                default: Some(serde_json::json!(0.5)),
                min: Some(-1.0),
                max: Some(1.0),
                label: None,
                values: None,
                labels: None,
                identity: None,
            },
            ISFInput {
                name: "rot_y".into(),
                input_type: "float".into(),
                default: Some(serde_json::json!(0.3)),
                min: Some(-1.0),
                max: Some(1.0),
                label: None,
                values: None,
                labels: None,
                identity: None,
            },
            ISFInput {
                name: "rot_z".into(),
                input_type: "float".into(),
                default: Some(serde_json::json!(0.0)),
                min: Some(-1.0),
                max: Some(1.0),
                label: None,
                values: None,
                labels: None,
                identity: None,
            },
        ];
        let params = ShaderParams::from_inputs(&isf_inputs);

        accumulate_phase_times(&mut accum, 0.1, Some(&inputs), &params);
        assert!((accum[0] - 0.1).abs() < 1e-5); // speed=1.0 * 0.1
        assert!((accum[1] - 0.05).abs() < 1e-5); // rot_x=0.5 * 0.1
        assert!((accum[2] - 0.03).abs() < 1e-5); // rot_y=0.3 * 0.1
        assert!((accum[3] - 0.0).abs() < 1e-5); // rot_z=0.0 * 0.1
    }

    #[test]
    fn accumulate_phase_times_none_is_noop() {
        let mut accum = [0.0f32; 4];
        let params = ShaderParams::from_inputs(&[]);
        accumulate_phase_times(&mut accum, 0.1, None, &params);
        assert_eq!(accum, [0.0; 4]);
    }

    // ── Offensive: zero-size texture guard on resize ─────────────────

    #[test]
    fn deck_resize_zero_dimensions_does_not_panic() {
        let gpu = crate::renderer::GpuContext::new_headless();
        let Ok(gpu) = gpu else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let mut deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64)
            .expect("solid color deck creation should succeed");

        // Zero width — must not panic (clamped to 1)
        deck.resize(&gpu, 0, 64);

        // Zero height — must not panic (clamped to 1)
        deck.resize(&gpu, 64, 0);

        // Both zero — must not panic (clamped to 1x1)
        deck.resize(&gpu, 0, 0);

        // Normal resize still works
        deck.resize(&gpu, 128, 128);
    }
}
