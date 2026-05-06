//! Engine trait implementations for VardaApp.

use super::VardaApp;
use crate::engine::traits::*;
use crate::engine::types::*;
use crate::deck::{Deck, Effect};
use crate::modulation::ModulationSource;
use super::{RENDER_WIDTH, RENDER_HEIGHT};
use anyhow::{Context as _, Result};

impl MixerCommands for VardaApp {
    fn set_crossfader(&mut self, position: f32) {
        self.mixer.snap_crossfader(position);
    }

    fn snap_crossfader(&mut self, position: f32) {
        self.mixer.snap_crossfader(position);
    }

    fn start_auto_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing) {
        self.mixer.start_crossfade(target, duration_secs, easing);
    }

    fn start_beat_crossfade(&mut self, target: f32, beats: f32) {
        self.mixer.start_beat_crossfade(target, beats);
    }

    fn add_deck(&mut self, channel_idx: usize, shader_name: &str) -> Result<()> {
        let generators = self.registry.generators();
        let shader = generators.iter()
            .find(|s| s.name() == shader_name)
            .context("Shader not found")?;
        let deck = Deck::new(&self.context, (*shader).clone(), RENDER_WIDTH, RENDER_HEIGHT)?;
        let ch = self.mixer.channel_mut(channel_idx).context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added deck {} to channel {} with shader: {}", idx, channel_idx, shader_name);
        Ok(())
    }

    fn add_image_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()> {
        let deck = Deck::new_from_image(&self.context, path, RENDER_WIDTH, RENDER_HEIGHT)?;
        let ch = self.mixer.channel_mut(channel_idx).context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added image deck {} to channel {}: {}", idx, channel_idx, name);
        Ok(())
    }

    fn add_video_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()> {
        let deck = Deck::new_from_video(&self.context, path, RENDER_WIDTH, RENDER_HEIGHT)?;
        let ch = self.mixer.channel_mut(channel_idx).context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added video deck {} to channel {}: {}", idx, channel_idx, name);
        Ok(())
    }

    fn add_solid_color_deck(&mut self, channel_idx: usize, color: [f32; 4]) -> Result<()> {
        let deck = Deck::new_solid_color(&self.context, color, RENDER_WIDTH, RENDER_HEIGHT)?;
        let ch = self.mixer.channel_mut(channel_idx).context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added solid color deck {} to channel {}: {}", idx, channel_idx, name);
        Ok(())
    }

    fn add_camera_deck(&mut self, channel_idx: usize, camera_id: CameraId) -> Result<()> {
        let cam_name = self.camera_manager.devices().iter()
            .find(|d| d.id == camera_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("Camera {}", camera_id));
        let (src_w, src_h) = self.camera_manager.open_camera(camera_id, &self.context.device)?;
        let deck = Deck::new_from_camera(&self.context, camera_id, &cam_name, src_w, src_h, RENDER_WIDTH, RENDER_HEIGHT)?;
        let ch = self.mixer.channel_mut(channel_idx).context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added camera deck {} to channel {}: {}", idx, channel_idx, cam_name);
        Ok(())
    }

    fn remove_deck(&mut self, channel_idx: usize, deck_idx: usize) -> Result<()> {
        // Release camera if applicable
        if let Some(ch) = self.mixer.channels.get(channel_idx) {
            if let Some(slot) = ch.decks.get(deck_idx) {
                if let Some(cam_id) = slot.deck.camera_id() {
                    self.camera_manager.release_camera(cam_id);
                }
            }
        }
        let ch = self.mixer.channel_mut(channel_idx).context("Invalid channel")?;
        if deck_idx < ch.decks.len() {
            ch.remove_deck(deck_idx);
            log::info!("Removed deck {} from channel {}", deck_idx, channel_idx);
        }
        Ok(())
    }

    fn move_deck(&mut self, src_ch: usize, src_deck: usize, dst_ch: usize) -> Result<()> {
        if src_ch != dst_ch && src_ch < self.mixer.channels.len() && dst_ch < self.mixer.channels.len() {
            if src_deck < self.mixer.channels[src_ch].decks.len() {
                let slot = self.mixer.channels[src_ch].remove_deck_slot(src_deck).unwrap();
                let new_idx = self.mixer.channels[dst_ch].add_deck_slot(slot);
                log::info!("Moved deck {} from ch{} to ch{} (new idx {})", src_deck, src_ch, dst_ch, new_idx);
            }
        }
        Ok(())
    }

    fn set_deck_opacity(&mut self, channel_idx: usize, deck_idx: usize, opacity: f32) {
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

    fn set_channel_opacity(&mut self, channel_idx: usize, opacity: f32) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            ch.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    fn set_channel_blend_mode(&mut self, channel_idx: usize, mode: BlendMode) {
        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
            ch.blend_mode = mode;
        }
    }

    fn add_channel(&mut self) -> Result<usize> {
        self.mixer.add_channel(&self.context, RENDER_WIDTH, RENDER_HEIGHT)
    }

    fn remove_channel(&mut self, channel_idx: usize) -> Result<()> {
        if self.mixer.remove_channel(channel_idx) {
            // Fix selections
            if let Some((sel_ch, _)) = self.selected_deck {
                if sel_ch == channel_idx { self.selected_deck = None; }
                else if sel_ch > channel_idx {
                    self.selected_deck = Some((sel_ch - 1, self.selected_deck.unwrap().1));
                }
            }
            if let Some(sel_ch) = self.selected_channel {
                if sel_ch == channel_idx { self.selected_channel = None; }
                else if sel_ch > channel_idx { self.selected_channel = Some(sel_ch - 1); }
            }
            Ok(())
        } else {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }
    }

    fn add_effect(&mut self, target: EffectTarget, shader_name: &str) -> Result<()> {
        let filters = self.registry.filters();
        let shader = filters.iter()
            .find(|s| s.name() == shader_name)
            .context("Filter shader not found")?;
        match target {
            EffectTarget::Deck(ch_idx, deck_idx) => {
                let effect = Effect::new(&self.context, (*shader).clone())?;
                let ch = self.mixer.channel_mut(ch_idx).context("Invalid channel")?;
                if deck_idx < ch.decks.len() {
                    ch.decks[deck_idx].deck.add_effect(effect);
                    log::info!("Added effect {} to ch{} deck {}", shader_name, ch_idx, deck_idx);
                }
            }
            EffectTarget::Channel(ch_idx) => {
                let effect = Effect::new_with_format(&self.context, (*shader).clone(), self.context.texture_format)?;
                let ch = self.mixer.channel_mut(ch_idx).context("Invalid channel")?;
                ch.add_effect(effect);
                log::info!("Added channel effect {} to ch{}", shader_name, ch_idx);
            }
            EffectTarget::Master => {
                let effect = Effect::new_with_format(&self.context, (*shader).clone(), self.context.texture_format)?;
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
                if effect_idx < self.mixer.master_effects.len() {
                    self.mixer.master_effects[effect_idx].enabled = !self.mixer.master_effects[effect_idx].enabled;
                }
            }
        }
    }

    fn move_effect(&mut self, target: EffectTarget, from_idx: usize, to_idx: usize) {
        if from_idx == to_idx { return; }
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
                if from_idx < self.mixer.master_effects.len() && to_idx < self.mixer.master_effects.len() {
                    let effect = self.mixer.master_effects.remove(from_idx);
                    self.mixer.master_effects.insert(to_idx, effect);
                }
            }
        }
    }

    fn set_transition(&mut self, shader_name: Option<&str>) -> Result<()> {
        match shader_name {
            None => { self.mixer.clear_transition(); Ok(()) }
            Some(name) => {
                let shader = self.registry.get(name).context("Transition shader not found")?;
                self.mixer.set_transition(&self.context, shader.clone())
            }
        }
    }

    fn set_param(&mut self, path: &str, value: ParamValue) {
        // Convert ParamValue to f32 for the MIDI param router
        let f_value = match value {
            ParamValue::Float(v) => v,
            ParamValue::Bool(b) => if b { 1.0 } else { 0.0 },
            ParamValue::Long(i) => i as f32,
            ParamValue::Color(c) => c[0],
            ParamValue::Point2D(p) => p[0],
        };
        crate::midi::apply_midi_to_param(&mut self.mixer, path, f_value);
    }
}

