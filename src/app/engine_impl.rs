//! Engine trait implementations for `VardaApp`.

use super::resolve::EffectChain;
use super::VardaApp;
use crate::deck::{Deck, Effect};
use crate::depth::preprocess::{AcquiredSensor, DepthPreprocessParams};
use crate::engine::traits::{
    AnalyzerCommands, AnalyzerQueries, AudioCommands, AudioQueries, DetectCommands, MacroCommands,
    MacroQueries, MixerCommands, MixerQueries, ModulationCommands, ModulationQueries,
    OutputCommands, OutputQueries, SurfaceCommands, SurfaceQueries,
};
use crate::engine::types::{
    AnalyzerScalarInfo, AnalyzerTypeInfo, AudioBandPreset, AudioDeviceSnapshot,
    AudioPassthroughSnapshot, AudioSnapshot, AudioSourceId, BlendMode, CameraId, ContentMapping,
    CrossfadeEasing, DeliveryHealthSnapshot, EffectTarget, LFOWaveform, MixerSnapshot,
    ModulationAssignmentSnapshot, ModulationSnapshot, ModulationSourceSnapshot,
    ModulationSourceSnapshotEntry, MonitorSnapshot, OutputSnapshot, OutputSource,
    OutputWindowSnapshot, ParamValue, ScalingMode, SurfaceAssignmentSnapshot, SurfaceOutputType,
    SurfaceSnapshot,
};
use crate::modulation::ModulationSource;

use anyhow::{Context as _, Result};

