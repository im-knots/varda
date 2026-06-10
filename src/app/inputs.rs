//! Input processing — shader hot-reload, audio polling, OSC, MIDI.
//!
//! Called once per frame before the render pass.
//! After processing, changed parameters are broadcast via OSC feedback.

use super::VardaApp;

impl VardaApp {
    /// Process all external inputs: shader hot-reload, audio, OSC, MIDI.
    /// Changed parameter paths are collected and broadcast to OSC feedback targets.
    pub fn process_inputs(&mut self) {
        // Collect (path, value) pairs changed this frame for OSC feedback
        let mut changed_params: Vec<(String, f32)> = Vec::new();
        // Poll for shader file changes (hot-reload)
        let shader_events = self.registry.poll_changes();
        for event in &shader_events {
            match event {
                crate::registry::ShaderEvent::Changed(path) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown");
                    self.session
                        .notifications
                        .info(format!("Shader reloaded: {}", name));
                }
                crate::registry::ShaderEvent::Removed(path) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown");
                    self.session
                        .notifications
                        .warn(format!("Shader removed: {}", name));
                }
                crate::registry::ShaderEvent::Error(path, err) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown");
                    self.session
                        .notifications
                        .error(format!("Shader error in {}: {}", name, err));
                }
            }
        }

        // Poll all audio sources
        self.audio_manager.poll();

        // Update audio textures (using primary source)
        self.audio_textures
            .update(&self.context.queue, self.audio_manager.get_primary_data());

        // Pre-update modulation with fresh audio so snapshots read current values
        {
            let mut av = crate::modulation::AudioValues::default();
            for id in self.audio_manager.active_source_ids() {
                if let Some(data) = self.audio_manager.get_data(id) {
                    av.sources.insert(
                        id,
                        crate::modulation::AudioSourceValues {
                            fft: data.fft.clone(),
                            level: data.level,
                            sample_rate: data.sample_rate,
                        },
                    );
                }
            }
            let analyzer_vals = crate::modulation::AnalyzerValues::default();
            self.mixer.update_modulation(&av, &analyzer_vals);
        }

        // Process OSC messages via shared param router
        if let Some(osc) = &self.input.osc_receiver {
            while let Some(input) = osc.try_recv() {
                match input {
                    crate::osc::OscInput::Param { ref path, value } => {
                        if path.starts_with("action/") && value > 0.5 {
                            match path.as_str() {
                                "action/undo" => self.midi_pending_undo = true,
                                "action/redo" => self.midi_pending_redo = true,
                                "action/save" => self.midi_pending_save = true,
                                _ => {
                                    log::debug!("Unknown OSC action: {}", path);
                                }
                            }
                        } else if crate::param_router::apply_param_by_path(
                            &mut self.mixer,
                            path,
                            value,
                        ) {
                            changed_params.push((path.clone(), value));
                        }
                    }
                    crate::osc::OscInput::ClockBpm(bpm) => {
                        self.input.clock_manager.process_osc_bpm(bpm);
                    }
                    crate::osc::OscInput::ClockBeat(phase) => {
                        self.input.clock_manager.process_osc_beat(phase);
                    }
                    crate::osc::OscInput::Unknown(addr) => {
                        log::debug!("Unknown OSC address: {}", addr);
                    }
                }
            }
        }

        // Process MIDI messages → apply to mixer via mapping store, forward clock to ClockManager
        if let Some(midi) = &self.input.midi_devices {
            while let Some(msg) = midi.try_recv() {
                // Forward clock messages to ClockManager
                match &msg {
                    crate::midi::MidiMessage::ClockTick { device_id } => {
                        let dev_name = midi
                            .device(*device_id)
                            .map(|d| d.name.as_str())
                            .unwrap_or("Unknown");
                        self.input
                            .clock_manager
                            .process_midi_tick(*device_id, dev_name);
                        continue;
                    }
                    crate::midi::MidiMessage::ClockStart { .. } => {
                        self.input.clock_manager.process_midi_start();
                        continue;
                    }
                    crate::midi::MidiMessage::ClockContinue { .. } => {
                        self.input.clock_manager.process_midi_continue();
                        continue;
                    }
                    crate::midi::MidiMessage::ClockStop { .. } => {
                        self.input.clock_manager.process_midi_stop();
                        continue;
                    }
                    _ => {}
                }

                let key = match msg.mapping_key() {
                    Some(k) => k,
                    None => continue,
                };

                // Auto-map: intercept keys owned by auto-mapping before normal lookup
                if self
                    .input
                    .auto_map_engine
                    .handles_key(msg.device_id(), &key)
                {
                    match &msg {
                        crate::midi::MidiMessage::NoteOn {
                            device_id,
                            note,
                            velocity,
                            channel,
                            ..
                        } => {
                            if *velocity > 0 {
                                self.input
                                    .auto_map_engine
                                    .process_note_on(*device_id, *note, *channel);
                            } else {
                                self.input.auto_map_engine.process_note_off(
                                    *device_id,
                                    *note,
                                    *channel,
                                    &mut self.mixer,
                                );
                            }
                        }
                        crate::midi::MidiMessage::NoteOff {
                            device_id,
                            note,
                            channel,
                            ..
                        } => {
                            self.input.auto_map_engine.process_note_off(
                                *device_id,
                                *note,
                                *channel,
                                &mut self.mixer,
                            );
                        }
                        crate::midi::MidiMessage::ControlChange {
                            device_id,
                            cc,
                            value,
                            ..
                        } => {
                            self.input.auto_map_engine.process_cc(
                                *device_id,
                                *cc,
                                *value,
                                &mut self.mixer,
                            );
                        }
                        _ => {}
                    }
                    continue;
                }

                let value = msg.normalized_value();

                // Learn mode: map next MIDI input to the learn target
                if self.input.midi_mappings.learn_mode {
                    self.input.midi_mappings.process_learn(key);
                }

                // Apply mapped value to mixer, clock, or global actions
                if let Some(path) = self.input.midi_mappings.get(&key).cloned() {
                    if path == "clock/bpm" {
                        // Map normalized 0.0–1.0 → 20–300 BPM range
                        let bpm = 20.0 + value * 280.0;
                        if !matches!(
                            self.input.clock_manager.preference(),
                            crate::clock::ClockPreference::ForceManual { .. }
                        ) {
                            self.input
                                .clock_manager
                                .set_preference(crate::clock::ClockPreference::ForceManual { bpm });
                        } else {
                            self.input.clock_manager.set_manual_bpm(bpm);
                        }
                    } else if path.starts_with("action/") && value > 0.5 {
                        // Global actions — trigger on note-on / CC > 50%
                        match path.as_str() {
                            "action/undo" => self.midi_pending_undo = true,
                            "action/redo" => self.midi_pending_redo = true,
                            "action/save" => self.midi_pending_save = true,
                            _ => {
                                log::debug!("Unknown action path: {}", path);
                            }
                        }
                    } else if crate::param_router::apply_param_by_path(
                        &mut self.mixer,
                        &path,
                        value,
                    ) {
                        changed_params.push((path.clone(), value));
                    }
                } else if !self.input.midi_mappings.learn_mode {
                    log::debug!("Unmapped MIDI: {} value={:.2}", key, value);
                }
            }
        }

        // Feed audio BPM to ClockManager
        {
            let primary = self.audio_manager.get_primary_data();
            self.input
                .clock_manager
                .update_audio(primary.bpm, primary.beat_phase());
        }

        // Resolve clock priority
        self.input.clock_manager.update();

        // Broadcast changed parameters to OSC feedback targets
        if !changed_params.is_empty() {
            if let Some(ref sender) = self.input.osc_feedback {
                if sender.has_targets() {
                    for (path, value) in &changed_params {
                        sender.send_param(path, *value);
                    }
                }
            }
        }
    }
}
