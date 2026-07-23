//! Round-trip command tests verifying the Step 5 delegation layer.
//!
//! Each test sends a command through `execute_command()` via the command
//! channel, then verifies the resulting state through `build_engine_state()`.

use varda::app::{AppConfig, VardaApp};
use varda::engine::{CommandResult, EngineCommand, ErrorCode};

use clap::Parser;

fn parse_args(args: &[&str]) -> AppConfig {
    AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
}

fn headless_app() -> Option<VardaApp> {
    let gpu = varda::renderer::context::GpuContext::new_headless().ok()?;
    let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
    VardaApp::new(gpu, &config).ok()
}

fn send_cmd(app: &mut VardaApp, cmd: EngineCommand) -> CommandResult {
    let tx = app.command_sender();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send((cmd, Some(reply_tx))).unwrap();
    app.process_commands();
    reply_rx.blocking_recv().unwrap()
}

// ── Surface Commands ────────────────────────────────────────────────

#[test]
fn surface_add_and_remove_roundtrip() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::AddSurface {
            name: "Test Surface".into(),
            source: varda::renderer::context::OutputSource::Master,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert!(state
        .outputs
        .surfaces
        .iter()
        .any(|s| s.name == "Test Surface"));
    let uuid = state
        .outputs
        .surfaces
        .iter()
        .find(|s| s.name == "Test Surface")
        .unwrap()
        .uuid
        .clone();
    let r = send_cmd(&mut app, EngineCommand::RemoveSurface { uuid });
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert!(!state
        .outputs
        .surfaces
        .iter()
        .any(|s| s.name == "Test Surface"));
}

#[test]
fn surface_duplicate_roundtrip() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddSurface {
            name: "Original".into(),
            source: varda::renderer::context::OutputSource::Master,
        },
    );
    let uuid = app
        .build_engine_state()
        .outputs
        .surfaces
        .iter()
        .find(|s| s.name == "Original")
        .unwrap()
        .uuid
        .clone();
    let r = send_cmd(&mut app, EngineCommand::DuplicateSurface { uuid });
    assert!(matches!(r, CommandResult::OkWithId { .. }));
    let state = app.build_engine_state();
    // Should have original + duplicate
    let originals: Vec<_> = state
        .outputs
        .surfaces
        .iter()
        .filter(|s| s.name.starts_with("Original"))
        .collect();
    assert!(originals.len() >= 2);
}