/// Sanitize a float to the 0.0..=1.0 range with a fallback for NaN/Inf.
/// Used at every command boundary that accepts a unit-range float.
#[inline]
fn sanitize_unit(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

impl VardaApp {
    /// Post-construction wiring every new shader deck needs before it joins a
    /// channel: start its CPU analyzers and acquire any device its
    /// `PREPROCESSORS` block requires.
    ///
    /// **Every** path that builds a deck must call this. There are two — the
    /// synchronous `add_deck` command and the UI's background loader
    /// (`spawn_deck_loads`, completed in `usecases/ui/runner.rs`) — and they
    /// silently diverged: a shader dropped from the Library skipped analyzer
    /// startup and device acquisition entirely, so a `depth_sensor` shader
    /// rendered against blank 1x1 textures with no error.
    ///
    /// Returns `Err` when a required preprocessor cannot be satisfied; the
    /// caller must discard the deck and surface the message.
    pub(crate) fn finalize_new_deck(&mut self, deck: &mut Deck) -> Result<()> {
        deck.ensure_preprocessor_analyzers(&self.analyzer_registry);
        let Some(metadata) = deck.shader().map(|s| s.metadata.clone()) else {
            return Ok(());
        };
        let name = deck.source_name().to_string();
        if let Some(sensor) = self.acquire_depth_preprocessor(&metadata, &name)? {
            deck.attach_depth_preprocessor(
                sensor.id,
                sensor.name,
                sensor.pipeline,
                DepthPreprocessParams::default(),
            );
        }
        Ok(())
    }

    /// Acquire the depth sensor a shader's `PREPROCESSORS` block requires.
    ///
    /// `Ok(None)` when the shader declares no `depth_sensor` preprocessor; `Err`
    /// when it does but no sensor is available. `depth_sensor` is a *required*
    /// preprocessor, so callers must propagate the error and abandon the load
    /// rather than degrade to a deck with nothing to draw.
    /// See spec/depth-sensor-preprocessor.md § Device Acquisition.
    fn acquire_depth_preprocessor(
        &mut self,
        metadata: &crate::isf::ISFMetadata,
        shader_name: &str,
    ) -> Result<Option<AcquiredSensor>> {
        self.check_required_preprocessors(metadata, shader_name)?;
        crate::depth::preprocess::acquire_for_shader(
            &mut self.depth_manager,
            &self.context.device,
            metadata,
            shader_name,
        )
    }

    /// Reject a shader that declares a required preprocessor the engine cannot
    /// service. Requiredness is a registry property, so this stays correct as
    /// device-backed types are added.
    ///
    /// Unknown and optional types are *not* rejected — they degrade to default
    /// outputs, per /spec/effect-preprocessing.md Decision #2.
    fn check_required_preprocessors(
        &self,
        metadata: &crate::isf::ISFMetadata,
        shader_name: &str,
    ) -> Result<()> {
        for pp in &metadata.preprocessors {
            let ty = pp.preprocessor_type.as_str();
            let Some(category) = self.analyzer_registry.category_for(ty) else {
                log::warn!(
                    "Shader '{shader_name}' declares unknown preprocessor '{ty}'; \
                     its outputs will be blank"
                );
                continue;
            };
            if category.is_required() && ty != crate::depth::preprocess::PREPROCESSOR_TYPE {
                anyhow::bail!(
                    "Shader '{shader_name}' requires preprocessor '{ty}', which this build \
                     cannot provide."
                );
            }
        }
        Ok(())
    }
}

impl MixerCommands for VardaApp {
    fn set_crossfader(&mut self, position: f32) {
        let position = sanitize_unit(position, 0.5);
        self.mixer.snap_crossfader(position);
        if let Some(ref sender) = self.input.osc_feedback {
            sender.send_param("crossfader", position);
        }
    }

    fn start_auto_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing) {
        let target = sanitize_unit(target, 0.5);
        self.mixer.start_crossfade(target, duration_secs, easing);
    }

    fn start_beat_crossfade(&mut self, target: f32, beats: f32) {
        let target = sanitize_unit(target, 0.5);
        self.mixer.start_beat_crossfade(target, beats);
    }

    fn add_deck(&mut self, channel_uuid: &str, shader_name: &str) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let generators = self.registry.generators();
        let shader = generators
            .iter()
            .find(|s| s.name() == shader_name)
            .context("Shader not found")?;
        let shader_clone = (*shader).clone();
        let is_compute = shader_clone.metadata.is_compute();
        let mut deck = if is_compute {
            Deck::new_from_compute_shader(
                &self.context,
                shader_clone,
                self.render_width,
                self.render_height,
            )?
        } else {
            Deck::new(
                &self.context,
                shader_clone,
                self.render_width,
                self.render_height,
            )?
        };
        // Shared with the UI's background loader — see `finalize_new_deck`.
        // On failure the deck is dropped and the error surfaces as a toast.
        self.finalize_new_deck(&mut deck)?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added deck {idx} to channel {channel_idx} with shader: {shader_name}");
        Ok(uuid)
    }

    fn add_image_deck(&mut self, channel_uuid: &str, path: &std::path::Path) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let deck =
            Deck::new_from_image(&self.context, path, self.render_width, self.render_height)?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added image deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    fn add_video_deck(&mut self, channel_uuid: &str, path: &std::path::Path) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let deck =
            Deck::new_from_video(&self.context, path, self.render_width, self.render_height)?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added video deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    fn add_solid_color_deck(&mut self, channel_uuid: &str, color: [f32; 4]) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let deck =
            Deck::new_solid_color(&self.context, color, self.render_width, self.render_height)?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added solid color deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    fn add_camera_deck(&mut self, channel_uuid: &str, camera_id: CameraId) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let cam_name = self
            .camera_manager
            .devices()
            .iter()
            .find(|d| d.id == camera_id)
            .map_or_else(|| format!("Camera {camera_id}"), |d| d.name.clone());
        let (src_w, src_h) = self
            .camera_manager
            .open_camera(camera_id, &self.context.device)?;
        let deck = Deck::new_from_camera(
            &self.context,
            camera_id,
            &cam_name,
            src_w,
            src_h,
            self.render_width,
            self.render_height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added camera deck {idx} to channel {channel_idx}: {cam_name}");
        Ok(uuid)
    }

    fn add_screen_capture_deck(
        &mut self,
        channel_uuid: &str,
        target: &crate::scene::CaptureTargetConfig,
        options: crate::screen_capture::backend::CaptureConfig,
    ) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let identity = crate::screen_capture::backend::TargetIdentity::from(target);
        let info = self
            .screen_capture_manager
            .find_target(&identity)
            .cloned()
            .with_context(|| {
                format!(
                    "No capture target matches '{}' — rescan and try again",
                    target.label()
                )
            })?;

        // Default the capture-time downscale to the largest deck-sized frame
        // that keeps the target's own shape. Without a cap a 4K display would
        // move 33 MB per frame only to be scaled down immediately after; without
        // the shape, a window narrower than the stage would arrive pre-squashed
        // and the deck's scaling mode would have nothing left to do.
        // See spec/screen-capture.md § Performance.
        let config = crate::screen_capture::backend::CaptureConfig {
            scale_to: options.scale_to.or_else(|| {
                Some(crate::screen_capture::resample::fit_within(
                    info.width,
                    info.height,
                    self.render_width,
                    self.render_height,
                ))
            }),
            ..options
        };
        let (capture_id, src_w, src_h) = self
            .screen_capture_manager
            .open(&info, config.clone(), &self.context.device)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let deck = Deck::new_from_screen_capture(
            &self.context,
            crate::deck::ScreenCaptureState {
                capture_id,
                identity,
                config,
                config_dirty: false,
            },
            &info.label,
            src_w,
            src_h,
            self.render_width,
            self.render_height,
        )
        .inspect_err(|_| self.screen_capture_manager.release(capture_id))?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!(
            "Added screen capture deck {idx} to channel {channel_idx}: {}",
            info.label
        );
        Ok(uuid)
    }

    fn add_tap_deck(
        &mut self,
        channel_uuid: &str,
        source: &crate::scene::TapSourceConfig,
    ) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let tap_source = crate::deck::TapSource::from(source);
        let label = tap_source.label(&self.channel_labels());
        let deck = Deck::new_from_tap(
            &self.context,
            tap_source,
            &label,
            self.render_width,
            self.render_height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added tap deck {idx} to channel {channel_idx}: {label}");
        Ok(uuid)
    }

    fn set_tap_source(
        &mut self,
        deck_uuid: &str,
        source: &crate::scene::TapSourceConfig,
    ) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        let labels = self.channel_labels();
        let deck = &mut self.mixer.channels_mut()[ch].decks[dk].deck;
        let state = deck
            .tap
            .as_mut()
            .context("Deck is not a tap and has no source to repoint")?;
        state.source = crate::deck::TapSource::from(source);
        let label = state.source.label(&labels);
        deck.set_source_name(format!("🔁 {label}"));
        Ok(())
    }

    fn add_depth_sensor_deck(
        &mut self,
        channel_uuid: &str,
        depth_sensor_id: crate::depth::DepthSensorId,
    ) -> Result<String> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        let name = self
            .depth_manager
            .devices()
            .iter()
            .find(|d| d.id == depth_sensor_id)
            .map_or_else(
                || format!("Depth Sensor {depth_sensor_id}"),
                |d| d.name.clone(),
            );
        let (src_w, src_h) = crate::depth::open_depth_sensor(
            &mut self.depth_manager,
            depth_sensor_id,
            &self.context.device,
        )?;
        let deck = Deck::new_from_depth_sensor(
            &self.context,
            depth_sensor_id,
            &name,
            src_w,
            src_h,
            self.render_width,
            self.render_height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added depth sensor deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    fn remove_deck(&mut self, deck_uuid: &str) -> Result<()> {
        let (channel_idx, deck_idx) = self.resolve_deck(deck_uuid)?;
        // Release external resources before removal
        if let Some(ch) = self.mixer.channels().get(channel_idx) {
            if let Some(slot) = ch.decks.get(deck_idx) {
                if let Some(cam_id) = slot.deck.camera_id() {
                    self.camera_manager.release_camera(cam_id);
                }
                // Depth sensors were missing from this teardown, so the capture
                // thread and USB handle outlived the last deck that used them.
                // Covers both point-cloud sources and shader preprocessors.
                // See spec/depth-sensors.md § Known defect.
                for sensor_id in slot.deck.held_depth_sensors() {
                    self.depth_manager.release(sensor_id);
                }
                if let Some(capture_id) = slot.deck.screen_capture_id() {
                    self.screen_capture_manager.release(capture_id);
                }
                if let Some(idx) = slot.deck.srt_receiver_idx() {
                    self.external_io.stream_manager.stop_receive(idx);
                }
                if let Some(idx) = slot.deck.ndi_receiver_idx() {
                    self.external_io.ndi_manager.stop_receive(idx);
                }
                #[cfg(target_os = "macos")]
                if let Some(idx) = slot.deck.syphon_client_idx() {
                    self.external_io.syphon_manager.stop_receive(idx);
                }
            }
        }
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        // Capture effect UUIDs before removal so their modulation goes with them.
        let effect_uuids: Vec<String> = ch.decks[deck_idx]
            .deck
            .effects
            .iter()
            .map(|e| e.uuid().to_owned())
            .collect();
        ch.remove_deck(deck_idx);
        log::info!("Removed deck {deck_uuid} from channel {channel_idx}");
        self.mixer
            .modulation_mut()
            .remove_assignments_with_prefix(&format!("deck_{deck_uuid}:"));
        for fx_uuid in &effect_uuids {
            self.mixer
                .modulation_mut()
                .remove_assignments_with_prefix(&format!("fx_{fx_uuid}:"));
        }
        // A lane is where this deck sits in show time, so it leaves with the
        // deck. See /spec/arrangement.md § A lane is a deck.
        self.drop_lane(deck_uuid);
        Ok(())
    }

    fn move_deck(&mut self, deck_uuid: &str, dst_channel_uuid: &str) -> Result<()> {
        let (src_ch, src_deck) = self.resolve_deck(deck_uuid)?;
        let dst_ch = self.resolve_channel(dst_channel_uuid)?;
        if src_ch == dst_ch {
            return Ok(());
        }
        let channels = self.mixer.channels_mut();
        // Two mutable borrows into different vec elements require raw indexing
        // (split_at_mut or index — Rust's borrow checker doesn't allow two
        //  channel_mut() calls in the same scope)
        let Some(slot) = channels[src_ch].remove_deck_slot(src_deck) else {
            anyhow::bail!("deck '{deck_uuid}' vanished during move");
        };
        channels[dst_ch].add_deck_slot(slot);
        log::info!("Moved deck {deck_uuid} from ch{src_ch} to ch{dst_ch}");
        Ok(())
    }

    fn reorder_deck(&mut self, channel_uuid: &str, from_idx: usize, to_idx: usize) -> Result<()> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        if from_idx == to_idx {
            return Ok(());
        }
        let channel = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        if from_idx >= channel.decks.len() || to_idx >= channel.decks.len() {
            anyhow::bail!(
                "reorder_deck: ordinals {from_idx}->{to_idx} out of range for {} decks",
                channel.decks.len()
            );
        }
        let slot = channel.decks.remove(from_idx);
        channel.decks.insert(to_idx, slot);
        log::info!("Reordered deck in ch {channel_uuid}: {from_idx} -> {to_idx}");
        Ok(())
    }

    fn set_deck_opacity(&mut self, deck_uuid: &str, opacity: f32) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.mixer.channels_mut()[ch].decks[dk].opacity = sanitize_unit(opacity, 1.0);
        Ok(())
    }

    fn set_deck_blend_mode(&mut self, deck_uuid: &str, mode: BlendMode) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.mixer.channels_mut()[ch].decks[dk].blend_mode = mode;
        Ok(())
    }

    fn set_deck_solo(&mut self, deck_uuid: &str, solo: bool) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.mixer.channels_mut()[ch].set_deck_solo(dk, solo);
        Ok(())
    }

    fn set_deck_mute(&mut self, deck_uuid: &str, mute: bool) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.mixer.channels_mut()[ch].set_deck_mute(dk, mute);
        Ok(())
    }

    fn set_deck_scaling_mode(&mut self, deck_uuid: &str, mode: ScalingMode) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.mixer.channels_mut()[ch].decks[dk]
            .deck
            .set_scaling_mode(mode);
        Ok(())
    }

    fn set_deck_transparent(&mut self, deck_uuid: &str, transparent: bool) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.mixer.channels_mut()[ch].decks[dk]
            .deck
            .set_transparent(transparent);
        Ok(())
    }

    fn set_channel_opacity(&mut self, channel_uuid: &str, opacity: f32) -> Result<()> {
        let ch = self.resolve_channel(channel_uuid)?;
        self.mixer.channels_mut()[ch].opacity = sanitize_unit(opacity, 1.0);
        Ok(())
    }

    fn set_channel_blend_mode(&mut self, channel_uuid: &str, mode: BlendMode) -> Result<()> {
        let ch = self.resolve_channel(channel_uuid)?;
        self.mixer.channels_mut()[ch].blend_mode = mode;
        Ok(())
    }

    fn add_channel(&mut self) -> Result<String> {
        let idx = self
            .mixer
            .add_channel(&self.context, self.render_width, self.render_height)?;
        Ok(self.mixer.channels()[idx].uuid().to_string())
    }

    fn remove_channel(&mut self, channel_uuid: &str) -> Result<()> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        // Asked before anything is torn down, because a refusal has to leave the
        // channel exactly as it was rather than empty.
        if self.mixer.channels().len() <= 2 {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }

        // Through the deck path rather than dropping the column wholesale, so a
        // camera, a depth sensor, a stream receiver, and an arrangement lane all
        // leave with the deck that held them, exactly as they do when the deck is
        // removed on its own.
        let channel = &self.mixer.channels()[channel_idx];
        let decks: Vec<String> = channel
            .decks
            .iter()
            .map(|slot| slot.deck.uuid().to_string())
            .collect();
        let effects: Vec<String> = channel
            .effects
            .iter()
            .map(|effect| effect.uuid().to_string())
            .collect();
        for uuid in decks {
            if let Err(e) = self.remove_deck(&uuid) {
                log::warn!("Removing deck {uuid} with its channel: {e}");
            }
        }
        for uuid in effects {
            self.mixer
                .modulation_mut()
                .remove_assignments_with_prefix(&format!("fx_{uuid}:"));
        }
        // The fader's own curves. A key that can never resolve again would be
        // persisted and reloaded as dead weight.
        self.mixer
            .modulation_mut()
            .remove_assignments_with_prefix(&format!("ch_{channel_uuid}:"));

        if self.mixer.remove_channel(channel_idx) {
            // Selection fixup is handled by the UI consumer (UIRunner)
            Ok(())
        } else {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }
    }

    fn add_effect(&mut self, target: EffectTarget, shader_name: &str) -> Result<String> {
        let chain = self.resolve_effect_target(&target)?;
        let filters = self.registry.filters();
        let shader = filters
            .iter()
            .find(|s| s.name() == shader_name)
            .context("Filter shader not found")?;
        match chain {
            EffectChain::Deck {
                channel_idx,
                deck_idx,
            } => {
                // Clone off the registry borrow so the device manager can be
                // borrowed mutably for acquisition.
                let shader = (*shader).clone();
                let metadata = shader.metadata.clone();
                drop(filters);
                // Acquire before mutating the deck so a missing sensor leaves the
                // effect chain untouched rather than half-added.
                let already_attached = self
                    .mixer
                    .channels()
                    .get(channel_idx)
                    .and_then(|c| c.decks.get(deck_idx))
                    .is_some_and(|s| s.deck.depth_prepro.is_some());
                let acquired = if already_attached {
                    // The deck's source shader already holds a session; reuse it
                    // rather than opening the device a second time.
                    None
                } else {
                    self.acquire_depth_preprocessor(&metadata, shader_name)?
                };

                let effect = Effect::new(&self.context, shader)?;
                let uuid = effect.uuid().to_owned();
                let ch = self
                    .mixer
                    .channel_mut(channel_idx)
                    .context("Invalid channel")?;
                let deck = &mut ch.decks[deck_idx].deck;
                deck.add_effect(effect);
                deck.ensure_preprocessor_analyzers(&self.analyzer_registry);
                if let Some(sensor) = acquired {
                    deck.attach_depth_preprocessor(
                        sensor.id,
                        sensor.name,
                        sensor.pipeline,
                        DepthPreprocessParams::default(),
                    );
                } else if already_attached {
                    deck.rebind_depth_preprocessor_slots();
                }
                log::info!("Added effect {shader_name} to deck chain ({uuid})");
                Ok(uuid)
            }
            EffectChain::Channel { channel_idx } => {
                if crate::depth::preprocess::requested_device(&shader.metadata).is_some() {
                    anyhow::bail!(
                        "Effect '{shader_name}' requires a depth sensor, which is only \
                         available on deck effect chains — not channel or master chains."
                    );
                }
                let effect = Effect::new_with_format(
                    &self.context,
                    (*shader).clone(),
                    self.context.compositing_format,
                )?;
                let uuid = effect.uuid().to_owned();
                let ch = self
                    .mixer
                    .channel_mut(channel_idx)
                    .context("Invalid channel")?;
                ch.add_effect(effect);
                log::info!("Added channel effect {shader_name} ({uuid})");
                Ok(uuid)
            }
            EffectChain::Master => {
                if crate::depth::preprocess::requested_device(&shader.metadata).is_some() {
                    anyhow::bail!(
                        "Effect '{shader_name}' requires a depth sensor, which is only \
                         available on deck effect chains — not channel or master chains."
                    );
                }
                let effect = Effect::new_with_format(
                    &self.context,
                    (*shader).clone(),
                    self.context.compositing_format,
                )?;
                let uuid = effect.uuid().to_owned();
                self.mixer.add_master_effect(effect);
                log::info!("Added master effect {shader_name} ({uuid})");
                Ok(uuid)
            }
        }
    }

    fn remove_effect(&mut self, effect_uuid: &str) -> Result<()> {
        let location = self.resolve_effect(effect_uuid)?;
        let (chain, idx) = self.mixer.effect_chain_at_mut(location);
        chain.remove(idx);
        // If that effect was the deck's only depth-sensor consumer, stop holding
        // the device — otherwise a capture thread and three GPU passes stay alive
        // for nothing. Two-step because the deck borrow must end before the
        // manager is touched.
        if let crate::mixer::EffectLocation::Deck {
            channel_idx: ch,
            deck_idx: dk,
            ..
        } = location
        {
            let released = self
                .mixer
                .channel_mut(ch)
                .and_then(|c| c.decks.get_mut(dk))
                .and_then(|s| s.deck.detach_depth_preprocessor_if_unused());
            if let Some(sensor_id) = released {
                self.depth_manager.release(sensor_id);
            }
        }
        // The effect's modulation assignments die with it — otherwise they point
        // at a UUID that no longer resolves.
        self.mixer
            .modulation_mut()
            .remove_assignments_with_prefix(&format!("fx_{effect_uuid}:"));
        Ok(())
    }

    fn toggle_effect(&mut self, effect_uuid: &str) -> Result<()> {
        let location = self.resolve_effect(effect_uuid)?;
        let (chain, idx) = self.mixer.effect_chain_at_mut(location);
        chain[idx].enabled = !chain[idx].enabled;
        Ok(())
    }

    fn move_effect(&mut self, target: EffectTarget, from_idx: usize, to_idx: usize) -> Result<()> {
        let chain = self.resolve_effect_target(&target)?;
        if from_idx == to_idx {
            return Ok(());
        }
        let effects = match chain {
            EffectChain::Deck {
                channel_idx,
                deck_idx,
            } => {
                &mut self.mixer.channels_mut()[channel_idx].decks[deck_idx]
                    .deck
                    .effects
            }
            EffectChain::Channel { channel_idx } => {
                &mut self.mixer.channels_mut()[channel_idx].effects
            }
            EffectChain::Master => self.mixer.master_effects_mut(),
        };
        if from_idx >= effects.len() || to_idx >= effects.len() {
            anyhow::bail!(
                "move_effect: ordinals {from_idx}->{to_idx} out of range for {} effects",
                effects.len()
            );
        }
        let effect = effects.remove(from_idx);
        effects.insert(to_idx, effect);
        Ok(())
    }

    fn set_transition(&mut self, shader_name: Option<&str>) -> Result<()> {
        match shader_name {
            None => {
                self.mixer.clear_transition();
                Ok(())
            }
            Some(name) => {
                let shader = self
                    .registry
                    .get(name)
                    .context("Transition shader not found")?;
                self.mixer.set_transition(&self.context, shader.clone())
            }
        }
    }

    fn set_tonemap_mode(&mut self, mode: crate::renderer::tonemap::TonemapMode) {
        self.mixer.set_tonemap_mode(&self.context.queue, mode);
    }

    fn load_lut(&mut self, filename: &str) -> Result<()> {
        let lut_dir = self.session.workspace.varda_dir().join("luts");
        let path = lut_dir.join(filename);
        let parsed = crate::renderer::lut::parse_lut_file(&path)?;
        self.mixer.load_lut(
            &self.context.device,
            &self.context.queue,
            &parsed,
            filename.to_string(),
        );
        Ok(())
    }

    fn unload_lut(&mut self) {
        self.mixer.unload_lut();
    }

    fn set_param(
        &mut self,
        path: &str,
        value: ParamValue,
    ) -> std::result::Result<(), crate::param_router::ParamRouteError> {
        // Typed routing preserves Color/Point2D for shader/effect params; scalar
        // paths flatten internally. OSC feedback stays a scalar f32.
        let feedback_value = crate::param_router::param_value_to_norm_f32(&value);
        match crate::param_router::apply_typed_param_by_path(&mut self.mixer, path, value) {
            Ok(()) => {
                self.note_live_route_write(path, feedback_value);
                // Broadcast to OSC feedback targets
                if let Some(ref sender) = self.input.osc_feedback {
                    if sender.has_targets() {
                        sender.send_param(path, feedback_value);
                    }
                }
                Ok(())
            }
            Err(e) => {
                log::warn!("set_param route failed ({path}): {e}");
                Err(e)
            }
        }
    }
}

