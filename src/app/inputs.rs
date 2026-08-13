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

        // One frame time for every signal ingested below, so LTC and MTC are
        // aged against the same instant rather than against each other.
        let now = std::time::Instant::now();

        // Poll all audio sources
        self.audio_manager.poll();

        // Timecode arrives on two paths. This is the audio one; the MIDI one is
        // answered in the drain below.
        self.tick_ltc(now);

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
                    // Timecode, like clock, is an engine-internal signal rather
                    // than a control, so it never reaches the mapping store.
                    crate::midi::MidiMessage::MtcQuarterFrame { device_id, .. }
                    | crate::midi::MidiMessage::MtcFullFrame { device_id, .. } => {
                        let device_id = *device_id;
                        self.input.timecode.ingest_midi(&msg, now);
                        if let Some(name) = midi.device(device_id).map(|d| d.name.clone()) {
                            self.input.timecode.name_device(device_id, &name);
                        }
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

        // Resolve timecode and hand the transport its position. Before the tick
        // below, so a master's locate is published as this frame's jump rather
        // than next frame's.
        self.chase_timecode(now);

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

        self.publish_timecode();

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

    /// Republish the show position over OSC, once per frame of timecode.
    ///
    /// Rate-limited by the label rather than by the render loop: a receiver
    /// wants the position at the rate positions exist, and 60 fps of identical
    /// frame numbers is noise on someone else's network.
    fn publish_timecode(&mut self) {
        let Some(sender) = &self.input.osc_feedback else {
            return;
        };
        if !sender.has_targets() || !self.transport.has_run() {
            return;
        }
        let label = self.transport.formatted_position();
        if self.session.published_timecode.as_deref() == Some(label.as_str()) {
            return;
        }
        sender.send_timecode(self.transport.position(), &label);
        self.session.published_timecode = Some(label);
    }

    /// Keep the LTC tap matching what is patched, and decode what it heard.
    ///
    /// The subscription is reconciled from the patch each frame rather than on
    /// a command, so naming an input, changing it, switching the preference to
    /// `Off`, and loading a scene all release the device by the same path.
    /// See /spec/audio-capture-lifecycle.md.
    fn tick_ltc(&mut self, now: std::time::Instant) {
        let wanted = self
            .input
            .timecode
            .wants_ltc()
            .then(|| self.input.timecode.ltc_input())
            .flatten();

        if self.input.ltc_tap.as_ref().map(|tap| tap.source_id) != wanted.map(|i| i.source_id) {
            if let Some(tap) = self.input.ltc_tap.take() {
                self.audio_manager.unsubscribe_pcm(tap.source_id, tap.token);
            }
            if let Some(input) = wanted {
                if let Some(sub) = self.audio_manager.subscribe_pcm(input.source_id) {
                    log::info!(
                        "Listening for LTC on audio source {} channel {}",
                        input.source_id,
                        input.channel + 1
                    );
                    self.input.ltc_tap = Some(crate::app::LtcTap {
                        source_id: input.source_id,
                        token: sub.token,
                        receiver: sub.receiver,
                        sample_rate: sub.format.sample_rate,
                        channels: sub.format.channels,
                    });
                } else {
                    self.session.notifications.warn(format!(
                        "Could not open audio source {} to listen for timecode",
                        input.source_id
                    ));
                    // Forget the patch, or this retries every frame for the
                    // rest of the show.
                    self.input.timecode.set_ltc_input(None);
                }
            }
        }

        let Some(tap) = &self.input.ltc_tap else {
            return;
        };
        let (source_id, channels, sample_rate) = (tap.source_id, tap.channels, tap.sample_rate);
        let chunks: Vec<crate::audio::PcmChunk> =
            std::iter::from_fn(|| tap.receiver.try_recv().ok()).collect();
        for chunk in chunks {
            self.input
                .timecode
                .ingest_pcm(source_id, &chunk.samples, channels, sample_rate, now);
        }
    }

    /// Resolve the incoming timecode and give the transport its position.
    fn chase_timecode(&mut self, now: std::time::Instant) {
        self.input.timecode.update(now);
        if self.transport.source() != crate::transport::TransportSource::Timecode {
            return;
        }

        let state = self.input.timecode.state();
        self.transport.chase(crate::transport::Chase {
            position: state.position,
            running: state.running,
            discontinuity: state.discontinuity,
            freewheeling: state.freewheeling,
            speed: state.speed,
        });

        // Armed to chase and hearing nothing looks exactly like a show that has
        // not started. Headless has nobody watching the popover, so it is said
        // out loud, once per silence rather than every frame of it.
        if state.running {
            self.session.chase_silent_since = None;
            self.session.chase_silence_reported = false;
        } else {
            let since = *self.session.chase_silent_since.get_or_insert(now);
            if now.duration_since(since) > std::time::Duration::from_secs(5)
                && !self.session.chase_silence_reported
            {
                self.session.chase_silence_reported = true;
                log::warn!(
                    "Transport is chasing timecode but nothing has arrived for 5 seconds \
                     (preference {:?}, {} input(s) seen)",
                    self.input.timecode.preference(),
                    self.input.timecode.inputs().len()
                );
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

    // ── Chasing timecode ────────────────────────────────────────

    /// Queue every quarter frame for one address, in order.
    fn a_master_at(frame: crate::timecode::TimecodeFrame) -> Vec<MidiMessage> {
        let hours = frame.hours | (crate::timecode::mtc::rate_bits(frame.rate) << 5);
        [
            frame.frames & 0x0F,
            frame.frames >> 4,
            frame.seconds & 0x0F,
            frame.seconds >> 4,
            frame.minutes & 0x0F,
            frame.minutes >> 4,
            hours & 0x0F,
            hours >> 4,
        ]
        .iter()
        .enumerate()
        .map(|(piece, value)| MidiMessage::MtcQuarterFrame {
            device_id: 0,
            data: ((piece as u8) << 4) | value,
        })
        .collect()
    }

    fn send(app: &mut VardaApp, messages: Vec<MidiMessage>) {
        let mut devices = MidiDeviceManager::detached();
        for message in messages {
            devices.inject(message);
        }
        app.input.midi_devices = Some(devices);
        app.process_inputs();
    }

    /// The end of the whole feature: bytes off a MIDI port become the show's
    /// position, and that is what engages the arrangement.
    #[test]
    fn an_incoming_master_drives_the_show() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        app.execute_command(C::SetTransportSource {
            source: crate::transport::TransportSource::Timecode,
        });

        send(
            &mut app,
            a_master_at(crate::timecode::TimecodeFrame::new(
                1,
                0,
                30,
                0,
                crate::transport::TimecodeRate::Fps25,
            )),
        );

        assert!(
            (app.transport.position() - 3630.0).abs() < 0.2,
            "an hour and half a minute in, got {}",
            app.transport.position()
        );
        assert!(app.transport.running());
        assert!(
            app.transport.has_run(),
            "a chased show engages the arrangement like any other"
        );
    }

    /// A cable left patched from yesterday must not drag the playhead of a show
    /// being run by hand.
    #[test]
    fn a_master_is_ignored_until_the_transport_is_asked_to_follow_it() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };

        send(
            &mut app,
            a_master_at(crate::timecode::TimecodeFrame::new(
                1,
                0,
                0,
                0,
                crate::transport::TimecodeRate::Fps25,
            )),
        );

        assert!(app.transport.position().abs() < 1e-9);
        assert!(
            !app.input.timecode.inputs().is_empty(),
            "but it is still heard, so the popover can offer it"
        );
    }

    /// A master that stops leaves the show where it stopped rather than
    /// releasing it, so a pulled cable holds the last look.
    #[test]
    fn the_show_holds_where_a_master_left_it() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        app.execute_command(C::SetTransportSource {
            source: crate::transport::TransportSource::Timecode,
        });
        send(
            &mut app,
            a_master_at(crate::timecode::TimecodeFrame::new(
                0,
                10,
                0,
                0,
                crate::transport::TimecodeRate::Fps25,
            )),
        );
        let held = app.transport.position();

        // Nothing arrives for well past the freewheel window.
        app.input
            .timecode
            .update(std::time::Instant::now() + std::time::Duration::from_secs(2));
        app.process_inputs();

        assert!(!app.transport.running(), "the master stopped");
        assert!(
            (app.transport.position() - held).abs() < 0.5,
            "and the position held rather than snapping home"
        );
        assert_eq!(
            app.transport.status(),
            crate::transport::TransportStatus::Stopped
        );
    }

    // ── Listening for LTC on an audio input ─────────────────────

    /// Every capture device this machine enumerated.
    fn audio_inputs(app: &VardaApp) -> Vec<crate::audio::AudioSourceId> {
        app.audio_manager.devices().iter().map(|d| d.id).collect()
    }

    /// Patch LTC to `source_id` and let one frame reconcile the tap, returning
    /// the source it actually opened.
    ///
    /// `None` when the device could not be opened, which is the CI case: there
    /// is no mock capture device, so the tests below skip the way the
    /// GPU-backed ones do with no adapter.
    fn patch_ltc(
        app: &mut VardaApp,
        source_id: crate::audio::AudioSourceId,
        channel: u16,
    ) -> Option<crate::audio::AudioSourceId> {
        app.execute_command(C::SetLtcInput {
            input: Some(crate::timecode::LtcInput {
                source_id,
                channel,
                rate: None,
            }),
        });
        app.process_inputs();
        app.input.ltc_tap.as_ref().map(|tap| tap.source_id)
    }

    /// Naming an input is what opens the interface, and unpatching it is what
    /// gives it back. A rehearsal that is done with timecode must not hold a
    /// channel of somebody else's console open for the rest of the night.
    #[test]
    fn unpatching_ltc_gives_the_audio_interface_back() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        let Some(&source_id) = audio_inputs(&app).first() else {
            return;
        };
        let Some(tapped) = patch_ltc(&mut app, source_id, 1) else {
            return;
        };
        assert_eq!(tapped, source_id);
        assert!(
            app.audio_manager.active_source_ids().contains(&source_id),
            "the interface is captured while timecode is being listened for"
        );

        app.execute_command(C::SetLtcInput { input: None });
        app.process_inputs();

        assert!(app.input.ltc_tap.is_none());
        assert!(
            !app.audio_manager.active_source_ids().contains(&source_id),
            "and let go once nobody is listening"
        );
    }

    /// `Off` is the same release by another route: deciding not to follow
    /// timecode this evening leaves the patch written down but must not leave
    /// the device open.
    #[test]
    fn switching_timecode_off_releases_the_audio_interface() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        let Some(&source_id) = audio_inputs(&app).first() else {
            return;
        };
        if patch_ltc(&mut app, source_id, 0).is_none() {
            return;
        }

        app.execute_command(C::SetTimecodePreference {
            preference: crate::timecode::TimecodePreference::Off,
        });
        app.process_inputs();

        assert!(app.input.ltc_tap.is_none());
        assert!(!app.audio_manager.active_source_ids().contains(&source_id));
        assert!(
            app.input.timecode.ltc_input().is_some(),
            "the patch itself is remembered, so turning timecode back on needs no re-patching"
        );
    }

    /// Moving the patch to another interface moves the tap rather than adding a
    /// second one: two taps would decode two positions, and the interface the
    /// patch left would still be held open by a reader nobody is asking.
    #[test]
    fn repatching_ltc_moves_the_tap_rather_than_stacking_one() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        let [first, second] = match audio_inputs(&app).as_slice() {
            [first, second, ..] => [*first, *second],
            _ => return,
        };
        if patch_ltc(&mut app, first, 0).is_none() {
            return;
        }
        let Some(moved) = patch_ltc(&mut app, second, 0) else {
            return;
        };

        assert_eq!(moved, second);
        assert!(
            !app.audio_manager.active_source_ids().contains(&first),
            "the interface the patch left is released"
        );
    }

    /// Both channels of one interface arrive on the same tap, so changing which
    /// one carries timecode must not reopen the device. A reopen is a gap in the
    /// capture, and on the standard field rig the other channel is the music
    /// going to the PA.
    #[test]
    fn changing_channel_on_the_same_interface_keeps_the_stream_open() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        let Some(&source_id) = audio_inputs(&app).first() else {
            return;
        };
        if patch_ltc(&mut app, source_id, 0).is_none() {
            return;
        }
        let token = app.input.ltc_tap.as_ref().map(|tap| tap.token);

        if patch_ltc(&mut app, source_id, 1).is_none() {
            return;
        }

        assert_eq!(
            app.input.ltc_tap.as_ref().map(|tap| tap.token),
            token,
            "the same subscription carries the other channel"
        );
        assert_eq!(
            app.input.timecode.ltc_input().map(|input| input.channel),
            Some(1),
            "but the decoder is now reading the channel that was asked for"
        );
    }

    /// A patch pointing at an interface the rig no longer has is reported and
    /// then forgotten. Left in place it would try to open a device that is not
    /// there sixty times a second, and log it sixty times a second, for the rest
    /// of the show.
    #[test]
    fn a_patch_naming_an_absent_interface_is_reported_once_and_dropped() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        let absent = audio_inputs(&app).len() as crate::audio::AudioSourceId + 1000;
        app.execute_command(C::SetLtcInput {
            input: Some(crate::timecode::LtcInput {
                source_id: absent,
                channel: 0,
                rate: None,
            }),
        });

        app.process_inputs();

        assert!(app.input.ltc_tap.is_none());
        assert_eq!(
            app.input.timecode.ltc_input(),
            None,
            "the patch is dropped rather than retried every frame"
        );
        let complaints = |app: &VardaApp| {
            app.session
                .notifications
                .visible()
                .iter()
                .filter(|n| n.message.contains("listen for timecode"))
                .count()
        };
        assert_eq!(complaints(&app), 1);
        assert!(
            app.session
                .notifications
                .visible()
                .iter()
                .any(|n| n.message.contains(&absent.to_string())),
            "and it names the source that could not be opened"
        );

        app.process_inputs();
        app.process_inputs();

        assert_eq!(complaints(&app), 1, "said once, not once per frame");
    }

    // ── Republishing the position over OSC ──────────────────────

    /// A feedback sender aimed at a socket this test owns, so what went on the
    /// wire can be read back.
    fn osc_loopback() -> Option<(std::net::UdpSocket, crate::osc::OscFeedbackSender)> {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").ok()?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .ok()?;
        let mut sender = crate::osc::OscFeedbackSender::new().ok()?;
        sender
            .add_target(&socket.local_addr().ok()?.to_string())
            .ok()?;
        Some((socket, sender))
    }

    /// Every OSC address waiting on the socket.
    fn received_addresses(socket: &std::net::UdpSocket) -> Vec<String> {
        let mut addresses = Vec::new();
        let mut buf = [0u8; 1024];
        while let Ok((size, _)) = socket.recv_from(&mut buf) {
            if let Ok((_, rosc::OscPacket::Message(msg))) = rosc::decoder::decode_udp(&buf[..size])
            {
                addresses.push(msg.addr);
            }
        }
        addresses
    }

    /// An app publishing to a socket the test owns, with the show parked at a
    /// position that has already been published once.
    fn app_publishing_position() -> Option<(VardaApp, std::net::UdpSocket)> {
        let (mut app, _deck) = app_with_a_deck()?;
        let (socket, sender) = osc_loopback()?;
        app.input.osc_feedback = Some(sender);
        // Nothing is published until the show has actually moved, so play a
        // frame and then park it.
        app.execute_command(C::TransportPlay);
        app.process_inputs();
        app.execute_command(C::TransportStop);
        app.process_inputs();
        let _ = received_addresses(&socket);
        Some((app, socket))
    }

    /// A receiver wants the position at the rate positions exist. Sixty frames
    /// a second of the same frame number is traffic on somebody else's show
    /// network, where traffic is dropped packets.
    #[test]
    fn a_position_that_has_not_moved_is_not_republished() {
        let Some((mut app, socket)) = app_publishing_position() else {
            return;
        };

        app.process_inputs();

        assert!(
            received_addresses(&socket).is_empty(),
            "the show is parked, so there is nothing new to say"
        );
    }

    /// A position that moved is published again, or software following Varda
    /// would sit at the old position through a locate.
    #[test]
    fn a_position_that_moved_is_published_again() {
        let Some((mut app, socket)) = app_publishing_position() else {
            return;
        };

        app.execute_command(C::TransportLocate { position: 90.0 });
        app.process_inputs();

        let addresses = received_addresses(&socket);
        assert!(
            addresses.contains(&"/varda/timecode/position".to_string()),
            "got {addresses:?}"
        );
        assert!(
            addresses.contains(&"/varda/timecode/string".to_string()),
            "got {addresses:?}"
        );
        let label = app.transport.formatted_position();
        assert_eq!(
            app.session.published_timecode.as_deref(),
            Some(label.as_str())
        );
    }

    // ── Chasing nothing ─────────────────────────────────────────

    /// Armed to chase with nothing arriving looks exactly like a show that has
    /// not started yet. A headless rig has nobody watching the popover, so it
    /// has to be said out loud.
    #[test]
    fn chasing_silence_is_reported_after_a_few_seconds() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        app.execute_command(C::SetTransportSource {
            source: crate::transport::TransportSource::Timecode,
        });
        let start = std::time::Instant::now();

        app.chase_timecode(start);
        assert!(
            !app.session.chase_silence_reported,
            "a gap between cues is not a fault"
        );

        app.chase_timecode(start + std::time::Duration::from_secs(6));
        assert!(app.session.chase_silence_reported);

        app.chase_timecode(start + std::time::Duration::from_secs(7));
        assert!(
            app.session.chase_silence_reported,
            "still one silence, not a second complaint about it"
        );
    }

    /// And it arms again when the master comes back, so a second dropout later
    /// in the night is reported rather than swallowed by the first one.
    #[test]
    fn the_silence_warning_arms_again_when_a_master_returns() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        app.execute_command(C::SetTransportSource {
            source: crate::transport::TransportSource::Timecode,
        });
        let start = std::time::Instant::now();
        app.chase_timecode(start);
        app.chase_timecode(start + std::time::Duration::from_secs(6));
        assert!(app.session.chase_silence_reported);

        let returned = start + std::time::Duration::from_secs(7);
        app.input.timecode.ingest(
            crate::timecode::TimecodeSource::Ltc {
                source_id: 0,
                channel: 0,
            },
            crate::timecode::TimecodeFrame::at(10.0, crate::transport::TimecodeRate::Fps25),
            returned,
        );
        app.chase_timecode(returned);

        assert!(app.transport.running(), "the master is back");
        assert!(!app.session.chase_silence_reported);
        assert!(
            app.session.chase_silent_since.is_none(),
            "and the next silence is timed from when it starts, not from tonight's first one"
        );
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