// ── Audio trait implementations ─────────────────────────────────────

impl AudioCommands for VardaApp {
    fn open_audio_source(&mut self, source_id: AudioSourceId) -> Result<()> {
        self.audio_manager.open_source(source_id)
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
            devices: self.audio_manager.devices().iter().map(|d| AudioDeviceSnapshot {
                id: d.id,
                name: d.name.clone(),
                active: active_ids.contains(&d.id),
            }).collect(),
            fft: primary_audio.fft.clone(),
            sample_rate: primary_audio.sample_rate,
        }
    }
}


// ── Modulation trait implementations ────────────────────────────────

impl ModulationCommands for VardaApp {
    fn add_lfo(&mut self, waveform: LFOWaveform, frequency: f32) -> usize {
        let source = ModulationSource::LFO {
            waveform, frequency, phase: 0.0, amplitude: 1.0, bipolar: false,
        };
        self.mixer.modulation.add_source(source)
    }

    fn add_audio_band(&mut self, preset: AudioBandPreset, source_id: Option<AudioSourceId>) -> usize {
        let (freq_low, freq_high) = preset.freq_range();
        let source = ModulationSource::AudioBand {
            source_id, freq_low, freq_high, gain: 1.0, smoothing: 0.6,
            mode: crate::modulation::AudioReactMode::Direct, noise_gate: 0.1,
        };
        self.mixer.modulation.add_source(source)
    }

