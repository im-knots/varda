//! Bottom-bar mode dispatch.
//!
//! The mode depends on the current selection. Each mode lives with its feature;
//! this module only routes.

use super::super::{UIActions, UIData};
use super::deck_detail::render_selected_deck_detail;
use super::effects::{render_channel_effect_detail, render_master_effect_detail};
use crate::engine::EngineCommand;

/// The bar's panel id.
pub(super) const PANEL_ID: &str = "bottom_panel";

/// The bar's height until something is selected.
pub(super) const DEFAULT_HEIGHT: f32 = 180.0;

/// Fits a deck's preview, source controls, and effect cards with nothing clipped.
const DETAIL_HEIGHT: f32 = 260.0;

/// Opens the bar to [`DETAIL_HEIGHT`] the first time a selection fills it this
/// session. Runs once, before the panel lays out. A taller bar stays as it is, and
/// the user's height is never touched after that.
pub(super) fn open_for_first_selection(ctx: &egui::Context, data: &UIData) {
    let filled_id = egui::Id::new("bottom_bar_filled");
    let filled: Option<bool> = ctx.data(|d| d.get_temp(filled_id));
    if filled == Some(true) {
        return;
    }

    let shows_selection = !data.stage_editor_open
        && (data.selected_deck.is_some()
            || data.selected_master
            || data.selected_channel.is_some()
            || data.selected_sequence.is_some()
            || data.selected_macro.is_some());
    ctx.data_mut(|d| d.insert_temp(filled_id, shows_selection));

    if filled == Some(false) && shows_selection {
        let panel_id = egui::Id::new(PANEL_ID);
        let Some(mut state) = egui::PanelState::load(ctx, panel_id) else {
            return;
        };
        if state.outer_rect.height() <= DEFAULT_HEIGHT {
            state
                .outer_rect
                .set_top(state.outer_rect.bottom() - DETAIL_HEIGHT);
            ctx.data_mut(|d| d.insert_persisted(panel_id, state));
        }
    }
}

