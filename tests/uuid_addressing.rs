//! Regression tests for UUID-only write addressing.
//!
//! See `/spec/api-addressing.md`. Before this change, write commands addressed
//! entities positionally (`channel_idx` / `deck_idx` / `effect_idx`). A client
//! that read state, then issued a write, could have its index invalidated by a
//! concurrent reorder or removal between the read and the write — the write
//! then landed on a *different* entity with no error. UUIDs make that
//! impossible: an address either resolves to the entity the caller meant or
//! fails with `NotFound`.

use varda::app::{AppConfig, VardaApp};
use varda::engine::{CommandResult, EffectTarget, EngineCommand, ErrorCode};

use clap::Parser;

fn headless_app() -> Option<VardaApp> {
    let gpu = varda::renderer::context::GpuContext::new_headless().ok()?;
    let config = AppConfig::parse_from(
        ["varda", "--headless", "--no-osc", "--no-ndi", "--no-syphon"].iter(),
    );
    VardaApp::new(gpu, &config).ok()
}

fn send_cmd(app: &mut VardaApp, cmd: EngineCommand) -> CommandResult {
    let tx = app.command_sender();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send((cmd, Some(reply_tx))).unwrap();
    app.process_commands();
    reply_rx.blocking_recv().unwrap()
}

fn fire(app: &mut VardaApp, cmd: EngineCommand) {
    let tx = app.command_sender();
    tx.send((cmd, None)).unwrap();
    app.process_commands();
}

fn new_uuid(result: CommandResult) -> String {
    match result {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    }
}

fn channel_uuid(app: &VardaApp, idx: usize) -> String {
    app.build_engine_state().mixer.channels[idx].uuid.clone()
}

/// Add `count` solid-color decks to a channel, returning their UUIDs in order.
fn add_decks(app: &mut VardaApp, channel_uuid: &str, count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            let shade = i as f32 / count as f32;
            new_uuid(send_cmd(
                app,
                EngineCommand::AddSolidColorDeck {
                    channel_uuid: channel_uuid.to_string(),
                    color: [shade, shade, shade, 1.0],
                },
            ))
        })
        .collect()
}

fn deck_opacity(app: &VardaApp, deck_uuid: &str) -> f32 {
    app.build_engine_state()
        .mixer
        .channels
        .iter()
        .flat_map(|ch| ch.decks.iter())
        .find(|d| d.uuid == deck_uuid)
        .map(|d| d.opacity)
        .unwrap_or_else(|| panic!("deck {deck_uuid} not found"))
}

// ── The race this migration exists to close ────────────────────────

/// A write issued against a UUID must reach that deck even when an unrelated
/// removal shifted every position in the channel first.
///
/// Positional addressing failed here: the client resolved deck D at index 3,
/// another client removed deck A, and the write to index 3 landed nowhere (or,
/// with a fourth deck present, on the wrong deck) without reporting an error.
#[test]
fn removal_does_not_repoint_a_pending_write() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&app, 0);
    let decks = add_decks(&mut app, &ch, 4);

    // Client reads state and decides to dim the last deck.
    let target = decks[3].clone();

    // Meanwhile another client removes the first deck; every later deck shifts
    // down one position.
    fire(
        &mut app,
        EngineCommand::RemoveDeck {
            deck_uuid: decks[0].clone(),
        },
    );

    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: target.clone(),
            opacity: 0.25,
        },
    );

    assert!(
        (deck_opacity(&app, &target) - 0.25).abs() < 1e-4,
        "write did not reach its target deck after a reindex"
    );
    // The decks that shifted into the vacated positions are untouched.
    for other in &decks[1..3] {
        assert!(
            (deck_opacity(&app, other) - 1.0).abs() < 1e-4,
            "write leaked onto deck {other}"
        );
    }
}

/// Reordering within a channel likewise must not repoint a write.
#[test]
fn reorder_does_not_repoint_a_pending_write() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&app, 0);
    let decks = add_decks(&mut app, &ch, 3);
    let target = decks[0].clone();

    fire(
        &mut app,
        EngineCommand::ReorderDeck {
            channel_uuid: ch.clone(),
            from_idx: 0,
            to_idx: 2,
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: target.clone(),
            opacity: 0.1,
        },
    );

    assert!((deck_opacity(&app, &target) - 0.1).abs() < 1e-4);
    for other in &decks[1..] {
        assert!(
            (deck_opacity(&app, other) - 1.0).abs() < 1e-4,
            "write leaked onto deck {other}"
        );
    }
}

/// Moving a deck to another channel keeps its UUID, so a write issued before
/// the move still finds it.
#[test]
fn cross_channel_move_preserves_addressability() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch0 = channel_uuid(&app, 0);
    let ch1 = channel_uuid(&app, 1);
    let deck = add_decks(&mut app, &ch0, 1).remove(0);

    fire(
        &mut app,
        EngineCommand::MoveDeck {
            deck_uuid: deck.clone(),
            dst_channel_uuid: ch1.clone(),
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.5,
        },
    );

    assert!((deck_opacity(&app, &deck) - 0.5).abs() < 1e-4);
    let state = app.build_engine_state();
    assert!(state.mixer.channels[0].decks.is_empty());
    assert_eq!(state.mixer.channels[1].decks[0].uuid, deck);
}

// ── Unresolvable addresses report NotFound ─────────────────────────