// ── Audio trait implementations ─────────────────────────────────────

impl AudioCommands for VardaApp {
    fn open_audio_source(&mut self, source_id: AudioSourceId) -> Result<()> {
        self.audio_manager
            .open_source(source_id)
            .map_err(|e| anyhow::anyhow!("Failed to open audio source: {e}"))
    }

    fn close_audio_source(&mut self, source_id: AudioSourceId) {
        self.audio_manager.close_source(source_id);
    }

    fn scan_audio_devices(&mut self) {
        self.audio_manager.scan_devices();
    }
}

impl AudioQueries for VardaApp {
    fn audio_snapshot(&self) -> AudioSnapshot {
        let primary_audio = self.audio_manager.get_primary_data();
        let active_ids = self.audio_manager.active_source_ids();
        AudioSnapshot {
            level: primary_audio.level,
            bass: primary_audio.bass(),
            mid: primary_audio.mid(),
            treble: primary_audio.treble(),
            bpm: primary_audio.bpm,
            beat_phase: primary_audio.beat_phase(),
            enabled: self.audio_manager.has_active_source(),
            devices: self
                .audio_manager
                .devices()
                .iter()
                .map(|d| AudioDeviceSnapshot {
                    id: d.id,
                    name: d.name.clone(),
                    active: active_ids.contains(&d.id),
                })
                .collect(),
            fft: primary_audio.fft.clone(),
            sample_rate: primary_audio.sample_rate,
        }
    }
}

