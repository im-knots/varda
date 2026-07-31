//! Bottom-bar mode dispatch.
//!
//! The bottom detail bar is context-sensitive: which mode renders depends on
//! what is currently selected. Each mode lives with the feature it belongs to —
//! this module only routes between them.

use super::super::{UIActions, UIData};
use super::deck_detail::render_selected_deck_detail;
use super::effects::{render_channel_effect_detail, render_master_effect_detail};

pub(super) fn render_bottom_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // MIDI learn status indicator
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
                            actions.session.midi_learn_toggle = true;
                        }
                    });
                });
            });
    }

    // While the stage editor is open the bottom bar hosts the per-surface warp
    // editor for the selected surface (8i.5).
    if data.stage_editor_open {
        super::stage::render_stage_bottom_bar(ui, data, actions);
        return;
    }

    // Context-sensitive bottom bar: master effects, channel effects, sequence, macro, or deck detail
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

    /// A fixture with one surface, selected in the stage editor's shared
    /// selection memory, so `render_bottom_panel` routes to the warp editor.
    fn fixture_with_surface(
        warp: Option<crate::renderer::warp::WarpMode>,
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
        // Two entries, only one of which resolves — still not a single selection.
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
        let warp = crate::renderer::warp::WarpMode::corner_pin([
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
            crate::renderer::warp::WarpMode::Mesh(crate::renderer::warp::WarpMesh::identity(3, 3));
        let (data, uuid) = fixture_with_surface(Some(warp), false);
        harness_with_selection(&data, &[uuid]);
    }

    #[test]
    fn render_bottom_panel_smoke_stage_editor_bezier() {
        let warp =
            crate::renderer::warp::WarpMode::Bezier(crate::renderer::warp::BezierWarp::from_mesh(
                &crate::renderer::warp::WarpMesh::identity(2, 2),
                crate::renderer::warp::DEFAULT_BEZIER_TESS,
            ));
        let (data, uuid) = fixture_with_surface(Some(warp), false);
        harness_with_selection(&data, &[uuid]);
    }

    /// `warp_bound` locks the controls read-only — a distinct render path.
    #[test]
    fn render_bottom_panel_smoke_stage_editor_warp_bound() {
        let warp = crate::renderer::warp::WarpMode::corner_pin([
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
        /// `ui.min_rect()` after rendering. The warp canvas is the last thing
        /// allocated and spans the full width, so its bottom-left corner is this
        /// rect's bottom-left — which is exactly where the BL handle is drawn.
        content: Option<egui::Rect>,
    }

    /// Primary-button drag from `start` to `end`, stepped so egui registers a
    /// drag (rather than a click) and captures the press origin near `start`.
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
    /// This guards the drag state machine in `render_surface_warp_editor`, whose
    /// in-progress handle lives in egui memory under an `ui.id()`-derived key.
    /// A regression there (or a change to that key) silently breaks warp editing
    /// with no compile error, so assert the whole gesture end to end.
    #[test]
    fn warp_corner_drag_emits_set_warp_corner() {
        let warp = crate::renderer::warp::WarpMode::corner_pin([
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
