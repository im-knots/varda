//! Input processing — shader hot-reload, audio polling, OSC, MIDI.
//!
//! Called once per frame before the render pass.
//! After processing, changed parameters are broadcast via OSC feedback.

use super::VardaApp;

/// The cue a control-surface write asks for, if it asks for one.
///
/// `cue/<uuid>/fire` is taken on the rising edge like `deck/<uuid>/trigger`, so
/// one press of a pad is one jump and a fader swept past halfway is one too.
fn fired_cue(path: &str, value: f32) -> Option<String> {
    if value <= 0.5 {
        return None;
    }
    let rest = path.strip_prefix("cue/")?;
    let uuid = rest.strip_suffix("/fire")?;
    (!uuid.is_empty() && !uuid.contains('/')).then(|| uuid.to_string())
}

impl VardaApp {
    /// One normalized write from a control surface, whichever surface it came
    /// from.
    ///
    /// MIDI and OSC address the same paths, so they ask the same three
    /// questions in the same order: is this a global action, is it a cue, is it
    /// a parameter. Cues are collected rather than fired here because the
    /// transport is not part of the mixer and a jump mid-drain would reorder the
    /// writes still queued behind it.
    fn apply_surface_write(
        &mut self,
        path: &str,
        value: f32,
        fired_cues: &mut Vec<String>,
        changed_params: &mut Vec<(String, f32)>,
    ) {
        if path.starts_with("action/") && value > 0.5 {
            // Global actions — trigger on note-on / CC > 50%
            match path {
                "action/undo" => self.midi_pending_undo = true,
                "action/redo" => self.midi_pending_redo = true,
                "action/save" => self.midi_pending_save = true,
                // A pad, so it toggles rather than needing two bindings.
                "action/record" => self.set_record_armed(!self.record_armed()),
                _ => log::debug!("Unknown action path: {path}"),
            }
        } else if let Some(uuid) = fired_cue(path, value) {
            fired_cues.push(uuid);
        } else {
            match crate::param_router::apply_param_by_path(&mut self.mixer, path, value) {
                Ok(()) => changed_params.push((path.to_string(), value)),
                Err(e) => log::warn!("Param route failed ({path}): {e}"),
            }
        }
    }

