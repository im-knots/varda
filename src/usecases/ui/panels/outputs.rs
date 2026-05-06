//! Output windows management and warp calibration.

use crate::renderer::context::{OutputSource, OutputTarget};
use crate::surface::SurfaceOutputType;
use super::super::{UIData, UIActions, OutputAction};

pub(super) fn render_output_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("📺 Outputs");

    // New output button
    ui.horizontal(|ui| {
        if ui.button("+ New Output").clicked() {
            actions.output_actions.push(OutputAction::Create);
        }
    });

    // List existing output windows
    if data.output_windows.is_empty() {
        ui.label(egui::RichText::new("No output windows").small().color(egui::Color32::GRAY));
    } else {
        for (idx, output) in data.output_windows.iter().enumerate() {
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(egui::Color32::from_rgb(30, 30, 45))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&output.name).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("x").on_hover_text("Close output").clicked() {
                                actions.output_actions.push(OutputAction::Close { idx });
                            }
                        });
                    });

                    // Display target selector
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Display:").small());
                        egui::ComboBox::from_id_salt(format!("output_target_{}", idx))
                            .selected_text(egui::RichText::new(&output.target_label).small())
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                // Windowed option
                                if ui.selectable_label(!output.is_on_display, "Windowed").clicked() {
                                    actions.output_actions.push(OutputAction::SetTarget {
                                        idx,
                                        target: OutputTarget::Windowed,
                                    });
                                }
                                // Available display monitors
                                for monitor in &data.available_monitors {
                                    let label = format!("{} ({}x{})", monitor.name, monitor.width, monitor.height);
                                    if ui.selectable_label(false, &label).clicked() {
                                        actions.output_actions.push(OutputAction::SetTarget {
                                            idx,
                                            target: OutputTarget::Display {
                                                name: monitor.name.clone(),
                                                monitor_index: monitor.index,
                                            },
                                        });
                                    }
                                }
                            });
                    });

                    // Calibration toggle
                    ui.horizontal(|ui| {
                        let cal_label = if output.calibration_mode { "🔧 Done" } else { "🔧 Calibrate" };
                        if ui.button(egui::RichText::new(cal_label).small()).clicked() {
                            actions.output_actions.push(OutputAction::ToggleCalibration { idx });
                        }
                    });

                    // Surface assignments
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Surfaces:").small().strong());

                    // Assign surface dropdown
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(format!("assign_surf_{}", idx))
                            .selected_text("+ Assign Surface")
                            .width(140.0)
                            .show_ui(ui, |ui| {
                                for (si, surface) in data.surfaces.iter().enumerate() {
                                    let already_assigned = output.surface_assignments.iter()
                                        .any(|a| a.surface_idx == si);
                                    if !already_assigned {
                                        if ui.selectable_label(false, &surface.name).clicked() {
                                            actions.output_actions.push(OutputAction::AssignSurface {
                                                output_idx: idx,
                                                surface_idx: si,
                                            });
                                        }
                                    }
                                }
                            });
                    });

                    // List assigned surfaces with warp controls
                    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&assignment.surface_name).small());
                            if ui.small_button("↺").on_hover_text("Reset warp").clicked() {
                                actions.output_actions.push(OutputAction::ResetWarp {
                                    output_idx: idx,
                                    assignment_idx: ai,
                                });
                            }
                            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                                actions.output_actions.push(OutputAction::UnassignSurface {
                                    output_idx: idx,
                                    assignment_idx: ai,
                                });
                            }
                        });
                    }

                    // Warp calibration mini-canvas (when in calibration mode)
                    if output.calibration_mode && !output.surface_assignments.is_empty() {
                        render_warp_calibration(ui, idx, output, actions);
                    }
                });
            ui.add_space(4.0);
        }
    }
}

