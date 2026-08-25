//! Persistence integration tests — save/load roundtrips with tempdir workspaces.

use varda::app::{AppConfig, VardaApp};
use varda::engine::{BlendMode, CommandResult, EffectTarget, EngineCommand};
use varda::modulation::LFOWaveform;
use varda::timebase::Timebase;
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
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

/// A beat-locked modulator must still be beat-locked after a reload, otherwise
/// a saved show silently reverts to wall time. See /spec/timebase.md.
#[test]
fn save_load_modulation_timebase() {
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
    let uuid = app.build_engine_state().modulation.sources[0].uuid.clone();
    fire(
        &mut app,
        EngineCommand::UpdateModulationTimebase {
            uuid,
            timebase: Timebase::Beat,
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(
        state.modulation.sources[0].timebase,
        Timebase::Beat,
        "timebase should survive a save/load roundtrip"
    );
}

/// An automation curve is only worth drawing if it survives the save. This also
/// covers `AssignmentMode::Absolute` persisting, since a curve that reloaded as
/// additive would ride on the fader instead of setting it.
/// See /spec/automation.md § Persistence.
#[test]
fn save_load_automation_envelope() {
    use varda::modulation::{Breakpoint, CurveKind};

    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck_uuid = match send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    };
    let target = format!("deck_{deck_uuid}:opacity");

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

    let drawn = vec![
        Breakpoint::new(0.0, 0.1),
        Breakpoint::new(8.5, 0.9).with_curve(CurveKind::Smooth),
    ];
    fire(
        &mut app,
        EngineCommand::SetEnvelopeBreakpoints {
            uuid: uuid.clone(),
            breakpoints: drawn.clone(),
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();

    let entry = state
        .modulation
        .sources
        .iter()
        .find(|s| s.uuid == uuid)
        .expect("envelope should survive the reload");
    assert_eq!(entry.timebase, Timebase::Transport);
    let varda::engine::types::ModulationSourceSnapshot::Envelope { breakpoints } = &entry.source
    else {
        panic!("expected the reloaded source to still be an envelope");
    };
    assert_eq!(breakpoints, &drawn, "the drawn curve should round-trip");

    let assigned = state
        .modulation
        .assignments
        .get(&target)
        .expect("the assignment should survive the reload");
    assert!(assigned.iter().any(|a| a.source_id == uuid));
}

/// Everything a timeline edit produces has to survive the file, or the show
/// authored on Friday is not the show that opens on Saturday.
/// See /spec/arrangement.md § Storage.
#[test]
fn save_load_arrangement_edits() {
    use varda::arrangement::RegionConfig;

    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let deck_uuid = match send_cmd(
        &mut app,
        EngineCommand::AddSolidColorDeck {
            channel_uuid: ch,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ) {
        CommandResult::OkWithId { uuid } => uuid,
        other => panic!("expected OkWithId, got {other:?}"),
    };

    // Author as the panel does: drop a region, then drag it into shape.
    fire(
        &mut app,
        EngineCommand::AddRegion {
            deck_uuid: deck_uuid.clone(),
            region: RegionConfig::new(4.0, 8.0),
        },
    );
    let shaped = RegionConfig::new(4.5, 12.25).with_fades(0.5, 1.5);
    fire(
        &mut app,
        EngineCommand::UpdateRegion {
            deck_uuid: deck_uuid.clone(),
            index: 0,
            region: shaped,
        },
    );
    fire(
        &mut app,
        EngineCommand::SetLaneCollapsed {
            deck_uuid: deck_uuid.clone(),
            collapsed: true,
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();

    let arrangement = state
        .arrangement
        .expect("the arrangement should survive the reload");
    let lane = arrangement
        .config
        .lane(&deck_uuid)
        .expect("the lane should still name its deck");
    assert_eq!(lane.regions, vec![shaped], "the edited span should reload");
    assert!(lane.collapsed, "a folded lane should reload folded");

    // The region compiles back to an opacity curve on load, so the deck is
    // driven without anyone having to touch it again.
    let key = varda::arrangement::opacity_param_key(&deck_uuid);
    assert!(
        state.modulation.assignments.contains_key(&key),
        "the reloaded region should drive the deck's opacity"
    );
}

/// Cues are how a show is navigated, so they belong to the scene rather than to
/// the session that dropped them. See /spec/arrangement.md § Cue points.
#[test]
fn save_load_cue_points() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::AddCue {
            at: 64.5,
            name: "Drop".to_string(),
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let cues = app2
        .build_engine_state()
        .arrangement
        .expect("an arrangement holding only cues should still reload")
        .config
        .cues;

    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].name, "Drop");
    assert!((cues[0].at - 64.5).abs() < 1e-9);
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    assert_eq!(app2.render_width(), 1280);
    assert_eq!(app2.render_height(), 720);
}

/// Which signal a rig follows belongs to the venue, so it comes back with the
/// stage rather than with the show.
#[test]
fn save_load_timecode_preference() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetTimecodePreference {
            preference: varda::timecode::TimecodePreference::Off,
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    assert_eq!(
        app2.build_engine_state().timecode.preference,
        varda::timecode::TimecodePreference::Off,
        "ignoring timecode is a decision, so it must survive a restart"
    );
}

/// The patch is written down as the name of a box, never the slot it enumerated
/// in: ids are handed out at scan time and move whenever the rig changes between
/// load-ins, so a saved id would point at whatever interface came up in that slot
/// tonight and the show would chase silence.
#[test]
fn save_load_ltc_patch_by_interface_name() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let Some(device) = app.build_engine_state().audio.devices.last().cloned() else {
        return;
    };
    let patched = varda::timecode::LtcInput {
        source_id: device.id,
        channel: 1,
        rate: None,
    };
    fire(
        &mut app,
        EngineCommand::SetLtcInput {
            input: Some(patched),
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let stage: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".varda").join("stage.json"))
            .expect("stage.json should exist"),
    )
    .expect("stage.json should be valid JSON");
    let saved = &stage["timecode"]["ltc_input"];
    assert_eq!(saved["device"], device.name);
    assert_eq!(saved["channel"], 1);
    assert!(
        saved.get("source_id").is_none(),
        "an id in the file would point at whatever enumerates in that slot next time"
    );

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    assert_eq!(
        app2.build_engine_state().timecode.ltc_input,
        Some(patched),
        "the name resolves to the id that interface holds now"
    );
}

/// A stage naming an interface the rig no longer has must leave the patch unset
/// rather than reading timecode off whichever cable took its number.
#[test]
fn load_ltc_patch_for_a_missing_interface_leaves_it_unset() {
    let tmp = TempDir::new().unwrap();
    let varda_dir = tmp.path().join(".varda");
    std::fs::create_dir_all(&varda_dir).unwrap();
    std::fs::write(
        varda_dir.join("stage.json"),
        r#"{"timecode":{"preference":"ForceLtc",
            "ltc_input":{"device":"Scarlett 2i2 That Stayed Home","channel":1}}}"#,
    )
    .unwrap();

    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app.load_workspace();

    let timecode = app.build_engine_state().timecode;
    assert_eq!(timecode.ltc_input, None);
    assert_eq!(
        timecode.preference,
        varda::timecode::TimecodePreference::ForceLtc,
        "the decision to follow LTC is kept, so the popover reads as waiting on an input"
    );
}

