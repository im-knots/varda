//! Mixer render pipeline — compositing, master effects, sub-mixes.

use crate::renderer::{GpuContext, ISFUniforms};
use anyhow::Result;
use super::{Mixer, CrossfadeEasing, AutoCrossfade};

impl Mixer {
    /// Pre-update modulation engine with latest audio data.
    pub fn update_modulation(&mut self, audio_values: &crate::modulation::AudioValues) {
        let time = self.start_time.elapsed().as_secs_f32();
        self.modulation.update(time, audio_values);
    }

    /// Render all channels and composite them via crossfader, then apply master effects.
    pub fn render(&mut self, context: &GpuContext, audio_data: &crate::audio::AudioData, audio_values: &crate::modulation::AudioValues) -> Result<()> {
        let now = std::time::Instant::now();
        let dt = (now - self.last_render_time).as_secs_f32();
        self.last_render_time = now;

        // Tick auto-crossfade
        if let Some(auto) = &mut self.auto_crossfade {
            match auto.tick(dt) {
                Some(value) => self.crossfader = value,
                None => {
                    let target = auto.to;
                    self.crossfader = target;
                    self.auto_crossfade = None;
                    log::info!("Auto-crossfade complete, crossfader = {:.2}", target);
                }
            }
        }

        // Handle beat-synced crossfade
        if let Some(bsc) = &mut self.beat_sync_crossfade {
            if !bsc.started {
                let phase = audio_data.beat_phase();
                if phase < 0.05 && audio_data.bpm.is_some() {
                    let bpm = audio_data.bpm.unwrap_or(120.0);
                    let duration_secs = bsc.beats * 60.0 / bpm;
                    bsc.auto = Some(AutoCrossfade::new(
                        self.crossfader, bsc.to, duration_secs, CrossfadeEasing::EaseInOut,
                    ));
                    bsc.started = true;
                    log::info!("Beat-synced crossfade started: {:.1} beats at {:.0} BPM = {:.2}s",
                        bsc.beats, bpm, duration_secs);
                }
            }

            if let Some(auto) = &mut bsc.auto {
                match auto.tick(dt) {
                    Some(value) => self.crossfader = value,
                    None => {
                        let target = bsc.to;
                        self.crossfader = target;
                        self.beat_sync_crossfade = None;
                        log::info!("Beat-synced crossfade complete, crossfader = {:.2}", target);
                    }
                }
            }
        }

        // Tick transition sequence
        let bpm = audio_data.bpm.map(|b| b as f64);
        self.tick_sequence(dt, bpm);

        // Update global modulation engine
        let time = self.start_time.elapsed().as_secs_f32();
        self.modulation.update(time, audio_values);

        // Compute effective opacity per channel
        let channel_count = self.channels.len();
        let effective_opacities: Vec<f32> = if channel_count == 2 {
            vec![
                (1.0 - self.crossfader) * self.channels[0].opacity,
                self.crossfader * self.channels[1].opacity,
            ]
        } else {
            self.channels.iter().map(|ch| ch.opacity).collect()
        };

        // Always tick video frames on every channel so players stay in sync
        // even when a channel is fully faded out by the crossfader.
        for channel in self.channels.iter_mut() {
            channel.tick_video_frames(context);
        }

        for (ch_idx, channel) in self.channels.iter_mut().enumerate() {
            if effective_opacities[ch_idx] < 0.001 {
                // Reset stats so culled channels don't show stale render metrics
                channel.render_time_ms = 0.0;
                channel.active_deck_count = 0;
                continue;
            }
            channel.render(context, audio_data, &self.modulation, ch_idx, time, dt)?;
        }

        self.sync_transition_progress();
        self.composite_channels(context)?;
        self.apply_master_effects(context, audio_data, time)?;

        self.frame_count += 1;
        Ok(())
    }


