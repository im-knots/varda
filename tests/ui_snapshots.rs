//! Snapshot tests for visual regression.
//!
//! Render the UI or single panels and compare against reference images in
//! `tests/snapshots/` (tracked in git; `.diff.png`, `.new.png` and `.old.png`
//! are ignored). Skipped without a GPU or software renderer.
//!
//! Not run in CI: references are rendered on Metal, CI renders on lavapipe, and
//! a few pixels differ by a couple of 8-bit steps. CI sets
//! `VARDA_SKIP_GOLDEN_SNAPSHOTS`. Other GPU suites do run in CI.

use std::rc::Rc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use varda::usecases::ui::panels::render_ui;
use varda::usecases::ui::{UIActions, UIData};

/// Logical-point size of the simulated window. `pixels_per_point` is 1.0, so
/// this is also the PNG's pixel size.
///
/// Panels are sized in points, and smaller windows produce layout defects (top
/// bar overlap, clipped bottom panel) that a maximized window never shows.
const SIZE: egui::Vec2 = egui::vec2(1920.0, 1080.0);

/// Build a sized harness, or `None` when golden comparison is disabled. Every
/// snapshot test goes through here so none can bypass the opt-out.
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

/// Collapsing the right panel keeps the telemetry visible.
#[test]
fn snapshot_full_ui_right_panel_closed() {
    let mut data = UIData::test_fixture();
    data.right_panel_open = false;
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("full_ui_right_panel_closed");
}

/// Arrangement mode swaps only the central area: the library, bottom bar, and
/// right panel stay unchanged.
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
        // A cue, so the snapshot shows the ruler dot and the dashed line down
        // the lanes.
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

/// The same show in performance mode: the arrangement buttons sit under the
/// mixer and macros without crowding either.
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

/// The video playback column, with a modulator on the speed slider and another
/// moving the playhead away from its anchor.
///
/// The shared fixture has no video deck, so no other snapshot covers this
/// column or its `〰` dropdowns and ghosts.
#[test]
fn snapshot_bottom_bar_video_playback() {
    use varda::source::DeckSourceProvider;
    let mut data = UIData::test_fixture();
    data.selected_deck = Some((0, 0));
    data.selected_channel = None;
    data.selected_master = false;

    let uuid = data.channels[0].decks[0].uuid.clone();
    // The video source type and a clip deck, as the engine would report them.
    let provider = varda::video::provider::VideoProvider;
    std::sync::Arc::make_mut(&mut data.sources).push(varda::source::ProviderTypeSnapshot {
        library_group: None,
        type_id: provider.id().into(),
        label: provider.label().into(),
        icon: provider.icon().into(),
        available: true,
        unavailable_reason: None,
        listed: true,
        params: provider.params().to_vec(),
        library: varda::source::LibrarySection::default(),
    });
    let deck = &mut data.channels[0].decks[0];
    deck.source.source_type = provider.id().into();
    let info = &mut deck.source.status.info;
    info.insert("playing".into(), true.into());
    info.insert("position".into(), 4.25.into());
    info.insert("duration".into(), 30.0.into());
    // Set point and live rate differ so the ghost is visible.
    info.insert("speed".into(), 1.0.into());
    info.insert("effective_speed".into(), 2.4.into());
    // Non-zero so the scrub bar's ghost sits apart from the handle.
    info.insert("position_offset".into(), (-3.0).into());
    info.insert("in_point".into(), 0.0.into());
    info.insert("out_point".into(), 0.0.into());
    info.insert("frame_rate".into(), 30.0.into());
    info.insert("loop_mode".into(), "Loop".into());
    info.insert(
        "transport_sync".into(),
        serde_json::to_value(varda::video::DeckTransportSync::default()).unwrap(),
    );
    deck.source
        .status
        .params
        .insert("speed".into(), varda::source::ControlValue::Float(0.5));
    for name in [
        varda::video::modulation::SPEED,
        varda::video::modulation::POSITION,
    ] {
        data.modulation_assignments.insert(
            format!("deck/{uuid}/{name}"),
            vec![varda::usecases::ui::ModAssignmentUI {
                source_id: "mod00001".to_string(),
                amount: 0.5,
            }],
        );
    }

    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    harness.snapshot("bottom_bar_video_playback");
}