#[test]
fn surface_flip_and_move() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddSurface {
            name: "Flip Test".into(),
            source: varda::renderer::context::OutputSource::Master,
        },
    );
    let uuid = app
        .build_engine_state()
        .outputs
        .surfaces
        .iter()
        .find(|s| s.name == "Flip Test")
        .unwrap()
        .uuid
        .clone();
    let r = send_cmd(
        &mut app,
        EngineCommand::FlipSurfaceHorizontal { uuid: uuid.clone() },
    );
    assert!(matches!(r, CommandResult::Ok));
    let r = send_cmd(
        &mut app,
        EngineCommand::FlipSurfaceVertical { uuid: uuid.clone() },
    );
    assert!(matches!(r, CommandResult::Ok));
    let r = send_cmd(
        &mut app,
        EngineCommand::MoveSurface {
            uuid,
            dx: 0.1,
            dy: -0.1,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
}

#[test]
fn surface_reorder_roundtrip() {
    use varda::surface::SurfaceReorderOp;
    let Some(mut app) = headless_app() else {
        return;
    };
    for name in ["A", "B", "C"] {
        send_cmd(
            &mut app,
            EngineCommand::AddSurface {
                name: name.into(),
                source: varda::renderer::context::OutputSource::Master,
            },
        );
    }
    let order = |app: &mut VardaApp| -> Vec<String> {
        app.build_engine_state()
            .outputs
            .surfaces
            .iter()
            .map(|s| s.name.clone())
            .collect()
    };
    assert_eq!(order(&mut app), vec!["A", "B", "C"]);

    let a = app
        .build_engine_state()
        .outputs
        .surfaces
        .iter()
        .find(|s| s.name == "A")
        .unwrap()
        .uuid
        .clone();

    // Index 0 = bottom, last = top. Bring A to front.
    let r = send_cmd(
        &mut app,
        EngineCommand::ReorderSurface {
            uuid: a.clone(),
            op: SurfaceReorderOp::ToFront,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    assert_eq!(order(&mut app), vec!["B", "C", "A"]);

    // Nudge A down one step (toward back).
    let r = send_cmd(
        &mut app,
        EngineCommand::ReorderSurface {
            uuid: a,
            op: SurfaceReorderOp::Down,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    assert_eq!(order(&mut app), vec!["B", "A", "C"]);

    // Unknown surface → NotFound.
    let r = send_cmd(
        &mut app,
        EngineCommand::ReorderSurface {
            uuid: "does-not-exist".into(),
            op: SurfaceReorderOp::ToBack,
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

// ── Sequence Commands ───────────────────────────────────────────────

#[test]
fn sequence_full_lifecycle() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(&mut app, EngineCommand::CreateSequence);
    assert!(matches!(r, CommandResult::Ok));
    assert_eq!(app.build_engine_state().mixer.sequences.len(), 1);

    // Add steps
    send_cmd(
        &mut app,
        EngineCommand::AddFadeStep {
            seq_idx: 0,
            from_ch: 0,
            to_ch: 1,
        },
    );
    send_cmd(&mut app, EngineCommand::AddWaitStep { seq_idx: 0 });
    send_cmd(
        &mut app,
        EngineCommand::AddGoToStep {
            seq_idx: 0,
            step_index: 0,
        },
    );
    assert_eq!(app.build_engine_state().mixer.sequences[0].steps.len(), 3);

    // Remove middle step
    send_cmd(
        &mut app,
        EngineCommand::RemoveStep {
            seq_idx: 0,
            step_idx: 1,
        },
    );
    assert_eq!(app.build_engine_state().mixer.sequences[0].steps.len(), 2);

    // Move step
    send_cmd(
        &mut app,
        EngineCommand::MoveStep {
            seq_idx: 0,
            from: 0,
            to: 1,
        },
    );

    // Delete sequence
    let r = send_cmd(&mut app, EngineCommand::DeleteSequence { idx: 0 });
    assert!(matches!(r, CommandResult::Ok));
    assert_eq!(app.build_engine_state().mixer.sequences.len(), 0);
}

#[test]
fn sequence_oob_returns_not_found() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(&mut app, EngineCommand::DeleteSequence { idx: 99 });
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::NotFound,
            ..
        }
    ));
    let r = send_cmd(
        &mut app,
        EngineCommand::AddFadeStep {
            seq_idx: 99,
            from_ch: 0,
            to_ch: 1,
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

// ── Output Commands ─────────────────────────────────────────────────

#[test]
fn headless_output_create_and_stop() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: varda::renderer::context::OutputTarget::NdiSend {
                sender_name: "Test NDI".into(),
            },
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    // Verify engine state can be built after the headless output command.
    // Headless outputs (e.g. NDI send) do not necessarily appear in the
    // windows list, so we only assert the state builds without panicking.
    let _state = app.build_engine_state();
}

#[test]
fn headless_output_syphon_create_and_start() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Create a headless output targeting a Syphon server — the same path the
    // API takes for `POST /api/outputs/headless` with a SyphonServer target.
    // This proves the API is co-equal with the UI's Syphon protocol dropdown.
    let r = send_cmd(
        &mut app,
        EngineCommand::CreateHeadlessOutput {
            target: varda::renderer::context::OutputTarget::SyphonServer {
                server_name: "Test Syphon".into(),
            },
        },
    );
    assert!(matches!(r, CommandResult::Ok));

    // A fresh headless app starts with no outputs, so the created Syphon output
    // is at index 0. Starting it activates the publisher on macOS; on other
    // platforms it must be rejected with Unavailable, mirroring the Syphon
    // receive deck path (cmd_add_syphon_deck).
    let r = send_cmd(&mut app, EngineCommand::StartOutput { idx: 0 });
    #[cfg(target_os = "macos")]
    assert!(matches!(r, CommandResult::Ok));
    #[cfg(not(target_os = "macos"))]
    assert!(matches!(
        r,
        CommandResult::Err {
            code: ErrorCode::Unavailable,
            ..
        }
    ));

    let _state = app.build_engine_state();
}

// ── Stream Library Commands ─────────────────────────────────────────

#[test]
fn hls_library_add_remove() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::AddHlsLibraryEntry {
            url: "http://example.com/stream.m3u8".into(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    // Add duplicate — should be idempotent
    send_cmd(
        &mut app,
        EngineCommand::AddHlsLibraryEntry {
            url: "http://example.com/stream.m3u8".into(),
        },
    );
    // Remove
    let r = send_cmd(
        &mut app,
        EngineCommand::RemoveHlsLibraryEntry {
            url: "http://example.com/stream.m3u8".into(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
}

// ── DeckSource Kind Verification ────────────────────────────────────

#[test]
fn solid_color_deck_source_kind() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_idx: 0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    // Deck-creating commands report the new deck's UUID (see ui-engine-boundary.md WS1).
    assert!(matches!(r, CommandResult::OkWithId { .. }));
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels[0].decks.len(), 1);
    // Solid color deck name is the hex color, not "Solid Color"
    assert!(!state.mixer.channels[0].decks[0].name.is_empty());
}

// ── Presets & ToggleParam ───────────────────────────────────────────

fn headless_app_in(dir: &std::path::Path) -> Option<VardaApp> {
    let gpu = varda::renderer::context::GpuContext::new_headless().ok()?;
    let ws = dir.to_str().unwrap();
    let config = parse_args(&[
        "--headless",
        "--no-osc",
        "--no-ndi",
        "--no-syphon",
        "--workspace",
        ws,
    ]);
    VardaApp::new(gpu, &config).ok()
}

#[test]
fn toggle_param_flips_crossfader() {
    let Some(mut app) = headless_app() else {
        return;
    };
    // Default crossfader is 0.0; toggling snaps it to the opposite extreme (1.0).
    let r = send_cmd(
        &mut app,
        EngineCommand::ToggleParam {
            path: "crossfader".into(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    assert!((app.build_engine_state().mixer.crossfader - 1.0).abs() < 1e-5);
    // Toggling again snaps back to 0.0.
    send_cmd(
        &mut app,
        EngineCommand::ToggleParam {
            path: "crossfader".into(),
        },
    );
    assert!(app.build_engine_state().mixer.crossfader.abs() < 1e-5);
}

#[test]
fn load_deck_preset_out_of_range_errors() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::LoadDeckPreset {
            channel_idx: 0,
            preset_idx: 99,
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
fn load_channel_preset_out_of_range_errors() {
    let Some(mut app) = headless_app() else {
        return;
    };
    let r = send_cmd(
        &mut app,
        EngineCommand::LoadChannelPreset {
            target_channel: None,
            preset_idx: 99,
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
fn deck_preset_save_then_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let Some(mut app) = headless_app_in(dir.path()) else {
        return;
    };
    // Create a source deck to save.
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_idx: 0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    );
    assert_eq!(app.build_engine_state().mixer.channels[0].decks.len(), 1);
    // Save it as a preset (writes to the temp workspace + refreshes the library).
    let r = send_cmd(
        &mut app,
        EngineCommand::SaveDeckPreset {
            channel_idx: 0,
            deck_idx: 0,
            name: "red".into(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    // Load the saved preset back into channel 0 → a second deck is appended.
    let r = send_cmd(
        &mut app,
        EngineCommand::LoadDeckPreset {
            channel_idx: 0,
            preset_idx: 0,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    assert_eq!(app.build_engine_state().mixer.channels[0].decks.len(), 2);
}

#[test]
fn channel_preset_save_then_load_appends_channel() {
    let dir = tempfile::tempdir().unwrap();
    let Some(mut app) = headless_app_in(dir.path()) else {
        return;
    };
    // Populate channel 0 with a deck, then save the channel as a preset.
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_idx: 0,
            color: [0.0, 1.0, 0.0, 1.0],
        },
    );
    let r = send_cmd(
        &mut app,
        EngineCommand::SaveChannelPreset {
            channel_idx: 0,
            name: "green".into(),
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let channels_before = app.build_engine_state().mixer.channels.len();
    // Load with no target → a new channel is appended carrying the preset's deck.
    let r = send_cmd(
        &mut app,
        EngineCommand::LoadChannelPreset {
            target_channel: None,
            preset_idx: 0,
        },
    );
    assert!(matches!(r, CommandResult::Ok));
    let state = app.build_engine_state();
    assert_eq!(state.mixer.channels.len(), channels_before + 1);
    assert_eq!(state.mixer.channels[channels_before].decks.len(), 1);
}

// ── Headless Render Smoke ───────────────────────────────────────────

#[test]
fn headless_render_smoke() {
    let Some(mut app) = headless_app() else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_idx: 0,
            color: [0.0, 1.0, 0.0, 1.0],
        },
    );
    for _ in 0..10 {
        app.update_frame_timing();
        app.render_mixer_frame();
    }
    let state = app.build_engine_state();
    assert!(state.fps >= 0.0);
    // frame_count is incremented during render_outputs, not render_mixer_frame in headless
    // Just verify no crash and FPS is valid
}
