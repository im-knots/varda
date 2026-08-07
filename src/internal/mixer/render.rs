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
    ///
    /// # Errors
    ///
    /// Returns an error if any channel fails to render its decks, or if master
    /// effect / tonemap / LUT compositing fails on the GPU device.
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
            if let Some(value) = auto.tick(dt) {
                self.crossfader = value;
            } else {
                let target = auto.to;
                self.crossfader = target;
                self.auto_crossfade = None;
                log::info!("Auto-crossfade complete, crossfader = {target:.2}");
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
                if let Some(value) = auto.tick(dt) {
                    self.crossfader = value;
                } else {
                    let target = bsc.to;
                    self.crossfader = target;
                    self.beat_sync_crossfade = None;
                    log::info!("Beat-synced crossfade complete, crossfader = {target:.2}");
                }
            }
        }

        // Tick transition sequence
        let bpm = audio_data.bpm.map(f64::from);
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
        for channel in &mut self.channels {
            channel.tick_video_frames(&mut video_encoder, target_fps);
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
            // A tapped channel is force-rendered for a stronger reason: some
            // visible deck reads its composite, so its own opacity says nothing
            // about whether it reaches the program. Culling it would show that
            // deck black. See spec/program-tap.md.
            let spared = preview_channels.contains(&ch_idx) || channel.tap_view().is_some();
            if culled && !spared {
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
                log::error!("Channel {ch_idx} render failed, skipping: {e}");
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
            context.submit(prefix_cmds);
        }

        // Request re-mapping of staging buffers AFTER the submit that
        // included the video upload commands. map_async can complete
        // synchronously on Metal/UMA so must not be called before that submit.
        for channel in &mut self.channels {
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
        // With a master tap the composite and master FX write into the tap
        // target instead, so the pre-tonemap program survives into next frame
        // without a copy. Swapping the fields keeps those two passes unaware.
        self.swap_master_tap();
        let t_mixer_composite = std::time::Instant::now();
        let composite_cmds = self.composite_channels(context);
        let mixer_composite_us = t_mixer_composite.elapsed().as_micros();

        let t_master_fx = std::time::Instant::now();
        let master_fx = self.apply_master_effects(context, audio_data, time, composite_cmds);
        let master_fx_us = t_master_fx.elapsed().as_micros();
        // Unconditional, so a failed effect chain cannot leave the composite
        // and tap fields permanently transposed.
        self.swap_master_tap();
        master_fx?;

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
                        let byte_count = u64::from(query_count) * 8;
                        let write_idx = self.staging_index;
                        enc.copy_buffer_to_buffer(
                            resolve_buf,
                            0,
                            &staging[write_idx],
                            0,
                            byte_count,
                        );
                        context.submit(std::iter::once(enc.finish()));

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
                        self.prev_timing_allocations.clone_from(&timing.allocations);
                        self.staging_index = 1 - self.staging_index;
                    }
                }
            }
        }

        self.frame_count += 1;

        // Update GPU load ratio: how much CPU-measured render cost underestimates
        // true GPU cost.
        let frame_budget_us = if target_fps > 0 {
            1_000_000.0 / target_fps as f32
        } else {
            f32::MAX
        };
        let actual_frame_us = dt * 1_000_000.0;
        // A frame that encoded essentially nothing carries no evidence either
        // way, and its `dt` is whatever wall-clock gap preceded it — a paused or
        // hidden window would otherwise drive the ratio up and leave decks
        // skipping for the first moments after they come back.
        let mixer_cpu_us = channels_us + mixer_composite_us + master_fx_us;
        let (deck_gpu_us, deck_encode_us) = self.active_deck_costs();
        if mixer_cpu_us > 100 {
            if let Some(raw_ratio) = raw_gpu_load_ratio(
                deck_gpu_us,
                deck_encode_us,
                actual_frame_us,
                frame_budget_us,
            ) {
                // EMA smoothing (α = 0.15) — responsive but not jittery
                self.gpu_load_ratio = 0.15 * raw_ratio + 0.85 * self.gpu_load_ratio;
            }
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
                .map(|(name, us)| format!("{name}={us}us"))
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

    fn composite_channels(&mut self, context: &GpuContext) -> Vec<wgpu::CommandBuffer> {
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
            return cmd_buffers;
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

                return Vec::new();
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

        cmd_buffers
    }

    /// Prepare sub-mix textures for all unique multi-channel surface sources.
    pub fn prepare_sub_mixes(&mut self, sources: &[Vec<usize>], context: &GpuContext) {
        let needed: std::collections::HashSet<Vec<usize>> = sources.iter().cloned().collect();
        self.sub_mix_cache.retain(|k, _| needed.contains(k));

        for mut indices in sources.iter().cloned() {
            indices.sort_unstable();
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
        let Some((sub_tex, sub_view)) = self.sub_mix_cache.get(indices) else {
            return;
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
            context.submit(cmd_buffers);
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
                context.submit(cmd_buffers);
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

        for effect in &mut self.master_effects {
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
            context.submit(cmd_buffers);
        }

        Ok(())
    }

    /// Exchange `composite_*` with the master tap target, when one exists.
    /// Called in pairs around the composite and master-FX passes.
    fn swap_master_tap(&mut self) {
        if let Some((mut tex, mut view)) = self.master_tap.take() {
            std::mem::swap(&mut self.composite_texture, &mut tex);
            std::mem::swap(&mut self.composite_view, &mut view);
            self.master_tap = Some((tex, view));
        }
    }

    /// Tonemap the master program.
    ///
    /// Without a tap this is in-place on the composite. With one, the
    /// pre-tonemap program is already in a separate target, so the pass reads
    /// straight from it and the in-place scratch copy disappears. Bypass still
    /// has to move the pixels across, which is the one case a tap costs a copy.
    fn apply_tonemap(&self, context: &GpuContext) {
        let Some((tap_texture, tap_view)) = &self.master_tap else {
            tonemap_in_place(
                self.tonemap_mode,
                &self.tonemap_pipeline,
                &self.composite_texture,
                &self.composite_view,
                &self.effect_ping_texture,
                &self.effect_ping_view,
                context,
            );
            return;
        };

        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Master Tap Tonemap Encoder"),
            });
        if self.tonemap_mode == crate::renderer::tonemap::TonemapMode::Bypass {
            encoder.copy_texture_to_texture(
                tap_texture.as_image_copy(),
                self.composite_texture.as_image_copy(),
                self.composite_texture.size(),
            );
        } else {
            self.tonemap_pipeline.render(
                &context.device,
                &mut encoder,
                tap_view,
                &self.composite_view,
            );
        }
        context.submit(Some(encoder.finish()));
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
            let tonemap_target = &self.tonemapped_channel_cache[&ch_idx].1;
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Channel Tonemap Encoder"),
                    });
            self.tonemap_pipeline
                .render(&context.device, &mut encoder, ch_view, tonemap_target);
            context.submit(Some(encoder.finish()));

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
                context.submit(Some(lut_encoder.finish()));
            }
        }
    }

    /// Summed GPU and CPU render costs over the decks that are actually drawing
    /// *and* have GPU timing for this frame.
    ///
    /// Both sums come from the same set of decks so their quotient stays a
    /// like-for-like comparison; a deck still waiting on its first timestamp
    /// would otherwise contribute CPU cost with no GPU cost and understate the
    /// ratio. Returns `(0.0, 0.0)` when no deck has timing yet, which is what
    /// puts [`raw_gpu_load_ratio`] onto its fallback.
    fn active_deck_costs(&self) -> (f32, f32) {
        self.channels
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .filter(|s| !s.mute && s.opacity > 0.0 && s.gpu_render_cost_us > 0.0)
            .fold((0.0, 0.0), |(gpu, cpu), s| {
                (gpu + s.gpu_render_cost_us, cpu + s.render_cost_us)
            })
    }
}

