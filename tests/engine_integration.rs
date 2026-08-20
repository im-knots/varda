//! Engine integration tests — multi-step command workflows through real headless `VardaApp`.

use varda::app::VardaApp;
use varda::engine::{
    BlendMode, CommandResult, DeckSnapshot, EffectTarget, EngineCommand, ErrorCode, SurfaceQueries,
};
use varda::modulation::LFOWaveform;
use varda::renderer::context::OutputSource;
use varda::surface::SurfacePath;
use varda::timebase::Timebase;

mod common;

fn headless_app() -> Option<VardaApp> {
    let gpu = common::headless_gpu()?;
    let config = varda::testing::headless_config();
    // Once a GPU exists, a construction failure is a bug, not a reason to skip.
    Some(VardaApp::new(gpu, &config).expect("VardaApp::new"))
}

/// Send a command with reply channel, process, and return result.
fn send_cmd(app: &mut VardaApp, cmd: EngineCommand) -> CommandResult {
    let tx = app.command_sender();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send((cmd, Some(reply_tx))).unwrap();
    app.process_commands();
    reply_rx.blocking_recv().unwrap()
}

/// Fire-and-forget command.
fn fire(app: &mut VardaApp, cmd: EngineCommand) {
    let tx = app.command_sender();
    tx.send((cmd, None)).unwrap();
    app.process_commands();
}

/// The UUID a creating command reports (see ui-engine-boundary.md WS1).
fn new_uuid(result: CommandResult) -> String {
    match result {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    }
}

/// UUID of the channel currently at `idx`.
fn channel_uuid(app: &mut VardaApp, idx: usize) -> String {
    app.build_engine_state().mixer.channels[idx].uuid.clone()
}

/// The deck with `uuid`, wherever it currently lives.
fn deck_snapshot(app: &mut VardaApp, uuid: &str) -> DeckSnapshot {
    app.build_engine_state()
        .mixer
        .channels
        .iter()
        .flat_map(|ch| ch.decks.iter())
        .find(|d| d.uuid == uuid)
        .cloned()
        .unwrap_or_else(|| panic!("no deck with UUID '{uuid}'"))
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn multi_step_add_deck_set_opacity_verify() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.42,
        },
    );
    assert!((deck_snapshot(&mut app, &deck).opacity - 0.42).abs() < 1e-4);
}

#[test]
fn add_deck_add_effect_verify_chain() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [0.0, 1.0, 0.0, 1.0],
        },
    ));
    let r = send_cmd(
        &mut app,
        EngineCommand::AddEffect {
            target: EffectTarget::Deck(deck),
            shader_name: "invert".to_string(),
        },
    );
    // If the shader exists the effect is added; otherwise the command must fail
    // gracefully rather than panic.
    assert!(matches!(
        r,
        CommandResult::OkWithId { .. } | CommandResult::Err { .. }
    ));
}

#[test]
fn add_lfo_assign_modulation_verify() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert!(!state.modulation.sources.is_empty());
    let lfo_id = state.modulation.sources[0].uuid.clone();
    let r = send_cmd(
        &mut app,
        EngineCommand::AssignModulation {
            target: "crossfader".to_string(),
            source_id: lfo_id,
            amount: 0.5,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert!(state.modulation.assignments.contains_key("crossfader"));
}

// ── Transport ───────────────────────────────────────────────────

/// Transport control travels the command path and lands in the snapshot both
/// the UI and the REST API read. See /spec/transport.md.
#[test]
fn transport_control_roundtrip() {
    use varda::transport::{LoopRegion, TransportSource};

    let Some(mut app) = headless_app() else {
        return;
    };

    let t = app.build_engine_state().transport;
    assert!(!t.has_run, "a fresh session has not run");
    assert_eq!(t.status_label, "Idle");
    assert_eq!(t.timecode, "00:00:00;00");

    fire(&mut app, EngineCommand::TransportPlay);
    assert!(app.build_engine_state().transport.running);

    fire(
        &mut app,
        EngineCommand::TransportLocate { position: 3600.0 },
    );
    let t = app.build_engine_state().transport;
    assert!((t.position - 3600.0).abs() < 1e-9);
    assert_eq!(t.timecode, "01:00:00;00");

    fire(&mut app, EngineCommand::TransportStop);
    assert!(!app.build_engine_state().transport.running);

    fire(
        &mut app,
        EngineCommand::SetTransportLoop {
            region: Some(LoopRegion {
                start: 10.0,
                end: 20.0,
            }),
        },
    );
    assert_eq!(
        app.build_engine_state().transport.loop_region,
        Some(LoopRegion {
            start: 10.0,
            end: 20.0
        })
    );

    fire(
        &mut app,
        EngineCommand::SetTransportSource {
            source: TransportSource::Timecode,
        },
    );
    let t = app.build_engine_state().transport;
    assert_eq!(t.source, TransportSource::Timecode);
    assert!(
        !t.running,
        "arming to chase must stop local playback rather than race the master"
    );
}

/// The API can send an inverted range; the engine has to reject it rather than
/// store a loop that can never wrap.
#[test]
fn transport_rejects_an_inverted_loop_region() {
    use varda::transport::LoopRegion;

    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::SetTransportLoop {
            region: Some(LoopRegion {
                start: 9.0,
                end: 2.0,
            }),
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::InvalidInput,
            ..
        }
    ));
    assert_eq!(app.build_engine_state().transport.loop_region, None);
}

/// Position is the master's while chasing, so a locate has to be refused with a
/// reason rather than silently ignored.
#[test]
fn transport_refuses_to_scrub_while_chasing_timecode() {
    use varda::transport::TransportSource;

    let Some(mut app) = headless_app() else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetTransportSource {
            source: TransportSource::Timecode,
        },
    );
    assert_eq!(
        app.build_engine_state().transport.status_label,
        "Waiting for signal",
        "armed but silent must not read as merely stopped"
    );

    for cmd in [
        EngineCommand::TransportPlay,
        EngineCommand::TransportLocate { position: 42.0 },
    ] {
        assert!(matches!(
            send_cmd(&mut app, cmd),
            CommandResult::Err {
                code: ErrorCode::InvalidInput,
                ..
            }
        ));
    }
    assert!(app.build_engine_state().transport.position.abs() < f64::EPSILON);
}

#[test]
fn timecode_rate_changes_how_the_position_reads() {
    use varda::transport::TimecodeRate;

    let Some(mut app) = headless_app() else {
        return;
    };
    fire(&mut app, EngineCommand::TransportLocate { position: 1.0 });

    fire(
        &mut app,
        EngineCommand::SetTimecodeRate {
            rate: TimecodeRate::Fps25,
        },
    );
    assert_eq!(app.build_engine_state().transport.timecode, "00:00:01:00");

    fire(
        &mut app,
        EngineCommand::SetTimecodeRate {
            rate: TimecodeRate::Fps24,
        },
    );
    assert_eq!(app.build_engine_state().transport.timecode, "00:00:01:00");
}

/// A source's timebase is set by command and visible in the engine snapshot,
/// which is the path both the UI and the REST API read. See /spec/timebase.md.
#[test]
fn set_modulation_timebase_roundtrip() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
        },
    );
    let uuid = app.build_engine_state().modulation.sources[0].uuid.clone();

    assert_eq!(
        app.build_engine_state().modulation.sources[0].timebase,
        Timebase::FreeRun,
        "a new source free-runs until told otherwise"
    );

    fire(
        &mut app,
        EngineCommand::UpdateModulationTimebase {
            uuid: uuid.clone(),
            timebase: Timebase::Beat,
        },
    );
    assert_eq!(
        app.build_engine_state().modulation.sources[0].timebase,
        Timebase::Beat
    );

    fire(
        &mut app,
        EngineCommand::UpdateModulationTimebase {
            uuid,
            timebase: Timebase::FreeRun,
        },
    );
    assert_eq!(
        app.build_engine_state().modulation.sources[0].timebase,
        Timebase::FreeRun
    );
}

/// "Add automation lane" creates the envelope, locks it to the transport, and
/// assigns it, all in one gesture. See /spec/automation.md.
#[test]
fn add_automation_lane_creates_an_assigned_transport_locked_envelope() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let target = "deck_abc:opacity".to_string();
    let uuid = match send_cmd(
        &mut app,
        EngineCommand::AddAutomationLane {
            target: target.clone(),
            timebase: Timebase::Transport,
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected the new envelope's uuid, got {other:?}"),
    };

    let state = app.build_engine_state();
    let entry = state
        .modulation
        .sources
        .iter()
        .find(|s| s.uuid == uuid)
        .expect("the envelope should exist");
    assert_eq!(entry.timebase, Timebase::Transport);
    assert!(
        state
            .modulation
            .assignments
            .get(&target)
            .is_some_and(|a| a.iter().any(|m| m.source_id == uuid)),
        "the lane should be assigned to its target as part of the same gesture"
    );
}

/// A pass played through the command bus.
///
/// Each write lands wherever the rolling transport has got to, which is what a
/// hand on a control does. Tests that need the pass to cover a named stretch of
/// show drive the transport directly instead; see the unit tests beside the
/// recorder.
fn record_a_pass(app: &mut VardaApp, deck: &str, values: &[f32]) {
    fire(app, EngineCommand::SetRecordArmed { armed: true });
    for opacity in values {
        step(app);
        fire(
            app,
            EngineCommand::SetDeckOpacity {
                deck_uuid: deck.to_string(),
                opacity: *opacity,
            },
        );
    }
    fire(app, EngineCommand::SetRecordArmed { armed: false });
}

/// Every breakpoint on the envelope assigned to `param_key`.
fn recorded_curve(app: &mut VardaApp, param_key: &str) -> Vec<(f64, f32)> {
    let state = app.build_engine_state();
    let source_id = state
        .modulation
        .assignments
        .get(param_key)
        .and_then(|a| a.first())
        .map_or_else(
            || panic!("nothing is assigned to '{param_key}'"),
            |m| m.source_id.clone(),
        );
    let entry = state
        .modulation
        .sources
        .iter()
        .find(|s| s.uuid == source_id)
        .expect("the assigned source");
    match &entry.source {
        varda::engine::types::ModulationSourceSnapshot::Envelope { breakpoints } => {
            breakpoints.iter().map(|b| (b.position, b.value)).collect()
        }
        _ => panic!("'{param_key}' should be driven by an envelope"),
    }
}

/// The point of the feature, end to end: play the show and keep what you
/// played. The lane is created on the spot, because a performer reaching for a
/// fader has not first gone to the timeline to make one.
#[test]
fn a_recorded_pass_becomes_a_curve_on_a_parameter_that_had_none() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));
    let key = varda::arrangement::opacity_param_key(&deck);

    record_a_pass(&mut app, &deck, &[0.2, 0.6, 1.0]);

    let curve = recorded_curve(&mut app, &key);
    assert!(
        curve.len() >= 2,
        "the pass should have left a curve, got {curve:?}"
    );
    assert!(
        curve.windows(2).all(|w| w[1].0 > w[0].0),
        "written in the order it was played: {curve:?}"
    );
    assert!(
        (curve[curve.len() - 1].1 - 1.0).abs() < 1e-4,
        "and ends where the hand left it: {curve:?}"
    );

    // The whole pass is one undo entry, not one per point: undo means "that
    // take was no good", and a performer should not have to press it fifty
    // times to be rid of one.
    fire(&mut app, EngineCommand::Undo);
    assert!(
        !app.build_engine_state()
            .modulation
            .assignments
            .contains_key(&key),
        "one undo should take the whole take back"
    );
}

/// A take that outlived the pass would leave the performer's hand holding the
/// parameter against the curve they just recorded.
#[test]
fn the_parameter_goes_back_to_its_curve_when_the_pass_ends() {
    let Some((mut app, deck)) = app_with_one_region(0.0, 30.0) else {
        return;
    };
    record_a_pass(&mut app, &deck, &[0.2, 0.6]);

    assert!(
        app.build_engine_state()
            .arrangement
            .expect("arrangement")
            .overridden_params
            .is_empty(),
        "nothing should still be held once the pass is over"
    );
}

/// Arming is not recording: a still playhead would put every point of a pass at
/// one position, which is not a curve.
#[test]
fn nothing_is_recorded_while_the_transport_is_not_running() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));
    fire(&mut app, EngineCommand::SetRecordArmed { armed: true });
    fire(&mut app, EngineCommand::TransportStop);
    step(&mut app);
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.4,
        },
    );
    fire(&mut app, EngineCommand::SetRecordArmed { armed: false });

    assert!(
        !app.build_engine_state()
            .modulation
            .assignments
            .contains_key(&varda::arrangement::opacity_param_key(&deck)),
        "a fader turn against a stopped transport is just a fader turn"
    );
}

