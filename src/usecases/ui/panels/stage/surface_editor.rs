//! Right-panel surface editor: the surface list with inline reorder, source and
//! mapping controls.
//!
//! Distinct from the full-screen stage editor in `super` — this is the compact
//! editor hosted by `panels::right_panel`.

use super::super::super::{UIActions, UIData};
use super::geometry::polygon_shape;
use crate::engine::EngineCommand;
use crate::renderer::context::OutputSource;
use crate::surface::{ContentMapping, SurfaceOutputType, SurfaceReorderOp};

/// Drag state for the surface canvas editor
#[derive(Debug, Clone, Default)]
enum SurfaceDragState {
    #[default]
    None,
    Moving {
        uuid: String,
        last_x: f32,
        last_y: f32,
    },
    DraggingVertex {
        uuid: String,
        vert_idx: usize,
    },
}

// dx_px/dy_px style x/y pairs are the clearest names for this canvas geometry.
#[allow(clippy::similar_names)]
pub(crate) fn render_surface_editor(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // Open/Close Editor button
    ui.horizontal(|ui| {
        let editor_label = if data.stage_editor_open {
            "✏ Close Editor"
        } else {
            "✏ Open Editor"
        };
        if ui.button(editor_label).clicked() {
            actions.session.toggle_stage_editor = true;
        }
    });

    ui.add_space(4.0);

    // 2D Canvas — draw surfaces as rectangles. Surfaces are stored in
    // normalised output coordinates, so the canvas has to carry the render
    // aspect or a square surface is drawn as a wide one.
    let width = ui.available_width() - 4.0;
    let canvas = crate::usecases::ui::panels::utils::preview_size(
        egui::vec2(width, width),
        data.render_width,
        data.render_height,
    );
    let canvas_width = canvas.x;
    let canvas_height = canvas.y;
    let (canvas_rect, canvas_response) =
        ui.allocate_exact_size(canvas, egui::Sense::click_and_drag());

    let painter = ui.painter_at(canvas_rect);

    // Canvas background (dark stage)
    painter.rect_filled(canvas_rect, 4.0, egui::Color32::from_rgb(15, 15, 25));
    painter.rect_stroke(
        canvas_rect,
        4.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 60, 80)),
        egui::StrokeKind::Outside,
    );

    // Grid lines
    for i in 1..4 {
        let x = canvas_rect.left() + canvas_width * (i as f32 / 4.0);
        painter.line_segment(
            [
                egui::pos2(x, canvas_rect.top()),
                egui::pos2(x, canvas_rect.bottom()),
            ],
            egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(30, 30, 45)),
        );
    }
    for i in 1..3 {
        let y = canvas_rect.top() + canvas_height * (i as f32 / 3.0);
        painter.line_segment(
            [
                egui::pos2(canvas_rect.left(), y),
                egui::pos2(canvas_rect.right(), y),
            ],
            egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(30, 30, 45)),
        );
    }

    // Draw each surface
    let surface_colors = [
        egui::Color32::from_rgb(80, 140, 220),
        egui::Color32::from_rgb(220, 120, 80),
        egui::Color32::from_rgb(80, 200, 120),
        egui::Color32::from_rgb(200, 80, 200),
        egui::Color32::from_rgb(200, 200, 80),
        egui::Color32::from_rgb(80, 200, 200),
    ];

    for (i, surface) in data.surfaces.iter().enumerate() {
        let color = surface_colors[i % surface_colors.len()];
        let fill = egui::Color32::from_rgba_premultiplied(
            color.r() / 4,
            color.g() / 4,
            color.b() / 4,
            160,
        );

        // Convert normalized vertices to canvas pixel positions
        let pixel_verts: Vec<egui::Pos2> = surface
            .vertices
            .iter()
            .map(|v| {
                egui::pos2(
                    canvas_rect.left() + v[0] * canvas_width,
                    canvas_rect.top() + v[1] * canvas_height,
                )
            })
            .collect();

        if pixel_verts.len() >= 3 {
            painter.add(polygon_shape(
                &pixel_verts,
                fill,
                egui::Stroke::new(1.5_f32, color),
            ));
        } else if pixel_verts.len() == 2 {
            painter.line_segment(
                [pixel_verts[0], pixel_verts[1]],
                egui::Stroke::new(1.5_f32, color),
            );
        }
        // Draw extra contours (combined non-overlapping surfaces)
        for ec in &surface.extra_contours {
            let ec_verts: Vec<egui::Pos2> = ec
                .iter()
                .map(|v| {
                    egui::pos2(
                        canvas_rect.left() + v[0] * canvas_width,
                        canvas_rect.top() + v[1] * canvas_height,
                    )
                })
                .collect();
            if ec_verts.len() >= 3 {
                painter.add(polygon_shape(
                    &ec_verts,
                    fill,
                    egui::Stroke::new(1.5_f32, color),
                ));
            }
        }

        // Surface label at center
        let n = surface.vertices.len().max(1) as f32;
        let center = surface
            .vertices
            .iter()
            .fold(egui::pos2(0.0, 0.0), |acc, v| {
                egui::pos2(acc.x + v[0] / n, acc.y + v[1] / n)
            });
        let center_px = egui::pos2(
            canvas_rect.left() + center.x * canvas_width,
            canvas_rect.top() + center.y * canvas_height,
        );
        let label = format!("{}\n{}", surface.name, surface.source);
        painter.text(
            center_px,
            egui::Align2::CENTER_CENTER,
            &label,
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );

        // Output type + mapping mode indicators
        let type_label = match surface.output_type {
            SurfaceOutputType::Projection => "📽",
            SurfaceOutputType::LEDDirect => "💡",
        };
        let mapping_label = match surface.content_mapping {
            ContentMapping::Fill => "▣",
            ContentMapping::Mapped => "▥",
        };
        // Place indicator near first vertex
        if let Some(v0) = pixel_verts.first() {
            painter.text(
                egui::pos2(v0.x + 4.0, v0.y + 4.0),
                egui::Align2::LEFT_TOP,
                format!("{mapping_label}{type_label}"),
                egui::FontId::proportional(9.0),
                egui::Color32::WHITE,
            );
        }

        // Vertex handles
        let handle_size = 5.0;
        for v in &pixel_verts {
            let handle_rect =
                egui::Rect::from_center_size(*v, egui::vec2(handle_size, handle_size));
            painter.rect_filled(handle_rect, 1.0, color);
        }
    }

    // Handle drag interactions on the canvas
    let drag_id = ui.id().with("surface_drag");
    let _drag_state = ui.memory(|mem| mem.data.get_temp::<SurfaceDragState>(drag_id));

    if canvas_response.drag_started() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            let nx = (pos.x - canvas_rect.left()) / canvas_width;
            let ny = (pos.y - canvas_rect.top()) / canvas_height;

            // Check if near a vertex (drag vertex) or inside a surface (move whole shape)
            // Use pixel-space distance for correct hit detection on non-square canvas
            let vertex_threshold_px = 14.0;
            let mut found_vertex = None;
            let mut found_surface = None;

            for (i, surface) in data.surfaces.iter().enumerate().rev() {
                if let Some(vert_idx) = surface
                    .vertices
                    .iter()
                    .enumerate()
                    .map(|(vi, v)| {
                        let dx_px = (nx - v[0]) * canvas_width;
                        let dy_px = (ny - v[1]) * canvas_height;
                        (vi, (dx_px * dx_px + dy_px * dy_px).sqrt())
                    })
                    .filter(|(_, d)| *d < vertex_threshold_px)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(vi, _)| vi)
                {
                    found_vertex = Some((i, vert_idx));
                    break;
                }
                // Point-in-polygon test for move
                if found_surface.is_none() {
                    let verts = &surface.vertices;
                    let n = verts.len();
                    if n >= 3 {
                        let mut inside = false;
                        let mut j = n - 1;
                        for k in 0..n {
                            let (xi, yi) = (verts[k][0], verts[k][1]);
                            let (xj, yj) = (verts[j][0], verts[j][1]);
                            if ((yi > ny) != (yj > ny))
                                && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi)
                            {
                                inside = !inside;
                            }
                            j = k;
                        }
                        if inside {
                            found_surface = Some((i, nx, ny));
                        }
                    }
                }
            }

            let state = if let Some((surf_idx, vert_idx)) = found_vertex {
                let uuid = data.surfaces[surf_idx].uuid.clone();
                SurfaceDragState::DraggingVertex { uuid, vert_idx }
            } else if let Some((surf_idx, start_x, start_y)) = found_surface {
                let uuid = data.surfaces[surf_idx].uuid.clone();
                SurfaceDragState::Moving {
                    uuid,
                    last_x: start_x,
                    last_y: start_y,
                }
            } else {
                SurfaceDragState::None
            };

            ui.memory_mut(|mem| mem.data.insert_temp(drag_id, state));
        }
    }

    if canvas_response.dragged() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            let nx = ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0);
            let ny = ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0);

            let state = ui.memory(|mem| {
                mem.data
                    .get_temp::<SurfaceDragState>(drag_id)
                    .unwrap_or(SurfaceDragState::None)
            });

            match state {
                SurfaceDragState::Moving {
                    ref uuid,
                    last_x,
                    last_y,
                } => {
                    if data.surfaces.iter().any(|s| s.uuid == *uuid) {
                        let dx = nx - last_x;
                        let dy = ny - last_y;
                        actions.commands.push(EngineCommand::MoveSurface {
                            uuid: uuid.clone(),
                            dx,
                            dy,
                        });
                        ui.memory_mut(|mem| {
                            mem.data.insert_temp(
                                drag_id,
                                SurfaceDragState::Moving {
                                    uuid: uuid.clone(),
                                    last_x: nx,
                                    last_y: ny,
                                },
                            )
                        });
                    }
                }
                SurfaceDragState::DraggingVertex { ref uuid, vert_idx } => {
                    if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                        let mut new_verts = surface.vertices.clone();
                        if vert_idx < new_verts.len() {
                            new_verts[vert_idx] = [nx, ny];
                            actions
                                .commands
                                .push(EngineCommand::UpdateSurfaceContourVertices {
                                    uuid: uuid.clone(),
                                    contour: 0,
                                    vertices: new_verts,
                                });
                        }
                    }
                }
                SurfaceDragState::None => {}
            }
        }
    }

    if canvas_response.drag_stopped() {
        ui.memory_mut(|mem| mem.data.insert_temp(drag_id, SurfaceDragState::None));
    }

    ui.add_space(4.0);

    // Surface list with properties
    for (i, surface) in data.surfaces.iter().enumerate() {
        let color = surface_colors[i % surface_colors.len()];
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(3.0)
            .stroke(egui::Stroke::new(1.0_f32, color.linear_multiply(0.5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Color swatch
                    let (swatch_rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 16.0), egui::Sense::hover());
                    ui.painter().rect_filled(swatch_rect, 2.0, color);

                    ui.label(egui::RichText::new(&surface.name).strong().size(11.0));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("x").clicked() {
                            actions.commands.push(EngineCommand::RemoveSurface {
                                uuid: surface.uuid.clone(),
                            });
                        }
                        // Stacking order (8i.12): list is bottom→top (index 0 =
                        // bottom/drawn-first). Up moves toward the front (top).
                        let last = data.surfaces.len().saturating_sub(1);
                        ui.add_enabled_ui(i < last, |ui| {
                            if ui
                                .small_button("▲")
                                .on_hover_text("Move up (toward front)")
                                .clicked()
                            {
                                actions.commands.push(EngineCommand::ReorderSurface {
                                    uuid: surface.uuid.clone(),
                                    op: SurfaceReorderOp::Up,
                                });
                            }
                        });
                        ui.add_enabled_ui(i > 0, |ui| {
                            if ui
                                .small_button("▼")
                                .on_hover_text("Move down (toward back)")
                                .clicked()
                            {
                                actions.commands.push(EngineCommand::ReorderSurface {
                                    uuid: surface.uuid.clone(),
                                    op: SurfaceReorderOp::Down,
                                });
                            }
                        });
                    });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Source:").weak().size(10.0));
                    let current_label = format!("{}", surface.source);
                    let response = ui.button(format!("{current_label} ▼"));
                    let popup_id = response.id.with("surf_src_popup");
                    egui::Popup::from_toggle_button_response(&response)
                        .id(popup_id)
                        .width(response.rect.width())
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                        .show(|ui| {
                            ui.set_min_width(150.0);
                            // Master option
                            if ui
                                .selectable_label(surface.source == OutputSource::Master, "Master")
                                .clicked()
                            {
                                actions.commands.push(EngineCommand::SetSurfaceSource {
                                    uuid: surface.uuid.clone(),
                                    source: OutputSource::Master,
                                });
                            }
                            ui.separator();
                            ui.label(egui::RichText::new("Channels:").weak().size(10.0));
                            // Get currently selected channel indices
                            let selected_indices: Vec<usize> = match &surface.source {
                                OutputSource::Channel(idx) => vec![*idx],
                                OutputSource::Channels(indices) => indices.clone(),
                                _ => vec![],
                            };
                            for ch in &data.channels {
                                let is_selected = selected_indices.contains(&ch.ch_idx);
                                let mut checked = is_selected;
                                if ui.checkbox(&mut checked, &ch.name).changed() {
                                    let mut new_indices = selected_indices.clone();
                                    if checked {
                                        if !new_indices.contains(&ch.ch_idx) {
                                            new_indices.push(ch.ch_idx);
                                        }
                                    } else {
                                        new_indices.retain(|&idx| idx != ch.ch_idx);
                                    }
                                    new_indices.sort_unstable();
                                    let new_source = match new_indices.len() {
                                        0 => OutputSource::Master,
                                        1 => OutputSource::Channel(new_indices[0]),
                                        _ => OutputSource::Channels(new_indices),
                                    };
                                    actions.commands.push(EngineCommand::SetSurfaceSource {
                                        uuid: surface.uuid.clone(),
                                        source: new_source,
                                    });
                                }
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Mapping:").weak().size(10.0));
                    egui::ComboBox::from_id_salt(format!("surf_map_{i}"))
                        .selected_text(format!("{}", surface.content_mapping))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    surface.content_mapping == ContentMapping::Fill,
                                    "Fill",
                                )
                                .on_hover_text("Entire source scaled to fill this surface")
                                .clicked()
                            {
                                actions
                                    .commands
                                    .push(EngineCommand::SetSurfaceContentMapping {
                                        uuid: surface.uuid.clone(),
                                        mapping: ContentMapping::Fill,
                                    });
                            }
                            if ui
                                .selectable_label(
                                    surface.content_mapping == ContentMapping::Mapped,
                                    "Mapped",
                                )
                                .on_hover_text("Surface position on canvas = UV crop into source")
                                .clicked()
                            {
                                actions
                                    .commands
                                    .push(EngineCommand::SetSurfaceContentMapping {
                                        uuid: surface.uuid.clone(),
                                        mapping: ContentMapping::Mapped,
                                    });
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Type:").weak().size(10.0));
                    egui::ComboBox::from_id_salt(format!("surf_type_{i}"))
                        .selected_text(format!("{}", surface.output_type))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    surface.output_type == SurfaceOutputType::Projection,
                                    "📽 Projection",
                                )
                                .clicked()
                            {
                                actions.commands.push(EngineCommand::SetSurfaceOutputType {
                                    uuid: surface.uuid.clone(),
                                    output_type: SurfaceOutputType::Projection,
                                });
                            }
                            if ui
                                .selectable_label(
                                    surface.output_type == SurfaceOutputType::LEDDirect,
                                    "💡 LED Direct",
                                )
                                .clicked()
                            {
                                actions.commands.push(EngineCommand::SetSurfaceOutputType {
                                    uuid: surface.uuid.clone(),
                                    output_type: SurfaceOutputType::LEDDirect,
                                });
                            }
                        });
                });

                // Precision transform: bounds of the primary contour (X/Y = position,
                // W/H = size). Editing emits Move/Scale so it stays in sync with the gizmo.
                {
                    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
                    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
                    for v in &surface.vertices {
                        min_x = min_x.min(v[0]);
                        min_y = min_y.min(v[1]);
                        max_x = max_x.max(v[0]);
                        max_y = max_y.max(v[1]);
                    }
                    if min_x <= max_x {
                        let (x0, y0, w0, h0) = (min_x, min_y, max_x - min_x, max_y - min_y);
                        let (mut xv, mut yv) = (x0, y0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("X").weak().size(10.0));
                            let rx =
                                ui.add(egui::DragValue::new(&mut xv).speed(0.002).max_decimals(3));
                            ui.label(egui::RichText::new("Y").weak().size(10.0));
                            let ry =
                                ui.add(egui::DragValue::new(&mut yv).speed(0.002).max_decimals(3));
                            if rx.changed() || ry.changed() {
                                actions.commands.push(EngineCommand::MoveSurface {
                                    uuid: surface.uuid.clone(),
                                    dx: xv - x0,
                                    dy: yv - y0,
                                });
                            }
                        });
                        let (mut wv, mut hv) = (w0, h0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("W").weak().size(10.0));
                            let rw =
                                ui.add(egui::DragValue::new(&mut wv).speed(0.002).max_decimals(3));
                            ui.label(egui::RichText::new("H").weak().size(10.0));
                            let rh =
                                ui.add(egui::DragValue::new(&mut hv).speed(0.002).max_decimals(3));
                            if rw.changed() || rh.changed() {
                                let sx = if w0 > 1e-5 { wv.max(0.001) / w0 } else { 1.0 };
                                let sy = if h0 > 1e-5 { hv.max(0.001) / h0 } else { 1.0 };
                                actions.commands.push(EngineCommand::ScaleSurface {
                                    uuid: surface.uuid.clone(),
                                    sx,
                                    sy,
                                    pivot: [x0, y0],
                                });
                            }
                        });
                    }
                }
            });
        ui.add_space(2.0);
    }

    if data.surfaces.is_empty() {
        ui.label(
            egui::RichText::new("No surfaces. Add one to define your stage layout.")
                .weak()
                .small(),
        );
    }
}