    /// Process all external inputs: shader hot-reload, audio, OSC, MIDI.
    /// Changed parameter paths are collected and broadcast to OSC feedback targets.
    pub fn process_inputs(&mut self) {
        // Collect (path, value) pairs changed this frame for OSC feedback
        let mut changed_params: Vec<(String, f32)> = Vec::new();
        // Cues a control surface asked for. Applied after the loops below,
        // which hold a borrow of the receiver they are draining.
        let mut fired_cues: Vec<String> = Vec::new();
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
                        .info(format!("Shader reloaded: {name}"));
                    // Lift any GPU quarantine: the author just changed the
                    // source, so whatever failed may be fixed. Without this a
                    // single bad save blacks the deck out until restart.
                    for ch in self.mixer.channels_mut() {
                        for slot in &mut ch.decks {
                            slot.deck.clear_gpu_error();
                        }
                    }
                    self.session
                        .notifications
                        .clear_once_key_prefix("gpu_fault:");
                }
                crate::registry::ShaderEvent::Removed(path) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown");
                    self.session
                        .notifications
                        .warn(format!("Shader removed: {name}"));
                }
                crate::registry::ShaderEvent::Error(path, err) => {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown");
                    self.session
                        .notifications
                        .error(format!("Shader error in {name}: {err}"));
                }
            }
        }

        // Reconcile audio capture with modulator demand: capture a device only
        // while an AudioBand modulator references it (issue #76). Derived from
        // mixer state each frame, so add/remove/device-switch/scene-load are all
        // handled here with no per-handler bookkeeping.
        // See /spec/audio-capture-lifecycle.md.
        {
            let bands = self.mixer.modulation().audio_band_source_ids();
            let default = self.audio_manager.default_source_id();
            let needed = crate::audio::AudioManager::needed_from_bands(&bands, default);
            self.audio_manager.set_modulation_refs(&needed);
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
            let beat_time = self.input.clock_manager.beat_time();
            let transport = self.transport.sample();
            self.mixer
                .update_modulation(beat_time, transport, &av, &analyzer_vals);
        }

        // Process OSC messages via shared param router. Drained first because
        // dispatching a write needs the whole app, and the receiver is part of
        // it.
        let osc_inputs: Vec<crate::osc::OscInput> = self
            .input
            .osc_receiver
            .as_ref()
            .map(|osc| std::iter::from_fn(|| osc.try_recv()).collect())
            .unwrap_or_default();
        for input in osc_inputs {
            match input {
                crate::osc::OscInput::Param { ref path, value } => {
                    self.apply_surface_write(path, value, &mut fired_cues, &mut changed_params);
                }
                crate::osc::OscInput::ClockBpm(bpm) => {
                    self.input.clock_manager.process_osc_bpm(bpm);
                }
                crate::osc::OscInput::ClockBeat(phase) => {
                    self.input.clock_manager.process_osc_beat(phase);
                }
                crate::osc::OscInput::Unknown(addr) => {
                    log::debug!("Unknown OSC address: {addr}");
                }
            }
        }

        // Process MIDI messages → apply to mixer via mapping store, forward
        // clock to ClockManager. Clock messages are answered as they are
        // drained, since they need only the clock manager and the device name
        // beside it. Everything mappable is kept for the pass below, which needs
        // the whole app.
        let mut mappable: Vec<crate::midi::MidiMessage> = Vec::new();
        if let Some(midi) = &self.input.midi_devices {
            while let Some(msg) = midi.try_recv() {
                match &msg {
                    crate::midi::MidiMessage::ClockTick { device_id } => {
                        let dev_name = midi
                            .device(*device_id)
                            .map_or("Unknown", |d| d.name.as_str());
                        self.input
                            .clock_manager
                            .process_midi_tick(*device_id, dev_name);
                    }
                    crate::midi::MidiMessage::ClockStart { .. } => {
                        self.input.clock_manager.process_midi_start();
                    }
                    crate::midi::MidiMessage::ClockContinue { .. } => {
                        self.input.clock_manager.process_midi_continue();
                    }
                    crate::midi::MidiMessage::ClockStop { .. } => {
                        self.input.clock_manager.process_midi_stop();
                    }
                    _ => mappable.push(msg),
                }
            }
        }

        for msg in mappable {
            let Some(key) = msg.mapping_key() else {
                continue;
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
                    if matches!(
                        self.input.clock_manager.preference(),
                        crate::clock::ClockPreference::ForceManual { .. }
                    ) {
                        self.input.clock_manager.set_manual_bpm(bpm);
                    } else {
                        self.input
                            .clock_manager
                            .set_preference(crate::clock::ClockPreference::ForceManual { bpm });
                    }
                } else {
                    self.apply_surface_write(&path, value, &mut fired_cues, &mut changed_params);
                }
            } else if !self.input.midi_mappings.learn_mode {
                log::debug!("Unmapped MIDI: {key} value={value:.2}");
            }
        }

        // Locate to any cue a control surface pressed. Handled here rather than
        // in `param_router` because the transport is not part of the mixer, and
        // a cue that has since been deleted is ignored the way a mapping to a
        // deleted deck is. See /spec/arrangement.md § The cue bank.
        for uuid in fired_cues {
            if let crate::engine::CommandResult::Err { message, .. } = self.cmd_trigger_cue(&uuid) {
                log::debug!("Cue fire ignored ({uuid}): {message}");
            }
        }

        // Drain macro-triggered global actions. Trigger buttons routed through
        // `macro/<uuid>/value` queue these on the macro bank; here we forward
        // them onto the same pending flags as the MIDI `action/*` paths so the
        // runner dispatches undo/redo/save uniformly.
        for action in self.mixer.macros_mut().take_pending_actions() {
            match action {
                crate::macros::GlobalAction::Undo => self.midi_pending_undo = true,
                crate::macros::GlobalAction::Redo => self.midi_pending_redo = true,
                crate::macros::GlobalAction::Save => self.midi_pending_save = true,
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

        // Advance the show position. After command processing, so a locate that
        // arrived this frame is published rather than overwritten.
        self.transport.update();

        // Every surface write above is a performer's hand, so anything that
        // landed on a parameter the arrangement drives takes that lane back.
        // Done once at the end rather than per write, so a controller sweeping a
        // fader across a frame holds the parameter once.
        for (path, value) in &changed_params {
            self.note_live_route_write(path, *value);
        }

        // After the writes, so a take that opened this frame is not closed by
        // the same frame's stop, and after the transport, so a loop wrap has
        // already been published as the jump it is.
        self.tick_recorder();

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

#[cfg(test)]
mod tests {
    use super::{fired_cue, VardaApp};
    use crate::engine::{EngineCommand as C, MixerQueries};
    use crate::midi::{MidiDeviceManager, MidiKey, MidiMessage};
    use crate::osc::{OscInput, OscReceiver};

    /// An app with one white deck in channel 0, returning the deck's UUID.
    ///
    /// Returns `None` with no GPU adapter, the way every other GPU-backed test
    /// in the tree does.
    fn app_with_a_deck() -> Option<(VardaApp, String)> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let mut app = VardaApp::new(gpu, &crate::testing::headless_config()).ok()?;
        let channel = app.mixer_snapshot().channels[0].uuid.clone();
        let uuid = match app.execute_command(C::AddSolidColorDeck {
            channel_uuid: channel,
            color: [1.0, 1.0, 1.0, 1.0],
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("expected a new deck, got {other:?}"),
        };
        Some((app, uuid))
    }

    fn opacity(app: &mut VardaApp) -> f32 {
        app.mixer_snapshot().channels[0].decks[0].opacity
    }

    /// Wire up an OSC receiver with nothing behind it and send one message.
    fn osc(app: &mut VardaApp, input: OscInput) {
        let (receiver, sender) = OscReceiver::detached();
        sender.send(input).expect("the receiver is right there");
        app.input.osc_receiver = Some(receiver);
    }

    /// Wire up a MIDI manager with nothing behind it and queue one message.
    fn midi(app: &mut VardaApp, msg: MidiMessage) {
        let mut devices = MidiDeviceManager::detached();
        devices.inject(msg);
        app.input.midi_devices = Some(devices);
    }

    const KNOB: MidiKey = MidiKey::CC(0, 0, 7);

    fn a_turn_of_the_knob(value: u8) -> MidiMessage {
        MidiMessage::ControlChange {
            device_id: 0,
            channel: 0,
            cc: 7,
            value,
        }
    }

    #[test]
    fn an_osc_write_moves_the_parameter_it_names() {
        let Some((mut app, deck)) = app_with_a_deck() else {
            return;
        };
        osc(
            &mut app,
            OscInput::Param {
                path: format!("deck/{deck}/opacity"),
                value: 0.25,
            },
        );

        app.process_inputs();

        assert!((opacity(&mut app) - 0.25).abs() < 1e-4);
    }

    /// A path that names nothing is logged and dropped: a stale mapping in
    /// someone's controller must not take the show down.
    #[test]
    fn a_write_to_a_path_that_names_nothing_is_survivable() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        osc(
            &mut app,
            OscInput::Param {
                path: "deck/no-such-deck/opacity".to_string(),
                value: 0.25,
            },
        );

        app.process_inputs();

        assert!((opacity(&mut app) - 1.0).abs() < 1e-4, "nothing moved");
    }

    /// Actions are edges, not states, so they arm on the press and ignore the
    /// release. The runner clears the flag when it dispatches.
    #[test]
    fn an_action_path_arms_on_the_press_and_ignores_the_release() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };

        osc(
            &mut app,
            OscInput::Param {
                path: "action/undo".to_string(),
                value: 0.0,
            },
        );
        app.process_inputs();
        assert!(!app.midi_pending_undo, "a release is not a press");

        osc(
            &mut app,
            OscInput::Param {
                path: "action/undo".to_string(),
                value: 1.0,
            },
        );
        app.process_inputs();
        assert!(app.midi_pending_undo);
    }

    #[test]
    fn a_mapped_control_moves_the_parameter_it_was_learned_to() {
        let Some((mut app, deck)) = app_with_a_deck() else {
            return;
        };
        app.input
            .midi_mappings
            .set(KNOB, format!("deck/{deck}/opacity"));
        midi(&mut app, a_turn_of_the_knob(0));

        app.process_inputs();

        assert!(opacity(&mut app).abs() < 1e-4, "the knob is at its bottom");
    }

    /// A controller sends far more than it is mapped for, and every unmapped
    /// message is a message the show has to ignore quietly.
    #[test]
    fn an_unmapped_control_moves_nothing() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        midi(&mut app, a_turn_of_the_knob(0));

        app.process_inputs();

        assert!((opacity(&mut app) - 1.0).abs() < 1e-4);
    }

    /// Clock messages carry no mapping key, so they must reach the clock and
    /// never be looked up as controls.
    #[test]
    fn clock_messages_are_not_mistaken_for_controls() {
        let Some((mut app, deck)) = app_with_a_deck() else {
            return;
        };
        app.input
            .midi_mappings
            .set(KNOB, format!("deck/{deck}/opacity"));
        midi(&mut app, MidiMessage::ClockStart { device_id: 0 });

        app.process_inputs();

        assert!((opacity(&mut app) - 1.0).abs() < 1e-4);
    }

    /// The pad on the desk and the pad in the UI are the same press: both locate
    /// the show to the cue and leave the transport as they found it.
    #[test]
    fn a_mapped_pad_fires_the_cue_it_names() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        let cue = match app.execute_command(C::AddCue {
            at: 12.0,
            name: "Verse".to_string(),
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("expected a new cue, got {other:?}"),
        };
        app.input.midi_mappings.set(KNOB, format!("cue/{cue}/fire"));
        midi(&mut app, a_turn_of_the_knob(127));

        app.process_inputs();

        assert!((app.transport.position() - 12.0).abs() < 1e-9);
        assert!(!app.transport.running(), "a cue locates, it does not start");
    }

    /// A hand on a controller is a hand on the show: a mapped write during an
    /// arranged run takes that parameter back from the arrangement, exactly as
    /// dragging the fader in the UI does.
    #[test]
    fn a_surface_write_takes_the_parameter_back_from_the_show() {
        let Some((mut app, deck)) = app_with_a_deck() else {
            return;
        };
        app.execute_command(C::AddRegion {
            deck_uuid: deck.clone(),
            region: crate::arrangement::RegionConfig {
                start: 10.0,
                end: 20.0,
                fade_in: 0.0,
                fade_out: 0.0,
            },
        });
        app.execute_command(C::TransportLocate { position: 15.0 });
        app.execute_command(C::TransportPlay);
        app.process_inputs();
        app.render_mixer_frame();

        osc(
            &mut app,
            OscInput::Param {
                path: format!("deck/{deck}/opacity"),
                value: 0.25,
            },
        );
        app.process_inputs();

        assert_eq!(
            app.build_engine_state()
                .arrangement
                .expect("arrangement")
                .overridden_params,
            vec![format!("deck_{deck}:opacity")]
        );
    }

    /// The same press against a cue that has since been deleted is ignored, the
    /// way a mapping to a deleted deck is.
    #[test]
    fn a_pad_pointing_at_a_deleted_cue_is_ignored() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        app.input
            .midi_mappings
            .set(KNOB, "cue/nosuch01/fire".to_string());
        midi(&mut app, a_turn_of_the_knob(127));

        app.process_inputs();

        assert!(app.transport.position().abs() < 1e-9);
    }

    #[test]
    fn a_cue_path_names_its_cue_on_the_rising_edge() {
        assert_eq!(
            fired_cue("cue/ab12cd34/fire", 1.0).as_deref(),
            Some("ab12cd34")
        );
        assert_eq!(
            fired_cue("cue/ab12cd34/fire", 0.5),
            None,
            "a release, or a fader below halfway, is not a press"
        );
    }

    #[test]
    fn other_paths_are_left_to_the_router() {
        for path in [
            "deck/ab12cd34/trigger",
            "cue/ab12cd34",
            "cue//fire",
            "cue/ab12cd34/extra/fire",
        ] {
            assert_eq!(fired_cue(path, 1.0), None, "{path}");
        }
    }
}
