//! Input processing — shader hot-reload, audio polling, OSC, MIDI.
//!
//! Called once per frame before the render pass.

use super::VardaApp;

impl VardaApp {
    /// Process all external inputs: shader hot-reload, audio, OSC, MIDI.
    pub fn process_inputs(&mut self) {
        // Poll for shader file changes (hot-reload)
        let shader_events = self.registry.poll_changes();
        for event in &shader_events {
            match event {
                crate::registry::ShaderEvent::Changed(path) => {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                    self.notifications.info(format!("Shader reloaded: {}", name));
                }
                crate::registry::ShaderEvent::Removed(path) => {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                    self.notifications.warn(format!("Shader removed: {}", name));
                }
                crate::registry::ShaderEvent::Error(path, err) => {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                    self.notifications.error(format!("Shader error in {}: {}", name, err));
                }
            }
        }

        // Poll all audio sources
        self.audio_manager.poll();

        // Update audio textures (using primary source)
        self.audio_textures.update(&self.context.queue, self.audio_manager.get_primary_data());

        // Pre-update modulation with fresh audio so snapshots read current values
        {
            let mut av = crate::modulation::AudioValues::default();
            for id in self.audio_manager.active_source_ids() {
                if let Some(data) = self.audio_manager.get_data(id) {
                    av.sources.insert(id, crate::modulation::AudioSourceValues {
                        fft: data.fft.clone(),
                        level: data.level,
                        sample_rate: data.sample_rate,
                    });
                }
            }
            self.mixer.update_modulation(&av);
        }

        // Process OSC messages (mapped to channel A for now)
        if let Some(osc) = &self.osc_receiver {
            while let Some(ctrl) = osc.try_recv() {
                match ctrl {
                    crate::osc::OscControl::SetOpacity(deck_idx, val) => {
                        if let Some(ch) = self.mixer.channel_mut(0) {
                            ch.set_deck_opacity(deck_idx, val);
                        }
                    }
                    crate::osc::OscControl::SetSolo(deck_idx, enabled) => {
                        if let Some(ch) = self.mixer.channel_mut(0) {
                            ch.set_deck_solo(deck_idx, enabled);
                        }
                    }
                    crate::osc::OscControl::SetMute(deck_idx, enabled) => {
                        if let Some(ch) = self.mixer.channel_mut(0) {
                            ch.set_deck_mute(deck_idx, enabled);
                        }
                    }
                    crate::osc::OscControl::ClockBpm(bpm) => {
                        self.clock_manager.process_osc_bpm(bpm);
                    }
                    crate::osc::OscControl::ClockBeat(phase) => {
                        self.clock_manager.process_osc_beat(phase);
                    }
                    crate::osc::OscControl::Unknown(addr, args) => {
                        log::debug!("Unknown OSC: {} {:?}", addr, args);
                    }
                    _ => {}
                }
            }
        }

        // Process MIDI messages → apply to mixer via mapping store, forward clock to ClockManager
        if let Some(midi) = &self.midi_devices {
            while let Some(msg) = midi.try_recv() {
                // Forward clock messages to ClockManager
                match &msg {
                    crate::midi::MidiMessage::ClockTick { device_id } => {
                        let dev_name = midi.device(*device_id)
                            .map(|d| d.name.as_str()).unwrap_or("Unknown");
                        self.clock_manager.process_midi_tick(*device_id, dev_name);
                        continue;
                    }
                    crate::midi::MidiMessage::ClockStart { .. } => {
                        self.clock_manager.process_midi_start();
                        continue;
                    }
                    crate::midi::MidiMessage::ClockContinue { .. } => {
                        self.clock_manager.process_midi_continue();
                        continue;
                    }
                    crate::midi::MidiMessage::ClockStop { .. } => {
                        self.clock_manager.process_midi_stop();
                        continue;
                    }
                    _ => {}
                }

                let key = match msg.mapping_key() {
                    Some(k) => k,
                    None => continue,
                };
                let value = msg.normalized_value();

                // Learn mode: map next MIDI input to the learn target
                if self.midi_mappings.learn_mode {
                    self.midi_mappings.process_learn(key);
                }

                // Apply mapped value to mixer or clock (both normal and learn mode)
                if let Some(path) = self.midi_mappings.get(&key).cloned() {
                    if path == "clock/bpm" {
                        // Map normalized 0.0–1.0 → 20–300 BPM range
                        let bpm = 20.0 + value * 280.0;
                        if !matches!(self.clock_manager.preference(), crate::clock::ClockPreference::ForceManual { .. }) {
                            self.clock_manager.set_preference(crate::clock::ClockPreference::ForceManual { bpm });
                        } else {
                            self.clock_manager.set_manual_bpm(bpm);
                        }
                    } else {
                        crate::midi::apply_midi_to_param(&mut self.mixer, &path, value);
                    }
                } else if !self.midi_mappings.learn_mode {
                    log::debug!("Unmapped MIDI: {} value={:.2}", key, value);
                }
            }
        }

        // Feed audio BPM to ClockManager
        {
            let primary = self.audio_manager.get_primary_data();
            self.clock_manager.update_audio(primary.bpm, primary.beat_phase());
        }

        // Resolve clock priority
        self.clock_manager.update();
    }
}
