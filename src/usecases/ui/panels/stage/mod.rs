//! Surface editor and stage editor panels.

mod camera_detect;
mod canvas;
mod dome;
mod geometry;
mod gizmo;
mod hit_test;
mod interaction;
mod state;
mod surface_editor;
mod toolbar;
mod warp_editor;

// The bottom-bar warp editor is a stage-editor mode; `panels` reaches it through
// this orchestrator rather than into the submodule.
pub(super) use surface_editor::render_surface_editor;
pub(super) use warp_editor::{render_stage_bottom_bar, stage_selection_id};

use super::super::{CameraDetectMode, DomeAction, UIActions, UIData};
use crate::engine::EngineCommand;
use crate::renderer::slicer::DomePreset;
use hit_test::CanvasGeometry;
use state::{StageEditorMode, StageEditorState};

/// Full-screen stage editor — replaces the deck view
// cx_px/cy_px, raw_sx/raw_sy and friends are the clearest names for this canvas geometry.
#[allow(clippy::similar_names)]
pub(super) fn render_stage_editor(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let state_id = ui.id().with("stage_editor_state");
    let mut state = ui.memory(|mem| {
        mem.data
            .get_temp::<StageEditorState>(state_id)
            .unwrap_or_default()
    });

    toolbar::render(ui, data, actions, &mut state);

    // ── Camera detection mode: takes over the entire canvas ──
    match &data.camera_detect_mode {
        CameraDetectMode::Live { .. } => {
            camera_detect::render_camera_detect_live(ui, data, actions);
            ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
            return;
        }
        CameraDetectMode::Preview { .. } => {
            camera_detect::render_camera_detect_preview(ui, data, actions);
            ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
            return;
        }
        CameraDetectMode::Off => {} // continue normal rendering
    }

    let mode = if data.dome_mode_active {
        StageEditorMode::Dome3D
    } else {
        StageEditorMode::Polygon2D
    };

    // Dome config toolbar (second row, only in Dome3D mode)
    if mode == StageEditorMode::Dome3D {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🔮 Dome:").strong());

            // Preset dropdown
            let presets = [
                DomePreset::Single,
                DomePreset::Dual,
                DomePreset::Triple,
                DomePreset::Quad,
                DomePreset::Penta,
                DomePreset::Hexa,
                DomePreset::Octa,
            ];
            let mut current_preset = data.dome_preset;
            egui::ComboBox::from_id_salt("dome_preset")
                .selected_text(format!("{current_preset}"))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for preset in &presets {
                        if ui
                            .selectable_value(&mut current_preset, *preset, format!("{preset}"))
                            .clicked()
                        {
                            actions
                                .session
                                .dome_actions
                                .push(DomeAction::SetPreset(*preset));
                        }
                    }
                });

            ui.separator();

            // Radius slider
            let mut radius = data.dome_geometry.radius;
            ui.label("R:");
            if ui
                .add(
                    egui::DragValue::new(&mut radius)
                        .range(0.5..=5.0)
                        .speed(0.01),
                )
                .changed()
            {
                actions
                    .session
                    .dome_actions
                    .push(DomeAction::SetRadius(radius));
            }

            // Truncation angle slider
            let mut trunc = data.dome_geometry.truncation_degrees;
            ui.label("Trunc:");
            if ui
                .add(
                    egui::DragValue::new(&mut trunc)
                        .range(30.0..=90.0)
                        .speed(0.5)
                        .suffix("°"),
                )
                .changed()
            {
                actions
                    .session
                    .dome_actions
                    .push(DomeAction::SetTruncation(trunc));
            }

            // Tilt slider
            let mut tilt = data.dome_geometry.tilt_degrees;
            ui.label("Tilt:");
            if ui
                .add(
                    egui::DragValue::new(&mut tilt)
                        .range(0.0..=45.0)
                        .speed(0.5)
                        .suffix("°"),
                )
                .changed()
            {
                actions.session.dome_actions.push(DomeAction::SetTilt(tilt));
            }

            ui.separator();

            // Content rotation controls
            let mut c_az = data.dome_geometry.content_azimuth_degrees;
            ui.label("Content Az:");
            if ui
                .add(
                    egui::DragValue::new(&mut c_az)
                        .range(-180.0..=180.0)
                        .speed(1.0)
                        .suffix("°"),
                )
                .changed()
            {
                actions
                    .session
                    .dome_actions
                    .push(DomeAction::SetContentAzimuth(c_az));
            }

            let mut c_el = data.dome_geometry.content_elevation_degrees;
            ui.label("Content El:");
            if ui
                .add(
                    egui::DragValue::new(&mut c_el)
                        .range(-90.0..=90.0)
                        .speed(1.0)
                        .suffix("°"),
                )
                .changed()
            {
                actions
                    .session
                    .dome_actions
                    .push(DomeAction::SetContentElevation(c_el));
            }

            let mut c_roll = data.dome_geometry.content_roll_degrees;
            ui.label("Content Roll:");
            if ui
                .add(
                    egui::DragValue::new(&mut c_roll)
                        .range(-180.0..=180.0)
                        .speed(1.0)
                        .suffix("°"),
                )
                .changed()
            {
                actions
                    .session
                    .dome_actions
                    .push(DomeAction::SetContentRoll(c_roll));
            }

            ui.separator();

            // Generate Slices button
            if ui
                .button("🎯 Generate Slices")
                .on_hover_text("Create per-projector surfaces with warp meshes")
                .clicked()
            {
                let setup = current_preset.to_setup_with_geometry(data.dome_geometry);
                actions
                    .commands
                    .push(EngineCommand::GenerateDomeSlices { setup });
            }
        });
    }

    ui.add_space(4.0);

    // ── Dome 3D mode: full-canvas interactive dome view ──
    if mode == StageEditorMode::Dome3D {
        dome::render_dome_canvas(ui, data, actions);
        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
        return;
    }

    // ── 2D Polygon mode: original canvas ──
    // Main canvas — fill available space
    let canvas_width = ui.available_width();
    let canvas_height = ui.available_height().max(200.0);
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );
    let grid_size = data.stage_editor_grid_size;
    let geom = CanvasGeometry::new(canvas_rect, grid_size, data.stage_editor_snap);

    let painter = ui.painter_at(canvas_rect);

    canvas::paint(&painter, &canvas_response, data, &state, geom);

    // --- Interaction handling ---
    interaction::handle_canvas(
        ui,
        &painter,
        &canvas_response,
        data,
        actions,
        &mut state,
        geom,
    );
    interaction::handle_keyboard(ui, data, actions, &mut state);

    // Publish the current selection so the bottom detail bar can edit the
    // selected surface's warp (8i.5).
    let published: Vec<String> = state.selected_surfaces.iter().cloned().collect();
    ui.ctx()
        .memory_mut(|mem| mem.data.insert_temp(stage_selection_id(), published));

    // Persist state
    ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
}

