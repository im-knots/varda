//! Persistence integration tests — save/load roundtrips with tempdir workspaces.

use varda::app::{AppConfig, VardaApp};
use varda::engine::{BlendMode, CommandResult, EffectTarget, EngineCommand};
use varda::modulation::LFOWaveform;
use varda::usecases::ui::UILayoutState;

use clap::Parser;
use tempfile::TempDir;

mod common;

fn parse_args(args: &[&str]) -> AppConfig {
    AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
}

fn headless_app_in(workspace: &std::path::Path) -> Option<VardaApp> {
    let gpu = common::headless_gpu()?;
    let ws = workspace.to_str().unwrap();
    let config = parse_args(&[
        "--headless",
        "--no-osc",
        "--no-ndi",
        "--no-syphon",
        "--workspace",
        ws,
    ]);
    // Once a GPU exists, a construction failure is a bug, not a reason to skip.
    Some(VardaApp::new(gpu, &config).expect("VardaApp::new"))
}

fn send_cmd(app: &mut VardaApp, cmd: EngineCommand) -> CommandResult {
    let tx = app.command_sender();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    tx.send((cmd, Some(reply_tx))).unwrap();
    app.process_commands();
    reply_rx.blocking_recv().unwrap()
}

fn fire(app: &mut VardaApp, cmd: EngineCommand) {
    app.command_sender().send((cmd, None)).unwrap();
    app.process_commands();
}

/// UUID of the channel currently at `idx`.
fn channel_uuid(app: &mut VardaApp, idx: usize) -> String {
    app.build_engine_state().mixer.channels[idx].uuid.clone()
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn save_load_empty_workspace() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    app.save_workspace(&UILayoutState::default());
    // Reload
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 2);
}

#[test]
fn save_load_with_decks() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
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
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!(
        !state.mixer.channels[0].decks.is_empty(),
        "deck should survive roundtrip"
    );
}

#[test]
fn save_load_crossfader_position() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(&mut app, EngineCommand::SetCrossfader(0.75));
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!((state.mixer.crossfader - 0.75).abs() < 1e-4);
}

#[test]
fn save_load_modulation_sources() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    send_cmd(
        &mut app,
        EngineCommand::AddLfo {
            waveform: LFOWaveform::Sine,
            frequency: 2.0,
        },
    );
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!(
        !state.modulation.sources.is_empty(),
        "LFO should survive roundtrip"
    );
}

#[test]
fn save_load_render_resolution() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetRenderResolution {
            width: 1280,
            height: 720,
        },
    );
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    assert_eq!(app2.render_width(), 1280);
    assert_eq!(app2.render_height(), 720);
}

#[test]
fn save_load_multiple_channels() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(&mut app, EngineCommand::AddChannel);
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 3);
}

#[test]
fn load_missing_assets_graceful() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    // Add a video deck with a non-existent path
    let ch = channel_uuid(&mut app, 0);
    let _ = send_cmd(
        &mut app,
        EngineCommand::AddVideoDeck {
            channel_uuid: ch,
            path: std::path::PathBuf::from("/nonexistent/path/video.mp4"),
        },
    );
    app.save_workspace(&UILayoutState::default());
    // Reload — should not crash
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
}

#[test]
fn save_creates_varda_directory() {
    let tmp = TempDir::new().unwrap();
    let varda_dir = tmp.path().join(".varda");
    assert!(!varda_dir.exists());
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    app.save_workspace(&UILayoutState::default());
    assert!(varda_dir.exists());
}

#[test]
fn scene_json_valid_format() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    app.save_workspace(&UILayoutState::default());
    let scene_path = tmp.path().join(".varda").join("scene.json");
    let content = std::fs::read_to_string(scene_path).expect("scene.json should exist");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("should be valid JSON");
    assert!(parsed.is_object());
    assert!(parsed.get("channels").is_some());
}

#[test]
fn save_load_deck_fidelity_opacity_transparent_blend() {
    // A solid-color deck's opacity, transparent flag, and blend mode must all
    // survive the real snapshot_scene -> disk -> restore_scene path, not just
    // the deck's existence.
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = match send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [0.25, 0.5, 0.75, 1.0],
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    };
    fire(
        &mut app,
        EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.42,
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckTransparent {
            deck_uuid: deck.clone(),
            transparent: true,
        },
    );
    fire(
        &mut app,
        EngineCommand::SetDeckBlendMode {
            deck_uuid: deck,
            mode: BlendMode::Multiply,
        },
    );
    app.save_workspace(&UILayoutState::default());

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    let restored = &state.mixer.channels[0].decks[0];
    assert!(
        (restored.opacity - 0.42).abs() < 1e-4,
        "opacity should survive: {}",
        restored.opacity
    );
    assert!(restored.transparent, "transparent flag should survive");
    assert_eq!(
        restored.blend_mode,
        BlendMode::Multiply,
        "blend mode should survive"
    );
}

#[test]
fn save_load_deck_effect_survives() {
    // A deck effect (ISF filter) and its enabled state must survive the
    // roundtrip. If the effect shader isn't available in this build the add
    // fails gracefully and the assertion is skipped (mirrors engine tests).
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck = match send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    };
    let effect_name = match send_cmd(
        &mut app,
        EngineCommand::AddEffect {
            target: EffectTarget::Deck(deck),
            shader_name: "invert".to_string(),
        },
    ) {
        CommandResult::OkWithId { .. } => "invert",
        // Effect shader unavailable in this build — nothing to assert.
        _ => return,
    };
    app.save_workspace(&UILayoutState::default());

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    let restored = &state.mixer.channels[0].decks[0];
    assert_eq!(
        restored.effects.len(),
        1,
        "one deck effect should survive roundtrip"
    );
    assert_eq!(restored.effects[0].name, effect_name);
    assert!(
        restored.effects[0].enabled,
        "effect should be enabled after roundtrip"
    );
}

#[test]
fn save_load_channel_opacity() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    fire(
        &mut app,
        EngineCommand::SetChannelOpacity {
            channel_uuid: ch,
            opacity: 0.5,
        },
    );
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!((state.mixer.channels[0].opacity - 0.5).abs() < 1e-4);
}