#[test]
fn save_load_domemaster_resolution() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(
        &mut app,
        EngineCommand::SetDomemasterResolution {
            resolution: varda::renderer::dome::DomemasterResolution::R4K,
        },
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    assert_eq!(
        app2.domemaster_resolution(),
        varda::renderer::dome::DomemasterResolution::R4K,
        "the dome belongs to the venue, so its size must come back with the stage"
    );
}

#[test]
fn save_load_multiple_channels() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    fire(&mut app, EngineCommand::AddChannel);
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(state.mixer.channels.len(), 3);
}

#[test]
fn save_load_svg_image_deck() {
    // SVG rides the existing image source config — no new scene.json shape —
    // so the roundtrip has to prove the restored deck comes back rasterized
    // rather than failing the way an unknown format would.
    let tmp = TempDir::new().unwrap();
    let art = tmp.path().join("logo.svg");
    std::fs::write(
        &art,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 50"
            width="200" height="50"><rect width="200" height="50" fill="#20c0a0"/></svg>"##,
    )
    .unwrap();

    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    let ch = channel_uuid(&mut app, 0);
    let result = send_cmd(
        &mut app,
        EngineCommand::AddImageDeck {
            channel_uuid: ch,
            path: art.clone(),
        },
    );
    assert!(
        matches!(result, CommandResult::OkWithId { .. }),
        "adding an SVG deck should succeed, got {result:?}"
    );
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert_eq!(
        state.mixer.channels[0].decks.len(),
        1,
        "the SVG deck must survive the roundtrip"
    );
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
    assert!(varda_dir.exists());
}

#[test]
fn scene_json_valid_format() {
    let tmp = TempDir::new().unwrap();
    let Some(mut app) = headless_app_in(tmp.path()) else {
        return;
    };
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");

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
    app.save_workspace(&UILayoutState::default())
        .expect("save workspace");
    let Some(mut app2) = headless_app_in(tmp.path()) else {
        return;
    };
    let _ = app2.load_workspace();
    let state = app2.build_engine_state();
    assert!((state.mixer.channels[0].opacity - 0.5).abs() < 1e-4);
}
