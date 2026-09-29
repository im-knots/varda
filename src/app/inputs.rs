//! Per-frame input processing before render: shader hot reload, audio, OSC,
//! MIDI, and timecode. Changed parameters are sent as OSC feedback.

use super::VardaApp;
use crate::engine::EngineCommand;
use crate::engine::value::param::ParamAddress;

/// Undo/redo/save requested by MIDI, OSC or a macro trigger, not yet dispatched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingGlobalActions {
    pub(crate) undo: bool,
    pub(crate) redo: bool,
    pub(crate) save: bool,
}

/// Bucket a normalized write into one of `n` choices.
fn bucket(value: f32, n: usize) -> usize {
    ((value.clamp(0.0, 1.0) * n as f32) as usize).min(n.saturating_sub(1))
}

impl VardaApp {
    /// The command for a write to a target outside the mixer (cue, output,
    /// surface). `None` for paths the parameter router handles. Presses fire
    /// on the rising edge, like `deck/<uuid>/trigger`.
    pub(crate) fn surface_command(&self, path: &str, value: f32) -> Option<EngineCommand> {
        use crate::engine::value::param::OutputControl;
        let pressed = value > 0.5;
        match path.parse::<ParamAddress>().ok()? {
            ParamAddress::CueFire { cue } => {
                pressed.then_some(EngineCommand::TriggerCue { uuid: cue })
            }
            ParamAddress::Output { output, target } => match target {
                OutputControl::Start => pressed.then_some(EngineCommand::StartOutput {
                    output_uuid: output,
                }),
                OutputControl::Stop => pressed.then_some(EngineCommand::StopOutput {
                    output_uuid: output,
                }),
                OutputControl::Active => Some(if pressed {
                    EngineCommand::StartOutput {
                        output_uuid: output,
                    }
                } else {
                    EngineCommand::StopOutput {
                        output_uuid: output,
                    }
                }),
                OutputControl::Calibration => {
                    use crate::renderer::context::CalibrationMode as M;
                    let modes = [M::Off, M::Projector, M::Surfaces];
                    Some(EngineCommand::SetCalibrationMode {
                        output_uuid: output,
                        mode: modes[bucket(value, modes.len())],
                    })
                }
                OutputControl::Rotation => {
                    let all = crate::renderer::context::OutputRotation::ALL;
                    Some(EngineCommand::SetOutputRotation {
                        output_uuid: output,
                        rotation: all[bucket(value, all.len())],
                    })
                }
                OutputControl::Surface(surface) => {
                    Some(EngineCommand::SetSurfaceAssignmentEnabled {
                        output_uuid: output,
                        surface_uuid: surface,
                        enabled: pressed,
                    })
                }
                OutputControl::Sink(route) => {
                    let name = self.sink_setting_for_route(&output, &route)?;
                    Some(EngineCommand::SetSinkParam {
                        output_uuid: output,
                        name,
                        value: crate::engine::value::provider::ControlValue::Float(value),
                    })
                }
            },
            // A surface source is text; a fader can't set it.
            _ => None,
        }
    }

    /// The command for a toggle of a target outside the mixer: a cue or output
    /// press, or flipping an output's delivery or surface assignment.
    pub(crate) fn surface_toggle(&self, path: &str) -> Option<EngineCommand> {
        use crate::engine::value::param::OutputControl;
        let address = path.parse::<ParamAddress>().ok()?;
        if let ParamAddress::Output { output, target } = &address {
            let current = self.output.outputs.iter().find(|o| o.uuid == *output)?;
            match target {
                OutputControl::Active => {
                    return self.surface_command(path, if current.active { 0.0 } else { 1.0 });
                }
                OutputControl::Surface(surface) => {
                    let shown = current
                        .surface_assignments
                        .iter()
                        .any(|a| a.surface_uuid == *surface && a.enabled);
                    return self.surface_command(path, if shown { 0.0 } else { 1.0 });
                }
                _ => {}
            }
        }
        self.surface_command(path, 1.0)
    }