/// Arming and then reaching for play is two gestures for one intent, so the
/// button does both.
#[test]
fn arming_from_a_stop_rolls_the_transport() {
    let Some(mut app) = headless_app() else {
        return;
    };
    assert!(!app.build_engine_state().transport.running);

    fire(&mut app, EngineCommand::SetRecordArmed { armed: true });

    let transport = app.build_engine_state().transport;
    assert!(transport.running, "the pass starts where the press was");
    assert!(transport.record_armed);
}

/// A channel fader is among the most-played controls in a show, so a curve on
/// it has to reach the composite. The stored fader position is left alone, the
/// way every modulated parameter but a deck's opacity works.
#[test]
fn a_curve_on_a_channel_fader_drives_the_composite() {
    use varda::modulation::Breakpoint;

    // Through a scene with a region in it, because taking a parameter back is
    // gated on the arrangement being engaged at all.
    let Some((mut app, _deck)) = app_with_one_region(0.0, 30.0) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let target = varda::arrangement::channel_opacity_param_key(&ch);
    let envelope = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddAutomationLane {
            target: target.clone(),
            timebase: Timebase::Transport,
        },
    ));
    fire(
        &mut app,
        EngineCommand::SetEnvelopeBreakpoints {
            uuid: envelope,
            breakpoints: vec![Breakpoint::new(0.0, 0.25), Breakpoint::new(10.0, 0.25)],
        },
    );

    run_from(&mut app, 5.0);

    assert!(
        (app.mixer_ref().channel_opacity(0) - 0.25).abs() < 1e-4,
        "the curve should set the fader the frame reads, got {}",
        app.mixer_ref().channel_opacity(0)
    );
    assert!(
        (app.mixer_ref().channels()[0].opacity - 1.0).abs() < 1e-4,
        "and should not have overwritten the position the performer left"
    );

    // A hand on the fader wins, exactly as it does on a deck's.
    fire(
        &mut app,
        EngineCommand::SetChannelOpacity {
            channel_uuid: ch,
            opacity: 0.8,
        },
    );
    step(&mut app);

    assert!(
        (app.mixer_ref().channel_opacity(0) - 0.8).abs() < 1e-4,
        "the performer's value must win while the override is held"
    );
    assert!(
        app.build_engine_state()
            .arrangement
            .expect("arrangement")
            .overridden_params
            .contains(&target),
        "and the held fader should be reported so the UI can offer a re-arm"
    );
}

/// A key that can never resolve again would be persisted and reloaded as dead
/// weight, so the fader's curves leave with the channel.
#[test]
fn deleting_a_channel_takes_its_fader_curve_with_it() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(&mut app, EngineCommand::AddChannel);
    let ch = channel_uuid(&mut app, 2);
    let target = varda::arrangement::channel_opacity_param_key(&ch);
    send_cmd(
        &mut app,
        EngineCommand::AddAutomationLane {
            target: target.clone(),
            timebase: Timebase::Transport,
        },
    );
    assert!(app
        .build_engine_state()
        .modulation
        .assignments
        .contains_key(&target));

    fire(
        &mut app,
        EngineCommand::RemoveChannel {
            channel_uuid: ch.clone(),
        },
    );

    assert!(
        !app.build_engine_state()
            .modulation
            .assignments
            .contains_key(&target),
        "the fader's assignments should have gone with the channel"
    );
}

/// Breakpoints are stored sorted regardless of the order they arrive in, so an
/// API caller does not have to maintain the invariant.
#[test]
fn envelope_breakpoints_are_sorted_on_write() {
    use varda::modulation::Breakpoint;

    let Some(mut app) = headless_app() else {
        return;
    };
    let uuid = match send_cmd(
        &mut app,
        EngineCommand::AddAutomationLane {
            target: "deck_abc:opacity".into(),
            timebase: Timebase::Transport,
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    };

    fire(
        &mut app,
        EngineCommand::SetEnvelopeBreakpoints {
            uuid: uuid.clone(),
            breakpoints: vec![
                Breakpoint::new(9.0, 1.0),
                Breakpoint::new(1.0, 0.0),
                Breakpoint::new(5.0, 0.5),
            ],
        },
    );

    let state = app.build_engine_state();
    let entry = state
        .modulation
        .sources
        .iter()
        .find(|s| s.uuid == uuid)
        .unwrap();
    let varda::engine::types::ModulationSourceSnapshot::Envelope { breakpoints } = &entry.source
    else {
        panic!("expected an envelope");
    };
    let positions: Vec<f64> = breakpoints.iter().map(|b| b.position).collect();
    assert_eq!(positions, vec![1.0, 5.0, 9.0]);
}

#[test]
fn setting_breakpoints_on_a_non_envelope_reports_not_found() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
        },
    );
    let lfo = app.build_engine_state().modulation.sources[0].uuid.clone();

    let r = send_cmd(
        &mut app,
        EngineCommand::SetEnvelopeBreakpoints {
            uuid: lfo,
            breakpoints: vec![],
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
}

#[test]
fn set_modulation_timebase_on_unknown_source_reports_not_found() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::UpdateModulationTimebase {
            uuid: "does-not-exist".into(),
            timebase: Timebase::Beat,
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
}

#[test]
fn modulation_values_change_over_frames() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 10.0,
        },
    );
    let state0 = app.build_engine_state();
    let uuid = &state0.modulation.sources[0].uuid;
    let v0 = state0
        .modulation
        .current_values
        .get(uuid)
        .copied()
        .unwrap_or(0.0);
    for _ in 0..30 {
        app.update_frame_timing();
        app.render_mixer_frame();
    }
    let state1 = app.build_engine_state();
    let v1 = state1
        .modulation
        .current_values
        .get(uuid)
        .copied()
        .unwrap_or(0.0);
    // At 10 Hz over 30 frames (~0.5 s at 60fps), the LFO advances through
    // multiple cycles. The current value must stay a finite, unipolar value.
    assert!(
        v1.is_finite() && (0.0..=1.0).contains(&v1),
        "LFO value out of range: v0={v0}, v1={v1}"
    );
}

#[test]
fn macro_value_modulation_drives_targets_live() {
    use varda::macros::MacroKind;
    let Some(mut app) = headless_app() else {
        return;
    };
    // Deck on channel 0 to receive the modulated macro value via its opacity.
    let ch = channel_uuid(&mut app, 0);
    let deck_uuid = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));

    // Knob macro at base 0.5 driving that deck's opacity.
    send_cmd(
        &mut app,
        EngineCommand::AddMacro {
            kind: MacroKind::Knob,
        },
    );
    let macro_uuid = app.build_engine_state().macros[0].uuid.clone();
    fire(
        &mut app,
        EngineCommand::SetMacroValue {
            uuid: macro_uuid.clone(),
            value: 0.5,
        },
    );
    fire(
        &mut app,
        EngineCommand::AddMacroTarget {
            uuid: macro_uuid.clone(),
            path: format!("deck/{deck_uuid}/opacity"),
        },
    );

    // LFO assigned to the macro's *value* key (the exact path the UI uses).
    send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 10.0,
        },
    );
    let lfo_id = app.build_engine_state().modulation.sources[0].uuid.clone();
    let r = send_cmd(
        &mut app,
        EngineCommand::AssignModulation {
            target: format!("macro_{macro_uuid}:value"),
            source_id: lfo_id,
            amount: 1.0,
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");

    // Step frames and observe the deck opacity swing as the LFO drives the macro.
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for _ in 0..60 {
        app.update_frame_timing();
        app.render_mixer_frame();
        let op = deck_snapshot(&mut app, &deck_uuid).opacity;
        min = min.min(op);
        max = max.max(op);
    }
    assert!(
        max - min > 0.05,
        "deck opacity should oscillate from macro-value modulation: min={min} max={max}"
    );
}

#[test]
fn add_multiple_channels_verify_order() {
    let Some(mut app) = headless_app() else {
        return;
    };
    for _ in 0..3 {
        fire(&mut app, EngineCommand::AddChannel);
    }
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 5); // 2 default + 3 added
    for (i, ch) in state.mixer.channels.iter().enumerate() {
        assert_eq!(ch.idx, i);
    }
}

#[test]
fn remove_middle_channel_state_consistent() {
    let Some(mut app) = headless_app() else {
        return;
    };
    fire(&mut app, EngineCommand::AddChannel); // now 3
    let middle = channel_uuid(&mut app, 1);
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveChannel {
            channel_uuid: middle.clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 2);
    assert!(state.mixer.channels.iter().all(|c| c.uuid != middle));
}

#[test]
fn deck_solo_mute_interactions() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let first = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    let second = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [0.0, 0.0, 1.0, 1.0],
        },
    ));
    fire(
        &mut app,
        EngineCommand::SetDeckMute {
            deck_uuid: first.clone(),
            mute: true,
        },
    );
    assert!(deck_snapshot(&mut app, &first).mute);
    // Note: effective_opacity reflects transition phase, not mute state.
    // Mute is applied at render time by skipping the deck entirely.
    fire(
        &mut app,
        EngineCommand::SetDeckSolo {
            deck_uuid: second.clone(),
            solo: true,
        },
    );
    assert!(deck_snapshot(&mut app, &second).solo);
    assert!(!deck_snapshot(&mut app, &first).solo);
}

#[test]
fn crossfader_clamping() {
    let Some(mut app) = headless_app() else {
        return;
    };
    fire(&mut app, EngineCommand::SetCrossfader(5.0));
    let state = app.build_engine_state();
    assert!(state.mixer.crossfader <= 1.0);
    fire(&mut app, EngineCommand::SetCrossfader(-3.0));
    let state = app.build_engine_state();
    assert!(state.mixer.crossfader >= 0.0);
}

#[test]
fn blend_mode_roundtrip() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));
    fire(
        &mut app,
        EngineCommand::SetDeckBlendMode {
            deck_uuid: deck.clone(),
            mode: BlendMode::Add,
        },
    );
    assert_eq!(deck_snapshot(&mut app, &deck).blend_mode, BlendMode::Add);
    fire(
        &mut app,
        EngineCommand::SetDeckBlendMode {
            deck_uuid: deck.clone(),
            mode: BlendMode::Multiply,
        },
    );
    assert_eq!(
        deck_snapshot(&mut app, &deck).blend_mode,
        BlendMode::Multiply
    );
}

#[test]
fn render_frames_after_mutations() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    fire(&mut app, EngineCommand::SetCrossfader(0.5));
    for _ in 0..10 {
        app.update_frame_timing();
        app.render_mixer_frame();
    }
    let state = app.build_engine_state();
    assert!(state.fps >= 0.0);
}

#[test]
fn many_mutations_state_consistency() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Rapid-fire 50 commands
    for i in 0..50 {
        let pos = (i as f32) / 50.0;
        fire(&mut app, EngineCommand::SetCrossfader(pos));
    }
    let state = app.build_engine_state();
    // Last command was SetCrossfader(49/50 = 0.98)
    assert!((state.mixer.crossfader - 0.98).abs() < 0.02);
    assert_eq!(state.mixer.channels.len(), 2);
}

#[test]
fn command_reply_correctness() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Valid command — creation reports the new channel's UUID.
    let r = send_cmd(&mut app, EngineCommand::AddChannel);
    assert!(matches!(r, CommandResult::OkWithId { .. }), "{r:?}");
    // Invalid: a channel UUID that names nothing.
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveChannel {
            channel_uuid: "does-not-exist".into(),
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
}

#[test]
fn add_step_sequencer_modulation() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::AddStepSequencer {
            num_steps: 8,
            rate: 2.0,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert!(!state.modulation.sources.is_empty());
    let src = &state.modulation.sources.last().unwrap().source;
    assert!(matches!(
        src,
        varda::engine::types::ModulationSourceSnapshot::StepSequencer { .. }
    ));
}

#[test]
fn undo_redo_crossfader_value() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // History push only happens in the UI runner, not via execute_command.
    // Undo/Redo on an empty history should return Err, not crash.
    let r = send_cmd(&mut app, EngineCommand::Undo);
    assert!(
        matches!(r, CommandResult::Err { .. }),
        "Undo on empty history should error"
    );
    let r = send_cmd(&mut app, EngineCommand::Redo);
    assert!(
        matches!(r, CommandResult::Err { .. }),
        "Redo on empty history should error"
    );
    // SetCrossfader still works independently
    fire(&mut app, EngineCommand::SetCrossfader(0.5));
    let state = app.build_engine_state();
    assert!((state.mixer.crossfader - 0.5).abs() < 1e-4);
}