    fn add_adsr(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> usize {
        let source = ModulationSource::adsr(attack, decay, sustain, release);
        self.mixer.modulation.add_source(source)
    }

    fn add_step_sequencer(&mut self, num_steps: usize, rate: f32) -> usize {
        let source = ModulationSource::step_sequencer(num_steps, rate);
        self.mixer.modulation.add_source(source)
    }

    fn remove_modulation_source(&mut self, idx: usize) {
        self.mixer.modulation.remove_source(idx);
    }

    fn assign_modulation(&mut self, target: &str, source_idx: usize, amount: f32) {
        self.mixer.modulation.assign(target, source_idx, amount, None);
    }

    fn clear_modulation(&mut self, target: &str) {
        self.mixer.modulation.clear_assignments(target);
    }
}

impl ModulationQueries for VardaApp {
    fn modulation_snapshot(&self) -> ModulationSnapshot {
        let m = &self.mixer;
        let sources = m.modulation.sources.iter().map(|src| {
            match src {
                ModulationSource::LFO { waveform, frequency, phase, amplitude, bipolar } => {
                    ModulationSourceSnapshot::LFO {
                        waveform: *waveform, frequency: *frequency, phase: *phase,
                        amplitude: *amplitude, bipolar: *bipolar,
                    }
                }
                ModulationSource::AudioBand { source_id, freq_low, freq_high, gain, smoothing, mode, noise_gate } => {
                    ModulationSourceSnapshot::Audio {
                        source_id: *source_id, freq_low: *freq_low, freq_high: *freq_high,
                        gain: *gain, smoothing: *smoothing, mode: *mode, noise_gate: *noise_gate,
                    }
                }
                ModulationSource::ADSR { attack, decay, sustain, release, stage, .. } => {
                    ModulationSourceSnapshot::ADSR {
                        attack: *attack, decay: *decay, sustain: *sustain,
                        release: *release, stage: *stage,
                    }
                }
                ModulationSource::StepSequencer { steps, rate, interpolation, bipolar } => {
                    ModulationSourceSnapshot::StepSequencer {
                        steps: steps.clone(), rate: *rate,
                        interpolation: *interpolation, bipolar: *bipolar,
                    }
                }
            }
        }).collect();
        let current_values = m.modulation.current_values().to_vec();
        let assignments = m.modulation.assignments.iter().map(|(k, v)| {
            (k.clone(), v.iter().map(|pm| ModulationAssignmentSnapshot {
                source_idx: pm.source_idx,
                amount: pm.amount,
            }).collect())
        }).collect();
        ModulationSnapshot { sources, current_values, assignments }
    }
}

// ── Output trait implementations ────────────────────────────────────

impl OutputCommands for VardaApp {
    fn request_create_output(&mut self) {
        self.pending_output_creates.push(());
    }

    fn close_output(&mut self, idx: usize) {
        if idx < self.output_windows.len() {
            let name = self.output_windows[idx].name.clone();
            let output = self.output_windows.remove(idx);
            output.destroy();
            log::info!("Closed output window '{}'", name);
        }
    }

    fn set_output_display(&mut self, idx: usize, monitor_name: &str) {
        if let Some(output) = self.output_windows.get_mut(idx) {
            if let Some((mi, (_, handle))) = self.cached_monitors.iter().enumerate()
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
            windows: self.output_windows.iter().map(|o| {
                OutputWindowSnapshot {
                    name: o.name.clone(),
                    target_label: format!("{}", o.target),
                    is_on_display: matches!(o.target, crate::renderer::context::OutputTarget::Display { .. }),
                    surface_assignments: o.surface_assignments.iter().map(|a| {
                        let surface_name = self.surface_manager.surfaces.get(a.surface_idx)
                            .map(|s| s.name.clone())
                            .unwrap_or_else(|| format!("Surface {}", a.surface_idx));
                        SurfaceAssignmentSnapshot {
                            surface_idx: a.surface_idx,
                            surface_name,
                            warp_corners: a.warp_corners,
                            enabled: a.enabled,
                        }
                    }).collect(),
                    calibration_mode: o.calibration_mode,
                }
            }).collect(),
            surfaces: self.surface_manager.surfaces.iter().map(|s| SurfaceSnapshot {
                name: s.name.clone(),
                vertices: s.vertices.clone(),
                extra_contours: s.extra_contours.clone(),
                source: s.source.clone(),
                content_mapping: s.content_mapping,
                output_type: s.output_type,
                circle_hint: s.circle_hint,
            }).collect(),
            monitors: self.cached_monitors.iter().enumerate().map(|(i, (name, handle))| {
                let size = handle.size();
                MonitorSnapshot {
                    name: name.clone(),
                    index: i,
                    width: size.width,
                    height: size.height,
                }
            }).collect(),
        }
    }
}

// ── MixerQueries ────────────────────────────────────────────────────

impl MixerQueries for VardaApp {
    fn mixer_snapshot(&self) -> MixerSnapshot {
        crate::app::snapshot::build_mixer_snapshot(self)
    }
}