//! E2E snapshot tests for visual regression.
//!
//! Render the UI (or specific panels) and compare against reference images.
//! Reference images are stored in `tests/snapshots/` and tracked in git.
//! `.diff.png` and `.new.png` files are git-ignored.
//!
//! These require wgpu — they will be skipped if no GPU/software renderer is available.

use std::rc::Rc;

use egui_kittest::Harness;
use varda::usecases::ui::panels::render_ui;
use varda::usecases::ui::{UIActions, UIData};

/// Helper: build a sized harness (1280×720) for snapshot rendering.
fn snapshot_harness(data: UIData) -> Harness<'static, UIActions> {
    let data = Rc::new(data);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .build_ui_state(
            move |ui, actions: &mut UIActions| {
                *actions = render_ui(ui, &data);
            },
            UIActions::new(),
        );
    harness.run();
    harness
}

// ── Full UI layout ──────────────────────────────────────────────────

#[test]
fn snapshot_full_ui_default() {
    let mut harness = snapshot_harness(UIData::test_fixture());
    harness.snapshot("full_ui_default");
}

#[test]
fn snapshot_full_ui_library_closed() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = false;
    let mut harness = snapshot_harness(data);
    harness.snapshot("full_ui_library_closed");
}

// ── Bottom bar contexts ─────────────────────────────────────────────

#[test]
fn snapshot_bottom_bar_deck_detail() {
    let mut data = UIData::test_fixture();
    data.selected_deck = Some((0, 0));
    data.selected_channel = None;
    data.selected_master = false;
    let mut harness = snapshot_harness(data);
    harness.snapshot("bottom_bar_deck_detail");
}

#[test]
fn snapshot_bottom_bar_channel_fx() {
    let mut data = UIData::test_fixture();
    data.selected_deck = None;
    data.selected_channel = Some(0);
    data.selected_master = false;
    let mut harness = snapshot_harness(data);
    harness.snapshot("bottom_bar_channel_fx");
}

#[test]
fn snapshot_bottom_bar_master_fx() {
    let mut data = UIData::test_fixture();
    data.selected_deck = None;
    data.selected_channel = None;
    data.selected_master = true;
    let mut harness = snapshot_harness(data);
    harness.snapshot("bottom_bar_master_fx");
}

#[test]
fn snapshot_bottom_bar_nothing_selected() {
    let mut data = UIData::test_fixture();
    data.selected_deck = None;
    data.selected_channel = None;
    data.selected_master = false;
    let mut harness = snapshot_harness(data);
    harness.snapshot("bottom_bar_nothing_selected");
}