/// Render the warp calibration mini-canvas for an output.
/// Shows surface assignments as quads with draggable corner handles.
pub(super) fn render_warp_calibration(
    ui: &mut egui::Ui,
    output_idx: usize,
    output: &super::super::OutputWindowUI,
    actions: &mut UIActions,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("⊞ Drag corners to warp").small().color(egui::Color32::YELLOW));

    let canvas_width = ui.available_width() - 4.0;
    let canvas_height = canvas_width * 0.5625; // 16:9
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Dark background
    painter.rect_filled(canvas_rect, 2.0, egui::Color32::from_rgb(15, 15, 25));

    // Convert normalized [0..1] to canvas pixel position
    let to_screen = |nx: f32, ny: f32| -> egui::Pos2 {
        egui::pos2(
            canvas_rect.left() + nx * canvas_width,
            canvas_rect.top() + ny * canvas_height,
        )
    };
    let from_screen = |pos: egui::Pos2| -> [f32; 2] {
        [
            ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0),
            ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0),
        ]
    };

    let corner_colors = [
        egui::Color32::from_rgb(255, 100, 100), // TL - red
        egui::Color32::from_rgb(100, 255, 100), // TR - green
        egui::Color32::from_rgb(100, 100, 255), // BR - blue
        egui::Color32::from_rgb(255, 255, 100), // BL - yellow
    ];
    let corner_labels = ["TL", "TR", "BR", "BL"];

    // State for dragging — store as Option<(usize, usize)> consistently
    let state_id = ui.id().with("warp_cal").with(output_idx);
    let mut dragging: Option<(usize, usize)> = ui.memory(|mem| {
        mem.data.get_temp::<Option<(usize, usize)>>(state_id).flatten()
    });

    // Draw each assigned surface's warp quad and handles
    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        let corners = assignment.warp_corners;

        // Draw the warp quad outline
        let screen_corners: Vec<egui::Pos2> = corners.iter()
            .map(|c| to_screen(c[0], c[1]))
            .collect();
        for i in 0..4 {
            let j = (i + 1) % 4;
            painter.line_segment(
                [screen_corners[i], screen_corners[j]],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(200, 200, 200)),
            );
        }

        // Draw surface name at center
        let cx = corners.iter().map(|c| c[0]).sum::<f32>() / 4.0;
        let cy = corners.iter().map(|c| c[1]).sum::<f32>() / 4.0;
        painter.text(
            to_screen(cx, cy),
            egui::Align2::CENTER_CENTER,
            &assignment.surface_name,
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );

        // Draw corner handles
        let handle_radius = 8.0;
        for (ci, corner) in corners.iter().enumerate() {
            let pos = to_screen(corner[0], corner[1]);
            let is_dragging = dragging == Some((ai, ci));
            let r = if is_dragging { handle_radius + 2.0 } else { handle_radius };
            painter.circle_filled(pos, r, corner_colors[ci]);
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                corner_labels[ci],
                egui::FontId::proportional(9.0),
                egui::Color32::BLACK,
            );
        }
    }

    // Handle dragging
    if canvas_response.drag_started() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            // Find nearest corner
            let mut best: Option<(usize, usize, f32)> = None;
            for (ai, assignment) in output.surface_assignments.iter().enumerate() {
                for (ci, corner) in assignment.warp_corners.iter().enumerate() {
                    let screen_pos = to_screen(corner[0], corner[1]);
                    let dist = pos.distance(screen_pos);
                    if dist < 20.0 {
                        if best.is_none() || dist < best.unwrap().2 {
                            best = Some((ai, ci, dist));
                        }
                    }
                }
            }
            if let Some((ai, ci, _)) = best {
                dragging = Some((ai, ci));
            }
        }
    }

    if let Some((ai, ci)) = dragging {
        if canvas_response.dragged() {
            if let Some(pos) = canvas_response.interact_pointer_pos() {
                let new_pos = from_screen(pos);
                actions.output_actions.push(OutputAction::SetWarpCorner {
                    output_idx,
                    assignment_idx: ai,
                    corner_idx: ci,
                    position: new_pos,
                });
            }
        }
    }

    if canvas_response.drag_stopped() {
        dragging = None;
    }

    ui.memory_mut(|mem| mem.data.insert_temp(state_id, dragging));
}