    /// The name of the setting `route` names on output `output`'s sink.
    pub(crate) fn sink_setting_for_route(&self, output: &str, route: &str) -> Option<String> {
        self.output
            .outputs
            .iter()
            .find(|o| o.uuid == output)?
            .sink()
            .schema()
            .iter()
            .find(|spec| spec.route.as_deref() == Some(route))
            .map(|spec| spec.name.clone())
    }

    /// Take the pending control-surface actions for a windowed consumer.
    pub(crate) fn take_pending_global_actions(&mut self) -> PendingGlobalActions {
        std::mem::take(&mut self.input.pending_actions)
    }

    /// Run and clear the pending control-surface actions, for headless use.
    pub(crate) fn run_pending_global_actions(&mut self) {
        use crate::engine::CommandResult;
        let pending = self.take_pending_global_actions();
        for (requested, cmd) in [
            (pending.undo, EngineCommand::Undo),
            (pending.redo, EngineCommand::Redo),
            (pending.save, EngineCommand::SaveWorkspace),
        ] {
            if !requested {
                continue;
            }
            if let CommandResult::Err { message, .. } = self.execute_command(cmd) {
                log::info!("Control-surface action not applied: {message}");
            }
        }
    }

    /// One normalized write from MIDI or OSC. Checked in order: global action,
    /// target outside the mixer, parameter. Commands for targets outside the
    /// mixer fire on the rising edge and are deferred, since a cue jump
    /// mid-drain would reorder the queued writes.
    fn apply_surface_write(
        &mut self,
        path: &str,
        value: f32,
        deferred: &mut Vec<EngineCommand>,
        changed_params: &mut Vec<(String, f32)>,
    ) {
        if path.starts_with("action/") && value > 0.5 {
            // Global actions trigger on note-on or CC > 50%.
            match path {
                "action/undo" => self.input.pending_actions.undo = true,
                "action/redo" => self.input.pending_actions.redo = true,
                "action/save" => self.input.pending_actions.save = true,
                // Toggles, so one pad is enough.
                "action/record" => self.set_record_armed(!self.show.recorder.armed()),
                _ => log::debug!("Unknown action path: {path}"),
            }
        } else if path.parse::<ParamAddress>().is_ok_and(|a| {
            matches!(
                a,
                ParamAddress::CueFire { .. }
                    | ParamAddress::Output { .. }
                    | ParamAddress::SurfaceSource { .. }
            )
        }) {
            if let Some(cmd) = self.surface_command(path, value) {
                deferred.push(cmd);
            }
        } else {
            match crate::param_router::apply_param_by_path(&mut self.mixer, path, value) {
                Ok(()) => changed_params.push((path.to_string(), value)),
                Err(e) => log::warn!("Param route failed ({path}): {e}"),
            }
        }
    }