#[test]
fn set_render_resolution_and_verify() {
    let Some(mut app) = headless_app() else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 1280,
            height: 720,
        },
    );
    assert_eq!(app.render_width(), 1280);
    assert_eq!(app.render_height(), 720);
    // Render a frame to verify no crash at new resolution
    app.update_frame_timing();
    app.render_mixer_frame();
}

#[test]
fn set_domemaster_resolution_rebuilds_the_renderer() {
    use varda::renderer::dome::DomemasterResolution;

    let Some(mut app) = headless_app() else {
        return;
    };
    // Nothing built yet: the setting still takes, so a stage restored before any
    // dome surface exists comes up at the right size.
    fire(
        &mut app,
        EngineCommand::SetDomemasterResolution {
            resolution: DomemasterResolution::R1K,
        },
    );
    assert_eq!(app.domemaster_resolution(), DomemasterResolution::R1K);

    app.ensure_domemaster();
    assert_eq!(
        app.domemaster_output_size(),
        Some(1024),
        "the renderer must be built at the configured size, not the default"
    );

    // Changing it with a renderer live rebuilds in place rather than being
    // silently ignored until the next restart.
    fire(
        &mut app,
        EngineCommand::SetDomemasterResolution {
            resolution: DomemasterResolution::R4K,
        },
    );
    assert_eq!(app.domemaster_resolution(), DomemasterResolution::R4K);
    assert_eq!(app.domemaster_output_size(), Some(4096));

    app.update_frame_timing();
    app.render_mixer_frame();
}

#[test]
fn publish_state_reflects_mutations() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let reader = app.state_reader();
    let ch = channel_uuid(&mut app, 0);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    app.publish_state();
    let guard = reader.read().unwrap();
    let state = guard.as_ref().expect("state published");
    assert!(!state.mixer.channels[0].decks.is_empty());
}

#[test]
fn effect_toggle_and_remove() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    let effect = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddEffect {
            target: EffectTarget::Deck(deck.clone()),
            shader_name: "invert".into(),
        },
    ));

    let r = send_cmd(
        &mut app,
        EngineCommand::ToggleEffect {
            effect_uuid: effect.clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let toggled = deck_snapshot(&mut app, &deck)
        .effects
        .iter()
        .find(|e| e.uuid == effect)
        .expect("effect in deck chain")
        .enabled;
    assert!(!toggled);

    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveEffect {
            effect_uuid: effect,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    assert!(deck_snapshot(&mut app, &deck).effects.is_empty());
}

#[test]
fn move_deck_between_channels() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&mut app, 0);
    let ch1 = channel_uuid(&mut app, 1);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    let before_ch0 = app.build_engine_state().mixer.channels[0].decks.len();
    let before_ch1 = app.build_engine_state().mixer.channels[1].decks.len();
    let r = send_cmd(
        &mut app,
        EngineCommand::MoveDeck {
            deck_uuid: deck.clone(),
            dst_channel_uuid: ch1,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels[0].decks.len(), before_ch0 - 1);
    assert_eq!(state.mixer.channels[1].decks.len(), before_ch1 + 1);
    assert!(state.mixer.channels[1].decks.iter().any(|d| d.uuid == deck));
}

#[test]
fn reorder_deck_via_command() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let first = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    let second = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [0.0, 1.0, 0.0, 1.0],
        },
    ));
    let r = send_cmd(
        &mut app,
        EngineCommand::ReorderDeck {
            channel_uuid: ch,
            from_idx: 0,
            to_idx: 1,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let decks = app.build_engine_state().mixer.channels[0].decks.clone();
    assert_eq!(decks.len(), 2);
    assert_eq!(decks[0].uuid, second);
    assert_eq!(decks[1].uuid, first);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Chaos Tests Round 3: GPU Headless — adversarial engine commands
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ── G: Adversarial scene values ──────────────────────────────────────

#[test]
fn chaos_unknown_channel_uuid_does_not_panic() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Add deck to a channel that does not exist
    let r = send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: "no-such-channel".into(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    assert!(
        matches!(
            r,
            CommandResult::Err {
                code: ErrorCode::NotFound,
                ..
            }
        ),
        "unknown channel should error gracefully: {r:?}"
    );
    // Remove a deck that does not exist
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveDeck {
            deck_uuid: "no-such-deck".into(),
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
    // Set opacity on entities that do not exist
    fire(
        &mut app,
        EngineCommand::SetChannelOpacity {
            channel_uuid: "no-such-channel".into(),
            opacity: 0.5,
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: "no-such-deck".into(),
            opacity: 0.5,
        },
    );
    // Render should still work
    app.update_frame_timing();
    app.render_mixer_frame();
}

#[test]
fn chaos_unknown_deck_uuid_errors_not_found() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&mut app, 0);
    let ch1 = channel_uuid(&mut app, 1);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    let is_not_found = |r: &CommandResult| {
        matches!(
            r,
            CommandResult::Err {
                code: ErrorCode::NotFound,
                ..
            }
        )
    };
    // Remove a deck nobody owns
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveDeck {
            deck_uuid: "no-such-deck".into(),
        },
    );
    assert!(is_not_found(&r), "{r:?}");
    // Move a deck nobody owns
    let r = send_cmd(
        &mut app,
        EngineCommand::MoveDeck {
            deck_uuid: "no-such-deck".into(),
            dst_channel_uuid: ch1,
        },
    );
    assert!(is_not_found(&r), "{r:?}");
    // Move a live deck to a channel nobody owns
    let r = send_cmd(
        &mut app,
        EngineCommand::MoveDeck {
            deck_uuid: deck.clone(),
            dst_channel_uuid: "no-such-channel".into(),
        },
    );
    assert!(is_not_found(&r), "{r:?}");
    // Deck should still be present (none of the above should have touched it)
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels[0].decks.len(), 1);
    assert_eq!(state.mixer.channels[0].decks[0].uuid, deck);
    app.update_frame_timing();
    app.render_mixer_frame();
}

#[test]
fn chaos_nan_crossfader_via_command() {
    let Some(mut app) = headless_app() else {
        return;
    };
    fire(&mut app, EngineCommand::SetCrossfader(f32::NAN));
    // NaN propagates but must not crash the render
    app.update_frame_timing();
    app.render_mixer_frame();
    fire(&mut app, EngineCommand::SetCrossfader(f32::INFINITY));
    app.render_mixer_frame();
    fire(&mut app, EngineCommand::SetCrossfader(f32::NEG_INFINITY));
    app.render_mixer_frame();
    // Restore sane value
    fire(&mut app, EngineCommand::SetCrossfader(0.5));
    app.render_mixer_frame();
}

#[test]
fn chaos_nan_opacity_via_command() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck,
            opacity: f32::NAN,
        },
    );
    app.update_frame_timing();
    app.render_mixer_frame();
    fire(
        &mut app,
        EngineCommand::SetChannelOpacity {
            channel_uuid: ch.clone(),
            opacity: f32::INFINITY,
        },
    );
    app.render_mixer_frame();
    fire(
        &mut app,
        EngineCommand::SetChannelOpacity {
            channel_uuid: ch,
            opacity: f32::NEG_INFINITY,
        },
    );
    app.render_mixer_frame();
}

#[test]
fn chaos_extreme_render_resolution() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Tiny resolution
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 1,
            height: 1,
        },
    );
    app.update_frame_timing();
    app.render_mixer_frame();
    // Asymmetric ultra-wide
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 4096,
            height: 1,
        },
    );
    app.render_mixer_frame();
    // Restore normal
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 1920,
            height: 1080,
        },
    );
    app.render_mixer_frame();
}

#[test]
fn chaos_zero_render_resolution() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Zero dimensions — should be clamped or rejected, not crash
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 0,
            height: 0,
        },
    );
    app.update_frame_timing();
    app.render_mixer_frame();
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 0,
            height: 1080,
        },
    );
    app.render_mixer_frame();
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 1920,
            height: 0,
        },
    );
    app.render_mixer_frame();
}

// ── H: Deck lifecycle churn ──────────────────────────────────────────

#[test]
fn chaos_rapid_deck_add_remove_cycle() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let mut decks = Vec::new();
    for i in 0..20 {
        let color = [(i as f32) / 20.0, 0.0, 0.0, 1.0];
        decks.push(new_uuid(send_cmd(
            &mut app,
            EngineCommand::AddSolidColorDeck {
                channel_uuid: ch.clone(),
                color,
            },
        )));
    }
    assert_eq!(app.build_engine_state().mixer.channels[0].decks.len(), 20);
    // Remove all in reverse order
    for uuid in decks.iter().rev() {
        let r = send_cmd(
            &mut app,
            EngineCommand::RemoveDeck {
                deck_uuid: uuid.clone(),
            },
        );
        assert!(
            matches!(r, CommandResult::Ok),
            "Remove deck {uuid} failed: {r:?}"
        );
    }
    assert_eq!(app.build_engine_state().mixer.channels[0].decks.len(), 0);
    // Render with empty channel
    app.update_frame_timing();
    app.render_mixer_frame();
}

#[test]
fn chaos_rapid_channel_add_remove_cycle() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let initial = app.build_engine_state().mixer.channels.len();
    // Add 10 channels
    let mut added = Vec::new();
    for _ in 0..10 {
        added.push(new_uuid(send_cmd(&mut app, EngineCommand::AddChannel)));
    }
    assert_eq!(app.build_engine_state().mixer.channels.len(), initial + 10);
    // Render with many channels
    app.update_frame_timing();
    app.render_mixer_frame();
    // Remove the channels we added
    for uuid in added.iter().rev() {
        let r = send_cmd(
            &mut app,
            EngineCommand::RemoveChannel {
                channel_uuid: uuid.clone(),
            },
        );
        assert!(
            matches!(r, CommandResult::Ok),
            "Remove channel {uuid} failed: {r:?}"
        );
    }
    assert_eq!(app.build_engine_state().mixer.channels.len(), initial);
    app.render_mixer_frame();
}

#[test]
fn chaos_interleaved_add_remove_render() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    // Add deck, render, remove, render — 10 cycles
    for _ in 0..10 {
        let deck = new_uuid(send_cmd(
            &mut app,
            EngineCommand::AddSolidColorDeck {
                channel_uuid: ch.clone(),
                color: [1.0, 1.0, 1.0, 1.0],
            },
        ));
        app.update_frame_timing();
        app.render_mixer_frame();
        send_cmd(&mut app, EngineCommand::RemoveDeck { deck_uuid: deck });
        app.render_mixer_frame();
    }
    assert_eq!(app.build_engine_state().mixer.channels[0].decks.len(), 0);
}

#[test]
fn chaos_remove_last_channels_rejected() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Mixer enforces minimum 2 channels — add extras then remove down to 2
    send_cmd(&mut app, EngineCommand::AddChannel);
    send_cmd(&mut app, EngineCommand::AddChannel);
    assert_eq!(app.build_engine_state().mixer.channels.len(), 4);
    // Remove extras
    while app.build_engine_state().mixer.channels.len() > 2 {
        let last = app
            .build_engine_state()
            .mixer
            .channels
            .last()
            .unwrap()
            .uuid
            .clone();
        let r = send_cmd(
            &mut app,
            EngineCommand::RemoveChannel { channel_uuid: last },
        );
        assert!(matches!(r, CommandResult::Ok));
    }
    let remaining: Vec<String> = app
        .build_engine_state()
        .mixer
        .channels
        .iter()
        .map(|c| c.uuid.clone())
        .collect();
    assert_eq!(remaining.len(), 2);
    // Removing either of the last 2 should fail (minimum 2 enforced)
    for uuid in remaining {
        let r = send_cmd(
            &mut app,
            EngineCommand::RemoveChannel { channel_uuid: uuid },
        );
        assert!(
            matches!(r, CommandResult::Err { .. }),
            "Should not remove below minimum"
        );
    }
    assert_eq!(app.build_engine_state().mixer.channels.len(), 2);
    app.update_frame_timing();
    app.render_mixer_frame();
}

// ── I: Command storm ─────────────────────────────────────────────────

#[test]
fn chaos_command_storm_crossfader_sweep() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&mut app, 0);
    let ch1 = channel_uuid(&mut app, 1);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch1,
            color: [0.0, 0.0, 1.0, 1.0],
        },
    );
    // Sweep crossfader through 100 steps while rendering
    for i in 0..=100 {
        let val = i as f32 / 100.0;
        fire(&mut app, EngineCommand::SetCrossfader(val));
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    let state = app.build_engine_state();
    assert!(
        (state.mixer.crossfader - 1.0).abs() < 1e-4,
        "Final crossfader should be 1.0"
    );
}

