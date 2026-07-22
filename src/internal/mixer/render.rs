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
    /// Pre-update modulation engine with latest audio + analyzer data.
    pub fn update_modulation(
        &mut self,
        audio_values: &crate::modulation::AudioValues,
        analyzer_values: &crate::modulation::AnalyzerValues,
    ) {
        let time = self.start_time.elapsed().as_secs_f32();
        self.modulation.update(time, audio_values, analyzer_values);
    }

    /// Drive knob/fader macros from any modulation assigned to their value, then
    /// fan the modulated value out to every target. The macro's stored value is
    /// the base (manual set point); modulation rides on top as an offset. Only
    /// macros with an active assignment on `macro_<uuid>:value` pay the cost.
    /// See `/spec/macro-controls.md` §Macro Value Modulation.
    ///
    /// Must run after `ModulationEngine::update` and before compositing so
    /// opacity/param targets take effect the same frame.
    pub fn apply_macro_modulation(&mut self) {
        if self.macros.macros().is_empty() || self.modulation.source_count() == 0 {
            return;
        }
        // Gather writes first (shared borrows of macros + modulation), then apply
        // them mutably through the router — mirrors the `macro/<uuid>/value` route.
        let mut writes: Vec<(String, f32)> = Vec::new();
        for m in self.macros.macros() {
            let key = crate::macros::Macro::value_mod_key(&m.uuid);
            if !self.modulation.has_modulation(&key) {
                continue;
            }
            let offset = self.modulation.get_modulation(&key);
            writes.extend(m.modulated_fanout(offset));
        }
        for (path, value) in writes {
            if let Err(e) = crate::param_router::apply_param_by_path(self, &path, value) {
                log::debug!("macro modulation target '{path}' skipped: {e}");
            }
        }
    }

    /// Render all channels and composite them via crossfader, then apply master effects.
    /// `target_fps` is used for adaptive deck render skipping budget calculation.
    /// `preview_channels` are force-rendered even when culled by opacity, so their
    /// off-air previews update live — without affecting the compositor (see
    /// /spec/channel-preview.md).
    pub fn render(
        &mut self,
        context: &GpuContext,
        audio_data: &crate::audio::AudioData,
        audio_values: &crate::modulation::AudioValues,
        analyzer_values: &crate::modulation::AnalyzerValues,
        target_fps: u32,
        preview_channels: &[usize],
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

        // ── GPU Timestamp: read previous frame results ──────────────────
        // Only read if the map_async callback has fired (buffer is actually mapped).
        // staging_mapped_idx holds the index of the ready buffer, or usize::MAX if none.
        let ready_idx = self
            .staging_mapped_idx
            .load(std::sync::atomic::Ordering::Acquire);
        if ready_idx != usize::MAX {
            if let Some(ref staging) = self.staging_buffers {
                let buf = &staging[ready_idx];
                {
                    let slice = buf.slice(..);
                    let mapped = slice.get_mapped_range();
                    let timestamps: &[u64] = bytemuck::cast_slice(&mapped);
                    let period_us = self.timestamp_period / 1000.0; // ns → µs
                    self.last_frame_gpu_times.clear();
                    for &(ch_idx, deck_idx, begin, end) in &self.prev_timing_allocations {
                        if (end as usize) < timestamps.len() {
                            let begin_ts = timestamps[begin as usize];
                            let end_ts = timestamps[end as usize];
                            if end_ts > begin_ts {
                                let gpu_us = (end_ts - begin_ts) as f32 * period_us;
                                self.last_frame_gpu_times.insert((ch_idx, deck_idx), gpu_us);
                            }
                        }
                    }
                    drop(mapped);
                }
                buf.unmap();
                self.staging_mapped_idx
                    .store(usize::MAX, std::sync::atomic::Ordering::Release);
                // The map has been consumed and the buffer unmapped — clear the
                // in-flight guard so the resolve path may issue the next map.
                self.timing_map_inflight = false;
            }
        }

        // Apply GPU timing results to deck slots (EMA smoothing)
        if !self.last_frame_gpu_times.is_empty() {
            for (ch_idx, channel) in self.channels.iter_mut().enumerate() {
                for (dk_idx, slot) in channel.decks.iter_mut().enumerate() {
                    if let Some(&gpu_us) = self.last_frame_gpu_times.get(&(ch_idx, dk_idx)) {
                        if slot.gpu_render_cost_us > 0.0 {
                            slot.gpu_render_cost_us = 0.2 * gpu_us + 0.8 * slot.gpu_render_cost_us;
                        } else {
                            slot.gpu_render_cost_us = gpu_us;
                        }
                    }
                }
            }
        }

        // Update global modulation engine
        let t_modulation = std::time::Instant::now();
        let time = self.start_time.elapsed().as_secs_f32();
        self.modulation.update(time, audio_values, analyzer_values);
        // Drive any modulation-assigned macros and fan their values out to targets
        // before compositing reads opacities/params this frame.
        self.apply_macro_modulation();
        let modulation_us = t_modulation.elapsed().as_micros();

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
        // Uses a dedicated encoder for double-buffered staging uploads
        // (copy_buffer_to_texture) to avoid per-frame staging allocation stalls.
        // The finished command buffer is NOT submitted here — it is passed as a
        // prefix to the first channel's deck submit, eliminating a separate
        // queue.submit() call that would stall under GPU pressure.
        let t_video_tick = std::time::Instant::now();
        let mut video_encoder =
            context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Video Upload Encoder"),
                });
        for channel in self.channels.iter_mut() {
            channel.tick_video_frames(&mut video_encoder);
        }
        let mut prefix_cmds = vec![video_encoder.finish()];
        let video_tick_us = t_video_tick.elapsed().as_micros();

        // Count total active decks from last frame for per-deck budget calculation
        let total_active_decks: u32 = self.channels.iter().map(|ch| ch.active_deck_count).sum();

        // Snapshot the current GPU load ratio for this frame's skip decisions.
        // This is updated at the end of the frame based on actual vs CPU-measured time.
        let gpu_load_ratio = self.gpu_load_ratio;

        // Allocate per-frame GPU timing context (128 queries = 64 deck measurements)
        let mut timing_frame = if self.query_set.is_some() {
            Some(super::GpuTimingFrame::new(128))
        } else {
            None
        };

        let profiling = self.perf_profile_frames > 0;
        let t_channels = std::time::Instant::now();
        let mut rendered_channels: u32 = 0;
        let mut per_ch_gpu_us: Vec<(String, u128)> = Vec::new();
        for (ch_idx, channel) in self.channels.iter_mut().enumerate() {
            let culled = effective_opacities.get(ch_idx).copied().unwrap_or(0.0) < 0.001;
            // Cued channels are force-rendered so their off-air previews update
            // live; the compositor stays opacity-gated so they never leak to output.
            if culled && !preview_channels.contains(&ch_idx) {
                // Reset stats so culled channels don't show stale render metrics
                channel.render_time_ms = 0.0;
                channel.active_deck_count = 0;
                continue;
            }
            if let Err(e) = channel.render(
                context,
                audio_data,
                &self.modulation,
                ch_idx,
                time,
                dt,
                target_fps,
                total_active_decks,
                gpu_load_ratio,
                &mut prefix_cmds,
                timing_frame.as_mut(),
                self.query_set.as_ref(),
            ) {
                log::error!("Channel {} render failed, skipping: {}", ch_idx, e);
                continue;
            }
            rendered_channels += 1;

            // Per-channel GPU drain when profiling
            if profiling {
                let t = std::time::Instant::now();
                let _ = context.device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_millis(200)),
                });
                per_ch_gpu_us.push((channel.name.clone(), t.elapsed().as_micros()));
            }
        }
        let channels_us = t_channels.elapsed().as_micros();

        // If no channel consumed the video upload prefix (all channels faded
        // out or errored), submit it now so video players still advance.
        if !prefix_cmds.is_empty() {
            context.queue.submit(prefix_cmds);
        }

        // Request re-mapping of staging buffers AFTER the submit that
        // included the video upload commands. map_async can complete
        // synchronously on Metal/UMA so must not be called before that submit.
        for channel in self.channels.iter_mut() {
            channel.request_video_remap();
        }

        // GPU profiling: drain remaining channel GPU work (per-channel drains
        // already happened inside the loop above when profiling)
        let gpu_channels_us = if profiling {
            let t = std::time::Instant::now();
            let _ = context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_millis(200)),
            });
            t.elapsed().as_micros()
        } else {
            0
        };

        self.sync_transition_progress();
        let t_mixer_composite = std::time::Instant::now();
        let composite_cmds = self.composite_channels(context)?;
        let mixer_composite_us = t_mixer_composite.elapsed().as_micros();

        let t_master_fx = std::time::Instant::now();
        self.apply_master_effects(context, audio_data, time, composite_cmds)?;
        let master_fx_us = t_master_fx.elapsed().as_micros();

        // Tonemap pass: compress HDR composite into displayable [0,1] range.
        // Bypass mode is a no-op (values clamp at the output boundary anyway).
        self.apply_tonemap(context);
        self.apply_lut(context);

        // GPU profiling: drain composite + master FX GPU work
        let gpu_composite_us = if profiling {
            let t = std::time::Instant::now();
            let _ = context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_millis(200)),
            });
            t.elapsed().as_micros()
        } else {
            0
        };

        // ── GPU Timestamp: resolve + readback ───────────────────────────
        // Only issue a new resolve/copy/map when no map is in flight.
        // `timing_map_inflight` is set the moment a map_async is *issued* (not
        // when its callback fires), so the pending window between issue and
        // callback is covered. Deriving this from `staging_mapped_idx` alone is
        // unsound: it stays `MAX` until the callback runs, which would let a
        // second map_async be issued on the other buffer, leaving one buffer
        // permanently mapped and crashing the next submit with "still mapped".
        // Dropping an occasional measurement is harmless; a stuck map is fatal.
        if !self.timing_map_inflight {
            if let (Some(ref qs), Some(ref resolve_buf), Some(ref staging)) =
                (&self.query_set, &self.resolve_buffer, &self.staging_buffers)
            {
                if let Some(ref timing) = timing_frame {
                    let query_count = timing.query_count();
                    if query_count > 0 {
                        let mut enc = context.device.create_command_encoder(
                            &wgpu::CommandEncoderDescriptor {
                                label: Some("GPU Timing Resolve"),
                            },
                        );
                        enc.resolve_query_set(qs, 0..query_count, resolve_buf, 0);
                        let byte_count = (query_count as u64) * 8;
                        let write_idx = self.staging_index;
                        enc.copy_buffer_to_buffer(
                            resolve_buf,
                            0,
                            &staging[write_idx],
                            0,
                            byte_count,
                        );
                        context.queue.submit(std::iter::once(enc.finish()));

                        // Map the staging buffer for reading next frame.
                        // The callback stores the buffer index so the read
                        // path knows exactly which buffer to unmap.
                        let mapped_flag = self.staging_mapped_idx.clone();
                        staging[write_idx].slice(..).map_async(
                            wgpu::MapMode::Read,
                            move |result| {
                                if result.is_ok() {
                                    mapped_flag
                                        .store(write_idx, std::sync::atomic::Ordering::Release);
                                }
                            },
                        );

                        // Mark the map in flight from the moment it is issued so
                        // no second map_async can be started before the read path
                        // consumes and unmaps this buffer.
                        self.timing_map_inflight = true;

                        // Save allocations for readback next frame
                        self.prev_timing_allocations = timing.allocations.clone();
                        self.staging_index = 1 - self.staging_index;
                    }
                }
            }
        }

        self.frame_count += 1;

        // Update GPU load ratio: how much CPU-measured render cost underestimates
        // true GPU cost. Only meaningful when we're actually GPU-bound (missing target).
        // When meeting target, dt includes vsync idle wait which would inflate the ratio.
        let cpu_render_us = channels_us + mixer_composite_us + master_fx_us;
        let frame_budget_us = if target_fps > 0 {
            1_000_000.0 / target_fps as f32
        } else {
            f32::MAX
        };
        let actual_frame_us = dt * 1_000_000.0;
        if cpu_render_us > 100 && dt > 0.0 {
            let raw_ratio = if actual_frame_us > frame_budget_us * 1.05 {
                // GPU-bound: frame time exceeds budget, ratio captures real pressure
                actual_frame_us / cpu_render_us as f32
            } else {
                // Meeting target: decay toward 1.0 (no GPU pressure)
                1.0
            };
            let clamped = raw_ratio.clamp(1.0, 200.0);
            // EMA smoothing (α = 0.15) — responsive but not jittery
            self.gpu_load_ratio = 0.15 * clamped + 0.85 * self.gpu_load_ratio;
        }

        // Update GPU utilization %: sum of per-deck GPU costs / frame budget.
        // Prefer GPU timestamp data; fall back to CPU cost × gpu_load_ratio.
        if frame_budget_us < f32::MAX {
            let total_gpu_us: f32 = self
                .channels
                .iter()
                .flat_map(|ch| ch.decks.iter())
                .filter(|s| !s.mute && s.opacity > 0.0)
                .map(|s| {
                    if s.gpu_render_cost_us > 0.0 {
                        s.gpu_render_cost_us
                    } else {
                        s.render_cost_us * self.gpu_load_ratio
                    }
                })
                .sum();
            let raw_util = (total_gpu_us / frame_budget_us) * 100.0;
            let clamped = raw_util.clamp(0.0, 999.0);
            self.gpu_utilization = 0.15 * clamped + 0.85 * self.gpu_utilization;
        }

        // GPU profiling: detailed per-frame log
        if profiling {
            self.perf_profile_frames -= 1;
            // Per-channel GPU drain breakdown
            let ch_gpu_str: String = per_ch_gpu_us
                .iter()
                .map(|(name, us)| format!("{}={}us", name, us))
                .collect::<Vec<_>>()
                .join(", ");
            let total_per_ch_gpu: u128 = per_ch_gpu_us.iter().map(|(_, us)| us).sum();
            log::info!(
                "[PERF_PROFILE] frame={} | \
                 cpu_encode: channels={}us composite={}us master_fx={}us video={}us | \
                 gpu_drain: per_ch=[{}] ch_total={}us residual={}us composite={}us | \
                 channels_rendered={} total_decks={} | \
                 remaining={}",
                self.frame_count,
                channels_us,
                mixer_composite_us,
                master_fx_us,
                video_tick_us,
                ch_gpu_str,
                total_per_ch_gpu,
                gpu_channels_us,
                gpu_composite_us,
                rendered_channels,
                total_active_decks,
                self.perf_profile_frames,
            );
        }

        // Log mixer-level timing every 120 frames
        if self.frame_count.is_multiple_of(120) {
            let total_us = now.elapsed().as_micros();
            log::debug!(
                "[PERF] mixer | channels_rendered={} channels={}us | \
                 mixer_composite={}us master_fx={}us | \
                 modulation={}us video_tick={}us | \
                 gpu_load_ratio={:.1}x gpu_util={:.0}% | \
                 total={}us ({:.1}ms)",
                rendered_channels,
                channels_us,
                mixer_composite_us,
                master_fx_us,
                modulation_us,
                video_tick_us,
                self.gpu_load_ratio,
                self.gpu_utilization,
                total_us,
                total_us as f64 / 1000.0,
            );
        }

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
            if let Some(transition) = &mut self.active_transition {
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

                transition.params.build_buffer_data();
                if let Some(buf) = transition.params.buffer() {
                    context
                        .queue
                        .write_buffer(buf, 0, transition.params.scratch());
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
        // For 2-channel mode the first channel is blitted onto a cleared-to-
        // transparent target using ALPHA_BLENDING.  The hardware blend applies
        // SrcAlpha to the RGB output, so if the blit shader also multiplies alpha
        // by opacity, the effective weight becomes opacity² (double-application).
        //
        // To avoid this, the first channel is always blitted at full opacity and
        // the crossfader value is used solely as the second channel's composite
        // opacity.  The composite shader performs `mix(dst, src, src_a)`, which
        // yields the correct linear crossfade: (1-cf)·A + cf·B.
        //
        // The clear is TRANSPARENT (not BLACK) so the program output carries the
        // channels' alpha through to alpha-capable outputs. Because the clear RGB
        // is zero either way, RGB is byte-identical to the old over-black result
        // (the program becomes premultiplied-alpha); opaque content (alpha=1) and
        // the over-black display path are unchanged. See /spec/html-source.md §2.
        let opacities = self.compositing_opacities();

        // Batch channel compositing into command buffers for deferred submission.
        let mut is_first = true;
        let mut slot: usize = 0;
        for (channel, &opacity) in self.channels.iter().zip(opacities.iter()) {
            if opacity <= 0.0 {
                continue;
            }

            if is_first {
                // First visible channel: per-draw params blit.
                // Channel composites are premultiplied-alpha (see blit_pipeline blend).
                self.blit_pipeline.write_params_slot(
                    &context.queue,
                    slot,
                    opacity,
                    [1.0, 1.0],
                    [0.0, 0.0],
                    true,
                );
                let bind_group = self.blit_pipeline.create_ring_bind_group(
                    &context.device,
                    &channel.composite_view,
                    slot,
                );
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
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    self.blit_pipeline
                        .render_at_slot(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
                is_first = false;
            } else {
                // Subsequent channels: snapshot + per-draw params composite
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
                self.composite_pipeline.write_params_slot(
                    &context.queue,
                    slot,
                    opacity,
                    blend_mode.to_index(),
                    [1.0, 1.0],
                    [0.0, 0.0],
                    true,
                );
                let bind_group = self.composite_pipeline.create_ring_bind_group(
                    &context.device,
                    &channel.composite_view,
                    &self.effect_ping_view,
                    slot,
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
                        .render_at_slot(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(copy_encoder.finish());
                cmd_buffers.push(encoder.finish());
            }
            slot += 1;
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
                let tex = context.create_compositing_texture(width, height);
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                self.sub_mix_cache.insert(indices.clone(), (tex, view));
            }
            self.composite_sub_mix(&indices, context);

            // Tonemap the sub-mix in-place (same pattern as main composite).
            // Uses effect_ping as scratch — safe because composite_sub_mix has
            // already finished with it by the time we get here.
            if let Some((sub_tex, sub_view)) = self.sub_mix_cache.get(&indices) {
                tonemap_in_place(
                    self.tonemap_mode,
                    &self.tonemap_pipeline,
                    sub_tex,
                    sub_view,
                    &self.effect_ping_texture,
                    &self.effect_ping_view,
                    context,
                );
            }
            if let Some((sub_tex, sub_view)) = self.sub_mix_cache.get(&indices) {
                apply_lut_in_place(
                    &self.lut_pipeline,
                    self.active_lut.as_ref(),
                    sub_tex,
                    sub_view,
                    &self.effect_ping_texture,
                    &self.effect_ping_view,
                    context,
                );
            }
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
        let mut slot: usize = 0;
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
                // First visible channel: per-draw params blit.
                // Channel composites are premultiplied-alpha (see blit_pipeline blend).
                self.blit_pipeline.write_params_slot(
                    &context.queue,
                    slot,
                    opacity,
                    [1.0, 1.0],
                    [0.0, 0.0],
                    true,
                );
                let bind_group = self.blit_pipeline.create_ring_bind_group(
                    &context.device,
                    &channel.composite_view,
                    slot,
                );
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
                    self.blit_pipeline
                        .render_at_slot(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
                is_first = false;
            } else {
                // Subsequent channels: snapshot sub-mix → effect_ping, per-draw params composite
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
                self.composite_pipeline.write_params_slot(
                    &context.queue,
                    slot,
                    opacity,
                    blend_mode.to_index(),
                    [1.0, 1.0],
                    [0.0, 0.0],
                    true,
                );
                let bind_group = self.composite_pipeline.create_ring_bind_group(
                    &context.device,
                    &channel.composite_view,
                    &self.effect_ping_view,
                    slot,
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
                        .render_at_slot(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(copy_encoder.finish());
                cmd_buffers.push(encoder.finish());
            }
            slot += 1;
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

    /// Apply tonemap to the main composite texture in-place.
    fn apply_tonemap(&self, context: &GpuContext) {
        tonemap_in_place(
            self.tonemap_mode,
            &self.tonemap_pipeline,
            &self.composite_texture,
            &self.composite_view,
            &self.effect_ping_texture,
            &self.effect_ping_view,
            context,
        );
    }

    /// Apply LUT to the main composite texture in-place.
    /// Runs after tonemap, before outputs read the composite.
    fn apply_lut(&self, context: &GpuContext) {
        apply_lut_in_place(
            &self.lut_pipeline,
            self.active_lut.as_ref(),
            &self.composite_texture,
            &self.composite_view,
            &self.effect_ping_texture,
            &self.effect_ping_view,
            context,
        );
    }

    /// Prepare tonemapped copies of individual channel composites.
    /// Called for channels used as direct `OutputSource::Channel(idx)` sources.
    /// Channel composites can't be tonemapped in-place because they feed into
    /// the mixer composite on subsequent frames.
    pub fn prepare_channel_tonemaps(&mut self, channel_indices: &[usize], context: &GpuContext) {
        use crate::renderer::tonemap::TonemapMode;

        if self.tonemap_mode == TonemapMode::Bypass {
            self.tonemapped_channel_cache.clear();
            return;
        }

        // Remove stale entries
        self.tonemapped_channel_cache
            .retain(|idx, _| channel_indices.contains(idx));

        for &ch_idx in channel_indices {
            let ch_view = match self.channels.get(ch_idx) {
                Some(ch) => &ch.composite_view,
                None => continue,
            };

            // Create cached texture if needed
            if !self.tonemapped_channel_cache.contains_key(&ch_idx) {
                let width = self.composite_texture.width();
                let height = self.composite_texture.height();
                let tex = context.create_compositing_texture(width, height);
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                self.tonemapped_channel_cache.insert(ch_idx, (tex, view));
            }

            // Tonemap directly: channel composite → cached texture
            // (no copy needed since source and target are different textures)
            let cached_view = &self.tonemapped_channel_cache[&ch_idx].1;
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Channel Tonemap Encoder"),
                    });
            self.tonemap_pipeline
                .render(&context.device, &mut encoder, ch_view, cached_view);
            context.queue.submit(Some(encoder.finish()));

            // Apply LUT to the tonemapped channel copy
            if let Some(lut) = &self.active_lut {
                let (cache_tex, cache_view) = &self.tonemapped_channel_cache[&ch_idx];
                let mut lut_encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Channel LUT Encoder"),
                        });
                // Copy cache → effect_ping, then LUT ping → cache
                lut_encoder.copy_texture_to_texture(
                    cache_tex.as_image_copy(),
                    self.effect_ping_texture.as_image_copy(),
                    cache_tex.size(),
                );
                self.lut_pipeline.render(
                    &context.device,
                    &mut lut_encoder,
                    &self.effect_ping_view,
                    cache_view,
                    lut,
                );
                context.queue.submit(Some(lut_encoder.finish()));
            }
        }
    }
}

/// Tonemap a texture in-place using a scratch texture for the copy.
/// Skips the pass in Bypass mode.
fn tonemap_in_place(
    mode: crate::renderer::tonemap::TonemapMode,
    pipeline: &crate::renderer::tonemap::TonemapPipeline,
    target_tex: &wgpu::Texture,
    target_view: &wgpu::TextureView,
    scratch_tex: &wgpu::Texture,
    scratch_view: &wgpu::TextureView,
    context: &GpuContext,
) {
    use crate::renderer::tonemap::TonemapMode;

    if mode == TonemapMode::Bypass {
        return;
    }

    let mut encoder = context
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Tonemap Encoder"),
        });

    // Copy target → scratch so the shader can read scratch and write back to target.
    encoder.copy_texture_to_texture(
        target_tex.as_image_copy(),
        scratch_tex.as_image_copy(),
        target_tex.size(),
    );

    pipeline.render(&context.device, &mut encoder, scratch_view, target_view);
    context.queue.submit(Some(encoder.finish()));
}

/// Apply a LUT to a texture in-place using a scratch texture.
/// No-op if no LUT is loaded.
fn apply_lut_in_place(
    pipeline: &crate::renderer::lut::LutPipeline,
    lut: Option<&crate::renderer::lut::LoadedLut>,
    target_tex: &wgpu::Texture,
    target_view: &wgpu::TextureView,
    scratch_tex: &wgpu::Texture,
    scratch_view: &wgpu::TextureView,
    context: &GpuContext,
) {
    let lut = match lut {
        Some(l) => l,
        None => return,
    };

    let mut encoder = context
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("LUT Encoder"),
        });

    // Copy target → scratch so the shader can read scratch and write back to target.
    encoder.copy_texture_to_texture(
        target_tex.as_image_copy(),
        scratch_tex.as_image_copy(),
        target_tex.size(),
    );

    pipeline.render(
        &context.device,
        &mut encoder,
        scratch_view,
        target_view,
        lut,
    );
    context.queue.submit(Some(encoder.finish()));
}