// ── Modulation trait implementations ────────────────────────────────

impl ModulationCommands for VardaApp {
    fn add_lfo(&mut self, waveform: LFOWaveform, frequency: f32) -> String {
        let source = ModulationSource::LFO {
            waveform,
            frequency,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: false,
        };
        self.mixer.modulation_mut().add_source(source)
    }

    fn add_audio_band(
        &mut self,
        preset: AudioBandPreset,
        source_id: Option<AudioSourceId>,
    ) -> String {
        let (freq_low, freq_high) = preset.freq_range();
        let source = ModulationSource::AudioBand {
            source_id,
            freq_low,
            freq_high,
            gain: 1.0,
            smoothing: 0.6,
            mode: crate::modulation::AudioReactMode::Direct,
            noise_gate: 0.1,
        };
        self.mixer.modulation_mut().add_source(source)
    }

    fn add_adsr(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> String {
        let source = ModulationSource::adsr(attack, decay, sustain, release);
        self.mixer.modulation_mut().add_source(source)
    }

    fn add_step_sequencer(&mut self, num_steps: usize, rate: f32) -> String {
        let source = ModulationSource::step_sequencer(num_steps, rate);
        self.mixer.modulation_mut().add_source(source)
    }

    fn add_automation_lane(&mut self, target: &str, timebase: crate::timebase::Timebase) -> String {
        let modulation = self.mixer.modulation_mut();
        let uuid = modulation.add_source(ModulationSource::envelope(Vec::new()));
        modulation.set_timebase(&uuid, timebase);
        // Absolute is the whole point: a curve drawn to a value must produce
        // that value rather than depending on the saved fader position.
        modulation.assign_with_mode(
            target,
            &uuid,
            1.0,
            None,
            crate::modulation::AssignmentMode::Absolute,
        );
        uuid
    }

    fn set_envelope_breakpoints(
        &mut self,
        uuid: &str,
        breakpoints: Vec<crate::modulation::Breakpoint>,
    ) -> bool {
        self.mixer
            .modulation_mut()
            .set_envelope_breakpoints(uuid, breakpoints)
    }

    fn remove_modulation_source(&mut self, uuid: &str) {
        self.mixer.modulation_mut().remove_source(uuid);
    }

    fn assign_modulation(&mut self, target: &str, source_id: &str, amount: f32) {
        self.mixer
            .modulation_mut()
            .assign(target, source_id, amount, None);
    }

    fn clear_modulation(&mut self, target: &str) {
        self.mixer.modulation_mut().clear_assignments(target);
    }

    fn clear_modulation_source(&mut self, target: &str, source_id: &str) {
        self.mixer
            .modulation_mut()
            .clear_assignment_source(target, source_id);
    }
}

impl MacroCommands for VardaApp {
    fn add_macro(&mut self, kind: crate::macros::MacroKind) -> String {
        self.mixer.macros_mut().add_macro(kind)
    }