#[test]
fn chaos_command_storm_opacity_sweep() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    // Sweep deck opacity 0→1→0 rapidly
    for i in 0..=100 {
        let val = i as f32 / 100.0;
        fire(
            &mut app,
            EngineCommand::SetDeckOpacity {
                deck_uuid: deck.clone(),
                opacity: val,
            },
        );
    }
    for i in (0..=100).rev() {
        let val = i as f32 / 100.0;
        fire(
            &mut app,
            EngineCommand::SetDeckOpacity {
                deck_uuid: deck.clone(),
                opacity: val,
            },
        );
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    assert!(deck_snapshot(&mut app, &deck).opacity.abs() < 1e-4);
}

#[test]
fn chaos_command_storm_mixed_mutations() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&mut app, 0);
    let ch1 = channel_uuid(&mut app, 1);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch0.clone(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    // Fire 50 rapid mixed commands without rendering between them
    let tx = app.command_sender();
    for i in 0..50 {
        let cmd = match i % 5 {
            0 => EngineCommand::SetCrossfader(i as f32 / 50.0),
            1 => EngineCommand::SetChannelOpacity {
                channel_uuid: ch0.clone(),
                opacity: (50 - i) as f32 / 50.0,
            },
            2 => EngineCommand::AddSolidColorDeck {
                channel_uuid: if i % 2 == 0 { ch0.clone() } else { ch1.clone() },
                color: [1.0, 1.0, 1.0, 1.0],
            },
            3 => EngineCommand::SetDeckOpacity {
                deck_uuid: deck.clone(),
                opacity: i as f32 / 50.0,
            },
            _ => EngineCommand::SetCrossfader(0.5),
        };
        tx.send((cmd, None)).unwrap();
    }
    // Process all at once
    app.process_commands();
    // Render after burst
    app.update_frame_timing();
    app.render_mixer_frame();
    // State should be consistent
    let state = app.build_engine_state();
    assert!(!state.mixer.channels.is_empty());
}

#[test]
fn chaos_render_many_frames_with_content() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&mut app, 0);
    let ch1 = channel_uuid(&mut app, 1);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch1,
            color: [0.0, 1.0, 0.0, 1.0],
        },
    );
    // Render 100 frames — looking for GPU resource leaks or accumulation bugs
    for _ in 0..100 {
        app.update_frame_timing();
        app.render_mixer_frame();
    }
}

// ── K: Mixer bounds ──────────────────────────────────────────────────

#[test]
fn chaos_crossfader_extremes_render() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&mut app, 0);
    let ch1 = channel_uuid(&mut app, 1);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch1,
            color: [0.0, 0.0, 1.0, 1.0],
        },
    );
    for &val in &[0.0, 1.0, -1.0, 2.0, -100.0, 100.0, f32::MIN, f32::MAX] {
        fire(&mut app, EngineCommand::SetCrossfader(val));
        app.update_frame_timing();
        app.render_mixer_frame();
    }
}

#[test]
fn chaos_opacity_extremes_render() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ));
    for &val in &[
        0.0,
        1.0,
        -1.0,
        2.0,
        -100.0,
        100.0,
        f32::MIN,
        f32::MAX,
        f32::NAN,
        f32::INFINITY,
    ] {
        fire(
            &mut app,
            EngineCommand::SetDeckOpacity {
                deck_uuid: deck.clone(),
                opacity: val,
            },
        );
        app.update_frame_timing();
        app.render_mixer_frame();
    }
    for &val in &[0.0, 1.0, -1.0, 2.0, f32::NAN, f32::INFINITY] {
        fire(
            &mut app,
            EngineCommand::SetChannelOpacity {
                channel_uuid: ch.clone(),
                opacity: val,
            },
        );
        app.render_mixer_frame();
    }
}

#[test]
fn chaos_resolution_change_during_render() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    app.update_frame_timing();
    app.render_mixer_frame();
    // Change resolution and render immediately
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 640,
            height: 480,
        },
    );
    app.render_mixer_frame();
    // Change again rapidly
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 3840,
            height: 2160,
        },
    );
    app.render_mixer_frame();
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 1920,
            height: 1080,
        },
    );
    app.render_mixer_frame();
    assert_eq!(app.render_width(), 1920);
    assert_eq!(app.render_height(), 1080);
}

#[test]
fn chaos_blend_mode_rapid_cycling() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    let top = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [0.0, 1.0, 0.0, 1.0],
        },
    ));
    let modes = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
    ];
    for mode in modes {
        fire(
            &mut app,
            EngineCommand::SetDeckBlendMode {
                deck_uuid: top.clone(),
                mode,
            },
        );
        app.update_frame_timing();
        app.render_mixer_frame();
    }
}

#[test]
fn chaos_solo_mute_all_decks() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Add 5 decks to channel 0
    let ch = channel_uuid(&mut app, 0);
    let mut decks = Vec::new();
    for i in 0..5 {
        let c = i as f32 / 5.0;
        decks.push(new_uuid(send_cmd(
            &mut app,
            EngineCommand::AddSolidColorDeck {
                channel_uuid: ch.clone(),
                color: [c, c, c, 1.0],
            },
        )));
    }
    // Mute all
    for uuid in &decks {
        fire(
            &mut app,
            EngineCommand::SetDeckMute {
                deck_uuid: uuid.clone(),
                mute: true,
            },
        );
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    // Solo one
    fire(
        &mut app,
        EngineCommand::SetDeckSolo {
            deck_uuid: decks[2].clone(),
            solo: true,
        },
    );
    app.render_mixer_frame();
    // Unmute all, unsolo
    for uuid in &decks {
        fire(
            &mut app,
            EngineCommand::SetDeckMute {
                deck_uuid: uuid.clone(),
                mute: false,
            },
        );
    }
    fire(
        &mut app,
        EngineCommand::SetDeckSolo {
            deck_uuid: decks[2].clone(),
            solo: false,
        },
    );
    app.render_mixer_frame();
}

#[test]
fn chaos_state_consistency_after_storm() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Setup: 3 channels with 2 decks each
    send_cmd(&mut app, EngineCommand::AddChannel);
    let channels: Vec<String> = app
        .build_engine_state()
        .mixer
        .channels
        .iter()
        .map(|c| c.uuid.clone())
        .collect();
    let mut decks: Vec<Vec<String>> = Vec::new();
    for ch in &channels {
        let first = new_uuid(send_cmd(
            &mut app,
            EngineCommand::AddSolidColorDeck {
                channel_uuid: ch.clone(),
                color: [1.0, 0.0, 0.0, 1.0],
            },
        ));
        let second = new_uuid(send_cmd(
            &mut app,
            EngineCommand::AddSolidColorDeck {
                channel_uuid: ch.clone(),
                color: [0.0, 1.0, 0.0, 1.0],
            },
        ));
        decks.push(vec![first, second]);
    }
    // Storm: 200 parameter mutations
    for i in 0..200 {
        let ch = i % 3;
        let deck = i % 2;
        fire(
            &mut app,
            EngineCommand::SetDeckOpacity {
                deck_uuid: decks[ch][deck].clone(),
                opacity: (i as f32 / 200.0),
            },
        );
        fire(
            &mut app,
            EngineCommand::SetChannelOpacity {
                channel_uuid: channels[ch].clone(),
                opacity: 1.0 - (i as f32 / 200.0),
            },
        );
    }
    fire(&mut app, EngineCommand::SetCrossfader(0.75));
    // Render to exercise the full pipeline
    app.update_frame_timing();
    app.render_mixer_frame();
    // Verify state is self-consistent
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 3);
    for ch in &state.mixer.channels {
        assert_eq!(ch.decks.len(), 2);
    }
    assert!((state.mixer.crossfader - 0.75).abs() < 1e-4);
}

// ── Mesh-warp editing (8i.5) ─────────────────────────────────────────