#[cfg(test)]
mod tests {
    use super::super::super::SurfaceUI;
    use super::state::DrawingTool;
    use super::*;

    // ── Tool state-machine characterization ─────────────────────────
    //
    // These drive `render_stage_editor` through a real `egui_kittest` harness and
    // pin down, per tool, both the `EngineCommand`s emitted and the resulting
    // `StageEditorState`. The canvas is allocated with `ui.available_width()` /
    // `available_height().max(200.0)` as the *last* item in the vertical layout,
    // so it always ends flush with `ui.min_rect()`'s bottom and spans full width.
    // Points are therefore taken relative to that bottom edge (within 200px of it)
    // rather than hard-coded, which keeps them valid if the toolbars change height.

    #[derive(Default)]
    struct StageProbe {
        commands: Vec<EngineCommand>,
        content: Option<egui::Rect>,
        state: Option<StageEditorState>,
        seeded: bool,
    }

    fn stage_harness(
        data: UIData,
        initial: StageEditorState,
    ) -> egui_kittest::Harness<'static, StageProbe> {
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_ui_state(
                move |ui, probe: &mut StageProbe| {
                    let state_id = ui.id().with("stage_editor_state");
                    if !probe.seeded {
                        ui.memory_mut(|mem| mem.data.insert_temp(state_id, initial.clone()));
                        probe.seeded = true;
                    }
                    let mut actions = UIActions::new();
                    render_stage_editor(ui, &data, &mut actions);
                    probe.content = Some(ui.min_rect());
                    probe.state = ui.memory(|mem| mem.data.get_temp::<StageEditorState>(state_id));
                    probe.commands.extend(actions.commands);
                },
                StageProbe::default(),
            )
    }

    /// A point inside the canvas, `up` pixels above its bottom edge. `up` must
    /// stay under 200 — the canvas's guaranteed minimum height.
    fn canvas_pt(content: egui::Rect, right: f32, up: f32) -> egui::Pos2 {
        assert!(
            up < 200.0,
            "point must be within the canvas's minimum height"
        );
        egui::pos2(content.left() + right, content.bottom() - up)
    }

    fn settle(harness: &mut egui_kittest::Harness<'static, StageProbe>) -> egui::Rect {
        harness.run();
        let content = harness.state().content.expect("content rect");
        harness.state_mut().commands.clear();
        content
    }

    fn stage_drag(
        harness: &mut egui_kittest::Harness<'static, StageProbe>,
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

    fn tool(tool: DrawingTool) -> StageEditorState {
        StageEditorState {
            tool,
            ..StageEditorState::default()
        }
    }

    fn added_polygons(probe: &StageProbe) -> Vec<Vec<[f32; 2]>> {
        probe
            .commands
            .iter()
            .filter_map(|c| match c {
                EngineCommand::AddPolygonSurface { vertices, .. } => Some(vertices.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rectangle_tool_drag_adds_axis_aligned_quad() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![];
        data.stage_editor_snap = false;
        let mut harness = stage_harness(data, tool(DrawingTool::Rectangle));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 100.0, 170.0),
            canvas_pt(content, 300.0, 40.0),
        );

        let polys = added_polygons(harness.state());
        assert_eq!(polys.len(), 1, "commands: {:?}", harness.state().commands);
        let v = &polys[0];
        assert_eq!(v.len(), 4, "rectangle has 4 vertices: {v:?}");
        // Wound clockwise from top-left: (x0,y0) (x1,y0) (x1,y1) (x0,y1).
        assert!((v[0][1] - v[1][1]).abs() < 1e-6, "top edge level: {v:?}");
        assert!((v[2][1] - v[3][1]).abs() < 1e-6, "bottom edge level: {v:?}");
        assert!((v[0][0] - v[3][0]).abs() < 1e-6, "left edge plumb: {v:?}");
        assert!((v[1][0] - v[2][0]).abs() < 1e-6, "right edge plumb: {v:?}");
        assert!(
            v[0][0] < v[1][0] && v[0][1] < v[2][1],
            "normalized order: {v:?}"
        );
        assert!(
            v.iter()
                .all(|p| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1])),
            "vertices stay in [0,1]: {v:?}"
        );
    }

    /// A rectangle thinner than 0.01 on either axis is discarded, so a near-flat
    /// drag never creates a degenerate surface.
    ///
    /// The drag is deliberately *wide* (200px) so egui registers a drag at all —
    /// a few-pixel drag stays below the click-vs-drag threshold and would never
    /// reach this guard, making the test vacuous. Snapping is off so the height
    /// stays a raw sub-threshold fraction rather than quantising to zero.
    #[test]
    fn rectangle_tool_rejects_drag_thinner_than_one_percent() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![];
        data.stage_editor_snap = false;
        let mut harness = stage_harness(data, tool(DrawingTool::Rectangle));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 100.0, 100.0),
            canvas_pt(content, 300.0, 99.0),
        );

        assert!(
            added_polygons(harness.state()).is_empty(),
            "commands: {:?}",
            harness.state().commands
        );
    }

    /// With snapping on, committed vertices land on the grid.
    #[test]
    fn rectangle_tool_snaps_vertices_to_grid() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![];
        data.stage_editor_snap = true;
        data.stage_editor_grid_size = 0.25;
        let mut harness = stage_harness(data, tool(DrawingTool::Rectangle));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 130.0, 170.0),
            canvas_pt(content, 480.0, 40.0),
        );

        let polys = added_polygons(harness.state());
        assert_eq!(polys.len(), 1, "commands: {:?}", harness.state().commands);
        for p in &polys[0] {
            for c in *p {
                let steps = c / 0.25;
                assert!(
                    (steps - steps.round()).abs() < 1e-4,
                    "{c} is not a multiple of the 0.25 grid: {:?}",
                    polys[0]
                );
            }
        }
    }

    /// Dragging from inside an existing surface moves it instead of drawing, and
    /// the tool flips to Select. Easy to break when this arm is refactored.
    #[test]
    fn rectangle_tool_drag_inside_surface_moves_it_and_switches_to_select() {
        let mut data = UIData::test_fixture();
        // Covers the whole canvas so any interior point lands inside it.
        data.surfaces = vec![SurfaceUI::test_quad("a", 0.0, 0.0, 1.0, 1.0)];
        let mut harness = stage_harness(data, tool(DrawingTool::Rectangle));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 100.0, 170.0),
            canvas_pt(content, 300.0, 40.0),
        );

        assert!(
            added_polygons(harness.state()).is_empty(),
            "must not draw a new surface: {:?}",
            harness.state().commands
        );
        let state = harness.state().state.clone().expect("state persisted");
        assert_eq!(state.tool, DrawingTool::Select, "tool switched to Select");
        assert!(
            state.selected_surfaces.contains("a"),
            "surface selected: {:?}",
            state.selected_surfaces
        );
    }

    #[test]
    fn circle_tool_drag_adds_circle_surface() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![];
        let mut harness = stage_harness(data, tool(DrawingTool::Circle));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 400.0, 120.0),
            canvas_pt(content, 500.0, 40.0),
        );

        let circles = harness
            .state()
            .commands
            .iter()
            .filter(|c| matches!(c, EngineCommand::AddCircleSurface { .. }))
            .count();
        assert_eq!(circles, 1, "commands: {:?}", harness.state().commands);
    }

    /// Selecting is published to the shared memory key the bottom-bar warp editor
    /// reads, so this pins the stage editor's half of that contract.
    #[test]
    fn select_tool_click_selects_surface_and_publishes_it() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![SurfaceUI::test_quad("a", 0.0, 0.0, 1.0, 1.0)];
        let mut harness = stage_harness(data, tool(DrawingTool::Select));
        let content = settle(&mut harness);

        let pt = canvas_pt(content, 500.0, 100.0);
        stage_drag(&mut harness, pt, pt + egui::vec2(2.0, 2.0));

        let state = harness.state().state.clone().expect("state persisted");
        assert!(
            state.selected_surfaces.contains("a"),
            "clicking inside a surface selects it: {:?}",
            state.selected_surfaces
        );
        let published = harness
            .ctx
            .memory(|mem| mem.data.get_temp::<Vec<String>>(stage_selection_id()));
        assert_eq!(
            published,
            Some(vec!["a".to_string()]),
            "selection published for the bottom-bar warp editor"
        );
    }

    /// Companion to the negative case below: proves the marquee machinery works,
    /// so "selects nothing" there is a real result rather than a gesture that
    /// never landed. Selection is by bounding-box intersection, not containment.
    #[test]
    fn select_tool_marquee_selects_intersecting_surface() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![SurfaceUI::test_quad("a", 0.5, 0.5, 0.4, 0.4)];
        let mut harness = stage_harness(data, tool(DrawingTool::Select));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 600.0, 150.0),
            canvas_pt(content, 900.0, 30.0),
        );

        let state = harness.state().state.clone().expect("state persisted");
        assert!(
            state.selected_surfaces.contains("a"),
            "marquee overlapping the surface selects it: {:?}",
            state.selected_surfaces
        );
    }

    #[test]
    fn select_tool_marquee_on_empty_canvas_selects_nothing() {
        let mut data = UIData::test_fixture();
        // Small surface in the top-left; the drag happens far from it.
        data.surfaces = vec![SurfaceUI::test_quad("a", 0.0, 0.0, 0.05, 0.05)];
        let mut harness = stage_harness(data, tool(DrawingTool::Select));
        let content = settle(&mut harness);

        stage_drag(
            &mut harness,
            canvas_pt(content, 600.0, 150.0),
            canvas_pt(content, 900.0, 30.0),
        );

        let state = harness.state().state.clone().expect("state persisted");
        assert!(
            state.selected_surfaces.is_empty(),
            "marquee over empty canvas selects nothing: {:?}",
            state.selected_surfaces
        );
    }

    /// Polygon vertices accumulate in state and nothing is committed until the
    /// ring is closed.
    #[test]
    fn polygon_tool_accumulates_vertices_without_committing() {
        let mut data = UIData::test_fixture();
        data.surfaces = vec![];
        let mut harness = stage_harness(data, tool(DrawingTool::Polygon));
        let content = settle(&mut harness);

        for (right, up) in [(200.0, 150.0), (400.0, 150.0), (400.0, 50.0)] {
            let pt = canvas_pt(content, right, up);
            harness.event(egui::Event::PointerMoved(pt));
            harness.event(egui::Event::PointerButton {
                pos: pt,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            });
            harness.run();
            harness.event(egui::Event::PointerButton {
                pos: pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            });
            harness.run();
        }

        let state = harness.state().state.clone().expect("state persisted");
        assert_eq!(
            state.polygon_verts.len(),
            3,
            "three clicks accumulate three vertices: {:?}",
            state.polygon_verts
        );
        assert!(
            added_polygons(harness.state()).is_empty(),
            "nothing committed before the ring closes: {:?}",
            harness.state().commands
        );
    }
}