pub(super) fn render_bottom_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    if data.midi_learn_active {
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(4.0)
            .fill(egui::Color32::from_rgb(180, 80, 220))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(target) = &data.midi_learn_target {
                        ui.label(
                            egui::RichText::new(format!(
                                "🎹 MIDI LEARN — Move a control to map: {target}"
                            ))
                            .strong()
                            .color(egui::Color32::WHITE),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("🎹 MIDI LEARN — Click a parameter to select it")
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("x Exit MIDI Learn").clicked() {
                            actions.commands.push(EngineCommand::MidiLearnToggle);
                        }
                    });
                });
            });
    }

    // With the stage editor open, the bottom bar hosts the warp editor for the
    // selected surface.
    if data.stage_editor_open {
        super::stage::render_stage_bottom_bar(ui, data, actions);
        return;
    }

    // Otherwise: master effects, channel effects, sequence, macro, or deck detail.
    if data.selected_master {
        render_master_effect_detail(ui, data, actions);
    } else if let Some(ch_idx) = data.selected_channel {
        render_channel_effect_detail(ui, ch_idx, data, actions);
    } else if let Some(seq_idx) = data.selected_sequence {
        super::sequence::render_sequence_detail(ui, seq_idx, data, actions);
    } else if let Some(uuid) = data.selected_macro.clone() {
        super::macros::render_macro_detail(ui, &uuid, data, actions);
    } else {
        render_selected_deck_detail(ui, data, actions);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::SurfaceUI;
    use super::super::stage::stage_selection_id;
    use super::*;
    use crate::engine::EngineCommand;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A fixture with one surface, selected in the stage editor's shared
    /// selection memory, so `render_bottom_panel` routes to the warp editor.
    fn fixture_with_surface(
        warp: Option<crate::surface::warp::WarpMode>,
        warp_bound: bool,
    ) -> (UIData, String) {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.stage_editor_open = true;
        let uuid = "surf0001".to_string();
        let mut surface = SurfaceUI::test_quad(&uuid, 0.1, 0.1, 0.8, 0.8);
        surface.warp = warp;
        surface.warp_bound = warp_bound;
        data.surfaces.push(surface);
        (data, uuid)
    }

    /// Render the bottom panel with `uuids` published as the stage selection.
    fn harness_with_selection(data: &UIData, uuids: &[String]) {
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(stage_selection_id(), uuids.to_vec()));
            render_bottom_panel(ui, data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_stage_editor_no_selection() {
        let (data, _uuid) = fixture_with_surface(None, false);
        harness_with_selection(&data, &[]);
    }

    #[test]
    fn render_bottom_panel_smoke_stage_editor_multi_selection() {
        let (data, uuid) = fixture_with_surface(None, false);
        // Two entries, only one of which resolves: still not a single selection.
        harness_with_selection(&data, &[uuid, "surf0002".to_string()]);
    }

    /// No warp yet: the editor falls back to identity corners from the bbox.
    #[test]
    fn render_bottom_panel_smoke_stage_editor_unwarped() {
        let (data, uuid) = fixture_with_surface(None, false);
        harness_with_selection(&data, &[uuid]);
    }

    #[test]
    fn render_bottom_panel_smoke_stage_editor_corner_pin() {
        let warp = crate::surface::warp::WarpMode::corner_pin([
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ]);
        let (data, uuid) = fixture_with_surface(Some(warp), false);
        harness_with_selection(&data, &[uuid]);
    }

    #[test]
    fn render_bottom_panel_smoke_stage_editor_mesh() {
        let warp =
            crate::surface::warp::WarpMode::Mesh(crate::surface::warp::WarpMesh::identity(3, 3));
        let (data, uuid) = fixture_with_surface(Some(warp), false);
        harness_with_selection(&data, &[uuid]);
    }

    #[test]
    fn render_bottom_panel_smoke_stage_editor_bezier() {
        let warp =
            crate::surface::warp::WarpMode::Bezier(crate::surface::warp::BezierWarp::from_mesh(
                &crate::surface::warp::WarpMesh::identity(2, 2),
                crate::surface::warp::DEFAULT_BEZIER_TESS,
            ));
        let (data, uuid) = fixture_with_surface(Some(warp), false);
        harness_with_selection(&data, &[uuid]);
    }

    /// `warp_bound` makes the controls read-only, a separate render path.
    #[test]
    fn render_bottom_panel_smoke_stage_editor_warp_bound() {
        let warp = crate::surface::warp::WarpMode::corner_pin([
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ]);
        let (data, uuid) = fixture_with_surface(Some(warp), true);
        harness_with_selection(&data, &[uuid]);
    }

    /// Accumulated across frames of the warp-drag harness.
    #[derive(Default)]
    struct WarpProbe {
        commands: Vec<EngineCommand>,
        /// `ui.min_rect()` after rendering. The warp canvas is allocated last and spans
        /// the full width, so this rect's bottom-left corner is the BL handle.
        content: Option<egui::Rect>,
    }

    /// Primary-button drag from `start` to `end`, stepped so egui registers a drag
    /// with the press origin near `start`.
    fn drag_probe(
        harness: &mut egui_kittest::Harness<'static, WarpProbe>,
        start: egui::Pos2,
        end: egui::Pos2,
    ) {
        use egui::{Event, Modifiers, PointerButton};
        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        });
        harness.run();
        for t in [0.25_f32, 0.5, 0.75, 1.0] {
            harness.event(Event::PointerMoved(start + (end - start) * t));
            harness.run();
        }
        harness.event(Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        });
        harness.run();
    }

    /// Dragging a corner handle emits `SetWarpCorner` for that corner.
    ///
    /// The in-progress handle lives in egui memory under a `ui.id()`-derived key,
    /// so a change there breaks warp editing without a compile error.
    #[test]
    fn warp_corner_drag_emits_set_warp_corner() {
        let warp = crate::surface::warp::WarpMode::corner_pin([
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ]);
        let (data, uuid) = fixture_with_surface(Some(warp), false);
        let sel = vec![uuid.clone()];

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 200.0))
            .build_ui_state(
                move |ui, probe: &mut WarpProbe| {
                    ui.ctx()
                        .memory_mut(|m| m.data.insert_temp(stage_selection_id(), sel.clone()));
                    let mut actions = UIActions::new();
                    render_bottom_panel(ui, &data, &mut actions);
                    probe.content = Some(ui.min_rect());
                    probe.commands.extend(actions.commands);
                },
                WarpProbe::default(),
            );

        // Settle layout, then locate the bottom-left handle and clear the
        // commands emitted during those layout passes.
        harness.run();
        let content = harness.state().content.expect("content rect recorded");
        harness.state_mut().commands.clear();

        let bl_handle = content.left_bottom();
        drag_probe(&mut harness, bl_handle, bl_handle + egui::vec2(40.0, -30.0));

        let corners: Vec<usize> = harness
            .state()
            .commands
            .iter()
            .filter_map(|c| match c {
                EngineCommand::SetWarpCorner {
                    surface_uuid,
                    corner_idx,
                    ..
                } if *surface_uuid == uuid => Some(*corner_idx),
                _ => None,
            })
            .collect();

        assert!(
            !corners.is_empty(),
            "dragging the bottom-left warp handle at {bl_handle:?} emitted no \
             SetWarpCorner for surface {uuid}; commands seen: {:?}",
            harness.state().commands
        );
        assert!(
            corners.iter().all(|&i| i == 3),
            "expected only the bottom-left corner (index 3) to move, got {corners:?}"
        );
    }

    #[test]
    fn render_bottom_panel_smoke_macro_selected() {
        use crate::macros::{Macro, MacroKind, MacroTarget};
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        let mut knob = Macro::new(MacroKind::Knob, "Sweep");
        knob.targets.push(MacroTarget::new("crossfader"));
        data.macros.push(knob);
        data.selected_macro = Some(data.macros[0].uuid.clone());
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_deck_selected() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_channel_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = Some(0);
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_master_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_master = true;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_nothing_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = None;
        data.selected_master = false;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_sequence_selected() {
        use super::super::super::{SequenceStepKindUI, SequenceStepUI, SequenceUIData};
        use crate::channel::DurationUnit;
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.sequences.push(SequenceUIData {
            uuid: "seq00001".to_string(),
            name: "Test Seq".to_string(),
            enabled: true,
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
            steps: vec![SequenceStepUI {
                label: "Fade".into(),
                kind: SequenceStepKindUI::Fade {
                    from_ch: "ca000001".to_string(),
                    to_ch: "cb000001".to_string(),
                    duration_val: 5.0,
                    duration_unit: DurationUnit::Seconds,
                    easing: "Linear".into(),
                    transition_shader: None,
                    target_amount: 1.0,
                },
            }],
        });
        data.selected_sequence = Some(0);
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    fn bar_height(ctx: &egui::Context) -> f32 {
        egui::PanelState::load(ctx, egui::Id::new(PANEL_ID))
            .expect("the bottom bar has rendered")
            .outer_rect
            .height()
    }

    /// Stands in for the user dragging the bar to `height`.
    fn resize_bar(ctx: &egui::Context, height: f32) {
        let id = egui::Id::new(PANEL_ID);
        let mut outer_rect = egui::PanelState::load(ctx, id).unwrap().outer_rect;
        outer_rect.set_top(outer_rect.bottom() - height);
        ctx.data_mut(|d| d.insert_persisted(id, egui::PanelState { outer_rect }));
    }

    fn data_with_nothing_selected() -> UIData {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data
    }

    /// The whole UI over data a test can change between frames, as selecting
    /// something in the mixer does.
    fn live_ui(data: UIData) -> (Rc<RefCell<UIData>>, egui_kittest::Harness<'static>) {
        let data = Rc::new(RefCell::new(data));
        let rendered = Rc::clone(&data);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 720.0))
            .build_ui(move |ui| {
                let _ = super::super::render_ui(ui, &rendered.borrow());
            });
        harness.run();
        (data, harness)
    }

    #[test]
    fn the_first_selection_opens_the_bar_to_fit_its_controls() {
        let (data, mut harness) = live_ui(data_with_nothing_selected());
        assert_eq!(bar_height(&harness.ctx), DEFAULT_HEIGHT);

        data.borrow_mut().selected_deck = Some((0, 0));
        harness.run();

        assert_eq!(bar_height(&harness.ctx), DETAIL_HEIGHT);
    }

    #[test]
    fn any_first_selection_opens_the_bar() {
        let select: [fn(&mut UIData); 2] = [
            |d| d.selected_master = true,
            |d| d.selected_channel = Some(0),
        ];
        for select in select {
            let (data, mut harness) = live_ui(data_with_nothing_selected());
            select(&mut data.borrow_mut());
            harness.run();
            assert_eq!(bar_height(&harness.ctx), DETAIL_HEIGHT);
        }
    }

    #[test]
    fn the_first_selection_keeps_a_height_the_user_made_taller() {
        let (data, mut harness) = live_ui(data_with_nothing_selected());
        resize_bar(&harness.ctx, 340.0);
        harness.run();

        data.borrow_mut().selected_deck = Some((0, 0));
        harness.run();

        assert_eq!(bar_height(&harness.ctx), 340.0);
    }

    #[test]
    fn later_selections_keep_the_height_the_user_chose() {
        let (data, mut harness) = live_ui(data_with_nothing_selected());
        data.borrow_mut().selected_deck = Some((0, 0));
        harness.run();

        resize_bar(&harness.ctx, DEFAULT_HEIGHT);
        harness.run();
        data.borrow_mut().selected_deck = None;
        harness.run();
        data.borrow_mut().selected_deck = Some((0, 1));
        harness.run();

        assert_eq!(bar_height(&harness.ctx), DEFAULT_HEIGHT);
    }

    #[test]
    fn a_session_opening_with_a_selection_keeps_the_default_height() {
        let (_data, harness) = live_ui(UIData::test_fixture());
        assert_eq!(bar_height(&harness.ctx), DEFAULT_HEIGHT);
    }

    #[test]
    fn the_stage_editor_does_not_open_the_bar() {
        let mut empty = data_with_nothing_selected();
        empty.stage_editor_open = true;
        let (data, mut harness) = live_ui(empty);

        data.borrow_mut().selected_deck = Some((0, 0));
        harness.run();

        assert_eq!(bar_height(&harness.ctx), DEFAULT_HEIGHT);
    }

    #[test]
    fn render_bottom_panel_smoke_sequence_with_step_selected() {
        use super::super::super::{SequenceStepKindUI, SequenceStepUI, SequenceUIData};
        use crate::channel::DurationUnit;
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.sequences.push(SequenceUIData {
            uuid: "seq00001".to_string(),
            name: "Test Seq".to_string(),
            enabled: true,
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
            steps: vec![
                SequenceStepUI {
                    label: "Fade".into(),
                    kind: SequenceStepKindUI::Fade {
                        from_ch: "ca000001".to_string(),
                        to_ch: "cb000001".to_string(),
                        duration_val: 5.0,
                        duration_unit: DurationUnit::Seconds,
                        easing: "Linear".into(),
                        transition_shader: None,
                        target_amount: 1.0,
                    },
                },
                SequenceStepUI {
                    label: "Wait".into(),
                    kind: SequenceStepKindUI::Wait {
                        duration_val: 2.0,
                        duration_unit: DurationUnit::Seconds,
                    },
                },
            ],
        });
        data.selected_sequence = Some(0);
        data.selected_sequence_step = Some((0, 1));
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }
}
