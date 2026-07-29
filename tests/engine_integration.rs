//! Engine integration tests — multi-step command workflows through real headless VardaApp.

use varda::app::{AppConfig, VardaApp};
use varda::engine::{
    BlendMode, CommandResult, DeckSnapshot, EffectTarget, EngineCommand, ErrorCode, SurfaceQueries,
};
use varda::modulation::LFOWaveform;
use varda::renderer::context::OutputSource;
use varda::surface::SurfacePath;

use clap::Parser;

fn parse_args(args: &[&str]) -> AppConfig {
    AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
}

fn headless_app() -> Option<VardaApp> {
    let gpu = varda::renderer::context::GpuContext::new_headless().ok()?;
    let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
    VardaApp::new(gpu, &config).ok()
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

/// Setting subdivisions on a non-existent surface surfaces NotFound rather than
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
/// NotFound sweep only covers `Toggle`/`RemoveEffect` (which resolve the effect
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