    fn remove_macro(&mut self, uuid: &str) {
        self.mixer.macros_mut().remove_macro(uuid);
    }

    fn rename_macro(&mut self, uuid: &str, name: &str) {
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            m.name = name.to_string();
        }
    }

    fn set_macro_kind(&mut self, uuid: &str, kind: crate::macros::MacroKind) {
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            m.set_kind(kind);
        }
    }

    fn set_macro_value(&mut self, uuid: &str, value: f32) {
        // Route through the shared param router so the fan-out (and any global
        // trigger actions, drained in process_inputs) behave identically to a
        // MIDI/OSC-driven `macro/<uuid>/value`.
        let path = format!("macro/{uuid}/value");
        if let Err(e) = crate::param_router::apply_param_by_path(&mut self.mixer, &path, value) {
            log::debug!("set_macro_value {uuid}: {e}");
        }
    }

    fn add_macro_target(&mut self, uuid: &str, path: &str) {
        if path == "macro" || path.starts_with("macro/") {
            log::warn!("refusing macro target on macro path '{path}' (loop prevention)");
            return;
        }
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            m.targets.push(crate::macros::MacroTarget::new(path));
        }
    }

    fn remove_macro_target(&mut self, uuid: &str, target_idx: usize) {
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            if target_idx < m.targets.len() {
                m.targets.remove(target_idx);
            }
        }
    }

    fn update_macro_target(
        &mut self,
        uuid: &str,
        target_idx: usize,
        min: f32,
        max: f32,
        curve: crate::macros::MacroCurve,
        invert: bool,
    ) {
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            if let Some(t) = m.targets.get_mut(target_idx) {
                t.min = min;
                t.max = max;
                t.curve = curve;
                t.invert = invert;
            }
        }
    }

    fn set_macro_button_behavior(&mut self, uuid: &str, behavior: crate::macros::ButtonBehavior) {
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            match &mut m.button {
                Some(spec) => spec.behavior = behavior,
                None => {
                    m.button = Some(crate::macros::ButtonSpec {
                        behavior,
                        trigger: Vec::new(),
                    });
                }
            }
        }
    }

    fn set_macro_triggers(&mut self, uuid: &str, actions: Vec<crate::macros::TriggerAction>) {
        if let Some(m) = self.mixer.macros_mut().find_mut(uuid) {
            match &mut m.button {
                Some(spec) => spec.trigger = actions,
                None => {
                    m.button = Some(crate::macros::ButtonSpec {
                        behavior: crate::macros::ButtonBehavior::Trigger,
                        trigger: actions,
                    });
                }
            }
        }
    }
}

impl MacroQueries for VardaApp {
    fn macro_snapshot(&self) -> Vec<crate::macros::Macro> {
        self.mixer.macros().macros().to_vec()
    }
}

impl ModulationQueries for VardaApp {
    fn modulation_snapshot(&self) -> ModulationSnapshot {
        let m = &self.mixer;
        let sources = m
            .modulation()
            .sources
            .iter()
            .map(|entry| {
                let snapshot = match &entry.source {
                    ModulationSource::LFO {
                        waveform,
                        frequency,
                        phase,
                        amplitude,
                        bipolar,
                    } => ModulationSourceSnapshot::LFO {
                        waveform: *waveform,
                        frequency: *frequency,
                        phase: *phase,
                        amplitude: *amplitude,
                        bipolar: *bipolar,
                    },
                    ModulationSource::AudioBand {
                        source_id,
                        freq_low,
                        freq_high,
                        gain,
                        smoothing,
                        mode,
                        noise_gate,
                    } => ModulationSourceSnapshot::Audio {
                        source_id: *source_id,
                        freq_low: *freq_low,
                        freq_high: *freq_high,
                        gain: *gain,
                        smoothing: *smoothing,
                        mode: *mode,
                        noise_gate: *noise_gate,
                    },
                    ModulationSource::ADSR {
                        attack,
                        decay,
                        sustain,
                        release,
                        stage,
                        ..
                    } => ModulationSourceSnapshot::ADSR {
                        attack: *attack,
                        decay: *decay,
                        sustain: *sustain,
                        release: *release,
                        stage: *stage,
                    },
                    ModulationSource::StepSequencer {
                        steps,
                        rate,
                        interpolation,
                        bipolar,
                    } => ModulationSourceSnapshot::StepSequencer {
                        steps: steps.clone(),
                        rate: *rate,
                        interpolation: *interpolation,
                        bipolar: *bipolar,
                    },
                    ModulationSource::Analyzer {
                        deck_id,
                        analyzer_type,
                        output_name,
                        smoothing,
                    } => ModulationSourceSnapshot::Analyzer {
                        deck_id: deck_id.clone(),
                        analyzer_type: analyzer_type.clone(),
                        output_name: output_name.clone(),
                        smoothing: *smoothing,
                    },
                    ModulationSource::Envelope { breakpoints, .. } => {
                        ModulationSourceSnapshot::Envelope {
                            breakpoints: breakpoints.clone(),
                        }
                    }
                };
                ModulationSourceSnapshotEntry {
                    uuid: entry.uuid.clone(),
                    source: snapshot,
                    timebase: entry.timebase,
                }
            })
            .collect();
        let current_values: std::collections::HashMap<String, f32> = m
            .modulation()
            .sources
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                (
                    entry.uuid.clone(),
                    m.modulation()
                        .current_values()
                        .get(i)
                        .copied()
                        .unwrap_or(0.0),
                )
            })
            .collect();
        let assignments = m
            .modulation()
            .assignments
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter()
                        .map(|pm| ModulationAssignmentSnapshot {
                            source_id: pm.source_id.clone(),
                            amount: pm.amount,
                        })
                        .collect(),
                )
            })
            .collect();
        ModulationSnapshot {
            sources,
            current_values,
            assignments,
        }
    }
}

// ── Output trait implementations ────────────────────────────────────

impl OutputCommands for VardaApp {
    fn request_create_output(&mut self) {
        self.output
            .pending_output_creates
            .push(crate::scene::OutputConfig::default_windowed());
    }

    fn close_output(&mut self, output_uuid: &str) -> Result<()> {
        let idx = self.resolve_output(output_uuid)?;
        let name = self.output.outputs[idx].name().to_string();
        // Stop active subprocess before removing to release ports/resources
        if let crate::renderer::context::UnifiedOutput::Headless(h) = &mut self.output.outputs[idx]
        {
            if let Some(mut sub) = h.subprocess.take() {
                sub.stop();
            }
        }
        let removed = self.output.outputs.remove(idx);
        if let crate::renderer::context::UnifiedOutput::Window(w) = removed {
            w.destroy();
        }
        log::info!("Closed output '{name}'");
        Ok(())
    }

    fn set_output_display(&mut self, output_uuid: &str, monitor_name: &str) -> Result<()> {
        let idx = self.resolve_output(output_uuid)?;
        let Some((mi, (_, handle))) = self
            .output
            .cached_monitors
            .iter()
            .enumerate()
            .find(|(_, (name, _))| name == monitor_name)
            .map(|(mi, (n, h))| (mi, (n.clone(), h.clone())))
        else {
            anyhow::bail!("No monitor named '{monitor_name}'");
        };
        if let crate::renderer::context::UnifiedOutput::Window(output) =
            &mut self.output.outputs[idx]
        {
            let target = crate::renderer::context::OutputTarget::Display {
                name: monitor_name.to_string(),
                monitor_index: mi,
            };
            output.set_target(target, Some(handle));
        }
        Ok(())
    }
}