/// Full pipeline: add a surface, create a headless output, assign the surface,
/// then subdivide its warp into a mesh and drag an interior point — verifying
/// each step through the engine snapshot. Mirrors how the UI and API both drive
/// per-assignment mesh warp.
#[test]
fn mesh_warp_subdivide_and_drag_point() {
    use varda::renderer::context::{OutputSource, OutputTarget};
    use varda::renderer::warp::WarpMode;

    let Some(mut app) = headless_app() else {
        return;
    };

    // Surface + output + assignment.
    send_cmd(
        &mut app,
        EngineCommand::AddSurface {
            name: "Warp Target".into(),
            source: OutputSource::Master,
        },
    );
    let surface_uuid = app
        .build_engine_state()
        .outputs
        .surfaces
        .iter()
        .find(|s| s.name == "Warp Target")
        .unwrap()
        .uuid
        .clone();
    send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: OutputTarget::NdiSend {
                sender_name: "Warp Out".into(),
            },
        },
    );
    let output_uuid = app
        .build_engine_state()
        .outputs
        .windows
        .last()
        .expect("headless output created")
        .uuid
        .clone();
    let r = send_cmd(
        &mut app,
        EngineCommand::AssignSurfaceToOutput {
            output_uuid,
            surface_uuid: surface_uuid.clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");

    let surface_warp = |app: &mut VardaApp| {
        app.build_engine_state()
            .outputs
            .surfaces
            .iter()
            .find(|s| s.uuid == surface_uuid)
            .unwrap()
            .warp
            .clone()
    };

    // Auto-warp: a fresh surface is shape-bound, so its *effective* warp is the
    // conforming mesh (never `None`). Unbind to enable manual mesh editing.
    assert!(surface_warp(&mut app).is_some());
    let r = send_cmd(
        &mut app,
        EngineCommand::SetWarpBound {
            surface_uuid: surface_uuid.clone(),
            bound: false,
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");

    // Subdivide → 3×3 mesh, preserving the (identity) deformation.
    let r = send_cmd(
        &mut app,
        EngineCommand::SetWarpSubdivisions {
            surface_uuid: surface_uuid.clone(),
            cols: 3,
            rows: 3,
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");
    let Some(WarpMode::Mesh(mesh)) = surface_warp(&mut app) else {
        panic!("expected mesh warp after subdivision");
    };
    assert_eq!(mesh.cols, 3);
    assert_eq!(mesh.rows, 3);
    assert_eq!(mesh.points.len(), 9);

    // Drag the centre point (row 1, col 1 → index 4).
    fire(
        &mut app,
        EngineCommand::SetWarpMeshPoint {
            surface_uuid: surface_uuid.clone(),
            row: 1,
            col: 1,
            position: [0.6, 0.4],
        },
    );
    let Some(WarpMode::Mesh(mesh)) = surface_warp(&mut app) else {
        panic!("expected mesh warp");
    };
    assert!((mesh.points[4].position[0] - 0.6).abs() < 1e-6);
    assert!((mesh.points[4].position[1] - 0.4).abs() < 1e-6);
}

/// Bezier warp (8i.6): convert an unbound surface's warp into a bezier cage,
/// edit an anchor and a tangent handle, and resize the cage — all through the
/// engine command path.
#[test]
fn bezier_warp_convert_and_edit() {
    use varda::renderer::context::OutputSource;
    use varda::renderer::warp::WarpMode;
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddSurface {
            name: "Bez".into(),
            source: OutputSource::Master,
        },
    );
    let uuid = app
        .build_engine_state()
        .outputs
        .surfaces
        .iter()
        .find(|s| s.name == "Bez")
        .unwrap()
        .uuid
        .clone();

    let warp = |app: &mut VardaApp| {
        app.build_engine_state()
            .outputs
            .surfaces
            .iter()
            .find(|s| s.uuid == uuid)
            .unwrap()
            .warp
            .clone()
    };

    // New surfaces are shape-bound; unbind to enable manual editing, then curve.
    send_cmd(
        &mut app,
        EngineCommand::SetWarpBound {
            surface_uuid: uuid.clone(),
            bound: false,
        },
    );
    send_cmd(
        &mut app,
        EngineCommand::ConvertWarpToBezier {
            surface_uuid: uuid.clone(),
        },
    );
    let Some(WarpMode::Bezier(b)) = warp(&mut app) else {
        panic!("expected bezier warp after convert");
    };
    assert_eq!((b.anchor_cols, b.anchor_rows), (2, 2));

    // Move a corner anchor.
    fire(
        &mut app,
        EngineCommand::MoveWarpAnchor {
            surface_uuid: uuid.clone(),
            row: 0,
            col: 0,
            position: [0.15, 0.25],
        },
    );
    let Some(WarpMode::Bezier(b)) = warp(&mut app) else {
        panic!("expected bezier warp");
    };
    assert!((b.anchor(0, 0)[0] - 0.15).abs() < 1e-6 && (b.anchor(0, 0)[1] - 0.25).abs() < 1e-6);

    // Curve the top edge by pulling its near-left tangent handle.
    fire(
        &mut app,
        EngineCommand::MoveWarpHandle {
            surface_uuid: uuid.clone(),
            horizontal: true,
            row: 0,
            col: 0,
            which: 0,
            position: [0.33, 0.05],
        },
    );
    let Some(WarpMode::Bezier(b)) = warp(&mut app) else {
        panic!("expected bezier warp");
    };
    assert!((b.h_horiz[0][0][1] - 0.05).abs() < 1e-6);

    // Resize the control cage to 3×3.
    send_cmd(
        &mut app,
        EngineCommand::SetBezierCageSubdivisions {
            surface_uuid: uuid.clone(),
            cols: 3,
            rows: 3,
        },
    );
    let Some(WarpMode::Bezier(b)) = warp(&mut app) else {
        panic!("expected bezier warp");
    };
    assert_eq!((b.anchor_cols, b.anchor_rows), (3, 3));
}

/// Setting subdivisions on a non-existent surface surfaces `NotFound` rather than
/// silently succeeding.
#[test]
fn mesh_warp_subdivisions_bad_index_errs() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::SetWarpSubdivisions {
            surface_uuid: "does-not-exist".into(),
            cols: 3,
            rows: 3,
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
}

#[test]
fn add_and_remove_surface_hole_workflow() {
    let Some(mut app) = headless_app() else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::AddSurface {
            name: "S".into(),
            source: OutputSource::Master,
        },
    );
    let uuid = app.surface_snapshot().first().unwrap().uuid.clone();

    // Add a hole → snapshot reflects it (holes + derived contours).
    let hole = SurfacePath::from_polygon(&[[0.3, 0.3], [0.6, 0.3], [0.6, 0.6], [0.3, 0.6]], true);
    let r = send_cmd(
        &mut app,
        EngineCommand::AddSurfaceHole {
            uuid: uuid.clone(),
            hole,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let snap = app.surface_snapshot();
    let s = snap.iter().find(|s| s.uuid == uuid).unwrap();
    assert_eq!(s.holes.len(), 1);
    assert_eq!(s.hole_contours.len(), 1);

    // Remove it → snapshot clears.
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveSurfaceHole {
            uuid: uuid.clone(),
            hole_index: 0,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let snap = app.surface_snapshot();
    let s = snap.iter().find(|s| s.uuid == uuid).unwrap();
    assert!(s.holes.is_empty());
    assert!(s.hole_contours.is_empty());

    // Out-of-range removal is a validation error.
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveSurfaceHole {
            uuid,
            hole_index: 5,
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::InvalidInput,
            ..
        }
    ));
}

#[test]
fn punch_surface_hole_workflow() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Target: full-canvas polygon.
    fire(
        &mut app,
        EngineCommand::AddPolygonSurface {
            name: "Target".into(),
            vertices: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            source: OutputSource::Master,
        },
    );
    // Source: small polygon centred inside the target.
    fire(
        &mut app,
        EngineCommand::AddPolygonSurface {
            name: "Source".into(),
            vertices: vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]],
            source: OutputSource::Master,
        },
    );
    let snap = app.surface_snapshot();
    let target_uuid = snap
        .iter()
        .find(|s| s.name == "Target")
        .unwrap()
        .uuid
        .clone();
    let source_uuid = snap
        .iter()
        .find(|s| s.name == "Source")
        .unwrap()
        .uuid
        .clone();

    // Punch: the source becomes a hole in the target and is consumed.
    let r = send_cmd(
        &mut app,
        EngineCommand::PunchSurfaceHole {
            source_uuid: source_uuid.clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let snap = app.surface_snapshot();
    assert!(
        snap.iter().all(|s| s.uuid != source_uuid),
        "source surface should be consumed"
    );
    let target = snap.iter().find(|s| s.uuid == target_uuid).unwrap();
    assert_eq!(target.holes.len(), 1);
    assert_eq!(target.hole_contours.len(), 1);

    // Nothing beneath the remaining surface → InvalidInput (no target resolved).
    let r = send_cmd(
        &mut app,
        EngineCommand::PunchSurfaceHole {
            source_uuid: target_uuid.clone(),
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::InvalidInput,
            ..
        }
    ));

    // Unknown source → NotFound.
    let r = send_cmd(
        &mut app,
        EngineCommand::PunchSurfaceHole {
            source_uuid: "does-not-exist".into(),
        },
    );
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
}

// ── Engine trait contracts (traits.rs / api-addressing.md) ─────────
//
// These assert promises the engine traits make at their boundary, distinct
// from the UUID-race regressions in tests/uuid_addressing.rs. They exercise
// contract *shape* — which failures are NotFound, which creations hand back a
// resolvable id, and which removals are permissive no-ops — rather than the
// reindex-safety those tests cover.

/// `MixerCommands::add_effect` resolves its *target* before doing anything.
/// An unresolvable Deck/Channel target is a precondition failure: the wire
/// result is `NotFound` and no effect is created anywhere. The existing
/// `NotFound` sweep only covers `Toggle`/`RemoveEffect` (which resolve the effect
/// uuid); `AddEffect` resolves the target chain, a separate code path.
#[test]
fn add_effect_on_unknown_target_is_not_found_and_creates_nothing() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));

    for target in [
        EffectTarget::Deck("no-such-deck".into()),
        EffectTarget::Channel("no-such-channel".into()),
    ] {
        let label = format!("{target:?}");
        let r = send_cmd(
            &mut app,
            EngineCommand::AddEffect {
                target,
                shader_name: "invert".to_string(),
            },
        );
        assert!(
            matches!(
                r,
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    ..
                }
            ),
            "AddEffect on {label} must be NotFound, got {r:?}"
        );
    }

    // No effect leaked onto the real deck or the master chain.
    let state = app.build_engine_state();
    let deck_effects = &state
        .mixer
        .channels
        .iter()
        .flat_map(|c| c.decks.iter())
        .find(|d| d.uuid == deck)
        .expect("deck must still exist")
        .effects;
    assert!(
        deck_effects.is_empty(),
        "no effect should have been created"
    );
    assert!(
        state.mixer.master_effects.is_empty(),
        "no effect should have leaked onto the master chain"
    );
}

/// The creation contract: `add_effect` returns `OkWithId`, and the reported
/// uuid resolves to a real effect in the very next state snapshot — for every
/// chain (deck, channel, master). This is the id-is-real half of the WS1 /
/// api-addressing promise (`uuid_addressing.rs` asserts the *toggle* works but
/// never that the reported id is findable in the snapshot).
#[test]
fn add_effect_reports_a_uuid_that_resolves_in_the_next_snapshot() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch.clone(),
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));

    // A build lacking the `invert` filter can't create effects; skip if so.
    let deck_fx = match send_cmd(
        &mut app,
        EngineCommand::AddEffect {
            target: EffectTarget::Deck(deck.clone()),
            shader_name: "invert".to_string(),
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        CommandResult::Err { .. } => return,
        other => panic!("expected OkWithId, got {other:?}"),
    };
    let channel_fx = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddEffect {
            target: EffectTarget::Channel(ch),
            shader_name: "invert".to_string(),
        },
    ));
    let master_fx = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddEffect {
            target: EffectTarget::Master,
            shader_name: "invert".to_string(),
        },
    ));

    let state = app.build_engine_state();
    let ch0 = &state.mixer.channels[0];
    assert_eq!(
        ch0.decks[0]
            .effects
            .iter()
            .filter(|e| e.uuid == deck_fx)
            .count(),
        1,
        "deck effect uuid must resolve in the snapshot"
    );
    assert_eq!(
        ch0.effects.iter().filter(|e| e.uuid == channel_fx).count(),
        1,
        "channel effect uuid must resolve in the snapshot"
    );
    assert_eq!(
        state
            .mixer
            .master_effects
            .iter()
            .filter(|e| e.uuid == master_fx)
            .count(),
        1,
        "master effect uuid must resolve in the snapshot"
    );
}

/// `ModulationCommands::remove_modulation_source` has no `Result` in its
/// signature — its contract is permissive: removing an unknown uuid is a silent
/// `Ok` no-op, not an error, and it must not disturb existing sources. This is
/// the inverse of the fallible-command contract (`Result`-returning commands
/// report `NotFound` and mutate nothing).
#[test]
fn remove_unknown_modulation_source_is_a_silent_noop() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Create a real source so we can prove the no-op leaves it intact.
    let before = send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
        },
    );
    assert!(
        matches!(before, CommandResult::Ok),
        "AddLfo should report Ok, got {before:?}"
    );
    let sources_before = app.build_engine_state().modulation.sources.len();
    assert_eq!(sources_before, 1, "one modulation source expected");

    // Removing a uuid that names nothing is Ok and changes nothing.
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveModulationSource {
            uuid: "deadbeef".into(),
        },
    );
    assert!(
        matches!(r, CommandResult::Ok),
        "removing an unknown modulation source must be a silent Ok, got {r:?}"
    );
    assert_eq!(
        app.build_engine_state().modulation.sources.len(),
        sources_before,
        "the existing source must be untouched by a no-op removal"
    );
}

// ── Arrangement ──────────────────────────────────────────────────────
//
// See /spec/arrangement.md. These exercise the whole path: command in,
// region compiled to an envelope, transport moved, deck opacity out.

/// A deck with one hard-edged region on it.
fn app_with_one_region(start: f64, end: f64) -> Option<(VardaApp, String)> {
    let mut app = headless_app()?;
    let ch = channel_uuid(&mut app, 0);
    let deck = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));
    add_region(&mut app, &deck, start, end);
    Some((app, deck))
}

fn add_region(app: &mut VardaApp, deck: &str, start: f64, end: f64) {
    let r = send_cmd(
        app,
        EngineCommand::AddRegion {
            deck_uuid: deck.to_string(),
            region: varda::arrangement::RegionConfig {
                start,
                end,
                fade_in: 0.0,
                fade_out: 0.0,
            },
        },
    );
    // The new region's index comes back so a caller can address it immediately.
    assert!(matches!(r, CommandResult::OkWithData { .. }), "{r:?}");
}

/// One full engine frame: inputs (which advance the transport), then render.
fn step(app: &mut VardaApp) {
    app.process_inputs();
    app.update_frame_timing();
    app.render_mixer_frame();
}

/// Put the playhead somewhere and actually run, since locating alone
/// deliberately does not engage the arrangement.
fn run_from(app: &mut VardaApp, position: f64) {
    fire(app, EngineCommand::TransportLocate { position });
    fire(app, EngineCommand::TransportPlay);
    step(app);
}

/// Opening a scene that has an arrangement must not black the output. Until the
/// transport has actually run, Performance mode still owns the decks.
#[test]
fn an_arrangement_stays_inert_until_the_transport_runs() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.8,
        },
    );

    step(&mut app);

    let state = app.build_engine_state();
    assert!(
        !state.arrangement.as_ref().expect("arrangement").engaged,
        "a parked transport must leave the arrangement inert"
    );
    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 0.8).abs() < 1e-4,
        "the performer's opacity must survive a cold start"
    );
}

#[test]
fn running_into_a_region_hands_the_deck_to_the_arrangement() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.0,
        },
    );
    run_from(&mut app, 15.0);

    assert!(
        app.build_engine_state()
            .arrangement
            .expect("arrangement")
            .engaged
    );
    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4,
        "inside its region the deck should be fully visible"
    );
}

/// A gap between two regions is authored silence, not idle: the arrangement did
/// say something about that stretch, and what it said was "nothing".
#[test]
fn a_gap_inside_the_arranged_range_hides_the_deck() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    add_region(&mut app, &deck, 40.0, 50.0);
    run_from(&mut app, 25.0);

    assert!(
        deck_snapshot(&mut app, &deck).opacity.abs() < 1e-4,
        "between regions the deck should be dark"
    );
}

/// `HoldPerformance` is the default, and it means the arrangement declines to
/// drive anything outside its range rather than driving it to zero.
#[test]
fn hold_performance_leaves_the_deck_alone_before_the_show() {
    let Some((mut app, deck)) = app_with_one_region(100.0, 200.0) else {
        return;
    };
    // Engage inside the range first, so the test is about idle behaviour rather
    // than about the transport never having run.
    run_from(&mut app, 150.0);
    assert!((deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4);

    fire(&mut app, EngineCommand::TransportLocate { position: 5.0 });
    step(&mut app);

    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4,
        "outside the arranged range HoldPerformance must not drive the deck"
    );
}

