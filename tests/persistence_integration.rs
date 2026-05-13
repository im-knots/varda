//! Persistence integration tests — save/load roundtrips with tempdir workspaces.

use varda::app::{AppConfig, VardaApp};
use varda::engine::{CommandResult, EngineCommand};
use varda::modulation::LFOWaveform;
use varda::usecases::ui::UILayoutState;

use clap::Parser;
use tempfile::TempDir;

fn parse_args(args: &[&str]) -> AppConfig {
    AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
}

fn headless_app_in(workspace: &std::path::Path) -> Option<VardaApp> {
    let gpu = varda::renderer::context::GpuContext::new_headless().ok()?;
    let ws = workspace.to_str().unwrap();
    let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon", "--workspace", ws]);
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
    app.command_sender().send((cmd, None)).unwrap();
    app.process_commands();
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn save_load_empty_workspace() {
    let tmp = TempDir::new().unwrap();
    let Some(app) = headless_app_in(tmp.path()) else { return; };
    app.save_workspace(&UILayoutState::default());
    // Reload
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 2);
}

#[test]
fn save_load_with_decks() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    send_cmd(&mut app, EngineCommand::AddSolidColorDeck { channel_idx: 0, color: [1.0, 0.0, 0.0, 1.0] });
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!(state.mixer.channels[0].decks.len() >= 1, "deck should survive roundtrip");
}

#[test]
fn save_load_crossfader_position() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    fire(&mut app, EngineCommand::SetCrossfader(0.75));
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!((state.mixer.crossfader - 0.75).abs() < 1e-4);
}

#[test]
fn save_load_modulation_sources() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    send_cmd(&mut app, EngineCommand::AddLfo { waveform: LFOWaveform::Sine, frequency: 2.0 });
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!(!state.modulation.sources.is_empty(), "LFO should survive roundtrip");
}

#[test]
fn save_load_render_resolution() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    fire(&mut app, EngineCommand::SetRenderResolution { width: 1280, height: 720 });
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    assert_eq!(app2.render_width(), 1280);
    assert_eq!(app2.render_height(), 720);
}

#[test]
fn save_load_multiple_channels() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    fire(&mut app, EngineCommand::AddChannel);
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 3);
}

#[test]
fn load_missing_assets_graceful() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    // Add a video deck with a non-existent path
    let _ = send_cmd(&mut app, EngineCommand::AddVideoDeck {
        channel_idx: 0,
        path: std::path::PathBuf::from("/nonexistent/path/video.mp4"),
    });
    app.save_workspace(&UILayoutState::default());
    // Reload — should not crash
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
}

#[test]
fn save_creates_varda_directory() {
    let tmp = TempDir::new().unwrap();
    let varda_dir = tmp.path().join(".varda");
    assert!(!varda_dir.exists());
    let Some(app) = headless_app_in(tmp.path()) else { return; };
    app.save_workspace(&UILayoutState::default());
    assert!(varda_dir.exists());
}

#[test]
fn scene_json_valid_format() {
    let tmp = TempDir::new().unwrap();
    let Some(app) = headless_app_in(tmp.path()) else { return; };
    app.save_workspace(&UILayoutState::default());
    let scene_path = tmp.path().join(".varda").join("scene.json");
    let content = std::fs::read_to_string(scene_path).expect("scene.json should exist");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("should be valid JSON");
    assert!(parsed.is_object());
    assert!(parsed.get("channels").is_some());
}

#[test]
fn save_load_channel_opacity() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else { return; };
    fire(&mut app, EngineCommand::SetChannelOpacity { channel_idx: 0, opacity: 0.5 });
    app.save_workspace(&UILayoutState::default());
    let Some(mut app2) = headless_app_in(tmp.path()) else { return; };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!((state.mixer.channels[0].opacity - 0.5).abs() < 1e-4);
}