impl OutputQueries for VardaApp {
    fn output_snapshot(&self) -> OutputSnapshot {
        OutputSnapshot {
            windows: self
                .output
                .outputs
                .iter()
                .map(|o| {
                    use crate::renderer::context::{OutputTarget, UnifiedOutput};
                    let assignments = match o {
                        UnifiedOutput::Window(w) => &w.surface_assignments,
                        UnifiedOutput::Headless(h) => &h.surface_assignments,
                    };
                    let surface_assignments = assignments
                        .iter()
                        .map(|a| {
                            let surface_name = self
                                .output
                                .surface_manager
                                .find_by_uuid(&a.surface_uuid).map_or_else(|| format!("Surface {}", a.surface_uuid), |(_, s)| s.name.clone());
                            SurfaceAssignmentSnapshot {
                                surface_uuid: a.surface_uuid.clone(),
                                surface_name,
                                enabled: a.enabled,
                            }
                        })
                        .collect();
                    let (target, is_on_display, is_active, calibration_mode, audio_passthrough, delivery) =
                        match o {
                            UnifiedOutput::Window(w) => (
                                w.target.clone(),
                                matches!(w.target, OutputTarget::Display { .. }),
                                false,
                                w.calibration_mode,
                                None,
                                None,
                            ),
                            UnifiedOutput::Headless(h) => {
                                let audio =
                                    h.audio_pcm.as_ref().map(|p| AudioPassthroughSnapshot {
                                        device: h
                                            .target
                                            .audio_device()
                                            .unwrap_or_default()
                                            .to_string(),
                                        frames_written: h
                                            .subprocess
                                            .as_deref()
                                            .and_then(super::super::internal::renderer::subprocess::FfmpegSubprocess::audio_frames_written)
                                            .unwrap_or(0),
                                        frames_dropped: p
                                            .dropped
                                            .load(std::sync::atomic::Ordering::Relaxed),
                                    });
                                let delivery = h.subprocess.as_deref().map(|sub| {
                                    DeliveryHealthSnapshot {
                                        frames_written: sub.frames_written(),
                                        frames_dropped: sub.frames_dropped(),
                                        frames_padded: sub.frames_padded(),
                                    }
                                });
                                (
                                    h.target.clone(),
                                    false,
                                    h.active,
                                    crate::renderer::context::CalibrationMode::Off,
                                    audio,
                                    delivery,
                                )
                            }
                        };
                    OutputWindowSnapshot {
                        uuid: o.uuid().to_string(),
                        name: o.name().to_string(),
                        target_label: format!("{target}"),
                        target,
                        is_on_display,
                        is_active,
                        surface_assignments,
                        calibration_mode,
                        presentation_request: o.presentation_request(),
                        resolved_presentation: o.resolved_presentation().clone(),
                        audio_passthrough,
                        delivery,
                    }
                })
                .collect(),
            surfaces: self
                .output
                .surface_manager
                .surfaces
                .iter()
                .map(|s| SurfaceSnapshot {
                    uuid: s.uuid.clone(),
                    name: s.name.clone(),
                    vertices: s.vertices.clone(),
                    extra_contours: s.extra_contours.clone(),
                    source: s.source.clone(),
                    content_mapping: s.content_mapping,
                    output_type: s.output_type,
                    circle_hint: s.circle_hint,
                    warp: s.effective_warp(),
                    warp_bound: s.warp_bound,
                    path: s.path.clone(),
                    holes: s.holes.clone(),
                    hole_contours: s.hole_contours.clone(),
                })
                .collect(),
            monitors: self
                .output
                .cached_monitors
                .iter()
                .enumerate()
                .map(|(i, (name, handle))| {
                    let size = handle.size();
                    MonitorSnapshot {
                        name: name.clone(),
                        index: i,
                        width: size.width,
                        height: size.height,
                    }
                })
                .collect(),
        }
    }
}

// ── MixerQueries ────────────────────────────────────────────────────

impl MixerQueries for VardaApp {
    fn mixer_snapshot(&self) -> MixerSnapshot {
        crate::app::snapshot::build_mixer_snapshot(self)
    }
}

// ── SurfaceCommands / SurfaceQueries ────────────────────────────────

impl SurfaceCommands for VardaApp {
    fn add_surface(&mut self, name: &str, source: OutputSource) -> String {
        let uuid = self
            .output
            .surface_manager
            .add_surface(name.to_string(), source);
        log::info!("Added surface '{name}' (uuid {uuid})");
        uuid
    }

    fn add_polygon_surface(
        &mut self,
        name: &str,
        vertices: &[[f32; 2]],
        source: OutputSource,
    ) -> String {
        let uuid = self.output.surface_manager.add_polygon_surface(
            name.to_string(),
            vertices.to_vec(),
            source,
        );
        log::info!(
            "Added polygon surface '{}' with {} vertices (uuid {})",
            name,
            vertices.len(),
            uuid
        );
        uuid
    }

    fn add_circle_surface(
        &mut self,
        name: &str,
        center: [f32; 2],
        radius: f32,
        sides: u32,
        aspect_ratio: f32,
        source: OutputSource,
    ) -> String {
        let hint = crate::surface::CircleHint {
            center,
            radius,
            sides,
            aspect_ratio,
        };
        let uuid = self
            .output
            .surface_manager
            .add_circle_surface(name.to_string(), hint, source);
        log::info!("Added circle surface '{name}' (uuid {uuid})");
        uuid
    }

    fn remove_surface(&mut self, uuid: &str) {
        self.output.surface_manager.remove_surface(uuid);
    }

    fn set_surface_source(&mut self, uuid: &str, source: OutputSource) {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.source = source;
        }
    }

    fn set_surface_output_type(&mut self, uuid: &str, output_type: SurfaceOutputType) {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.output_type = output_type;
        }
    }

    fn set_surface_content_mapping(&mut self, uuid: &str, mapping: ContentMapping) {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.content_mapping = mapping;
        }
    }

    fn rename_surface(&mut self, uuid: &str, name: &str) {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.name = name.to_string();
        }
    }

    fn assign_surface_to_output(&mut self, output_uuid: &str, surface_uuid: &str) {
        if let Some(output) = self
            .output
            .outputs
            .iter_mut()
            .find(|o| o.uuid() == output_uuid)
        {
            let assignments = output.surface_assignments_mut();
            // Warp lives on the surface now — the assignment is membership only.
            if !assignments.iter().any(|a| a.surface_uuid == surface_uuid)
                && self
                    .output
                    .surface_manager
                    .find_by_uuid(surface_uuid)
                    .is_some()
            {
                assignments.push(crate::renderer::context::SurfaceAssignment {
                    surface_uuid: surface_uuid.to_string(),
                    enabled: true,
                    overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                });
            }
        }
    }

    fn unassign_surface_from_output(&mut self, output_uuid: &str, surface_uuid: &str) {
        if let Some(output) = self
            .output
            .outputs
            .iter_mut()
            .find(|o| o.uuid() == output_uuid)
        {
            output
                .surface_assignments_mut()
                .retain(|a| a.surface_uuid != surface_uuid);
        }
    }
}