/// "Run this loop until the schedule starts" needs the pre-show state to be
/// something rather than nothing.
#[test]
fn show_deck_lights_a_deck_before_the_arranged_range() {
    let Some((mut app, deck)) = app_with_one_region(100.0, 200.0) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetIdleBehaviour {
            idle: varda::arrangement::IdleBehaviour::ShowDeck {
                deck_uuid: deck.clone(),
            },
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.0,
        },
    );
    run_from(&mut app, 5.0);

    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4,
        "the idle deck should be up before the arranged range"
    );
}

/// Touching an automated parameter must take effect immediately, not fight the
/// envelope for the rest of the show.
#[test]
fn a_live_touch_takes_a_parameter_back_from_the_arrangement() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    run_from(&mut app, 15.0);
    assert!((deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4);

    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.25,
        },
    );
    step(&mut app);

    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 0.25).abs() < 1e-4,
        "the performer's value must win while the override is held"
    );
    let state = app.build_engine_state();
    assert_eq!(
        state.arrangement.expect("arrangement").overridden_params,
        vec![format!("deck_{deck}:opacity")],
        "the held parameter should be reported so the UI can offer a re-arm"
    );
}

#[test]
fn re_arming_returns_the_parameter_to_the_arrangement() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    run_from(&mut app, 15.0);
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.25,
        },
    );
    // Zero seconds is an immediate handover, which keeps the test off the clock.
    fire(
        &mut app,
        EngineCommand::RearmParam {
            param_key: format!("deck_{deck}:opacity"),
            seconds: Some(0.0),
        },
    );
    step(&mut app);

    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4,
        "after re-arming the envelope should drive the deck again"
    );
    assert!(app
        .build_engine_state()
        .arrangement
        .expect("arrangement")
        .overridden_params
        .is_empty());
}

/// A lane is a deck's row, so removing it must give the deck back rather than
/// leave it pinned at whatever the envelope last said.
#[test]
fn removing_a_lane_returns_the_deck_to_performance() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    add_region(&mut app, &deck, 40.0, 50.0);
    run_from(&mut app, 25.0);
    assert!(deck_snapshot(&mut app, &deck).opacity.abs() < 1e-4);

    fire(
        &mut app,
        EngineCommand::RemoveLane {
            deck_uuid: deck.clone(),
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.7,
        },
    );
    step(&mut app);

    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 0.7).abs() < 1e-4,
        "a removed lane must stop driving its deck"
    );
}

#[test]
fn arrangement_commands_reject_decks_that_do_not_exist() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::AddLane {
            deck_uuid: "no-such-deck".into(),
        },
    );
    assert!(
        matches!(&r, CommandResult::Err { code, .. } if *code == ErrorCode::NotFound),
        "{r:?}"
    );
}

/// How many regions a lane holds, for tests that care that a refusal stored
/// nothing rather than merely reporting failure.
fn region_count(app: &mut VardaApp, deck: &str) -> usize {
    app.build_engine_state()
        .arrangement
        .and_then(|a| {
            a.config
                .lanes
                .iter()
                .find(|l| l.deck_uuid == deck)
                .map(|l| l.regions.len())
        })
        .unwrap_or_default()
}

/// A region that ends before it starts compiles to nothing sensible, so it is
/// refused at the door rather than stored and skipped later.
#[test]
fn an_inverted_region_is_refused_rather_than_stored() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let inverted = varda::arrangement::RegionConfig {
        start: 30.0,
        end: 25.0,
        fade_in: 0.0,
        fade_out: 0.0,
    };

    let added = send_cmd(
        &mut app,
        EngineCommand::AddRegion {
            deck_uuid: deck.clone(),
            region: inverted,
        },
    );
    assert!(
        matches!(&added, CommandResult::Err { code, .. } if *code == ErrorCode::InvalidInput),
        "{added:?}"
    );
    assert_eq!(
        region_count(&mut app, &deck),
        1,
        "a refused region must not land on the lane"
    );

    let updated = send_cmd(
        &mut app,
        EngineCommand::UpdateRegion {
            deck_uuid: deck.clone(),
            index: 0,
            region: inverted,
        },
    );
    assert!(
        matches!(&updated, CommandResult::Err { code, .. } if *code == ErrorCode::InvalidInput),
        "{updated:?}"
    );
    let span = app
        .build_engine_state()
        .arrangement
        .expect("arrangement")
        .config
        .lanes[0]
        .regions[0];
    assert!(
        (span.start - 10.0).abs() < 1e-9 && (span.end - 20.0).abs() < 1e-9,
        "a refused edit must leave the region it was aimed at alone"
    );
}

/// An index that arrived from a stale client is a miss, not a panic and not a
/// silent write to a neighbouring region.
#[test]
fn editing_a_region_that_is_not_there_is_an_error() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let region = varda::arrangement::RegionConfig {
        start: 1.0,
        end: 2.0,
        fade_in: 0.0,
        fade_out: 0.0,
    };

    for result in [
        send_cmd(
            &mut app,
            EngineCommand::UpdateRegion {
                deck_uuid: deck.clone(),
                index: 7,
                region,
            },
        ),
        send_cmd(
            &mut app,
            EngineCommand::RemoveRegion {
                deck_uuid: deck.clone(),
                index: 7,
            },
        ),
        send_cmd(
            &mut app,
            EngineCommand::RemoveRegion {
                deck_uuid: "no-such-deck".into(),
                index: 0,
            },
        ),
    ] {
        assert!(
            matches!(&result, CommandResult::Err { code, .. } if *code == ErrorCode::NotFound),
            "{result:?}"
        );
    }
    assert_eq!(region_count(&mut app, &deck), 1, "nothing was removed");
}

/// The idle deck is shown before the show reaches its first region, so naming
/// one that is gone would black the output at exactly the wrong moment.
#[test]
fn showing_a_deck_that_does_not_exist_before_the_show_is_refused() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let result = send_cmd(
        &mut app,
        EngineCommand::SetIdleBehaviour {
            idle: varda::arrangement::IdleBehaviour::ShowDeck {
                deck_uuid: "no-such-deck".into(),
            },
        },
    );
    assert!(
        matches!(&result, CommandResult::Err { code, .. } if *code == ErrorCode::NotFound),
        "{result:?}"
    );
}

/// A lane owns its curves, so removing the row has to take them out of the
/// modulation graph. An orphan envelope would keep driving a deck that no
/// longer has a row to edit it from.
#[test]
fn removing_a_lane_takes_its_curves_with_it() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let sources = |app: &mut VardaApp| app.build_engine_state().modulation.sources.len();
    let before = sources(&mut app);
    assert!(before > 0, "a region compiles to an opacity envelope");

    fire(
        &mut app,
        EngineCommand::RemoveLane {
            deck_uuid: deck.clone(),
        },
    );

    assert_eq!(
        sources(&mut app),
        before - 1,
        "the lane's envelope must leave with the lane"
    );
}

/// Lanes and curves for every deck the arrangement knows about.
fn arranged_decks(app: &mut VardaApp) -> Vec<String> {
    app.build_engine_state()
        .arrangement
        .map(|a| a.config.lanes.iter().map(|l| l.deck_uuid.clone()).collect())
        .unwrap_or_default()
}

fn source_count(app: &mut VardaApp) -> usize {
    app.build_engine_state().modulation.sources.len()
}

/// A lane is where a deck sits in show time rather than an object beside it, so
/// deleting the deck takes the lane and its curves too. An orphan lane draws no
/// row, because rows are read from the mixer, but it still saves and its
/// envelopes still drive a parameter key nothing answers to.
#[test]
fn deleting_a_deck_takes_its_lane_with_it() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let before = source_count(&mut app);
    assert_eq!(arranged_decks(&mut app), vec![deck.clone()]);

    fire(
        &mut app,
        EngineCommand::RemoveDeck {
            deck_uuid: deck.clone(),
        },
    );

    assert!(arranged_decks(&mut app).is_empty(), "the lane went with it");
    assert_eq!(
        source_count(&mut app),
        before - 1,
        "and so did the curve the region compiled to"
    );
}

/// Deleting a channel is deleting every deck in it, which means the same
/// teardown each of those decks would get on its own.
#[test]
fn deleting_a_channel_takes_its_decks_lanes_with_it() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    // A third channel, because the mixer keeps two.
    fire(&mut app, EngineCommand::AddChannel);
    let spare = channel_uuid(&mut app, 2);
    let stranger = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: spare,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));
    add_region(&mut app, &stranger, 30.0, 40.0);
    assert_eq!(source_count(&mut app), 2);

    let doomed = channel_uuid(&mut app, 2);
    fire(
        &mut app,
        EngineCommand::RemoveChannel {
            channel_uuid: doomed,
        },
    );

    assert_eq!(
        arranged_decks(&mut app),
        vec![deck],
        "only the deck in the surviving channel is still arranged"
    );
    assert_eq!(source_count(&mut app), 1);
}

/// The mixer keeps two channels, and a refusal has to leave the one it refused
/// exactly as it was rather than emptied of its decks on the way to finding out.
#[test]
fn a_refused_channel_removal_keeps_the_channels_decks() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let channel = channel_uuid(&mut app, 0);
    let before = source_count(&mut app);

    let result = send_cmd(
        &mut app,
        EngineCommand::RemoveChannel {
            channel_uuid: channel,
        },
    );

    assert!(
        matches!(&result, CommandResult::Err { code, .. } if *code == ErrorCode::InvalidInput),
        "{result:?}"
    );
    assert_eq!(app.build_engine_state().mixer.channels.len(), 2);
    assert_eq!(
        deck_snapshot(&mut app, &deck).uuid,
        deck,
        "the deck it holds is still there"
    );
    assert_eq!(arranged_decks(&mut app), vec![deck]);
    assert_eq!(source_count(&mut app), before);
}

/// The panic button: one press hands every held parameter back, not just the
/// one that happens to be selected.
#[test]
fn re_arming_everything_hands_back_every_parameter() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let second = new_uuid(send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ));
    add_region(&mut app, &second, 10.0, 20.0);
    run_from(&mut app, 15.0);

    for uuid in [&deck, &second] {
        fire(
            &mut app,
            EngineCommand::SetDeckOpacity {
                deck_uuid: uuid.clone(),
                opacity: 0.25,
            },
        );
    }
    assert_eq!(
        app.build_engine_state()
            .arrangement
            .expect("arrangement")
            .overridden_params
            .len(),
        2,
        "both hands are on the show"
    );

    fire(&mut app, EngineCommand::RearmAll { seconds: Some(0.0) });
    step(&mut app);

    assert!(app
        .build_engine_state()
        .arrangement
        .expect("arrangement")
        .overridden_params
        .is_empty());
    for uuid in [&deck, &second] {
        assert!(
            (deck_snapshot(&mut app, uuid).opacity - 1.0).abs() < 1e-4,
            "every parameter is driven by the show again"
        );
    }
}

/// A parameter handed back over a ramp must not snap: the ramp is what keeps a
/// re-arm from reading as a cut on the output. The badge, though, clears on the
/// press rather than at the end of the ramp, because the answer to "is a hand on
/// this?" became no the moment it was let go.
#[test]
fn a_re_armed_parameter_ramps_rather_than_snapping() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    run_from(&mut app, 15.0);
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.0,
        },
    );
    step(&mut app);

    fire(
        &mut app,
        EngineCommand::RearmParam {
            param_key: format!("deck_{deck}:opacity"),
            seconds: Some(30.0),
        },
    );
    step(&mut app);

    let opacity = deck_snapshot(&mut app, &deck).opacity;
    assert!(
        opacity < 0.9,
        "{opacity} should still be on its way back to the envelope's 1.0"
    );
    assert!(
        app.build_engine_state()
            .arrangement
            .expect("arrangement")
            .overridden_params
            .is_empty(),
        "a parameter on its way back is no longer held, so the badge is already down"
    );
}

