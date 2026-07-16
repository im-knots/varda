//! Engine trait implementations for VardaApp.

use super::VardaApp;
use crate::deck::{Deck, Effect};
use crate::engine::traits::*;
use crate::engine::types::*;
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

    fn add_deck(&mut self, channel_idx: usize, shader_name: &str) -> Result<()> {
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
        deck.ensure_preprocessor_analyzers(&self.analyzer_registry);
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!(
            "Added deck {} to channel {} with shader: {}",
            idx,
            channel_idx,
            shader_name
        );
        Ok(())
    }

    fn add_image_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()> {
        let deck =
            Deck::new_from_image(&self.context, path, self.render_width, self.render_height)?;
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!(
            "Added image deck {} to channel {}: {}",
            idx,
            channel_idx,
            name
        );
        Ok(())
    }

    fn add_video_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()> {
        let deck =
            Deck::new_from_video(&self.context, path, self.render_width, self.render_height)?;
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!(
            "Added video deck {} to channel {}: {}",
            idx,
            channel_idx,
            name
        );
        Ok(())
    }

    fn add_solid_color_deck(&mut self, channel_idx: usize, color: [f32; 4]) -> Result<()> {
        let deck =
            Deck::new_solid_color(&self.context, color, self.render_width, self.render_height)?;
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!(
            "Added solid color deck {} to channel {}: {}",
            idx,
            channel_idx,
            name
        );
        Ok(())
    }

    fn add_camera_deck(&mut self, channel_idx: usize, camera_id: CameraId) -> Result<()> {
        let cam_name = self
            .camera_manager
            .devices()
            .iter()
            .find(|d| d.id == camera_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("Camera {}", camera_id));
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
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!(
            "Added camera deck {} to channel {}: {}",
            idx,
            channel_idx,
            cam_name
        );
        Ok(())
    }

    fn remove_deck(&mut self, channel_idx: usize, deck_idx: usize) -> Result<()> {
        // Capture deck UUID before removal for modulation cleanup
        let deck_uuid = self
            .mixer
            .channels()
            .get(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .map(|slot| slot.deck.uuid().to_string());
        // Release external resources before removal
        if let Some(ch) = self.mixer.channels().get(channel_idx) {
            if let Some(slot) = ch.decks.get(deck_idx) {
                if let Some(cam_id) = slot.deck.camera_id() {
                    self.camera_manager.release_camera(cam_id);
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
        if deck_idx < ch.decks.len() {
            // Also capture effect UUIDs for modulation cleanup
            let effect_uuids: Vec<String> = ch.decks[deck_idx]
                .deck
                .effects
                .iter()
                .map(|e| e.uuid.clone())
                .collect();
            ch.remove_deck(deck_idx);
            log::info!("Removed deck {} from channel {}", deck_idx, channel_idx);
            // Clean up orphaned modulation assignments
            if let Some(uuid) = deck_uuid {
                self.mixer
                    .modulation_mut()
                    .remove_assignments_with_prefix(&format!("deck_{}:", uuid));
            }
            for fx_uuid in &effect_uuids {
                self.mixer
                    .modulation_mut()
                    .remove_assignments_with_prefix(&format!("fx_{}:", fx_uuid));
            }
        }
        Ok(())
    }

    fn move_deck(&mut self, src_ch: usize, src_deck: usize, dst_ch: usize) -> Result<()> {
        if src_ch == dst_ch {
            return Ok(());
        }
        let channels = self.mixer.channels_mut();
        if src_ch >= channels.len() || dst_ch >= channels.len() {
            return Ok(());
        }
        if src_deck >= channels[src_ch].decks.len() {
            return Ok(());
        }
        // Two mutable borrows into different vec elements require raw indexing
        // (split_at_mut or index — Rust's borrow checker doesn't allow two
        //  channel_mut() calls in the same scope)
        let Some(slot) = channels[src_ch].remove_deck_slot(src_deck) else {
            log::warn!(
                "move_deck: deck {} not found in channel {}",
                src_deck,
                src_ch
            );
            return Ok(());
        };
        let new_idx = channels[dst_ch].add_deck_slot(slot);
        log::info!(
            "Moved deck {} from ch{} to ch{} (new idx {})",
            src_deck,
            src_ch,
            dst_ch,
            new_idx
        );
        Ok(())
    }

    fn reorder_deck(&mut self, ch: usize, from_idx: usize, to_idx: usize) {
        if from_idx == to_idx {
            return;
        }
        if let Some(channel) = self.mixer.channel_mut(ch) {
            if from_idx < channel.decks.len() && to_idx < channel.decks.len() {
                let slot = channel.decks.remove(from_idx);
                channel.decks.insert(to_idx, slot);
                log::info!("Reordered deck in ch{}: {} -> {}", ch, from_idx, to_idx);
            }
        }
    }

    fn set_deck_opacity(&mut self, channel_idx: usize, deck_idx: usize, opacity: f32) {
        let opacity = sanitize_unit(opacity, 1.0);
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            if deck_idx < ch.decks.len() {
                ch.decks[deck_idx].opacity = opacity;
            }
        }
    }

    fn set_deck_blend_mode(&mut self, channel_idx: usize, deck_idx: usize, mode: BlendMode) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            if deck_idx < ch.decks.len() {
                ch.decks[deck_idx].blend_mode = mode;
            }
        }
    }

    fn set_deck_solo(&mut self, channel_idx: usize, deck_idx: usize, solo: bool) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            ch.set_deck_solo(deck_idx, solo);
        }
    }

    fn set_deck_mute(&mut self, channel_idx: usize, deck_idx: usize, mute: bool) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            ch.set_deck_mute(deck_idx, mute);
        }
    }

    fn set_deck_scaling_mode(&mut self, channel_idx: usize, deck_idx: usize, mode: ScalingMode) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            if deck_idx < ch.decks.len() {
                ch.decks[deck_idx].deck.set_scaling_mode(mode);
            }
        }
    }

    fn set_deck_transparent(&mut self, channel_idx: usize, deck_idx: usize, transparent: bool) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            if deck_idx < ch.decks.len() {
                ch.decks[deck_idx].deck.set_transparent(transparent);
            }
        }
    }

    fn set_channel_opacity(&mut self, channel_idx: usize, opacity: f32) {
        let opacity = sanitize_unit(opacity, 1.0);
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            ch.opacity = opacity;
        }
    }

    fn set_channel_blend_mode(&mut self, channel_idx: usize, mode: BlendMode) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            ch.blend_mode = mode;
        }
    }

    fn add_channel(&mut self) -> Result<usize> {
        self.mixer
            .add_channel(&self.context, self.render_width, self.render_height)
    }

    fn remove_channel(&mut self, channel_idx: usize) -> Result<()> {
        if self.mixer.remove_channel(channel_idx) {
            // Selection fixup is handled by the UI consumer (UIRunner)
            Ok(())
        } else {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }
    }

    fn add_effect(&mut self, target: EffectTarget, shader_name: &str) -> Result<()> {
        let filters = self.registry.filters();
        let shader = filters
            .iter()
            .find(|s| s.name() == shader_name)
            .context("Filter shader not found")?;
        match target {
            EffectTarget::Deck(ch_idx, deck_idx) => {
                let effect = Effect::new(&self.context, (*shader).clone())?;
                let ch = self.mixer.channel_mut(ch_idx).context("Invalid channel")?;
                if deck_idx < ch.decks.len() {
                    ch.decks[deck_idx].deck.add_effect(effect);
                    ch.decks[deck_idx]
                        .deck
                        .ensure_preprocessor_analyzers(&self.analyzer_registry);
                    log::info!(
                        "Added effect {} to ch{} deck {}",
                        shader_name,
                        ch_idx,
                        deck_idx
                    );
                }
            }
            EffectTarget::Channel(ch_idx) => {
                let effect = Effect::new_with_format(
                    &self.context,
                    (*shader).clone(),
                    self.context.compositing_format,
                )?;
                let ch = self.mixer.channel_mut(ch_idx).context("Invalid channel")?;
                ch.add_effect(effect);
                log::info!("Added channel effect {} to ch{}", shader_name, ch_idx);
            }
            EffectTarget::Master => {
                let effect = Effect::new_with_format(
                    &self.context,
                    (*shader).clone(),
                    self.context.compositing_format,
                )?;
                self.mixer.add_master_effect(effect);
                log::info!("Added master effect: {}", shader_name);
            }
        }
        Ok(())
    }

    fn remove_effect(&mut self, target: EffectTarget, effect_idx: usize) {
        match target {
            EffectTarget::Deck(ch_idx, deck_idx) => {
                if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                    if deck_idx < ch.decks.len() {
                        ch.decks[deck_idx].deck.remove_effect(effect_idx);
                    }
                }
            }
            EffectTarget::Channel(ch_idx) => {
                if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                    ch.remove_effect(effect_idx);
                }
            }
            EffectTarget::Master => {
                self.mixer.remove_master_effect(effect_idx);
            }
        }
    }

    fn toggle_effect(&mut self, target: EffectTarget, effect_idx: usize) {
        match target {
            EffectTarget::Deck(ch_idx, deck_idx) => {
                if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                    if deck_idx < ch.decks.len() {
                        let deck = &mut ch.decks[deck_idx].deck;
                        if effect_idx < deck.effects.len() {
                            deck.effects[effect_idx].enabled = !deck.effects[effect_idx].enabled;
                        }
                    }
                }
            }
            EffectTarget::Channel(ch_idx) => {
                if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                    if effect_idx < ch.effects.len() {
                        ch.effects[effect_idx].enabled = !ch.effects[effect_idx].enabled;
                    }
                }
            }
            EffectTarget::Master => {
                if effect_idx < self.mixer.master_effects().len() {
                    self.mixer.master_effects_mut()[effect_idx].enabled =
                        !self.mixer.master_effects_mut()[effect_idx].enabled;
                }
            }
        }
    }

    fn move_effect(&mut self, target: EffectTarget, from_idx: usize, to_idx: usize) {
        if from_idx == to_idx {
            return;
        }
        match target {
            EffectTarget::Deck(ch_idx, deck_idx) => {
                if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                    if deck_idx < ch.decks.len() {
                        let effects = &mut ch.decks[deck_idx].deck.effects;
                        if from_idx < effects.len() && to_idx < effects.len() {
                            let effect = effects.remove(from_idx);
                            effects.insert(to_idx, effect);
                        }
                    }
                }
            }
            EffectTarget::Channel(ch_idx) => {
                if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                    if from_idx < ch.effects.len() && to_idx < ch.effects.len() {
                        let effect = ch.effects.remove(from_idx);
                        ch.effects.insert(to_idx, effect);
                    }
                }
            }
            EffectTarget::Master => {
                if from_idx < self.mixer.master_effects().len()
                    && to_idx < self.mixer.master_effects().len()
                {
                    let effect = self.mixer.master_effects_mut().remove(from_idx);
                    self.mixer.master_effects_mut().insert(to_idx, effect);
                }
            }
        }
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

    fn set_param(&mut self, path: &str, value: ParamValue) {
        // Convert ParamValue to f32 for the param router
        let f_value = match value {
            ParamValue::Float(v) => v,
            ParamValue::Bool(b) => {
                if b {
                    1.0
                } else {
                    0.0
                }
            }
            ParamValue::Long(i) => i as f32,
            ParamValue::Color(c) => c[0],
            ParamValue::Point2D(p) => p[0],
        };
        if crate::param_router::apply_param_by_path(&mut self.mixer, path, f_value) {
            // Broadcast to OSC feedback targets
            if let Some(ref sender) = self.input.osc_feedback {
                if sender.has_targets() {
                    sender.send_param(path, f_value);
                }
            }
        }
    }
}