impl DetectCommands for VardaApp {
    fn detect_from_image(
        &self,
        image_data: &[u8],
        params: &crate::surface::detect::DetectionParams,
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError> {
        crate::surface::import::detect_from_image(image_data, params)
    }

    fn detect_from_svg(
        &self,
        svg_data: &[u8],
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError> {
        crate::surface::import::detect_from_svg(svg_data)
    }

    fn detect_from_dxf(
        &self,
        dxf_data: &[u8],
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError> {
        crate::surface::import::detect_from_dxf(dxf_data)
    }

    fn detect_from_camera(
        &mut self,
        camera_id: CameraId,
        params: &crate::surface::detect::DetectionParams,
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError> {
        // If camera isn't active yet, open it temporarily for the snapshot.
        let was_inactive = !self.camera_manager.is_active(camera_id);
        if was_inactive {
            self.camera_manager
                .open_camera(camera_id, &self.context.device)
                .map_err(|e| {
                    crate::surface::import::ImportError::ImageLoad(format!(
                        "Failed to open camera {camera_id}: {e}"
                    ))
                })?;
        }

        // Spin-wait for a frame (capture thread needs time to produce one).
        // Budget: up to 500ms in 10ms increments.
        let mut frame = None;
        for _ in 0..50 {
            if let Some(f) = self.camera_manager.snapshot_frame(camera_id) {
                frame = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Release the camera if we opened it just for this snapshot.
        if was_inactive {
            self.camera_manager.release_camera(camera_id);
        }

        let (rgba, w, h) = frame.ok_or_else(|| {
            crate::surface::import::ImportError::ImageLoad(format!(
                "No frame received from camera {camera_id} within timeout"
            ))
        })?;
        crate::surface::import::detect_from_rgba(&rgba, w, h, params)
    }

    fn confirm_detected_contours(
        &mut self,
        contours: &[crate::surface::detect::DetectedContour],
    ) -> Vec<String> {
        let mut uuids = Vec::with_capacity(contours.len());
        for contour in contours {
            let uuid = if contour.is_circular {
                if let Some((center, radius)) = contour.circle_fit {
                    let hint = crate::surface::CircleHint {
                        center,
                        radius,
                        sides: 32,
                        aspect_ratio: 1.0,
                    };
                    self.output.surface_manager.add_circle_surface(
                        contour.suggested_name.clone(),
                        hint,
                        OutputSource::Master,
                    )
                } else {
                    self.output.surface_manager.add_polygon_surface(
                        contour.suggested_name.clone(),
                        contour.vertices.clone(),
                        OutputSource::Master,
                    )
                }
            } else if let Some(path) = contour.path.as_ref().filter(|p| p.has_cubic()) {
                // SVG import captured curvature: create an editable curve surface.
                self.output.surface_manager.add_path_surface(
                    contour.suggested_name.clone(),
                    path.clone(),
                    OutputSource::Master,
                )
            } else {
                self.output.surface_manager.add_polygon_surface(
                    contour.suggested_name.clone(),
                    contour.vertices.clone(),
                    OutputSource::Master,
                )
            };
            log::info!(
                "Created surface '{}' from detection (uuid {})",
                contour.suggested_name,
                uuid
            );
            uuids.push(uuid);
        }
        uuids
    }
}

impl SurfaceQueries for VardaApp {
    fn surface_snapshot(&self) -> Vec<SurfaceSnapshot> {
        self.output
            .surface_manager
            .surfaces
            .iter()
            .map(|s| SurfaceSnapshot {
                uuid: s.uuid.clone(),
                name: s.name.clone(),
                vertices: s.vertices.clone(),
                extra_contours: s.extra_contours.clone(),
                source: s.source.clone(),
                content_mapping: s.content_mapping,
                output_type: s.output_type,
                circle_hint: s.circle_hint,
                warp: s.effective_warp(),
                warp_bound: s.warp_bound,
                path: s.path.clone(),
                holes: s.holes.clone(),
                hole_contours: s.hole_contours.clone(),
            })
            .collect()
    }
}

// ── Analyzer trait implementations ──────────────────────────────────

impl AnalyzerQueries for VardaApp {
    fn available_analyzers(&self) -> Vec<AnalyzerTypeInfo> {
        self.analyzer_registry
            .available_types()
            .into_iter()
            .filter_map(|t| {
                let schema = self.analyzer_registry.schema_for(t)?;
                Some(AnalyzerTypeInfo {
                    analyzer_type: t.to_owned(),
                    scalar_outputs: schema
                        .scalars
                        .iter()
                        .map(|s| AnalyzerScalarInfo {
                            name: s.name.clone(),
                            description: s.description.clone(),
                            range: s.range,
                            default_smoothing: s.default_smoothing,
                        })
                        .collect(),
                    texture_outputs: schema.textures.iter().map(|t| t.name.clone()).collect(),
                })
            })
            .collect()
    }

    fn is_analyzer_running(&self, deck_id: &str, analyzer_type: &str) -> bool {
        if let Some((ch, dk)) = self.mixer.find_deck_by_uuid(deck_id) {
            self.mixer
                .channel(ch)
                .and_then(|c| c.decks.get(dk))
                .and_then(|slot| slot.deck.analyzers.latest_snapshot(analyzer_type))
                .is_some()
        } else {
            false
        }
    }
}

impl AnalyzerCommands for VardaApp {
    fn request_analyzer(
        &mut self,
        deck_id: &str,
        analyzer_type: &str,
        options: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let (ch, dk) = self
            .mixer
            .find_deck_by_uuid(deck_id)
            .ok_or_else(|| anyhow::anyhow!("Deck '{deck_id}' not found"))?;
        let slot = self
            .mixer
            .channel_mut(ch)
            .and_then(|c| c.decks.get_mut(dk))
            .ok_or_else(|| anyhow::anyhow!("Deck slot not accessible"))?;
        slot.deck
            .analyzers
            .request(analyzer_type, &self.analyzer_registry, options)
            .ok_or_else(|| anyhow::anyhow!("Failed to start analyzer '{analyzer_type}'"))?;
        Ok(())
    }

    fn release_analyzer(&mut self, deck_id: &str, analyzer_type: &str) {
        if let Some((ch, dk)) = self.mixer.find_deck_by_uuid(deck_id) {
            if let Some(slot) = self.mixer.channel_mut(ch).and_then(|c| c.decks.get_mut(dk)) {
                slot.deck.analyzers.release(analyzer_type);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> Option<super::super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let config = crate::testing::headless_config();
        super::super::VardaApp::new(gpu, &config).ok()
    }

    fn channel_uuid(app: &super::super::VardaApp, idx: usize) -> String {
        app.mixer_snapshot().channels[idx].uuid.clone()
    }

    // ── Depth-sensor preprocessor ────────────────────────────────────────────
    //
    // These need no hardware: without a Kinect attached (and on builds with the
    // `depth` feature compiled out) the manager enumerates nothing, which is
    // exactly the failure path being asserted. The release regression uses the
    // mock backend.

    #[test]
    fn depth_sensor_shader_fails_to_load_without_a_sensor() {
        let Some(mut app) = headless_app() else {
            return;
        };
        if !app.depth_manager.devices().is_empty() {
            // A real sensor is attached; this test asserts the absent case.
            return;
        }
        let ch0 = channel_uuid(&app, 0);
        let err = app
            .add_deck(&ch0, "liquid_light_depth")
            .expect_err("must not load without a depth sensor");
        let msg = err.to_string();
        assert!(
            msg.contains("depth sensor") && msg.contains("none detected"),
            "unhelpful message: {msg}"
        );
        // The load aborted, so no deck was left behind.
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 0);
    }

    #[test]
    fn background_constructed_decks_are_finalized_too() {
        // Regression: the UI's background loader builds the `Deck` off-thread
        // and adds it straight to a channel, bypassing `add_deck`. A
        // `depth_sensor` shader dropped from the Library therefore skipped
        // acquisition entirely and rendered blank preprocessor textures with no
        // error. Both paths now share `finalize_new_deck`; this asserts it
        // rejects the same case `add_deck` does.
        let Some(mut app) = headless_app() else {
            return;
        };
        if !app.depth_manager.devices().is_empty() {
            return;
        }
        let shader = app
            .registry
            .generators()
            .iter()
            .find(|s| s.name() == "liquid_light_depth")
            .map(|s| (*s).clone())
            .expect("showcase shader is registered");
        let mut deck = Deck::new(&app.context, shader, 64, 64).expect("deck builds");
        let err = app
            .finalize_new_deck(&mut deck)
            .expect_err("must reject without a depth sensor");
        assert!(err.to_string().contains("none detected"), "{err}");
        assert!(deck.depth_prepro.is_none());
    }

    #[test]
    fn optional_preprocessor_shaders_still_load() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        // A plain generator with no PREPROCESSORS block must be unaffected by
        // the new required-preprocessor pre-flight.
        assert!(app.add_deck(&ch0, "liquid_light").is_ok());
    }

    #[test]
    fn removing_a_depth_deck_releases_the_sensor() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Two decks sharing one mock sensor: the session must survive the first
        // removal and be torn down by the second.
        app.depth_manager
            .open_mock(0, 32, 24, &app.context.device)
            .expect("open mock");
        app.depth_manager
            .open_mock(0, 32, 24, &app.context.device)
            .expect("share mock");
        assert_eq!(app.depth_manager.ref_count(0), 2);

        let ch0 = channel_uuid(&app, 0);
        let mut uuids = Vec::new();
        for _ in 0..2 {
            let deck =
                Deck::new_from_depth_sensor(&app.context, 0, "Mock Depth (#0)", 32, 24, 64, 64)
                    .expect("build depth deck");
            uuids.push(deck.uuid().to_string());
            let ch_idx = app.resolve_channel(&ch0).expect("channel");
            app.mixer
                .channel_mut(ch_idx)
                .expect("channel")
                .add_deck(deck);
        }

        app.remove_deck(&uuids[0]).expect("remove first");
        assert_eq!(
            app.depth_manager.ref_count(0),
            1,
            "one consumer left, session must stay open"
        );

        app.remove_deck(&uuids[1]).expect("remove second");
        assert!(
            !app.depth_manager.is_active(0),
            "last consumer removed — the capture session must be torn down"
        );
    }

    #[test]
    fn move_deck_same_channel_noop() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let deck = app
            .add_solid_color_deck(&ch0, [1.0, 0.0, 0.0, 1.0])
            .unwrap();
        let result = app.move_deck(&deck, &ch0);
        assert!(result.is_ok());
        // Deck should still be in channel 0
        let snap = app.mixer_snapshot();
        assert_eq!(snap.channels[0].decks.len(), 1);
    }

    #[test]
    fn move_deck_unknown_deck_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        assert!(app.move_deck("nosuchdk", &ch0).is_err());
    }

    #[test]
    fn move_deck_unknown_dst_channel_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let deck = app
            .add_solid_color_deck(&ch0, [1.0, 0.0, 0.0, 1.0])
            .unwrap();
        assert!(app.move_deck(&deck, "nosuchch").is_err());
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 1);
    }

    #[test]
    fn move_deck_valid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        let ch1 = channel_uuid(&app, 1);
        let deck = app
            .add_solid_color_deck(&ch0, [1.0, 0.0, 0.0, 1.0])
            .unwrap();
        let snap = app.mixer_snapshot();
        assert_eq!(snap.channels[0].decks.len(), 1);
        assert_eq!(snap.channels[1].decks.len(), 0);

        let result = app.move_deck(&deck, &ch1);
        assert!(result.is_ok());
        let snap = app.mixer_snapshot();
        assert_eq!(snap.channels[0].decks.len(), 0);
        assert_eq!(snap.channels[1].decks.len(), 1);
    }