    /// Process all external inputs and send changed parameters as OSC feedback.
    pub fn process_inputs(&mut self) {
        // (path, value) pairs changed this frame.
        let mut changed_params: Vec<(String, f32)> = Vec::new();
        // Run after the loops below, which borrow the receivers.
        let mut deferred: Vec<EngineCommand> = Vec::new();
        let shader_events = self.sources.registry.poll_changes();
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
                    // Lift GPU quarantine, since the source changed.
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

        // Capture a device only while an AudioBand modulator references it.
        // Derived from mixer state each frame, so no handler tracks it.
        {
            let bands = self.mixer.modulation().audio_band_source_ids();
            let default = self.audio.manager.default_source_id();
            let needed = crate::audio::AudioManager::needed_from_bands(&bands, default);
            self.audio.manager.set_modulation_refs(&needed);
        }

        // One timestamp for LTC and MTC this frame.
        let now = std::time::Instant::now();

        self.audio.manager.poll();

        // LTC (audio). MTC is handled in the MIDI drain below.
        self.tick_ltc(now);

        // From the primary source.
        self.audio.textures.update(
            &self.render.context.queue,
            self.audio.manager.get_primary_data(),
        );

        // Drained first, since dispatch needs `&mut self`.
        let osc_inputs: Vec<crate::osc::OscInput> = self
            .input
            .osc_receiver
            .as_ref()
            .map(|osc| std::iter::from_fn(|| osc.try_recv()).collect())
            .unwrap_or_default();
        for input in osc_inputs {
            match input {
                crate::osc::OscInput::Param { ref path, value } => {
                    self.apply_surface_write(path, value, &mut deferred, &mut changed_params);
                }
                crate::osc::OscInput::Text { path, value } => {
                    deferred.push(EngineCommand::SetPathText { path, value });
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

        // Clock messages are handled while draining; mappable messages are
        // kept for the pass below, which needs `&mut self`.
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
                    // Timecode never reaches the mapping store.
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

            // Auto-mapped keys take precedence.
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

            // Learn mode maps the next input to the learn target.
            if self.input.midi_mappings.learn_mode {
                self.input.midi_mappings.process_learn(key);
            }

            if let Some(path) = self.input.midi_mappings.get(&key).cloned() {
                if path == "clock/bpm" {
                    // 0..1 to 20..300 BPM.
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
                    self.apply_surface_write(&path, value, &mut deferred, &mut changed_params);
                }
            } else if !self.input.midi_mappings.learn_mode {
                log::debug!("Unmapped MIDI: {key} value={value:.2}");
            }
        }

        // A deleted cue or deck is ignored.
        for cmd in deferred {
            if let crate::engine::CommandResult::Err { message, .. } = self.execute_command(cmd) {
                log::debug!("Control-surface command ignored: {message}");
            }
        }

        // Macro trigger actions join the MIDI `action/*` pending actions.
        for action in self.mixer.macros_mut().take_pending_actions() {
            match action {
                crate::macros::GlobalAction::Undo => self.input.pending_actions.undo = true,
                crate::macros::GlobalAction::Redo => self.input.pending_actions.redo = true,
                crate::macros::GlobalAction::Save => self.input.pending_actions.save = true,
            }
        }

        {
            let primary = self.audio.manager.get_primary_data();
            self.input
                .clock_manager
                .update_audio(primary.bpm, primary.beat_phase());
        }

        self.input.clock_manager.update();

        // Before the transport tick, so a master's locate lands this frame.
        self.chase_timecode(now);

        // After commands, so a locate this frame isn't overwritten.
        self.show.transport.update();

        // Control-surface writes override the arrangement, once per param per frame.
        for (path, value) in &changed_params {
            self.note_live_route_write(path, *value);
        }

        // After the writes and the transport tick, so loop wraps are already visible.
        self.tick_recorder();

        self.publish_timecode();

        if !changed_params.is_empty()
            && let Some(ref sender) = self.input.osc_feedback
            && sender.has_targets()
        {
            for (path, value) in &changed_params {
                sender.send_param(path, *value);
            }
        }
    }

    /// Publish the show position over OSC when the timecode frame changes.
    fn publish_timecode(&mut self) {
        let Some(sender) = &self.input.osc_feedback else {
            return;
        };
        if !sender.has_targets() || !self.show.transport.has_run() {
            return;
        }
        let label = self.show.transport.formatted_position();
        if self.show.published_timecode.as_deref() == Some(label.as_str()) {
            return;
        }
        sender.send_timecode(self.show.transport.position(), &label);
        self.show.published_timecode = Some(label);
    }

    /// Match the LTC tap to the patch and decode it. Reconciled each frame,
    /// so every patch change releases the device the same way.
    fn tick_ltc(&mut self, now: std::time::Instant) {
        let wanted = self
            .input
            .timecode
            .wants_ltc()
            .then(|| self.input.timecode.ltc_input())
            .flatten();

        if self.input.ltc_tap.as_ref().map(|tap| tap.source_id) != wanted.map(|i| i.source_id) {
            if let Some(tap) = self.input.ltc_tap.take() {
                self.audio.manager.unsubscribe_pcm(tap.source_id, tap.token);
            }
            if let Some(input) = wanted {
                if let Some(sub) = self.audio.manager.subscribe_pcm(input.source_id) {
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
                    // Forget the patch so it isn't retried every frame.
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
        if self.show.transport.source() != crate::transport::TransportSource::Timecode {
            return;
        }

        let state = self.input.timecode.state();
        self.show.transport.chase(crate::transport::Chase {
            position: state.position,
            running: state.running,
            discontinuity: state.discontinuity,
            freewheeling: state.freewheeling,
            speed: state.speed,
        });

        // Warn once per silence when chasing and nothing arrives.
        if state.running {
            self.show.chase_silent_since = None;
            self.show.chase_silence_reported = false;
        } else {
            let since = *self.show.chase_silent_since.get_or_insert(now);
            if now.duration_since(since) > std::time::Duration::from_secs(5)
                && !self.show.chase_silence_reported
            {
                self.show.chase_silence_reported = true;
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
    use super::{EngineCommand, VardaApp};
    use crate::engine::EngineCommand as C;
    use crate::midi::{MidiDeviceManager, MidiKey, MidiMessage};
    use crate::osc::{OscInput, OscReceiver};

    /// An app with one white deck in channel 0, and its UUID. `None` without a GPU.
    fn app_with_a_deck() -> Option<(VardaApp, String)> {
        let mut app = crate::testing::headless_app()?;
        let channel = crate::app::snapshot::build_mixer_snapshot(&app).channels[0]
            .uuid
            .clone();
        let uuid = match app.execute_command(C::AddDeck {
            channel_uuid: channel,
            source: crate::solid_color::SolidColor::config_for([1.0, 1.0, 1.0, 1.0]),
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("expected a new deck, got {other:?}"),
        };
        Some((app, uuid))
    }

    /// Modulation updates only in render, where analyzer values exist; input
    /// processing leaves it unchanged.
    #[test]
    fn input_processing_leaves_modulation_where_the_last_render_left_it() {
        let Some((mut app, _)) = app_with_a_deck() else {
            return;
        };
        app.execute_command(C::AddLfo {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 7.3,
        });
        app.begin_frame();
        app.render_frame();
        let rendered = app.mixer_ref().modulation().current_values()[0];
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.begin_frame();
        assert_eq!(app.mixer_ref().modulation().current_values()[0], rendered);
    }

    fn opacity(app: &mut VardaApp) -> f32 {
        crate::app::snapshot::build_mixer_snapshot(app).channels[0].decks[0].opacity
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

    /// A path that names nothing is logged and dropped.
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

    /// Actions fire on press and ignore release. The runner clears the flag.
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
        assert!(
            !app.take_pending_global_actions().undo,
            "a release is not a press"
        );

        osc(
            &mut app,
            OscInput::Param {
                path: "action/undo".to_string(),
                value: 1.0,
            },
        );
        app.process_inputs();
        assert!(app.take_pending_global_actions().undo);
        assert!(
            !app.take_pending_global_actions().undo,
            "taking an action consumes it"
        );
    }

    /// Headless runs actions through the HTTP API's command arm.
    #[test]
    fn a_controller_undo_runs_without_a_window() {
        let Some((mut app, deck)) = app_with_a_deck() else {
            return;
        };
        app.command_sender()
            .send((
                C::SetDeckOpacity {
                    deck_uuid: deck,
                    opacity: 0.25,
                },
                None,
            ))
            .expect("the receiver is in the app");
        app.process_commands();
        assert!((opacity(&mut app) - 0.25).abs() < 1e-4);

        osc(
            &mut app,
            OscInput::Param {
                path: "action/undo".to_string(),
                value: 1.0,
            },
        );
        app.process_inputs();
        app.run_pending_global_actions();

        assert!((opacity(&mut app) - 1.0).abs() < 1e-4, "the undo landed");
        assert!(!app.take_pending_global_actions().undo, "and was consumed");
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

    /// Unmapped messages are ignored quietly.
    #[test]
    fn an_unmapped_control_moves_nothing() {
        let Some((mut app, _deck)) = app_with_a_deck() else {
            return;
        };
        midi(&mut app, a_turn_of_the_knob(0));

        app.process_inputs();

        assert!((opacity(&mut app) - 1.0).abs() < 1e-4);
    }

    /// Clock messages reach the clock and are never looked up as controls.
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

    /// A mapped cue press locates to the cue without changing run state, like the UI.
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

        assert!((app.show.transport.position() - 12.0).abs() < 1e-9);
        assert!(
            !app.show.transport.running(),
            "a cue locates, it does not start"
        );
    }

    /// A mapped write during an arranged run overrides the arrangement, like the UI.
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
            vec![format!("deck/{deck}/opacity")]
        );
    }

    /// A press for a deleted cue is ignored.
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

        assert!(app.show.transport.position().abs() < 1e-9);
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

    /// MTC from a MIDI port sets the show position and engages the arrangement.
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
            (app.show.transport.position() - 3630.0).abs() < 0.2,
            "an hour and half a minute in, got {}",
            app.show.transport.position()
        );
        assert!(app.show.transport.running());
        assert!(
            app.show.transport.has_run(),
            "a chased show engages the arrangement like any other"
        );
    }

    /// Timecode is ignored when not chasing.
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

        assert!(app.show.transport.position().abs() < 1e-9);
        assert!(
            !app.input.timecode.inputs().is_empty(),
            "but it is still heard, so the popover can offer it"
        );
    }

    /// When the master stops, the show holds its last position.
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
        let held = app.show.transport.position();

        // Nothing arrives for well past the freewheel window.
        app.input
            .timecode
            .update(std::time::Instant::now() + std::time::Duration::from_secs(2));
        app.process_inputs();

        assert!(!app.show.transport.running(), "the master stopped");
        assert!(
            (app.show.transport.position() - held).abs() < 0.5,
            "and the position held rather than snapping home"
        );
        assert_eq!(
            app.show.transport.status(),
            crate::transport::TransportStatus::Stopped
        );
    }

    // ── Listening for LTC on an audio input ─────────────────────

    /// Every capture device this machine enumerated.
    fn audio_inputs(app: &VardaApp) -> Vec<crate::audio::AudioSourceId> {
        app.audio.manager.devices().iter().map(|d| d.id).collect()
    }

    /// Patch LTC to `source_id`, run one frame, and return the opened source.
    /// `None` if the device couldn't open (as in CI), and the tests skip.
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

    /// Patching an input opens the device; unpatching releases it.
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
            app.audio.manager.active_source_ids().contains(&source_id),
            "the interface is captured while timecode is being listened for"
        );

        app.execute_command(C::SetLtcInput { input: None });
        app.process_inputs();

        assert!(app.input.ltc_tap.is_none());
        assert!(
            !app.audio.manager.active_source_ids().contains(&source_id),
            "and let go once nobody is listening"
        );
    }

    /// `Off` keeps the patch but releases the device.
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
        assert!(!app.audio.manager.active_source_ids().contains(&source_id));
        assert!(
            app.input.timecode.ltc_input().is_some(),
            "the patch itself is remembered, so turning timecode back on needs no re-patching"
        );
    }

