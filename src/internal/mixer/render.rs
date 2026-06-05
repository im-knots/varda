//! Mixer render pipeline — compositing, master effects, sub-mixes.

use super::{AutoCrossfade, CrossfadeEasing, Mixer};
use crate::renderer::{GpuContext, ISFUniforms};
use anyhow::Result;

/// Stack-friendly container for per-channel compositing opacities.
///
/// The common 2-channel case uses a fixed-size array on the stack, avoiding a
/// heap allocation.  N-channel mode falls back to `Vec`.  Derefs to `&[f32]`
/// so callers can use `.iter()`, `.get()`, indexing, etc. unchanged.
enum CompositingOpacities {
    Two([f32; 2]),
    Many(Vec<f32>),
}

impl std::ops::Deref for CompositingOpacities {
    type Target = [f32];

    fn deref(&self) -> &[f32] {
        match self {
            CompositingOpacities::Two(arr) => arr,
            CompositingOpacities::Many(vec) => vec,
        }
    }
}

impl Mixer {
    /// Pre-update modulation engine with latest audio data.
    pub fn update_modulation(&mut self, audio_values: &crate::modulation::AudioValues) {
        let time = self.start_time.elapsed().as_secs_f32();
        self.modulation.update(time, audio_values);
    }

    /// Render all channels and composite them via crossfader, then apply master effects.
    pub fn render(
        &mut self,
        context: &GpuContext,
        audio_data: &crate::audio::AudioData,
        audio_values: &crate::modulation::AudioValues,
    ) -> Result<()> {
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
                        self.crossfader,
                        bsc.to,
                        duration_secs,
                        CrossfadeEasing::EaseInOut,
                    ));
                    bsc.started = true;
                    log::info!(
                        "Beat-synced crossfade started: {:.1} beats at {:.0} BPM = {:.2}s",
                        bsc.beats,
                        bpm,
                        duration_secs
                    );
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

        // Compute effective opacity per channel (stack-allocated for the common 2-channel case)
        let channel_count = self.channels.len();
        let two_ch_buf: [f32; 2];
        let n_ch_buf: Vec<f32>;
        let effective_opacities: &[f32] = if channel_count == 2 {
            two_ch_buf = [
                (1.0 - self.crossfader) * self.channels[0].opacity,
                self.crossfader * self.channels[1].opacity,
            ];
            &two_ch_buf
        } else {
            n_ch_buf = self.channels.iter().map(|ch| ch.opacity).collect();
            &n_ch_buf
        };

        // Always tick video frames on every channel so players stay in sync
        // even when a channel is fully faded out by the crossfader.
        for channel in self.channels.iter_mut() {
            channel.tick_video_frames(context);
        }

        for (ch_idx, channel) in self.channels.iter_mut().enumerate() {
            if effective_opacities.get(ch_idx).copied().unwrap_or(0.0) < 0.001 {
                // Reset stats so culled channels don't show stale render metrics
                channel.render_time_ms = 0.0;
                channel.active_deck_count = 0;
                continue;
            }
            if let Err(e) = channel.render(context, audio_data, &self.modulation, ch_idx, time, dt)
            {
                log::error!("Channel {} render failed, skipping: {}", ch_idx, e);
                continue;
            }
        }

        self.sync_transition_progress();
        let composite_cmds = self.composite_channels(context)?;
        self.apply_master_effects(context, audio_data, time, composite_cmds)?;

        self.frame_count += 1;
        Ok(())
    }

    fn composite_channels(&mut self, context: &GpuContext) -> Result<Vec<wgpu::CommandBuffer>> {
        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        let channel_count = self.channels.len();
        if channel_count == 0 {
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                    multiview_mask: None,
                });
            }
            cmd_buffers.push(encoder.finish());
            return Ok(cmd_buffers);
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

                return Ok(Vec::new());
            }
        }

        // Fallback: opacity-based crossfade
        //
        // For 2-channel mode the first channel is blitted onto a cleared-to-black
        // target using ALPHA_BLENDING.  The hardware blend applies SrcAlpha to the
        // RGB output, so if the blit shader also multiplies alpha by opacity, the
        // effective weight becomes opacity² (double-application).
        //
        // To avoid this, the first channel is always blitted at full opacity and
        // the crossfader value is used solely as the second channel's composite
        // opacity.  The composite shader performs `mix(dst, src, src_a)`, which
        // yields the correct linear crossfade: (1-cf)·A + cf·B.
        let opacities = self.compositing_opacities();

        // Batch channel compositing into command buffers for deferred submission.
        let mut is_first = true;
        for (_i, (channel, &opacity)) in self.channels.iter().zip(opacities.iter()).enumerate() {
            if opacity <= 0.0 {
                continue;
            }

            if is_first {
                // First visible channel: simple blit copy
                self.blit_pipeline.set_opacity(&context.queue, opacity);
                let bind_group = self
                    .blit_pipeline
                    .create_bind_group(&context.device, &channel.composite_view);
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Mixer Composite Encoder (first)"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Mixer Composite Pass (first)"),
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
                        multiview_mask: None,
                    });
                    self.blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
                is_first = false;
            } else {
                // Subsequent channels: snapshot + composite shader
                let mut copy_encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Mixer Snapshot Copy"),
                        });
                copy_encoder.copy_texture_to_texture(
                    self.composite_texture.as_image_copy(),
                    self.effect_ping_texture.as_image_copy(),
                    self.composite_texture.size(),
                );

                let blend_mode = channel.blend_mode;
                self.composite_pipeline.set_params(
                    &context.queue,
                    opacity,
                    blend_mode.to_index(),
                    [1.0, 1.0],
                    [0.0, 0.0],
                );
                let bind_group = self.composite_pipeline.create_bind_group(
                    &context.device,
                    &channel.composite_view,
                    &self.effect_ping_view,
                );
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Mixer Composite Encoder"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Mixer Composite Pass"),
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
                        multiview_mask: None,
                    });
                    self.composite_pipeline
                        .render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(copy_encoder.finish());
                cmd_buffers.push(encoder.finish());
            }
        }

        Ok(cmd_buffers)
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
        let (sub_tex, sub_view) = match self.sub_mix_cache.get(indices) {
            Some(entry) => entry,
            None => return,
        };

        let opacities = self.compositing_opacities();

        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        let mut is_first = true;
        for &ch_idx in indices {
            if ch_idx >= self.channels.len() {
                continue;
            }
            let channel = &self.channels[ch_idx];
            let opacity = opacities.get(ch_idx).copied().unwrap_or(0.0);
            if opacity <= 0.0 {
                continue;
            }

            if is_first {
                // First visible channel: simple blit copy
                self.blit_pipeline.set_opacity(&context.queue, opacity);
                let bind_group = self
                    .blit_pipeline
                    .create_bind_group(&context.device, &channel.composite_view);
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Sub-mix Composite Encoder (first)"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Sub-mix Composite Pass (first)"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: sub_view,
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
                    self.blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
                is_first = false;
            } else {
                // Subsequent channels: snapshot sub-mix → effect_ping, composite shader
                let mut copy_encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Sub-mix Snapshot Copy"),
                        });
                copy_encoder.copy_texture_to_texture(
                    sub_tex.as_image_copy(),
                    self.effect_ping_texture.as_image_copy(),
                    sub_tex.size(),
                );

                let blend_mode = channel.blend_mode;
                self.composite_pipeline.set_params(
                    &context.queue,
                    opacity,
                    blend_mode.to_index(),
                    [1.0, 1.0],
                    [0.0, 0.0],
                );
                let bind_group = self.composite_pipeline.create_bind_group(
                    &context.device,
                    &channel.composite_view,
                    &self.effect_ping_view,
                );
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Sub-mix Composite Encoder"),
                        });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Sub-mix Composite Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: sub_view,
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
                    self.composite_pipeline
                        .render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(copy_encoder.finish());
                cmd_buffers.push(encoder.finish());
            }
        }

        if is_first {
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Sub-mix Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Sub-mix Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: sub_view,
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
            }
            cmd_buffers.push(encoder.finish());
        }

        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }
    }

    /// Compute per-channel compositing opacities.
    ///
    /// For 2-channel mode the first channel is always 1.0 (blitted at full
    /// opacity) and the crossfader value drives the second channel's composite
    /// weight.  For N-channel mode each channel uses its own opacity.
    fn compositing_opacities(&self) -> CompositingOpacities {
        if self.channels.len() == 2 {
            CompositingOpacities::Two([1.0, self.crossfader])
        } else {
            CompositingOpacities::Many(self.channels.iter().map(|ch| ch.opacity).collect())
        }
    }

    /// Get the sub-mix texture view for a given set of channel indices.
    pub fn get_sub_mix_view(&self, indices: &[usize]) -> Option<&wgpu::TextureView> {
        self.sub_mix_cache.get(indices).map(|(_, v)| v)
    }

    fn apply_master_effects(
        &mut self,
        context: &GpuContext,
        audio_data: &crate::audio::AudioData,
        time: f32,
        mut cmd_buffers: Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        if self.master_effects.is_empty() {
            if !cmd_buffers.is_empty() {
                context.queue.submit(cmd_buffers);
            }
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

        for effect in self.master_effects.iter_mut() {
            if !effect.enabled {
                continue;
            }

            let (input_view, output_view) = if read_from_composite {
                (&self.composite_view, &self.effect_ping_view)
            } else {
                (&self.effect_ping_view, &self.composite_view)
            };

            effect.apply(
                context,
                input_view,
                output_view,
                &uniforms,
                &mut cmd_buffers,
            )?;
            read_from_composite = !read_from_composite;
        }

        if !read_from_composite {
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