/// Ceiling on the load ratio. Generous enough to express a genuinely
/// GPU-bound shader — a 10 ms pass encoded in 200 µs really is 50× — while
/// still bounding how far one bad frame can push every deck toward skipping.
const MAX_GPU_LOAD_RATIO: f32 = 64.0;

/// How much to scale CPU-measured deck cost to estimate true GPU cost.
///
/// `None` means the frame carried no usable signal, leaving the smoothed ratio
/// untouched rather than dragging it toward an invented value.
///
/// GPU timestamps are the honest answer: they time the same work the CPU figure
/// times, so their quotient is the underestimation factor and nothing else.
/// Without them, how far the frame overran its budget is a coarser stand-in,
/// but it is proportionate and bounded by construction.
///
/// What this must not be is whole-frame wall time over the mixer's encode time.
/// That quotient is roughly `frame_budget / encode_time` — around 20× on a
/// perfectly healthy frame — so any render-thread work outside the mixer (a
/// screen-capture upload, say) that pushes one frame past budget made every
/// deck look twentyfold more expensive than it is, ratcheting the smoothed
/// ratio up until unrelated shader decks started skipping while the frame loop
/// still reported its target rate.
fn raw_gpu_load_ratio(
    deck_gpu_us: f32,
    deck_encode_us: f32,
    actual_frame_us: f32,
    frame_budget_us: f32,
) -> Option<f32> {
    if deck_gpu_us > 0.0 && deck_encode_us > 0.0 {
        return Some((deck_gpu_us / deck_encode_us).clamp(1.0, MAX_GPU_LOAD_RATIO));
    }
    if frame_budget_us >= f32::MAX || actual_frame_us <= 0.0 {
        return None;
    }
    // `dt` includes the vsync idle wait, so a frame at or under budget is
    // evidence of no GPU pressure rather than evidence of none being measurable.
    Some(if actual_frame_us > frame_budget_us * 1.05 {
        (actual_frame_us / frame_budget_us).clamp(1.0, MAX_GPU_LOAD_RATIO)
    } else {
        1.0
    })
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
    context.submit(Some(encoder.finish()));
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
    let Some(lut) = lut else {
        return;
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
    context.submit(Some(encoder.finish()));
}

#[cfg(test)]
mod gpu_load_ratio_tests {
    use super::{raw_gpu_load_ratio, MAX_GPU_LOAD_RATIO};

    const BUDGET_60FPS: f32 = 1_000_000.0 / 60.0;

    #[test]
    fn timestamps_give_the_plain_gpu_over_cpu_underestimation_factor() {
        // 4 ms of GPU work encoded in 500 us: deck cost is understated 8x.
        let ratio =
            raw_gpu_load_ratio(4000.0, 500.0, BUDGET_60FPS, BUDGET_60FPS).expect("timing present");
        assert!((ratio - 8.0).abs() < 1e-3, "expected 8x, got {ratio}");
    }

    #[test]
    fn timestamps_never_report_less_than_parity() {
        // Encoding can cost more than execution for a trivial pass; that is not
        // evidence decks are cheaper than measured, so the floor is 1.0.
        let ratio = raw_gpu_load_ratio(100.0, 900.0, BUDGET_60FPS, BUDGET_60FPS).expect("timing");
        assert!((ratio - 1.0).abs() < f32::EPSILON, "got {ratio}");
    }

    /// The regression this pins. A screen-capture upload costs ~1 ms of
    /// render-thread time outside the mixer, which pushes the frame a little
    /// over budget. The old formula divided whole-frame wall time by the
    /// mixer's encode time and called the result GPU pressure, yielding ~24x
    /// for a frame only 1.14x over — enough, once smoothed, to force unrelated
    /// shader decks into skipping while the loop still hit 60 fps.
    #[test]
    fn an_over_budget_frame_caused_by_non_mixer_work_stays_proportionate() {
        let actual = BUDGET_60FPS + 2425.0; // ~19.1 ms, as measured on an M2 Max
        let mixer_encode_us = 800.0;

        let old = actual / mixer_encode_us;
        assert!(
            old > 20.0,
            "precondition: the old formula really did explode ({old}x)"
        );

        // No GPU timing available, so this is the fallback path.
        let ratio = raw_gpu_load_ratio(0.0, 0.0, actual, BUDGET_60FPS).expect("over budget");
        assert!(
            (ratio - actual / BUDGET_60FPS).abs() < 1e-3,
            "overage should be reported as itself, got {ratio}"
        );
        assert!(ratio < 1.2, "a 14% overrun must not read as {ratio}x");
    }

    #[test]
    fn meeting_target_decays_toward_parity() {
        // dt includes vsync idle, so an on-time frame carries no GPU pressure.
        let ratio = raw_gpu_load_ratio(0.0, 0.0, BUDGET_60FPS, BUDGET_60FPS).expect("on time");
        assert!((ratio - 1.0).abs() < f32::EPSILON, "got {ratio}");
    }

    #[test]
    fn ratio_is_bounded_even_for_a_pathological_frame() {
        let stalled = raw_gpu_load_ratio(0.0, 0.0, 5_000_000.0, BUDGET_60FPS).expect("stalled");
        assert!((stalled - MAX_GPU_LOAD_RATIO).abs() < f32::EPSILON);
        let lopsided = raw_gpu_load_ratio(1_000_000.0, 1.0, BUDGET_60FPS, BUDGET_60FPS)
            .expect("timing present");
        assert!((lopsided - MAX_GPU_LOAD_RATIO).abs() < f32::EPSILON);
    }

    #[test]
    fn an_uncapped_target_with_no_timing_yields_no_signal() {
        assert!(raw_gpu_load_ratio(0.0, 0.0, BUDGET_60FPS, f32::MAX).is_none());
        assert!(raw_gpu_load_ratio(0.0, 0.0, 0.0, BUDGET_60FPS).is_none());
    }
}
