//! Stage editor toolbars: the title row, mode switch, tool palette, and the
//! contextual actions row (edit / order / import / detect).

use super::super::super::{CameraDetectAction, DomeAction, UIActions, UIData};
use super::state::{DrawingTool, StageEditorMode, StageEditorState};
use crate::engine::EngineCommand;
use crate::surface::SurfaceReorderOp;

#[allow(clippy::too_many_lines)]
pub(super) fn render(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
) {
    // Toolbar at top
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("🎨 Stage Editor").strong().size(16.0));
        ui.separator();

        // Tool buttons
        let tools = [
            (
                DrawingTool::Select,
                "⬚ Select",
                "Select and edit surfaces (S)",
            ),
            (
                DrawingTool::Rectangle,
                "▭ Rectangle",
                "Draw rectangle surfaces (R)",
            ),
            (
                DrawingTool::Polygon,
                "⬠ Polygon",
                "Draw polygon surfaces — click to add vertices, double-click to finish (P)",
            ),
            (
                DrawingTool::Circle,
                "⬤ Circle",
                "Draw circle/N-gon surfaces (C)",
            ),
            (
                DrawingTool::Bezier,
                "✒ Bezier",
                "Bezier edit — click an edge to curve/straighten it, drag anchors & handles",
            ),
        ];
        for (tool, label, tooltip) in &tools {
            let selected = state.tool == *tool;
            let btn = ui.selectable_label(selected, *label);
            if btn.on_hover_text(*tooltip).clicked() {
                state.tool = *tool;
                // Clear any in-progress drawing
                state.rect_start = None;
                state.polygon_verts.clear();
                state.circle_center = None;
            }
        }

        // Mode toggle + close stay on the tools row, pinned to the right.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("x Close Editor").clicked() {
                actions.session.toggle_stage_editor = true;
            }
            ui.separator();
            let mode = if data.dome_mode_active {
                StageEditorMode::Dome3D
            } else {
                StageEditorMode::Polygon2D
            };
            if ui
                .selectable_label(mode == StageEditorMode::Polygon2D, "⬡ 2D")
                .on_hover_text("2D Polygon mode")
                .clicked()
            {
                actions
                    .session
                    .dome_actions
                    .push(DomeAction::SetMode(false));
            }
            if ui
                .selectable_label(mode == StageEditorMode::Dome3D, "🔮 3D Dome")
                .on_hover_text("3D Dome mode")
                .clicked()
            {
                actions.session.dome_actions.push(DomeAction::SetMode(true));
            }
        });
    });

    // Second toolbar row: contextual actions (edit · order · import · detect).
    // Wraps onto more lines on narrow windows so the controls never bunch up.
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        // "Make Hole" (8i.7): turn the single selected surface into a cut-out in
        // the surface beneath it, consuming the source. Enabled only when
        // exactly one surface is selected.
        let can_punch = state.selected_surfaces.len() == 1;
        if ui
            .add_enabled(can_punch, egui::Button::new("◌ Make Hole"))
            .on_hover_text(
                "Cut the selected surface out of the surface beneath it (the source is consumed)",
            )
            .on_disabled_hover_text(
                "Select exactly one surface to cut it out of the one beneath it",
            )
            .clicked()
        {
            if let Some(source_uuid) = state.selected_surfaces.iter().next().cloned() {
                actions
                    .commands
                    .push(EngineCommand::PunchSurfaceHole { source_uuid });
                state.selected_surfaces.clear();
            }
        }

        ui.separator();

        // Grid controls
        let snap_label = if data.stage_editor_snap {
            "🧲 Snap: ON"
        } else {
            "🧲 Snap: OFF"
        };
        if ui.button(snap_label).clicked() {
            actions.session.toggle_snap = true;
        }

        // Grid size selector
        let grid_sizes = [
            (0.1, "10%"),
            (0.05, "5%"),
            (0.025, "2.5%"),
            (0.0125, "1.25%"),
        ];
        egui::ComboBox::from_id_salt("grid_size")
            .selected_text(format!("Grid: {:.1}%", data.stage_editor_grid_size * 100.0))
            .width(90.0)
            .show_ui(ui, |ui| {
                for (size, label) in &grid_sizes {
                    if ui
                        .selectable_value(&mut actions.session.set_grid_size, Some(*size), *label)
                        .clicked()
                    {
                        // handled by set_grid_size
                    }
                }
            });

        // Circle sides (only when circle tool selected)
        if state.tool == DrawingTool::Circle {
            ui.separator();
            ui.label("Sides:");
            if state.circle_sides == 0 {
                state.circle_sides = 32;
            }
            ui.add(
                egui::DragValue::new(&mut state.circle_sides)
                    .range(3..=128)
                    .speed(1),
            );
        }

        // Circle-specific toolbar: when exactly one circle is selected, show radius/sides/convert
        let selected_circle = if state.selected_surfaces.len() == 1 {
            let sel_uuid = state.selected_surfaces.iter().next().unwrap().clone();
            data.surfaces
                .iter()
                .find(|s| s.uuid == sel_uuid)
                .and_then(|s| s.circle_hint.map(|h| (sel_uuid, h)))
        } else {
            None
        };
        if let Some((sel_uuid, hint)) = selected_circle {
            ui.separator();
            ui.label("⬤ Circle:");
            let mut radius = hint.radius;
            if ui
                .add(
                    egui::DragValue::new(&mut radius)
                        .prefix("R: ")
                        .range(0.01..=1.0)
                        .speed(0.005),
                )
                .changed()
            {
                actions.commands.push(EngineCommand::SetCircleRadius {
                    uuid: sel_uuid.clone(),
                    radius,
                });
            }
            let mut sides = hint.sides;
            if ui
                .add(
                    egui::DragValue::new(&mut sides)
                        .prefix("Sides: ")
                        .range(3..=128)
                        .speed(1),
                )
                .changed()
            {
                actions.commands.push(EngineCommand::SetCircleSides {
                    uuid: sel_uuid.clone(),
                    sides,
                });
            }
            if ui
                .button("⬠ Convert to Polygon")
                .on_hover_text("Drop circle identity, keep vertices as polygon")
                .clicked()
            {
                actions
                    .commands
                    .push(EngineCommand::ConvertSurfaceToPolygon { uuid: sel_uuid });
            }
        }

        // Duplicate & flip (enabled when any surfaces are selected)
        ui.separator();
        let has_sel = !state.selected_surfaces.is_empty();
        ui.add_enabled_ui(has_sel, |ui| {
            if ui
                .button("📋 Dup")
                .on_hover_text("Duplicate selected (D)")
                .clicked()
            {
                for uuid in &state.selected_surfaces {
                    actions
                        .commands
                        .push(EngineCommand::DuplicateSurface { uuid: uuid.clone() });
                }
            }
            if ui
                .button("↔ Flip H")
                .on_hover_text("Flip horizontal (H)")
                .clicked()
            {
                for uuid in &state.selected_surfaces {
                    actions
                        .commands
                        .push(EngineCommand::FlipSurfaceHorizontal { uuid: uuid.clone() });
                }
            }
            if ui
                .button("↕ Flip V")
                .on_hover_text("Flip vertical (V)")
                .clicked()
            {
                for uuid in &state.selected_surfaces {
                    actions
                        .commands
                        .push(EngineCommand::FlipSurfaceVertical { uuid: uuid.clone() });
                }
            }
            if state.selected_surfaces.len() >= 2
                && ui
                    .button("🔗 Combine")
                    .on_hover_text("Combine selected surfaces (G)")
                    .clicked()
            {
                let uuids: Vec<String> = state.selected_surfaces.iter().cloned().collect();
                actions
                    .commands
                    .push(EngineCommand::CombineSurfaces { uuids });
                state.selected_surfaces.clear();
            }
            // Stacking order (8i.12): bring the selection to the very front/back.
            if ui
                .button("⤒ Front")
                .on_hover_text("Bring selected to front")
                .clicked()
            {
                for uuid in &state.selected_surfaces {
                    actions.commands.push(EngineCommand::ReorderSurface {
                        uuid: uuid.clone(),
                        op: SurfaceReorderOp::ToFront,
                    });
                }
            }
            if ui
                .button("⤓ Back")
                .on_hover_text("Send selected to back")
                .clicked()
            {
                for uuid in &state.selected_surfaces {
                    actions.commands.push(EngineCommand::ReorderSurface {
                        uuid: uuid.clone(),
                        op: SurfaceReorderOp::ToBack,
                    });
                }
            }
        });

        // Import from file
        if ui.button("📁 Import").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Stage Plans", &["png", "jpg", "jpeg", "svg", "dxf"])
                .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
                .add_filter("SVG", &["svg"])
                .add_filter("DXF", &["dxf"])
                .pick_file()
            {
                actions
                    .commands
                    .push(EngineCommand::ImportSurfacesFromFile { path });
            }
        }

        // Camera detect button — 0 cameras: hidden; 1: direct click; N: dropdown
        let active_cameras = &data.cameras;
        if active_cameras.len() == 1 {
            if ui
                .button("📷 Detect")
                .on_hover_text("Enter camera detection mode")
                .clicked()
            {
                actions
                    .session
                    .camera_detect_actions
                    .push(CameraDetectAction::Enter {
                        camera_id: active_cameras[0].1,
                    });
            }
        } else if active_cameras.len() > 1 {
            let cam_btn = ui
                .button("📷 Detect ▼")
                .on_hover_text("Enter camera detection mode");
            let cam_popup_id = cam_btn.id.with("cam_detect_popup");
            egui::Popup::from_toggle_button_response(&cam_btn)
                .id(cam_popup_id)
                .width(cam_btn.rect.width())
                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                .show(|ui| {
                    ui.set_min_width(150.0);
                    for (name, cam_id) in active_cameras {
                        if ui.button(name).clicked() {
                            actions
                                .session
                                .camera_detect_actions
                                .push(CameraDetectAction::Enter { camera_id: *cam_id });
                        }
                    }
                });
        }
    });
}
