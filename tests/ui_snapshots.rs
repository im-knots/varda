//! E2E snapshot tests for visual regression.
//!
//! Render the UI (or specific panels) and compare against reference images.
//! Reference images are stored in `tests/snapshots/` and tracked in git.
//! `.diff.png`, `.new.png` and `.old.png` files are git-ignored.
//!
//! These require wgpu — they will be skipped if no GPU/software renderer is available.
//!
//! **These do not run in CI.** The reference PNGs are generated on a developer's
//! GPU (Metal), while CI renders on lavapipe, and the two disagree by a couple of
//! 8-bit steps on a handful of pixels. Reconciling that needs either a per-pixel
//! tolerance wide enough to hide real regressions, or lavapipe-generated goldens
//! that can no longer be regenerated locally. Neither is worth it, so CI sets
//! `VARDA_SKIP_GOLDEN_SNAPSHOTS` and these stay a local-only check. Every other
//! GPU suite does run in CI on lavapipe.

use std::rc::Rc;

use egui_kittest::Harness;
use varda::usecases::ui::panels::render_ui;
use varda::usecases::ui::{UIActions, UIData};

/// Logical-point size of the simulated window. `pixels_per_point` is 1.0, so
/// this is both the point size and the PNG's pixel size.
///
/// 1920×1080 rather than a smaller box because the panels are sized in points:
/// at 1280×720 the top bar overlapped its own tonemap label, the bottom panel
/// clipped, and "Drag effects here" wrapped to one letter per line. None of that
/// reproduces at a realistic maximized-window size, so the smaller box was
/// pinning layout defects that no user would ever see.
const SIZE: egui::Vec2 = egui::vec2(1920.0, 1080.0);

/// Build a sized harness, or `None` when golden comparison is disabled.
///
/// The opt-out lives here rather than in each test so a snapshot test cannot be
/// added that bypasses it — they all have to come through this constructor.
fn snapshot_harness(data: UIData) -> Option<Harness<'static, UIActions>> {
    if std::env::var_os("VARDA_SKIP_GOLDEN_SNAPSHOTS").is_some() {
        eprintln!("VARDA_SKIP_GOLDEN_SNAPSHOTS set — skipping golden comparison");
        return None;
    }
    let data = Rc::new(data);
    let mut harness = Harness::builder().with_size(SIZE).build_ui_state(
        move |ui, actions: &mut UIActions| {
            *actions = render_ui(ui, &data);
        },
        UIActions::new(),
    );
    harness.run();
    Some(harness)
}

// ── Full UI layout ──────────────────────────────────────────────────

#[test]
fn snapshot_full_ui_default() {
    let Some(mut harness) = snapshot_harness(UIData::test_fixture()) else {
        return;
    };
    harness.snapshot("full_ui_default");
}

#[test]
fn snapshot_full_ui_library_closed() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = false;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("full_ui_library_closed");
}

/// Collapsing the right panel must not take the telemetry with it: the frame
/// rate matters most to someone who has just reclaimed screen space to chase
/// performance. See /spec/transport.md.
#[test]
fn snapshot_full_ui_right_panel_closed() {
    let mut data = UIData::test_fixture();
    data.right_panel_open = false;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("full_ui_right_panel_closed");
}

/// Arrangement mode swaps the central area and nothing else: the library,
/// bottom bar, and right panel must survive the mode switch intact. That is the
/// whole claim of /spec/arrangement.md § UI, and a picture is the only way to
/// check it.
#[test]
fn snapshot_full_ui_arrangement_mode() {
    let mut data = UIData::test_fixture();
    let deck_uuid = data.channels[0].decks[0].uuid.clone();
    let mut lane = varda::arrangement::LaneConfig::new(&deck_uuid);
    lane.regions.push(varda::arrangement::RegionConfig {
        start: 2.0,
        end: 14.0,
        fade_in: 1.5,
        fade_out: 3.0,
    });
    let config = varda::arrangement::ArrangementConfig {
        lanes: vec![lane],
        // A cue in the picture, because "yellow dot on the ruler, dashed line
        // down the lanes" is a claim only a picture can check.
        cues: vec![varda::arrangement::Cue {
            uuid: "cue00001".to_string(),
            name: "Drop".to_string(),
            at: 10.0,
        }],
        ..Default::default()
    };
    data.arrangement = Some(varda::engine::types::ArrangementSnapshot {
        duration: config.duration(),
        config,
        engaged: true,
        overridden_params: vec![],
    });
    data.arrangement_mode_open = true;
    data.transport.has_run = true;
    data.transport.position = 6.0;

    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("full_ui_arrangement_mode");
}

/// The same show back at the desk. "Two buttons wide, under the mixer and the
/// macros, without crowding either" is a claim only a picture can check.
#[test]
fn snapshot_full_ui_cue_bank() {
    let mut data = UIData::test_fixture();
    let config = varda::arrangement::ArrangementConfig {
        cues: vec![
            varda::arrangement::Cue {
                uuid: "cue00001".to_string(),
                name: "Intro".to_string(),
                at: 0.0,
            },
            varda::arrangement::Cue {
                uuid: "cue00002".to_string(),
                name: "Drop".to_string(),
                at: 10.0,
            },
            varda::arrangement::Cue {
                uuid: "cue00003".to_string(),
                name: "Breakdown".to_string(),
                at: 24.0,
            },
        ],
        ..Default::default()
    };
    data.arrangement = Some(varda::engine::types::ArrangementSnapshot {
        duration: config.duration(),
        config,
        engaged: false,
        overridden_params: vec![],
    });

    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("full_ui_cue_bank");
}

// ── Bottom bar contexts ─────────────────────────────────────────────

#[test]
fn snapshot_bottom_bar_deck_detail() {
    let mut data = UIData::test_fixture();
    data.selected_deck = Some((0, 0));
    data.selected_channel = None;
    data.selected_master = false;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("bottom_bar_deck_detail");
}

#[test]
fn snapshot_bottom_bar_channel_fx() {
    let mut data = UIData::test_fixture();
    data.selected_deck = None;
    data.selected_channel = Some(0);
    data.selected_master = false;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("bottom_bar_channel_fx");
}

#[test]
fn snapshot_bottom_bar_master_fx() {
    let mut data = UIData::test_fixture();
    data.selected_deck = None;
    data.selected_channel = None;
    data.selected_master = true;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("bottom_bar_master_fx");
}

#[test]
fn snapshot_bottom_bar_nothing_selected() {
    let mut data = UIData::test_fixture();
    data.selected_deck = None;
    data.selected_channel = None;
    data.selected_master = false;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("bottom_bar_nothing_selected");
}