    /// Moving the patch to another interface moves the tap instead of adding one.
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
            !app.audio.manager.active_source_ids().contains(&first),
            "the interface the patch left is released"
        );
    }

    /// Changing the LTC channel on one interface doesn't reopen the device.
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

    /// A patch to a missing interface is reported once, then forgotten.
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

    /// A feedback sender aimed at a socket the test reads.
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

    /// An app publishing to a test socket, parked at an already-published position.
    fn app_publishing_position() -> Option<(VardaApp, std::net::UdpSocket)> {
        let (mut app, _deck) = app_with_a_deck()?;
        let (socket, sender) = osc_loopback()?;
        app.input.osc_feedback = Some(sender);
        // Nothing is published until the show moves.
        app.execute_command(C::TransportPlay);
        app.process_inputs();
        app.execute_command(C::TransportStop);
        app.process_inputs();
        let _ = received_addresses(&socket);
        Some((app, socket))
    }

    /// An unchanged position is not republished.
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

    /// A changed position is published again.
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
        let label = app.show.transport.formatted_position();
        assert_eq!(app.show.published_timecode.as_deref(), Some(label.as_str()));
    }

    // ── Chasing nothing ─────────────────────────────────────────

    /// Chasing with no timecode arriving logs a warning.
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
            !app.show.chase_silence_reported,
            "a gap between cues is not a fault"
        );

        app.chase_timecode(start + std::time::Duration::from_secs(6));
        assert!(app.show.chase_silence_reported);

        app.chase_timecode(start + std::time::Duration::from_secs(7));
        assert!(
            app.show.chase_silence_reported,
            "still one silence, not a second complaint about it"
        );
    }

    /// The warning re-arms when the master returns, so a second dropout is reported.
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
        assert!(app.show.chase_silence_reported);

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

        assert!(app.show.transport.running(), "the master is back");
        assert!(!app.show.chase_silence_reported);
        assert!(
            app.show.chase_silent_since.is_none(),
            "and the next silence is timed from when it starts, not from tonight's first one"
        );
    }

    #[test]
    fn targets_outside_the_mixer_become_commands() {
        let Some((app, _)) = app_with_a_deck() else {
            return;
        };
        assert!(matches!(
            app.surface_command("cue/ab12cd34/fire", 1.0),
            Some(EngineCommand::TriggerCue { uuid }) if uuid == "ab12cd34"
        ));
        assert!(
            app.surface_command("cue/ab12cd34/fire", 0.0).is_none(),
            "fires on the rising edge"
        );
    }

    #[test]
    fn other_paths_are_left_to_the_router() {
        let Some((app, _)) = app_with_a_deck() else {
            return;
        };
        for path in [
            "deck/ab12cd34/trigger",
            "deck/ab12cd34/transparent",
            "deck/ab12cd34/html/reload",
            "cue/ab12cd34",
            "cue//fire",
            "cue/ab12cd34/extra/fire",
        ] {
            assert!(app.surface_command(path, 1.0).is_none(), "{path}");
        }
    }

    /// Output paths map to commands, so controllers can start a recording or pick
    /// a calibration card.
    #[test]
    fn output_addresses_become_output_commands() {
        use crate::renderer::context::{CalibrationMode, OutputRotation};
        let Some((app, _)) = app_with_a_deck() else {
            return;
        };
        assert!(matches!(
            app.surface_command("output/o1/start", 1.0),
            Some(EngineCommand::StartOutput { output_uuid }) if output_uuid == "o1"
        ));
        assert!(app.surface_command("output/o1/start", 0.0).is_none());
        assert!(matches!(
            app.surface_command("output/o1/active", 0.0),
            Some(EngineCommand::StopOutput { .. })
        ));
        assert!(matches!(
            app.surface_command("output/o1/calibration", 0.5),
            Some(EngineCommand::SetCalibrationMode {
                mode: CalibrationMode::Projector,
                ..
            })
        ));
        assert!(matches!(
            app.surface_command("output/o1/rotation", 1.0),
            Some(EngineCommand::SetOutputRotation {
                rotation: OutputRotation::Deg270,
                ..
            })
        ));
        assert!(matches!(
            app.surface_command("output/o1/surface/s1", 1.0),
            Some(EngineCommand::SetSurfaceAssignmentEnabled { enabled: true, .. })
        ));
        // A sink setting the output does not have resolves to nothing.
        assert!(app.surface_command("output/o1/nope", 1.0).is_none());
    }
}