// ── Audio trait implementations ─────────────────────────────────────

impl AudioCommands for VardaApp {
    fn open_audio_source(&mut self, source_id: AudioSourceId) -> Result<()> {
        self.audio_manager
            .open_source(source_id)
            .map_err(|e| anyhow::anyhow!("Failed to open audio source: {}", e))
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
                };
                ModulationSourceSnapshotEntry {
                    uuid: entry.uuid.clone(),
                    source: snapshot,
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

    fn close_output(&mut self, idx: usize) {
        if idx < self.output.outputs.len() {
            let name = self.output.outputs[idx].name().to_string();
            // Stop active subprocess before removing to release ports/resources
            if let crate::renderer::context::UnifiedOutput::Headless(h) =
                &mut self.output.outputs[idx]
            {
                if let Some(mut sub) = h.subprocess.take() {
                    sub.stop();
                }
            }
            let removed = self.output.outputs.remove(idx);
            if let crate::renderer::context::UnifiedOutput::Window(w) = removed {
                w.destroy();
            }
            log::info!("Closed output '{}'", name);
        }
    }

    fn set_output_display(&mut self, idx: usize, monitor_name: &str) {
        if let Some(crate::renderer::context::UnifiedOutput::Window(output)) =
            self.output.outputs.get_mut(idx)
        {
            if let Some((mi, (_, handle))) = self
                .output
                .cached_monitors
                .iter()
                .enumerate()
                .find(|(_, (name, _))| name == monitor_name)
            {
                let target = crate::renderer::context::OutputTarget::Display {
                    name: monitor_name.to_string(),
                    monitor_index: mi,
                };
                output.set_target(target, Some(handle.clone()));
            }
        }
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
                                .find_by_uuid(&a.surface_uuid)
                                .map(|(_, s)| s.name.clone())
                                .unwrap_or_else(|| format!("Surface {}", a.surface_uuid));
                            SurfaceAssignmentSnapshot {
                                surface_uuid: a.surface_uuid.clone(),
                                surface_name,
                                enabled: a.enabled,
                            }
                        })
                        .collect();
                    let (target, is_on_display, is_active, calibration_mode, audio_passthrough) =
                        match o {
                            UnifiedOutput::Window(w) => (
                                w.target.clone(),
                                matches!(w.target, OutputTarget::Display { .. }),
                                false,
                                w.calibration_mode,
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
                                            .as_ref()
                                            .and_then(|s| s.audio_frames_written())
                                            .unwrap_or(0),
                                        frames_dropped: p
                                            .dropped
                                            .load(std::sync::atomic::Ordering::Relaxed),
                                    });
                                (
                                    h.target.clone(),
                                    false,
                                    h.active,
                                    crate::renderer::context::CalibrationMode::Off,
                                    audio,
                                )
                            }
                        };
                    OutputWindowSnapshot {
                        uuid: o.uuid().to_string(),
                        name: o.name().to_string(),
                        target_label: format!("{}", target),
                        target,
                        is_on_display,
                        is_active,
                        surface_assignments,
                        calibration_mode,
                        audio_passthrough,
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
        log::info!("Added surface '{}' (uuid {})", name, uuid);
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
        log::info!("Added circle surface '{}' (uuid {})", name, uuid);
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
                    overlap_zones: Default::default(),
                });
            }
        }
    }

    fn unassign_surface_from_output(&mut self, output_uuid: &str, assignment_idx: usize) {
        if let Some(output) = self
            .output
            .outputs
            .iter_mut()
            .find(|o| o.uuid() == output_uuid)
        {
            let assignments = output.surface_assignments_mut();
            if assignment_idx < assignments.len() {
                assignments.remove(assignment_idx);
            }
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
                        "Failed to open camera {}: {}",
                        camera_id, e
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
                "No frame received from camera {} within timeout",
                camera_id
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
    use clap::Parser;

    fn parse_args(args: &[&str]) -> super::super::AppConfig {
        super::super::AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
    }

    fn headless_app() -> Option<super::super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
        super::super::VardaApp::new(gpu, &config).ok()
    }

    #[test]
    fn move_deck_same_channel_noop() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.add_solid_color_deck(0, [1.0, 0.0, 0.0, 1.0]).unwrap();
        let result = app.move_deck(0, 0, 0);
        assert!(result.is_ok());
        // Deck should still be in channel 0
        let snap = app.mixer_snapshot();
        assert_eq!(snap.channels[0].decks.len(), 1);
    }

    #[test]
    fn move_deck_invalid_src_channel() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let result = app.move_deck(99, 0, 0);
        assert!(result.is_ok()); // silent no-op
    }

    #[test]
    fn move_deck_invalid_src_deck() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let result = app.move_deck(0, 99, 1);
        assert!(result.is_ok()); // silent no-op
    }

    #[test]
    fn move_deck_valid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.add_solid_color_deck(0, [1.0, 0.0, 0.0, 1.0]).unwrap();
        let snap = app.mixer_snapshot();
        assert_eq!(snap.channels[0].decks.len(), 1);
        assert_eq!(snap.channels[1].decks.len(), 0);

        let result = app.move_deck(0, 0, 1);
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
        app.add_solid_color_deck(0, [1.0, 0.0, 0.0, 1.0]).unwrap();
        app.add_solid_color_deck(0, [0.0, 1.0, 0.0, 1.0]).unwrap();
        app.add_solid_color_deck(0, [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 3);
        app.reorder_deck(0, 0, 2);
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 3);
    }

    #[test]
    fn reorder_deck_same_position_noop() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.add_solid_color_deck(0, [1.0, 0.0, 0.0, 1.0]).unwrap();
        app.reorder_deck(0, 0, 0);
        assert_eq!(app.mixer_snapshot().channels[0].decks.len(), 1);
    }

    #[test]
    fn reorder_deck_invalid_channel() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.reorder_deck(99, 0, 1); // no crash
    }

    #[test]
    fn set_deck_opacity_invalid_channel() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_deck_opacity(99, 0, 0.5); // no crash
    }

    #[test]
    fn set_deck_opacity_invalid_deck() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_deck_opacity(0, 99, 0.5); // no crash
    }

    #[test]
    fn set_deck_blend_mode_invalid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_deck_blend_mode(99, 99, BlendMode::Add); // no crash
    }

    #[test]
    fn set_deck_solo_invalid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_deck_solo(99, 99, true); // no crash
    }

    #[test]
    fn set_deck_mute_invalid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_deck_mute(99, 99, true); // no crash
    }

    #[test]
    fn set_channel_opacity_clamps() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.set_channel_opacity(0, 2.0);
        let snap = app.mixer_snapshot();
        assert!(
            (snap.channels[0].opacity - 1.0).abs() < 1e-5,
            "should clamp to 1.0"
        );

        app.set_channel_opacity(0, -1.0);
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
        let result = app.remove_channel(0);
        assert!(result.is_err());
        assert_eq!(app.mixer_snapshot().channels.len(), 2);
    }

    #[test]
    fn toggle_effect_invalid_index() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Toggle on non-existent effect → no crash
        app.toggle_effect(EffectTarget::Deck(0, 0), 99);
        app.toggle_effect(EffectTarget::Channel(0), 99);
        app.toggle_effect(EffectTarget::Master, 99);
    }

    #[test]
    fn set_param_invalid_path() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Non-existent param path → silent failure
        app.set_param("ch99/deck99/nonexistent_param", ParamValue::Float(0.5));
    }
}
