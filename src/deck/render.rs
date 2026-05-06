//! Deck rendering — source rendering, effect chain, video frame updates, and resize.

use crate::audio::AudioData;
use crate::isf::ISFPass;
use crate::modulation::ModulationEngine;
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, UnifiedPipeline, ISFUniforms};
use anyhow::Result;
use std::collections::HashMap;
use std::time::Instant;
use super::{Deck, DeckSource, PassBuffer};

impl Deck {
    /// Update video frame (call before render if using video source).
    /// Handles both ffmpeg RGBA uploads and HAP BCn compressed uploads.
    pub fn update_video_frame(&mut self, context: &GpuContext) -> Result<()> {
        match &mut self.source {
            DeckSource::Video { ref mut player, ref texture, .. } => {
                if player.is_playing() {
                    let width = player.width();
                    let height = player.height();
                    if let Some(frame_data) = player.next_frame()? {
                        context.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture, mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            frame_data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(width * 4),
                                rows_per_image: Some(height),
                            },
                            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        );
                    }
                }
            }
            DeckSource::HapVideo { ref mut player, ref texture, ref alpha_texture, .. } => {
                if player.is_playing() {
                    let width = player.width();
                    let height = player.height();
                    if let Some(frame) = player.next_frame()? {
                        let blocks_x = (width + 3) / 4;
                        let blocks_y = (height + 3) / 4;
                        let color_bpr = blocks_x * frame.color_format.block_bytes();
                        context.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture, mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            frame.color_data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(color_bpr),
                                rows_per_image: Some(blocks_y),
                            },
                            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        );

                        if let (Some(alpha_data), Some(alpha_fmt), Some(alpha_tex)) =
                            (frame.alpha_data, frame.alpha_format, alpha_texture.as_ref())
                        {
                            let alpha_bpr = blocks_x * alpha_fmt.block_bytes();
                            context.queue.write_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: alpha_tex, mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                alpha_data,
                                wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(alpha_bpr),
                                    rows_per_image: Some(blocks_y),
                                },
                                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                            );
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Render the deck to its texture (source + effect chain)
    pub fn render(&mut self, context: &GpuContext, audio_data: &AudioData, modulation: &ModulationEngine, deck_idx: usize, cmd_buffers: &mut Vec<wgpu::CommandBuffer>) -> Result<()> {
        let prefix = format!("deck{}", deck_idx);
        self.render_with_prefix(context, audio_data, modulation, &prefix, cmd_buffers)
    }