    fn composite_channels(&mut self, context: &GpuContext) -> Result<()> {
        let channel_count = self.channels.len();
        if channel_count == 0 {
            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Mixer Clear Encoder"),
            });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Mixer Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.composite_view,
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
            }
            context.queue.submit(std::iter::once(encoder.finish()));
            return Ok(());
        }

        // If we have exactly 2 channels and a transition shader is active, use it
        if channel_count == 2 {
            if let Some(transition) = &self.active_transition {
                let width = self.composite_texture.width();
                let height = self.composite_texture.height();

                let uniforms = ISFUniforms {
                    time: self.start_time.elapsed().as_secs_f32(),
                    time_delta: 1.0 / 60.0,
                    frame_index: self.frame_count,
                    pass_index: 0,
                    render_size: [width as f32, height as f32],
                    phase_times: [0.0; 4],
                    ..Default::default()
                };

                let params_data = transition.params.build_buffer_data();
                if let Some(buf) = transition.params.buffer() {
                    context.queue.write_buffer(buf, 0, &params_data);
                }

                transition.pipeline.render_to(
                    context,
                    &self.channels[0].composite_view,
                    &self.channels[1].composite_view,
                    &self.composite_view,
                    &uniforms,
                    transition.params.buffer(),
                );

                return Ok(());
            }
        }

        // Fallback: opacity-based crossfade
        let opacities: Vec<f32> = if channel_count == 2 {
            vec![
                (1.0 - self.crossfader) * self.channels[0].opacity,
                self.crossfader * self.channels[1].opacity,
            ]
        } else {
            self.channels.iter().map(|ch| ch.opacity).collect()
        };

        let mut cmd_buffers = Vec::new();
        for (i, (channel, &opacity)) in self.channels.iter().zip(opacities.iter()).enumerate() {
            if opacity <= 0.0 { continue; }

            let blend_mode = channel.blend_mode;
            let pipeline = self.blend_pipeline(&blend_mode);

            pipeline.set_opacity(&context.queue, opacity);
            let bind_group = pipeline.create_bind_group(&context.device, &channel.composite_view);

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Mixer Composite Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Mixer Composite Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.composite_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if i == 0 {
                                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                pipeline.render(&mut render_pass, &bind_group);
            }

            cmd_buffers.push(encoder.finish());
        }

        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }

        Ok(())
    }

    /// Prepare sub-mix textures for all unique multi-channel surface sources.
    pub fn prepare_sub_mixes(&mut self, sources: &[Vec<usize>], context: &GpuContext) {
        let needed: std::collections::HashSet<Vec<usize>> = sources.iter().cloned().collect();
        self.sub_mix_cache.retain(|k, _| needed.contains(k));

        for mut indices in sources.iter().cloned() {
            indices.sort();
            indices.dedup();
            if !self.sub_mix_cache.contains_key(&indices) {
                let width = self.composite_texture.width();
                let height = self.composite_texture.height();
                let tex = context.create_render_texture(width, height);
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                self.sub_mix_cache.insert(indices.clone(), (tex, view));
            }
            self.composite_sub_mix(&indices, context);
        }
    }

    /// Composite a specific subset of channels into the cached sub-mix texture.
    fn composite_sub_mix(&self, indices: &[usize], context: &GpuContext) {
        let (_, view) = match self.sub_mix_cache.get(indices) {
            Some(entry) => entry,
            None => return,
        };

        let channel_count = self.channels.len();
        let opacities: Vec<f32> = if channel_count == 2 {
            vec![
                (1.0 - self.crossfader) * self.channels[0].opacity,
                self.crossfader * self.channels[1].opacity,
            ]
        } else {
            self.channels.iter().map(|ch| ch.opacity).collect()
        };

        let mut cmd_buffers = Vec::new();
        let mut is_first = true;
        for &ch_idx in indices {
            if ch_idx >= self.channels.len() { continue; }
            let channel = &self.channels[ch_idx];
            let opacity = opacities[ch_idx];
            if opacity <= 0.0 { continue; }

            let blend_mode = channel.blend_mode;
            let pipeline = self.blend_pipeline(&blend_mode);

            pipeline.set_opacity(&context.queue, opacity);
            let bind_group = pipeline.create_bind_group(&context.device, &channel.composite_view);

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Sub-mix Composite Encoder"),
            });
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Sub-mix Composite Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if is_first {
                                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pipeline.render(&mut render_pass, &bind_group);
            }
            cmd_buffers.push(encoder.finish());
            is_first = false;
        }

        if is_first {
            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Sub-mix Clear Encoder"),
            });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Sub-mix Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
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
            }
            cmd_buffers.push(encoder.finish());
        }

        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }
    }

    /// Get the sub-mix texture view for a given set of channel indices.
    pub fn get_sub_mix_view(&self, indices: &[usize]) -> Option<&wgpu::TextureView> {
        self.sub_mix_cache.get(indices).map(|(_, v)| v)
    }


    fn apply_master_effects(&mut self, context: &GpuContext, audio_data: &crate::audio::AudioData, time: f32) -> Result<()> {
        if self.master_effects.is_empty() {
            return Ok(());
        }

        let width = self.composite_texture.width();
        let height = self.composite_texture.height();

        let uniforms = ISFUniforms {
            time,
            time_delta: 1.0 / 60.0,
            frame_index: self.frame_count,
            pass_index: 0,
            render_size: [width as f32, height as f32],
            audio_level: audio_data.level,
            audio_bass: audio_data.bass(),
            audio_mid: audio_data.mid(),
            audio_treble: audio_data.treble(),
            audio_bpm: audio_data.bpm.unwrap_or(0.0),
            audio_beat_phase: audio_data.beat_phase(),
            date: crate::deck::get_current_date(),
            phase_times: [0.0; 4],
        };

        let mut read_from_composite = true;
        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();

        for effect in self.master_effects.iter_mut() {
            if !effect.enabled { continue; }

            let (input_view, output_view) = if read_from_composite {
                (&self.composite_view, &self.effect_ping_view)
            } else {
                (&self.effect_ping_view, &self.composite_view)
            };

            effect.apply(context, input_view, output_view, &uniforms, &mut cmd_buffers)?;
            read_from_composite = !read_from_composite;
        }

        if !read_from_composite {
            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Master Effect Final Copy Encoder"),
            });
            encoder.copy_texture_to_texture(
                self.effect_ping_texture.as_image_copy(),
                self.composite_texture.as_image_copy(),
                self.composite_texture.size(),
            );
            cmd_buffers.push(encoder.finish());
        }

        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }

        Ok(())
    }
}