    #[test]
    fn reorder_deck_within_channel() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        app.add_solid_color_deck(&ch0, [1.0, 0.0, 0.0, 1.0])
            .unwrap();
        app.add_solid_color_deck(&ch0, [0.0, 1.0, 0.0, 1.0])
            .unwrap();
        app.add_solid_color_deck(&ch0, [0.0, 0.0, 1.0, 1.0])
            .unwrap();
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 3);
        app.reorder_deck(&ch0, 0, 2).unwrap();
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 3);
    }

    #[test]
    fn reorder_deck_same_position_noop() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        app.add_solid_color_deck(&ch0, [1.0, 0.0, 0.0, 1.0])
            .unwrap();
        app.reorder_deck(&ch0, 0, 0).unwrap();
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 1);
    }

    #[test]
    fn reorder_deck_unknown_channel_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(app.reorder_deck("nosuchch", 0, 1).is_err());
    }

    #[test]
    fn set_deck_opacity_unknown_deck_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(app.set_deck_opacity("nosuchdk", 0.5).is_err());
    }

    #[test]
    fn set_deck_opacity_rejects_channel_uuid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        assert!(
            app.set_deck_opacity(&ch0, 0.5).is_err(),
            "a channel UUID does not name a deck"
        );
    }

    #[test]
    fn set_deck_blend_mode_unknown_deck_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(app.set_deck_blend_mode("nosuchdk", BlendMode::Add).is_err());
    }

    #[test]
    fn set_deck_solo_unknown_deck_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(app.set_deck_solo("nosuchdk", true).is_err());
    }

    #[test]
    fn set_deck_mute_unknown_deck_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(app.set_deck_mute("nosuchdk", true).is_err());
    }

    #[test]
    fn set_channel_opacity_clamps() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        app.set_channel_opacity(&ch0, 2.0).unwrap();
        let snap = app.mixer_snapshot();
        assert!(
            (snap.channels[0].opacity - 1.0).abs() < 1e-5,
            "should clamp to 1.0"
        );

        app.set_channel_opacity(&ch0, -1.0).unwrap();
        let snap = app.mixer_snapshot();
        assert!(
            (snap.channels[0].opacity).abs() < 1e-5,
            "should clamp to 0.0"
        );
    }

    #[test]
    fn add_channel_increases_count() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let before = app.mixer_snapshot().channels.len();
        assert_eq!(before, 2);
        app.add_channel().unwrap();
        let after = app.mixer_snapshot().channels.len();
        assert_eq!(after, 3);
    }

    #[test]
    fn remove_channel_enforces_minimum() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert_eq!(app.mixer_snapshot().channels.len(), 2);
        // Trying to remove should fail (minimum 2)
        let ch0 = channel_uuid(&app, 0);
        let result = app.remove_channel(&ch0);
        assert!(result.is_err());
        assert_eq!(app.mixer_snapshot().channels.len(), 2);
    }

    #[test]
    fn toggle_effect_unknown_uuid_errors() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(app.toggle_effect("nosuchfx").is_err());
    }

    #[test]
    fn set_param_invalid_path_is_reported() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // A path that matches no route must surface, not fail silently: the HTTP
        // API turns this into a 404 rather than reporting a write that never
        // landed as `{"status": "ok"}`.
        assert!(matches!(
            app.set_param("ch99/deck99/nonexistent_param", ParamValue::Float(0.5)),
            Err(crate::param_router::ParamRouteError::UnknownPath { .. })
        ));
    }
}