    /// Render the deck with a custom param prefix for modulation key lookup
    pub fn render_with_prefix(&mut self, context: &GpuContext, audio_data: &AudioData, modulation: &ModulationEngine, param_prefix: &str, cmd_buffers: &mut Vec<wgpu::CommandBuffer>) -> Result<()> {
        let now = Instant::now();
        let time = (now - self.start_time).as_secs_f32();
        let time_delta = (now - self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        self.frame_count += 1;

        let enabled_effects: Vec<usize> = self.effects.iter()
            .enumerate()
            .filter(|(_, e)| e.enabled)
            .map(|(i, _)| i)
            .collect();

        let source_to_b = enabled_effects.len() % 2 == 1;
        let generator_target = if source_to_b { &self.texture_b_view } else { &self.texture_view };

        match &mut self.source {
            DeckSource::Shader { pipeline, pass_buffers, passes, .. } => {
                if pipeline.num_pass_buffers > 0 {
                    Self::render_multi_pass_static(
                        context, pipeline, passes, pass_buffers,
                        time, time_delta, self.frame_count,
                        self.texture.width(), self.texture.height(),
                        generator_target, audio_data,
                        &mut self.generator_params, modulation, &param_prefix,
                        cmd_buffers,
                    )?;
                } else {
                    Self::render_simple_static(
                        context, pipeline, &self.texture,
                        time, time_delta, self.frame_count, generator_target, audio_data,
                        &mut self.generator_params, modulation, &param_prefix,
                        cmd_buffers,
                    )?;
                }
            }

            DeckSource::Video { ref texture_view, ref blit_pipeline, .. } => {
                let bind_group = blit_pipeline.create_bind_group(&context.device, texture_view);
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                    });
                    blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::HapVideo {
                ref texture_view, ref alpha_texture_view, ref dummy_alpha_view,
                ref convert_pipeline, ref hap_format, ref player, ..
            } => {
                let needs_ycocg = hap_format.needs_ycocg_convert();
                let has_alpha = player.is_dual_plane && alpha_texture_view.is_some();
                convert_pipeline.set_params(&context.queue, 1.0, needs_ycocg, has_alpha);

                let alpha_view = if let Some(ref av) = alpha_texture_view { av } else { dummy_alpha_view };
                let bind_group = convert_pipeline.create_bind_group(&context.device, texture_view, alpha_view);
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                    });
                    convert_pipeline.draw(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::Image { texture_view, blit_pipeline, source_width, source_height, scaling_mode, .. } => {
                let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                    *source_width, *source_height,
                    self.texture.width(), self.texture.height(),
                );
                blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

                let bind_group = blit_pipeline.create_bind_group(&context.device, texture_view);
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                    });
                    blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::SolidColor { color } => {
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                                    r: color[0], g: color[1], b: color[2], a: color[3],
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::Camera { blit_pipeline, source_width, source_height, scaling_mode, .. } => {
                if let Some(cam_view) = &self.camera_source_view {
                    let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                        *source_width, *source_height,
                        self.texture.width(), self.texture.height(),
                    );
                    blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

                    let bind_group = blit_pipeline.create_bind_group(&context.device, cam_view);
                    let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Camera Blit Encoder"),
                    });
                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Camera Blit Pass"),
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
                        });
                        blit_pipeline.render(&mut render_pass, &bind_group);
                    }
                    cmd_buffers.push(encoder.finish());
                }
            }
        }

        // Apply effect chain (ping-pong between textures)
        let mut read_from_b = source_to_b;
        for &effect_idx in &enabled_effects {
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
            };
            let (input_view, output_view) = if read_from_b {
                (&self.texture_b_view, &self.texture_view)
            } else {
                (&self.texture_view, &self.texture_b_view)
            };
            let fx_prefix = format!("{}_fx{}", param_prefix, effect_idx);
            self.effects[effect_idx].apply_with_modulation(
                context, input_view, output_view, &uniforms,
                Some(modulation), Some(&fx_prefix),
                cmd_buffers,
            )?;
            read_from_b = !read_from_b;
        }

        Ok(())
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
        };

        pipeline.update_uniforms(&context.queue, &uniforms);

        generator_params.ensure_buffer(&context.device);
        generator_params.update_buffer_with_modulation(&context.queue, modulation, Some(param_prefix));

        let user_params_buffer = generator_params.buffer().expect("Buffer should exist after ensure_buffer");
        let bind_group = pipeline.create_bind_group_with_params(&context.device, user_params_buffer);

        let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        generator_params.ensure_buffer(&context.device);
        generator_params.update_buffer_with_modulation(&context.queue, modulation, Some(param_prefix));
        let user_params_buffer = generator_params.buffer().expect("Buffer should exist after ensure_buffer");

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

            for iter in 0..iterations {
                let effective_frame = frame_count * SIMULATION_ITERATIONS as u32 + iter as u32;

                let uniforms = ISFUniforms {
                    time,
                    time_delta: time_delta / SIMULATION_ITERATIONS as f32,
                    frame_index: effective_frame,
                    pass_index: pass_idx as i32,
                    render_size: [render_width as f32, render_height as f32],
                    audio_level: audio_data.level,
                    audio_bass: audio_data.bass(),
                    audio_mid: audio_data.mid(),
                    audio_treble: audio_data.treble(),
                    audio_bpm: audio_data.bpm.unwrap_or(0.0),
                    audio_beat_phase: audio_data.beat_phase(),
                    date: get_current_date(),
                };

                multi_pass.update_uniforms(&context.queue, &uniforms);

                let pass_buffer_views: Vec<&wgpu::TextureView> = passes
                    .iter()
                    .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
                    .map(|pb| pb.read_view())
                    .collect();

                let bind_group = multi_pass.create_bind_group(&context.device, None, &pass_buffer_views, Some(user_params_buffer));

                let target_view = pass_buffers.get(target_name)
                    .map(|pb| pb.write_view())
                    .unwrap_or(final_target);

                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                    });

                    render_pass.set_pipeline(multi_pass.pipeline_for_format(format));
                    render_pass.set_bind_group(0, &bind_group, &[]);
                    render_pass.draw(0..3, 0..1);
                }

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
            };

            multi_pass.update_uniforms(&context.queue, &uniforms);

            let pass_buffer_views: Vec<&wgpu::TextureView> = passes
                .iter()
                .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
                .map(|pb| pb.read_view())
                .collect();

            let bind_group = multi_pass.create_bind_group(&context.device, None, &pass_buffer_views, Some(user_params_buffer));

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                });

                render_pass.set_pipeline(multi_pass.pipeline_for_format(wgpu::TextureFormat::Rgba8Unorm));
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            cmd_buffers.push(encoder.finish());
        }

        Ok(())
    }

    /// Resize the deck's render targets
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
        self.texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture (Linear)"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.texture_view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture B (Linear)"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.texture_b_view = self.texture_b.create_view(&wgpu::TextureViewDescriptor::default());
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
        .unwrap();

    let total_seconds = now.as_secs();
    let seconds_in_day = (total_seconds % 86400) as f32;

    let days_since_epoch = total_seconds / 86400;
    let year = 1970.0 + (days_since_epoch as f32 / 365.25);
    let day_of_year = (days_since_epoch % 365) as f32;
    let month = (day_of_year / 30.0).floor() + 1.0;
    let day = (day_of_year % 30.0) + 1.0;

    [year, month, day, seconds_in_day]
}