#[test]
fn unknown_uuids_report_not_found() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let missing = "deadbeef".to_string();
    let cases: Vec<EngineCommand> = vec![
        EngineCommand::SetDeckOpacity {
            deck_uuid: missing.clone(),
            opacity: 0.5,
        },
        EngineCommand::RemoveDeck {
            deck_uuid: missing.clone(),
        },
        EngineCommand::SetChannelOpacity {
            channel_uuid: missing.clone(),
            opacity: 0.5,
        },
        EngineCommand::RemoveChannel {
            channel_uuid: missing.clone(),
        },
        EngineCommand::AddSolidColorDeck {
            channel_uuid: missing.clone(),
            color: [1.0, 1.0, 1.0, 1.0],
        },
        EngineCommand::ToggleEffect {
            effect_uuid: missing.clone(),
        },
        EngineCommand::RemoveEffect {
            effect_uuid: missing.clone(),
        },
        EngineCommand::StopOutput {
            output_uuid: missing.clone(),
        },
        EngineCommand::PlaySequence {
            sequence_uuid: missing.clone(),
        },
        EngineCommand::DeleteSequence {
            sequence_uuid: missing.clone(),
        },
    ];
    for cmd in cases {
        let label = format!("{cmd:?}");
        match send_cmd(&mut app, cmd) {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                ..
            } => {}
            other => panic!("{label} should report NotFound, got {other:?}"),
        }
    }
}

/// A removed entity's UUID must not be reusable — writes to it fail rather than
/// silently landing on whatever took its place.
#[test]
fn writes_to_a_removed_deck_fail() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&app, 0);
    let decks = add_decks(&mut app, &ch, 2);
    fire(
        &mut app,
        EngineCommand::RemoveDeck {
            deck_uuid: decks[0].clone(),
        },
    );

    let r = send_cmd(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: decks[0].clone(),
            opacity: 0.0,
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
        "{r:?}"
    );
    // The surviving deck is untouched.
    assert!((deck_opacity(&app, &decks[1]) - 1.0).abs() < 1e-4);
}

// ── Effects: one UUID space across deck, channel and master chains ──

#[test]
fn effect_uuids_resolve_in_every_chain() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&app, 0);
    let deck = add_decks(&mut app, &ch, 1).remove(0);

    let targets = [
        EffectTarget::Deck(deck.clone()),
        EffectTarget::Channel(ch.clone()),
        EffectTarget::Master,
    ];
    for target in targets {
        let label = format!("{target:?}");
        let uuid = new_uuid(send_cmd(
            &mut app,
            EngineCommand::AddEffect {
                target,
                shader_name: "invert".to_string(),
            },
        ));
        let r = send_cmd(&mut app, EngineCommand::ToggleEffect { effect_uuid: uuid });
        assert!(
            matches!(r, CommandResult::Ok),
            "toggle by UUID failed for {label}: {r:?}"
        );
    }
}

/// Removing an effect earlier in a chain must not repoint a write aimed at a
/// later one.
#[test]
fn effect_removal_does_not_repoint_a_pending_write() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let ch = channel_uuid(&app, 0);
    let deck = add_decks(&mut app, &ch, 1).remove(0);

    let effects: Vec<String> = (0..2)
        .map(|_| {
            new_uuid(send_cmd(
                &mut app,
                EngineCommand::AddEffect {
                    target: EffectTarget::Deck(deck.clone()),
                    shader_name: "invert".to_string(),
                },
            ))
        })
        .collect();

    fire(
        &mut app,
        EngineCommand::RemoveEffect {
            effect_uuid: effects[0].clone(),
        },
    );
    let r = send_cmd(
        &mut app,
        EngineCommand::ToggleEffect {
            effect_uuid: effects[1].clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");

    let state = app.build_engine_state();
    let chain = &state.mixer.channels[0].decks[0].effects;
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].uuid, effects[1]);
    assert!(!chain[0].enabled, "toggle hit the wrong effect");
}

// ── Sequences ──────────────────────────────────────────────────────

#[test]
fn sequence_writes_survive_deletion_of_an_earlier_sequence() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let first = new_uuid(send_cmd(&mut app, EngineCommand::CreateSequence));
    let second = new_uuid(send_cmd(&mut app, EngineCommand::CreateSequence));

    fire(
        &mut app,
        EngineCommand::DeleteSequence {
            sequence_uuid: first,
        },
    );
    let r = send_cmd(
        &mut app,
        EngineCommand::ToggleSequence {
            sequence_uuid: second.clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");

    let seqs = app.build_engine_state().mixer.sequences;
    assert_eq!(seqs.len(), 1);
    assert_eq!(seqs[0].uuid, second);
    assert!(!seqs[0].enabled, "toggle hit the wrong sequence");
}

/// A fade step holds channel UUIDs, so deleting a channel that a step does not
/// reference must not repoint the step at a different channel.
#[test]
fn fade_steps_hold_channel_uuids() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let extra = new_uuid(send_cmd(&mut app, EngineCommand::AddChannel));
    let ch1 = channel_uuid(&app, 1);
    let seq = new_uuid(send_cmd(&mut app, EngineCommand::CreateSequence));
    let r = send_cmd(
        &mut app,
        EngineCommand::AddFadeStep {
            sequence_uuid: seq.clone(),
            from_channel_uuid: ch1.clone(),
            to_channel_uuid: extra.clone(),
        },
    );
    assert!(matches!(r, CommandResult::Ok), "{r:?}");

    // Delete channel 0, which the step does not reference. Under positional
    // addressing this would shift ch1 to index 0 and silently repoint the step.
    let ch0 = channel_uuid(&app, 0);
    fire(&mut app, EngineCommand::RemoveChannel { channel_uuid: ch0 });

    let state = app.build_engine_state();
    let step = &state.mixer.sequences[0].steps[0];
    match &step.kind {
        varda::engine::SequenceStepKindSnapshot::Fade { from_ch, to_ch, .. } => {
            assert_eq!(from_ch, &ch1);
            assert_eq!(to_ch, &extra);
        }
        other => panic!("expected a fade step, got {other:?}"),
    }
}