/// Drag the bottom panel to its maximum height.
///
/// At the default 180 points a grouped list renders below the fold. Seeding the
/// stored panel state avoids synthesizing a drag on the resize separator.
fn expand_bottom_panel(harness: &mut Harness<'static, UIActions>) {
    const PANEL_MAX: f32 = 400.0;
    harness.ctx.data_mut(|d| {
        d.insert_persisted(
            egui::Id::new("bottom_panel"),
            egui::PanelState {
                outer_rect: egui::Rect::from_min_size(
                    egui::pos2(0.0, SIZE.y - PANEL_MAX),
                    egui::vec2(SIZE.x, PANEL_MAX),
                ),
            },
        );
    });
    harness.run();
}

/// A grouped shader in the params column, with `long` and `point2D` rows.
///
/// The shared fixture has one ungrouped float, so this covers section headers,
/// the first-group-open rule, and the `long`/`point2D` widgets. Modeled on a
/// raymarched fractal shader's inputs.
fn grouped_param_fixture() -> UIData {
    use varda::params::ParamValue;
    use varda::usecases::ui::{ParamChoiceUI, ParamUIInfo};

    let param = |name: &str, group: Option<&str>, value: ParamValue| ParamUIInfo {
        name: name.to_string(),
        label: Some(
            name.split('_')
                .map(|w| {
                    let mut c = w.chars();
                    c.next().map_or_else(String::new, |f| {
                        f.to_uppercase().collect::<String>() + c.as_str()
                    })
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        value,
        min: Some(0.0),
        max: Some(10.0),
        group: group.map(ToString::to_string),
        choices: Vec::new(),
        event: false,
    };

    let mut data = UIData::test_fixture();
    data.selected_deck = Some((0, 0));
    data.selected_channel = None;
    data.selected_master = false;

    let mut params = vec![
        param("iterations", None, ParamValue::Float(8.0)),
        param("fly_speed", None, ParamValue::Float(0.4)),
        param("power", Some("Form"), ParamValue::Float(8.0)),
        param("bailout", Some("Form"), ParamValue::Float(4.0)),
        param("cam_x", Some("Camera"), ParamValue::Float(0.0)),
        param("fov", Some("Camera"), ParamValue::Float(1.0)),
        param("look_at", Some("Camera"), ParamValue::Point2D([0.25, -0.5])),
        param("ao_strength", Some("Lighting"), ParamValue::Float(0.7)),
        param("sun_elev", Some("Lighting"), ParamValue::Float(0.6)),
        param(
            "color1",
            Some("Palette"),
            ParamValue::Color([0.0, 0.8, 1.0, 1.0]),
        ),
        param("shadows", Some("Lighting"), ParamValue::Bool(true)),
    ];
    let mut color_mode = param("color_mode", Some("Palette"), ParamValue::Long(1));
    color_mode.choices = vec![
        ParamChoiceUI {
            value: 0,
            label: "Orbit Trap".to_string(),
        },
        ParamChoiceUI {
            value: 1,
            label: "Normal".to_string(),
        },
        ParamChoiceUI {
            value: 2,
            label: "Depth".to_string(),
        },
    ];
    params.push(color_mode);

    data.channels[0].decks[0].generator.params = params;
    data
}

/// Default state: ungrouped rows in view, Form open because it appears first,
/// every other section closed.
#[test]
fn snapshot_bottom_bar_param_groups() {
    let Some(mut harness) = snapshot_harness(grouped_param_fixture()) else {
        return;
    };
    expand_bottom_panel(&mut harness);
    harness.snapshot("bottom_bar_param_groups");
}

/// Every section closed, reached by closing Form, the one group open by
/// default.
#[test]
fn snapshot_bottom_bar_param_groups_collapsed() {
    let Some(mut harness) = snapshot_harness(grouped_param_fixture()) else {
        return;
    };
    expand_bottom_panel(&mut harness);
    harness.get_by_label("Form").click();
    harness.run();
    harness.snapshot("bottom_bar_param_groups_collapsed");
}

/// `COLUMNS`: Lighting and Palette leave the params column for a Light column
/// of their own, Lighting open as its first group; ungrouped rows, Form and
/// Camera stay in the params column.
#[test]
fn snapshot_bottom_bar_param_columns() {
    let mut data = grouped_param_fixture();
    data.channels[0].decks[0].generator.columns = vec![varda::usecases::ui::ParamColumnUI {
        title: "Light".to_string(),
        groups: vec!["Lighting".to_string(), "Palette".to_string()],
    }];
    let Some(mut harness) = snapshot_harness(data) else {
        return;
    };
    expand_bottom_panel(&mut harness);
    harness.snapshot("bottom_bar_param_columns");
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