/// Every surface write is a performer's hand, including the ones that arrive as
/// router paths from OSC, MIDI, and the API rather than as engine commands. The
/// write itself lands through the router; this is the half that stops the show
/// from writing over it a frame later.
#[test]
fn a_route_write_takes_the_parameter_back_from_the_show() {
    let Some((mut app, deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    run_from(&mut app, 15.0);

    app.note_live_route_write(&format!("deck/{deck}/opacity"), 0.25);
    step(&mut app);

    assert_eq!(
        app.build_engine_state()
            .arrangement
            .expect("arrangement")
            .overridden_params,
        vec![format!("deck_{deck}:opacity")],
        "a route write holds the parameter exactly as a UI drag does"
    );

    // Past the region, where the envelope would darken the deck if it still had
    // the parameter.
    run_from(&mut app, 25.0);
    assert!(
        (deck_snapshot(&mut app, &deck).opacity - 1.0).abs() < 1e-4,
        "a held parameter stays where the hand left it"
    );
}

/// A path that names nothing the modulation engine knows is dropped rather than
/// holding a parameter that does not exist.
#[test]
fn a_route_write_to_an_unknown_path_holds_nothing() {
    let Some((mut app, _deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    run_from(&mut app, 15.0);

    app.note_live_route_write("deck/no-such-deck/opacity", 0.25);
    app.note_live_route_write("not/a/path/at/all", 0.25);
    step(&mut app);

    assert!(app
        .build_engine_state()
        .arrangement
        .expect("arrangement")
        .overridden_params
        .is_empty());
}

// ── Cue points ───────────────────────────────────────────────────────
//
// See /spec/arrangement.md § Cue points.

fn cues(app: &mut VardaApp) -> Vec<(String, f64)> {
    app.build_engine_state()
        .arrangement
        .map(|a| {
            a.config
                .cues
                .iter()
                .map(|c| (c.name.clone(), c.at))
                .collect()
        })
        .unwrap_or_default()
}

fn add_cue(app: &mut VardaApp, at: f64) -> String {
    match send_cmd(
        app,
        EngineCommand::AddCue {
            at,
            name: String::new(),
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected a cue uuid, got {other:?}"),
    }
}

fn position(app: &mut VardaApp) -> f64 {
    app.build_engine_state().transport.position
}

/// Cues are named by how many exist and held in position order, so the arrows
/// can scan the list rather than sort it on every press.
#[test]
fn cues_are_named_as_they_are_dropped_and_kept_in_order() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 30.0);
    add_cue(&mut app, 10.0);

    assert_eq!(
        cues(&mut app),
        vec![("Cue 2".to_string(), 10.0), ("Cue 1".to_string(), 30.0)]
    );
}

/// A drag moves a cue past its neighbours, and navigation reads the list in
/// order, so the move has to re-sort.
#[test]
fn moving_a_cue_past_its_neighbour_reorders_the_list() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let first = add_cue(&mut app, 10.0);
    add_cue(&mut app, 30.0);

    fire(
        &mut app,
        EngineCommand::UpdateCue {
            uuid: first.clone(),
            at: Some(50.0),
            name: None,
        },
    );

    assert_eq!(
        cues(&mut app),
        vec![("Cue 2".to_string(), 30.0), ("Cue 1".to_string(), 50.0)]
    );

    fire(
        &mut app,
        EngineCommand::UpdateCue {
            uuid: first,
            at: None,
            name: Some("Drop".to_string()),
        },
    );
    assert_eq!(
        cues(&mut app)[1],
        ("Drop".to_string(), 50.0),
        "renaming leaves the position alone"
    );
}

/// Back with no earlier cue goes to zero, which is the way home now that the
/// return-to-zero arrow walks cues. Forward past the last stays put rather than
/// running off the end, and a cue level with the playhead is skipped in both
/// directions so that holding an arrow walks the list.
#[test]
fn the_arrows_walk_the_cue_list_and_stop_at_its_ends() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    add_cue(&mut app, 20.0);

    fire(&mut app, EngineCommand::TransportNextCue);
    assert!((position(&mut app) - 10.0).abs() < 1e-9);
    fire(&mut app, EngineCommand::TransportNextCue);
    assert!((position(&mut app) - 20.0).abs() < 1e-9);
    fire(&mut app, EngineCommand::TransportNextCue);
    assert!(
        (position(&mut app) - 20.0).abs() < 1e-9,
        "forward past the last cue holds"
    );

    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!((position(&mut app) - 10.0).abs() < 1e-9);
    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!(
        position(&mut app).abs() < 1e-9,
        "back past the first cue goes home"
    );
}

/// A cue button is a way to *go somewhere*, so it locates and leaves the
/// transport as it was rather than starting the show as a side effect.
#[test]
fn firing_a_cue_locates_without_starting_the_show() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let second = {
        add_cue(&mut app, 10.0);
        add_cue(&mut app, 20.0)
    };

    fire(&mut app, EngineCommand::TriggerCue { uuid: second });
    assert!((position(&mut app) - 20.0).abs() < 1e-9);
    assert!(
        !app.build_engine_state().transport.running,
        "firing a cue must not roll the show"
    );
}

/// The button and the arrows are one way of moving, so a press of back after a
/// press of a button steps from the cue that was pressed.
#[test]
fn firing_a_cue_is_where_the_arrows_carry_on_from() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    add_cue(&mut app, 20.0);
    let third = add_cue(&mut app, 30.0);

    fire(&mut app, EngineCommand::TriggerCue { uuid: third });
    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!((position(&mut app) - 20.0).abs() < 1e-9);
}

#[test]
fn firing_a_cue_that_is_gone_is_an_error() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    let result = send_cmd(
        &mut app,
        EngineCommand::TriggerCue {
            uuid: "nope".to_string(),
        },
    );
    assert!(
        matches!(result, CommandResult::Err { code, .. } if code == ErrorCode::NotFound),
        "{result:?}"
    );
}

/// The position belongs to the incoming signal while chasing, so a cue button
/// is refused there like every other way of moving the playhead.
#[test]
fn firing_a_cue_is_refused_while_chasing_timecode() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let cue = add_cue(&mut app, 10.0);
    fire(
        &mut app,
        EngineCommand::SetTransportSource {
            source: varda::transport::TransportSource::Timecode,
        },
    );
    let result = send_cmd(&mut app, EngineCommand::TriggerCue { uuid: cue });
    assert!(matches!(result, CommandResult::Err { .. }), "{result:?}");
    assert!(position(&mut app).abs() < 1e-9, "the playhead stayed put");
}

/// The arrows walk the list while the show is running, not just while it is
/// parked. Reading the live position each press would send every press back to
/// the same cue, because playback carries the playhead past it between them.
#[test]
fn the_back_arrow_keeps_walking_while_the_show_runs() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    add_cue(&mut app, 20.0);
    add_cue(&mut app, 30.0);
    fire(&mut app, EngineCommand::TransportLocate { position: 35.0 });
    fire(&mut app, EngineCommand::TransportPlay);

    for expected in [30.0, 20.0, 10.0, 0.0] {
        fire(&mut app, EngineCommand::TransportPrevCue);
        assert!(
            (position(&mut app) - expected).abs() < 1e-9,
            "expected {expected}, got {}",
            position(&mut app)
        );
        // Playback carries the playhead off the cue before the next press.
        app.process_inputs();
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.process_inputs();
    }
}

/// Scrubbing is not walking: the arrows step from the playhead again once a
/// hand has moved it, rather than from wherever the last press left off.
#[test]
fn scrubbing_ends_the_cue_walk() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    add_cue(&mut app, 20.0);
    add_cue(&mut app, 30.0);

    fire(&mut app, EngineCommand::TransportLocate { position: 25.0 });
    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!((position(&mut app) - 20.0).abs() < 1e-9);

    fire(&mut app, EngineCommand::TransportLocate { position: 35.0 });
    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!(
        (position(&mut app) - 30.0).abs() < 1e-9,
        "the scrub decides where back goes, not the earlier press"
    );
}

/// A second stop returns to zero, which is a move the walk did not make. The
/// next press has to read the playhead again, or back from the top would jump
/// forward to wherever the walk had reached.
#[test]
fn stopping_back_to_zero_ends_the_cue_walk() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    add_cue(&mut app, 20.0);

    fire(&mut app, EngineCommand::TransportLocate { position: 25.0 });
    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!((position(&mut app) - 20.0).abs() < 1e-9);

    // Stopped already, so this one returns to zero.
    fire(&mut app, EngineCommand::TransportStop);
    assert!(position(&mut app).abs() < 1e-9);

    fire(&mut app, EngineCommand::TransportNextCue);
    assert!(
        (position(&mut app) - 10.0).abs() < 1e-9,
        "forward from the top is the first cue, not the one the walk had reached"
    );
}

/// Handing the position to a timecode master ends the walk too: whatever the
/// master does with the playhead, it is not where the last press left it.
#[test]
fn chasing_timecode_ends_the_cue_walk() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    add_cue(&mut app, 20.0);

    fire(&mut app, EngineCommand::TransportLocate { position: 25.0 });
    fire(&mut app, EngineCommand::TransportPrevCue);
    for source in [
        varda::transport::TransportSource::Timecode,
        varda::transport::TransportSource::Internal,
    ] {
        fire(&mut app, EngineCommand::SetTransportSource { source });
    }

    fire(&mut app, EngineCommand::TransportPrevCue);
    assert!(
        (position(&mut app) - 10.0).abs() < 1e-9,
        "the walk resumed from the playhead the master left behind"
    );
}

/// Position is read-only while chasing, so the arrows are rejected like any
/// other locate rather than fighting the master.
#[test]
fn the_arrows_are_refused_while_chasing_timecode() {
    let Some(mut app) = headless_app() else {
        return;
    };
    add_cue(&mut app, 10.0);
    fire(
        &mut app,
        EngineCommand::SetTransportSource {
            source: varda::transport::TransportSource::Timecode,
        },
    );

    let r = send_cmd(&mut app, EngineCommand::TransportNextCue);
    assert!(
        matches!(&r, CommandResult::Err { code, .. } if *code == ErrorCode::InvalidInput),
        "{r:?}"
    );
    assert!(position(&mut app).abs() < 1e-9);
}

/// One button covers stop and return, because the arrangement's return-to-zero
/// arrow is now the cue back arrow. See /spec/transport.md § Stop Twice.
#[test]
fn stopping_a_second_time_returns_the_show_to_zero() {
    let Some(mut app) = headless_app() else {
        return;
    };
    fire(&mut app, EngineCommand::TransportLocate { position: 45.0 });
    fire(&mut app, EngineCommand::TransportPlay);

    fire(&mut app, EngineCommand::TransportStop);
    assert!(
        position(&mut app) >= 45.0,
        "the first stop holds where it stopped"
    );

    fire(&mut app, EngineCommand::TransportStop);
    assert!(position(&mut app).abs() < 1e-9);
}

#[test]
fn editing_a_cue_that_is_gone_is_an_error() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveCue {
            uuid: "nosuch01".into(),
        },
    );
    assert!(
        matches!(&r, CommandResult::Err { code, .. } if *code == ErrorCode::NotFound),
        "{r:?}"
    );
}

/// A cue before zero, or at a position arithmetic produced rather than a
/// performer, would sort into a list the arrows then walk into nowhere.
#[test]
fn a_cue_at_an_impossible_position_is_refused() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let uuid = add_cue(&mut app, 10.0);

    for at in [-1.0, f64::NAN, f64::INFINITY] {
        let added = send_cmd(
            &mut app,
            EngineCommand::AddCue {
                at,
                name: String::new(),
            },
        );
        assert!(
            matches!(&added, CommandResult::Err { code, .. } if *code == ErrorCode::InvalidInput),
            "adding at {at}: {added:?}"
        );
        let moved = send_cmd(
            &mut app,
            EngineCommand::UpdateCue {
                uuid: uuid.clone(),
                at: Some(at),
                name: None,
            },
        );
        assert!(
            matches!(&moved, CommandResult::Err { code, .. } if *code == ErrorCode::InvalidInput),
            "moving to {at}: {moved:?}"
        );
    }

    assert_eq!(
        cues(&mut app),
        vec![("Cue 1".to_string(), 10.0)],
        "the list is exactly as it was before the refusals"
    );
}

// ── Deck residency ───────────────────────────────────────────────────
//
// See /spec/deck-residency.md. A deck whose next region is far away stops
// pulling frames; anything the arrangement cannot predict keeps running.

fn asleep(app: &mut VardaApp, deck: &str) -> bool {
    deck_snapshot(app, deck).source_asleep
}

/// A long show with two short appearances, which is the shape residency exists
/// for. Everything between them is inside the arranged range and dark, so the
/// arrangement has actually said this deck is unwanted rather than said nothing.
fn app_with_sparse_regions() -> Option<(VardaApp, String)> {
    let (mut app, deck) = app_with_one_region(10.0, 20.0)?;
    add_region(&mut app, &deck, 290.0, 300.0);
    Some((app, deck))
}

/// The whole point: sixty decode threads should not run for a show that will
/// not show their decks for another forty minutes.
#[test]
fn a_deck_far_from_its_region_stops_pulling_frames() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 150.0);

    assert!(
        asleep(&mut app, &deck),
        "a deck two minutes from its next region should be asleep"
    );
}

/// Cueing a channel is how an operator looks at what is coming next, so a deck
/// being watched off-air keeps decoding however dark the arrangement has it.
/// The same exemption the opacity cull already makes.
#[test]
fn a_deck_in_a_previewed_channel_keeps_pulling_frames() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 150.0);
    assert!(asleep(&mut app, &deck), "asleep until someone looks at it");

    app.set_preview_channels(vec![0]);
    step(&mut app);
    assert!(
        !asleep(&mut app, &deck),
        "a cued channel is being watched, so its decks stay awake"
    );

    app.set_preview_channels(Vec::new());
    step(&mut app);
    assert!(
        asleep(&mut app, &deck),
        "and it goes back to sleep when the operator looks away"
    );
}

/// Frames must be flowing before the audience sees any, so the wake happens
/// ahead of the region rather than on its edge.
#[test]
fn a_deck_wakes_before_its_region_arrives() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 285.0);
    assert!(asleep(&mut app, &deck), "still five seconds out");

    fire(&mut app, EngineCommand::TransportLocate { position: 289.5 });
    step(&mut app);
    assert!(
        !asleep(&mut app, &deck),
        "half a second out, the deck must already be decoding"
    );
}

#[test]
fn a_deck_inside_its_region_never_sleeps() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 15.0);

    assert!(!asleep(&mut app, &deck));
}

/// A jump into the middle of a region has to resume decode on the frame it
/// lands, not on the next one.
#[test]
fn locating_into_a_region_wakes_the_deck_in_the_same_frame() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 150.0);
    assert!(asleep(&mut app, &deck));

    fire(&mut app, EngineCommand::TransportLocate { position: 295.0 });
    step(&mut app);

    assert!(
        !asleep(&mut app, &deck),
        "the deck the playhead landed inside must be awake immediately"
    );
}

/// Outside the arranged range `HoldPerformance` declines to speak, and silence
/// is not permission to stop a deck the performer may still have up.
#[test]
fn a_deck_outside_the_arranged_range_is_left_alone() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 400.0);

    assert!(!asleep(&mut app, &deck));
}

/// Performance mode has never gated anything, and a scene that has not been
/// started is Performance mode.
#[test]
fn a_parked_transport_leaves_every_deck_awake() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    step(&mut app);

    assert!(
        !asleep(&mut app, &deck),
        "an arrangement that has not run must not put anything to sleep"
    );
}

/// An LFO can raise a deck at any moment, so nothing about the timeline says
/// when its frames are safe to stop.
#[test]
fn a_live_modulator_on_opacity_keeps_the_deck_awake() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
        },
    );
    // The lane's own opacity envelope is source zero, so the LFO is the newest.
    let lfo = app
        .build_engine_state()
        .modulation
        .sources
        .last()
        .expect("the LFO just added")
        .uuid
        .clone();
    fire(
        &mut app,
        EngineCommand::AssignModulation {
            target: format!("deck_{deck}:opacity"),
            source_id: lfo,
            amount: 1.0,
        },
    );
    run_from(&mut app, 150.0);

    assert!(
        !asleep(&mut app, &deck),
        "a deck an LFO can raise must keep decoding"
    );
}

/// A hand on the fader is the least predictable driver there is.
#[test]
fn an_overridden_deck_keeps_decoding() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 150.0);
    assert!(asleep(&mut app, &deck));

    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.6,
        },
    );
    step(&mut app);

    assert!(
        !asleep(&mut app, &deck),
        "the performer took this deck back, so its frames are wanted again"
    );
}

/// Sleep is derived state, recomputed every frame, so removing the lane that
/// justified it has to hand the deck back.
#[test]
fn removing_a_lane_wakes_its_deck() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    run_from(&mut app, 150.0);
    assert!(asleep(&mut app, &deck));

    fire(
        &mut app,
        EngineCommand::RemoveLane {
            deck_uuid: deck.clone(),
        },
    );
    step(&mut app);

    assert!(!asleep(&mut app, &deck));
}

/// An arrangement that declines to speak outside its range has not said the
/// deck is unwanted, and the idle deck is the one thing on screen.
#[test]
fn the_idle_deck_is_never_put_to_sleep() {
    let Some((mut app, deck)) = app_with_sparse_regions() else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetIdleBehaviour {
            idle: varda::arrangement::IdleBehaviour::ShowDeck {
                deck_uuid: deck.clone(),
            },
        },
    );
    run_from(&mut app, 5.0);

    assert!(
        !asleep(&mut app, &deck),
        "the deck holding the pre-show output must keep decoding"
    );
}

/// An arrangement with authority and a free-running sequence would fight over
/// the crossfader, so the sequence is refused rather than allowed to lose.
#[test]
fn a_sequence_cannot_start_while_the_arrangement_has_authority() {
    let Some((mut app, _deck)) = app_with_one_region(10.0, 20.0) else {
        return;
    };
    let seq = new_uuid(send_cmd(&mut app, EngineCommand::CreateSequence));
    run_from(&mut app, 15.0);

    let r = send_cmd(&mut app, EngineCommand::PlaySequence { sequence_uuid: seq });
    assert!(
        matches!(r, CommandResult::Err { .. }),
        "playing a sequence under arrangement authority should be refused, got {r:?}"
    );
}

#[test]
fn presentation_request_keeps_ten_bit_intent_and_reports_ndi_fallback() {
    use varda::engine::value::render::{PresentationDepth, PresentationRequest};
    use varda::renderer::context::OutputTarget;

    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: OutputTarget::NdiSend {
                sender_name: "Precision Test".into(),
            },
        },
    );
    let output_uuid = app
        .build_engine_state()
        .outputs
        .windows
        .last()
        .expect("headless output created")
        .uuid
        .clone();
    assert!(matches!(
        send_cmd(
            &mut app,
            EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.clone(),
                request: PresentationRequest {
                    depth: PresentationDepth::Sdr10,
                    dither: true,
                },
            },
        ),
        CommandResult::Ok
    ));

    let state = app.build_engine_state();
    let output = state
        .outputs
        .windows
        .iter()
        .find(|output| output.uuid == output_uuid)
        .unwrap();
    assert_eq!(output.presentation_request.depth, PresentationDepth::Sdr10);
    assert_eq!(
        output.resolved_presentation.resolved,
        PresentationDepth::Sdr8
    );
    assert!(output.resolved_presentation.fallback_reason.is_some());
}

fn last_output_uuid(app: &mut VardaApp) -> String {
    app.build_engine_state()
        .outputs
        .windows
        .last()
        .expect("output created")
        .uuid
        .clone()
}

#[test]
fn presentation_request_unknown_output_is_not_found() {
    use varda::engine::value::render::{PresentationDepth, PresentationRequest};

    let Some(mut app) = headless_app() else {
        return;
    };
    let result = send_cmd(
        &mut app,
        EngineCommand::SetOutputPresentation {
            output_uuid: "no-such-output".into(),
            request: PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: true,
            },
        },
    );
    assert!(
        matches!(
            result,
            CommandResult::Err {
                code: ErrorCode::NotFound,
                ..
            }
        ),
        "unknown output should error: {result:?}"
    );
}

#[test]
fn presentation_request_keeps_ten_bit_intent_on_syphon_fallback() {
    use varda::engine::value::render::{
        PresentationDepth, PresentationPixelFormat, PresentationRequest,
    };
    use varda::renderer::context::OutputTarget;

    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: OutputTarget::SyphonServer {
                server_name: "Precision Syphon".into(),
            },
        },
    );
    let output_uuid = last_output_uuid(&mut app);
    assert!(matches!(
        send_cmd(
            &mut app,
            EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.clone(),
                request: PresentationRequest {
                    depth: PresentationDepth::Sdr10,
                    dither: false,
                },
            },
        ),
        CommandResult::Ok
    ));

    let output = app
        .build_engine_state()
        .outputs
        .windows
        .into_iter()
        .find(|output| output.uuid == output_uuid)
        .unwrap();
    assert_eq!(output.presentation_request.depth, PresentationDepth::Sdr10);
    assert!(!output.presentation_request.dither);
    assert_eq!(
        output.resolved_presentation.resolved,
        PresentationDepth::Sdr8
    );
    assert_eq!(
        output.resolved_presentation.pixel_format,
        PresentationPixelFormat::Bgra8
    );
    assert!(output.resolved_presentation.fallback_reason.is_some());
}

#[test]
fn chaos_unknown_output_uuid_presentation_does_not_panic() {
    use varda::engine::value::render::{PresentationDepth, PresentationRequest};

    let Some(mut app) = headless_app() else {
        return;
    };
    for uuid in ["", "no-such-output", "🔥", &"x".repeat(4096), "out\0null"] {
        fire(
            &mut app,
            EngineCommand::SetOutputPresentation {
                output_uuid: uuid.to_string(),
                request: PresentationRequest {
                    depth: PresentationDepth::Sdr10,
                    dither: uuid.len() % 2 == 0,
                },
            },
        );
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    assert!(app.build_engine_state().outputs.windows.is_empty());
}

#[test]
fn chaos_rapid_presentation_toggle_while_rendering() {
    use varda::engine::value::render::{PresentationDepth, PresentationRequest};
    use varda::renderer::context::OutputTarget;

    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: OutputTarget::NdiSend {
                sender_name: "Storm".into(),
            },
        },
    );
    let output_uuid = last_output_uuid(&mut app);
    for i in 0..64 {
        fire(
            &mut app,
            EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.clone(),
                request: PresentationRequest {
                    depth: if i % 2 == 0 {
                        PresentationDepth::Sdr10
                    } else {
                        PresentationDepth::Sdr8
                    },
                    dither: i % 3 != 0,
                },
            },
        );
        if i % 8 == 0 {
            app.update_frame_timing();
            app.render_mixer_frame();
        }
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    let output = app
        .build_engine_state()
        .outputs
        .windows
        .into_iter()
        .find(|output| output.uuid == output_uuid)
        .unwrap();
    assert_eq!(output.presentation_request.depth, PresentationDepth::Sdr8);
    assert!(!output.presentation_request.dither);
}

#[test]
fn chaos_retarget_and_presentation_storm() {
    use varda::engine::value::render::{
        PresentationDepth, PresentationRequest, RecordingCodec, StreamingCodec,
    };
    use varda::renderer::context::OutputTarget;

    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: OutputTarget::NdiSend {
                sender_name: "Storm".into(),
            },
        },
    );
    let output_uuid = last_output_uuid(&mut app);
    let targets = [
        OutputTarget::SyphonServer {
            server_name: "Storm Syphon".into(),
        },
        OutputTarget::Recording {
            path: "/tmp/varda-chaos-presentation.mov".into(),
            codec: RecordingCodec::Hap,
            audio_device: None,
        },
        OutputTarget::HlsStream {
            name: "storm".into(),
            codec: StreamingCodec::H265,
            low_latency: true,
            audio_device: None,
        },
        OutputTarget::NdiSend {
            sender_name: "Storm".into(),
        },
    ];
    for (i, target) in targets.into_iter().cycle().take(24).enumerate() {
        fire(
            &mut app,
            EngineCommand::SetOutputTarget {
                output_uuid: output_uuid.clone(),
                target,
            },
        );
        fire(
            &mut app,
            EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.clone(),
                request: PresentationRequest {
                    depth: PresentationDepth::Sdr10,
                    dither: i % 2 == 0,
                },
            },
        );
        if i % 5 == 0 {
            app.update_frame_timing();
            app.render_mixer_frame();
        }
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    let output = app
        .build_engine_state()
        .outputs
        .windows
        .into_iter()
        .find(|output| output.uuid == output_uuid)
        .unwrap();
    assert_eq!(output.presentation_request.depth, PresentationDepth::Sdr10);
    assert_eq!(
        output.resolved_presentation.requested,
        PresentationDepth::Sdr10
    );
}

#[test]
fn chaos_create_close_presentation_cycle() {
    use varda::engine::value::render::{PresentationDepth, PresentationRequest};
    use varda::renderer::context::OutputTarget;

    let Some(mut app) = headless_app() else {
        return;
    };
    for i in 0..16 {
        send_cmd(
            &mut app,
            EngineCommand::CreateHeadlessOutput {
                target: OutputTarget::NdiSend {
                    sender_name: format!("Cycle {i}"),
                },
            },
        );
        let output_uuid = last_output_uuid(&mut app);
        fire(
            &mut app,
            EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.clone(),
                request: PresentationRequest {
                    depth: PresentationDepth::Sdr10,
                    dither: true,
                },
            },
        );
        app.update_frame_timing();
        app.render_mixer_frame();
        fire(&mut app, EngineCommand::CloseOutput { output_uuid });
    }
    app.update_frame_timing();
    app.render_mixer_frame();
    assert!(app.build_engine_state().outputs.windows.is_empty());
}
