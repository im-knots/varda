//! Surface editor and stage editor panels.

use crate::renderer::context::OutputSource;
use crate::renderer::slicer::DomePreset;
use crate::surface::{CircleHint, ContentMapping, SurfaceOutputType};
use crate::surface::detect::{DetectionMethod, HullMode};
use super::super::{UIData, UIActions, SurfaceAction, DomeAction, CameraDetectMode, CameraDetectAction};
use super::geometry::polygon_shape;

/// Stage editor mode: 2D polygon editing or 3D dome mode.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum StageEditorMode {
    #[default]
    Polygon2D,
    Dome3D,
}

pub(super) fn render_surface_editor(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // Open/Close Editor button
    ui.horizontal(|ui| {
        let editor_label = if data.stage_editor_open { "✏ Close Editor" } else { "✏ Open Editor" };
        if ui.button(editor_label).clicked() {
            actions.toggle_stage_editor = true;
        }
    });

    ui.add_space(4.0);

    // 2D Canvas — draw surfaces as rectangles
    let canvas_width = ui.available_width() - 4.0;
    let canvas_height = canvas_width * 0.5625; // 16:9 aspect
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Canvas background (dark stage)
    painter.rect_filled(canvas_rect, 4.0, egui::Color32::from_rgb(15, 15, 25));
    painter.rect_stroke(canvas_rect, 4.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 80)), egui::StrokeKind::Outside);

    // Grid lines
    for i in 1..4 {
        let x = canvas_rect.left() + canvas_width * (i as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(x, canvas_rect.top()), egui::pos2(x, canvas_rect.bottom())],
            egui::Stroke::new(0.5, egui::Color32::from_rgb(30, 30, 45)),
        );
    }
    for i in 1..3 {
        let y = canvas_rect.top() + canvas_height * (i as f32 / 3.0);
        painter.line_segment(
            [egui::pos2(canvas_rect.left(), y), egui::pos2(canvas_rect.right(), y)],
            egui::Stroke::new(0.5, egui::Color32::from_rgb(30, 30, 45)),
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
        let fill = egui::Color32::from_rgba_premultiplied(color.r() / 4, color.g() / 4, color.b() / 4, 160);

        // Convert normalized vertices to canvas pixel positions
        let pixel_verts: Vec<egui::Pos2> = surface.vertices.iter().map(|v| {
            egui::pos2(
                canvas_rect.left() + v[0] * canvas_width,
                canvas_rect.top() + v[1] * canvas_height,
            )
        }).collect();

        if pixel_verts.len() >= 3 {
            painter.add(polygon_shape(&pixel_verts, fill, egui::Stroke::new(1.5, color)));
        } else if pixel_verts.len() == 2 {
            painter.line_segment([pixel_verts[0], pixel_verts[1]], egui::Stroke::new(1.5, color));
        }
        // Draw extra contours (combined non-overlapping surfaces)
        for ec in &surface.extra_contours {
            let ec_verts: Vec<egui::Pos2> = ec.iter().map(|v| {
                egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
            }).collect();
            if ec_verts.len() >= 3 {
                painter.add(polygon_shape(&ec_verts, fill, egui::Stroke::new(1.5, color)));
            }
        }

        // Surface label at center
        let n = surface.vertices.len().max(1) as f32;
        let center = surface.vertices.iter().fold(egui::pos2(0.0, 0.0), |acc, v| {
            egui::pos2(acc.x + v[0] / n, acc.y + v[1] / n)
        });
        let center_px = egui::pos2(
            canvas_rect.left() + center.x * canvas_width,
            canvas_rect.top() + center.y * canvas_height,
        );
        let label = format!("{}\n{}", surface.name, surface.source);
        painter.text(center_px, egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(10.0), egui::Color32::WHITE);

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
                &format!("{}{}", mapping_label, type_label),
                egui::FontId::proportional(9.0),
                egui::Color32::WHITE,
            );
        }

        // Vertex handles
        let handle_size = 5.0;
        for v in &pixel_verts {
            let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(handle_size, handle_size));
            painter.rect_filled(handle_rect, 1.0, color);
        }
    }

    // Handle drag interactions on the canvas
    let drag_id = ui.id().with("surface_drag");
    let _drag_state = ui.memory(|mem| {
        mem.data.get_temp::<SurfaceDragState>(drag_id)
    });

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
                if let Some(vert_idx) = surface.vertices.iter().enumerate()
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
                            if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
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
                SurfaceDragState::Moving { uuid, last_x: start_x, last_y: start_y }
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
                mem.data.get_temp::<SurfaceDragState>(drag_id).unwrap_or(SurfaceDragState::None)
            });

            match state {
                SurfaceDragState::Moving { ref uuid, last_x, last_y } => {
                    if data.surfaces.iter().any(|s| s.uuid == *uuid) {
                        let dx = nx - last_x;
                        let dy = ny - last_y;
                        actions.surface_actions.push(SurfaceAction::MoveDelta {
                            uuid: uuid.clone(), dx, dy,
                        });
                        ui.memory_mut(|mem| mem.data.insert_temp(drag_id,
                            SurfaceDragState::Moving { uuid: uuid.clone(), last_x: nx, last_y: ny }));
                    }
                }
                SurfaceDragState::DraggingVertex { ref uuid, vert_idx } => {
                    if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                        let mut new_verts = surface.vertices.clone();
                        if vert_idx < new_verts.len() {
                            new_verts[vert_idx] = [nx, ny];
                            actions.surface_actions.push(SurfaceAction::UpdateVertices {
                                uuid: uuid.clone(), contour: 0, vertices: new_verts,
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
            .stroke(egui::Stroke::new(1.0, color.linear_multiply(0.5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Color swatch
                    let (swatch_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), egui::Sense::hover());
                    ui.painter().rect_filled(swatch_rect, 2.0, color);

                    ui.label(egui::RichText::new(&surface.name).strong().size(11.0));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("x").clicked() {
                            actions.surface_actions.push(SurfaceAction::Remove { uuid: surface.uuid.clone() });
                        }
                    });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Source:").weak().size(10.0));
                    let current_label = format!("{}", surface.source);
                    let response = ui.button(format!("{} ▼", current_label));
                    let popup_id = response.id.with("surf_src_popup");
                    if response.clicked() {
                        #[allow(deprecated)]
                        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                    }
                    #[allow(deprecated)]
                    egui::popup_below_widget(ui, popup_id, &response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(150.0);
                        // Master option
                        if ui.selectable_label(surface.source == OutputSource::Master, "Master").clicked() {
                            actions.surface_actions.push(SurfaceAction::SetSource {
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
                                new_indices.sort();
                                let new_source = match new_indices.len() {
                                    0 => OutputSource::Master,
                                    1 => OutputSource::Channel(new_indices[0]),
                                    _ => OutputSource::Channels(new_indices),
                                };
                                actions.surface_actions.push(SurfaceAction::SetSource {
                                    uuid: surface.uuid.clone(),
                                    source: new_source,
                                });
                            }
                        }
                    });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Mapping:").weak().size(10.0));
                    egui::ComboBox::from_id_salt(format!("surf_map_{}", i))
                        .selected_text(format!("{}", surface.content_mapping))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(
                                surface.content_mapping == ContentMapping::Fill,
                                "Fill",
                            ).on_hover_text("Entire source scaled to fill this surface")
                            .clicked() {
                                actions.surface_actions.push(SurfaceAction::SetContentMapping {
                                    uuid: surface.uuid.clone(),
                                    mapping: ContentMapping::Fill,
                                });
                            }
                            if ui.selectable_label(
                                surface.content_mapping == ContentMapping::Mapped,
                                "Mapped",
                            ).on_hover_text("Surface position on canvas = UV crop into source")
                            .clicked() {
                                actions.surface_actions.push(SurfaceAction::SetContentMapping {
                                    uuid: surface.uuid.clone(),
                                    mapping: ContentMapping::Mapped,
                                });
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Type:").weak().size(10.0));
                    egui::ComboBox::from_id_salt(format!("surf_type_{}", i))
                        .selected_text(format!("{}", surface.output_type))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(
                                surface.output_type == SurfaceOutputType::Projection,
                                "📽 Projection",
                            ).clicked() {
                                actions.surface_actions.push(SurfaceAction::SetOutputType {
                                    uuid: surface.uuid.clone(),
                                    output_type: SurfaceOutputType::Projection,
                                });
                            }
                            if ui.selectable_label(
                                surface.output_type == SurfaceOutputType::LEDDirect,
                                "💡 LED Direct",
                            ).clicked() {
                                actions.surface_actions.push(SurfaceAction::SetOutputType {
                                    uuid: surface.uuid.clone(),
                                    output_type: SurfaceOutputType::LEDDirect,
                                });
                            }
                        });
                });
            });
        ui.add_space(2.0);
    }

    if data.surfaces.is_empty() {
        ui.label(egui::RichText::new("No surfaces. Add one to define your stage layout.").weak().small());
    }
}

/// Drag state for the surface canvas editor
#[derive(Debug, Clone, Default)]
enum SurfaceDragState {
    #[default]
    None,
    Moving { uuid: String, last_x: f32, last_y: f32 },
    DraggingVertex { uuid: String, vert_idx: usize },
}

/// Drawing tool for the stage editor
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum DrawingTool {
    #[default]
    Select,
    Rectangle,
    Polygon,
    Circle,
}

/// State for active drawing operations in the stage editor
#[derive(Debug, Clone, Default)]
struct StageEditorState {
    tool: DrawingTool,
    /// For rectangle tool: start position of drag
    rect_start: Option<[f32; 2]>,
    /// For polygon tool: accumulated vertices
    polygon_verts: Vec<[f32; 2]>,
    /// For circle tool: center position
    circle_center: Option<[f32; 2]>,
    /// Number of sides for circle/N-gon approximation
    circle_sides: u32,
    /// Currently selected surface UUIDs (supports multi-select)
    selected_surfaces: std::collections::BTreeSet<String>,
    /// Drag state for vertex editing in select mode
    dragging_vertex: Option<(String, usize, usize)>, // (surface_uuid, contour_idx, vertex_idx)
    /// Drag state for moving whole surface in select mode
    moving_surface: Option<(String, f32, f32)>, // (surface_uuid, last_x, last_y)
    /// Marquee selection: start position of drag rectangle in normalized coords
    selection_rect_start: Option<[f32; 2]>,
    /// Drag state for radius handle on circle surfaces
    dragging_radius: Option<String>, // surface_uuid
    /// Drag state for edge dragging: (surface_uuid, contour_idx, edge_start_idx,
    /// original_v0, original_v1, grab_point_on_edge)
    dragging_edge: Option<(String, usize, usize, [f32; 2], [f32; 2], [f32; 2])>,
}

/// Full-screen stage editor — replaces the deck view
pub(super) fn render_stage_editor(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let state_id = ui.id().with("stage_editor_state");
    let mut state = ui.memory(|mem| {
        mem.data.get_temp::<StageEditorState>(state_id).unwrap_or_default()
    });

    // Toolbar at top
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("🎨 Stage Editor").strong().size(16.0));
        ui.separator();

        // Tool buttons
        let tools = [
            (DrawingTool::Select, "⬚ Select", "Select and edit surfaces (S)"),
            (DrawingTool::Rectangle, "▭ Rectangle", "Draw rectangle surfaces (R)"),
            (DrawingTool::Polygon, "⬠ Polygon", "Draw polygon surfaces — click to add vertices, double-click to finish (P)"),
            (DrawingTool::Circle, "⬤ Circle", "Draw circle/N-gon surfaces (C)"),
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

        ui.separator();

        // Grid controls
        let snap_label = if data.stage_editor_snap { "🧲 Snap: ON" } else { "🧲 Snap: OFF" };
        if ui.button(snap_label).clicked() {
            actions.toggle_snap = true;
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
                    if ui.selectable_value(&mut actions.set_grid_size, Some(*size), *label).clicked() {
                        // handled by set_grid_size
                    }
                }
            });

        // Circle sides (only when circle tool selected)
        if state.tool == DrawingTool::Circle {
            ui.separator();
            ui.label("Sides:");
            if state.circle_sides == 0 { state.circle_sides = 32; }
            ui.add(egui::DragValue::new(&mut state.circle_sides).range(3..=128).speed(1));
        }

        // Circle-specific toolbar: when exactly one circle is selected, show radius/sides/convert
        let selected_circle = if state.selected_surfaces.len() == 1 {
            let sel_uuid = state.selected_surfaces.iter().next().unwrap().clone();
            data.surfaces.iter().find(|s| s.uuid == sel_uuid)
                .and_then(|s| s.circle_hint.map(|h| (sel_uuid, h)))
        } else {
            None
        };
        if let Some((sel_uuid, hint)) = selected_circle {
            ui.separator();
            ui.label("⬤ Circle:");
            let mut radius = hint.radius;
            if ui.add(egui::DragValue::new(&mut radius).prefix("R: ").range(0.01..=1.0).speed(0.005)).changed() {
                actions.surface_actions.push(SurfaceAction::SetCircleRadius { uuid: sel_uuid.clone(), radius });
            }
            let mut sides = hint.sides;
            if ui.add(egui::DragValue::new(&mut sides).prefix("Sides: ").range(3..=128).speed(1)).changed() {
                actions.surface_actions.push(SurfaceAction::SetCircleSides { uuid: sel_uuid.clone(), sides });
            }
            if ui.button("⬠ Convert to Polygon").on_hover_text("Drop circle identity, keep vertices as polygon").clicked() {
                actions.surface_actions.push(SurfaceAction::ConvertToPolygon { uuid: sel_uuid });
            }
        }

        // Duplicate & flip (enabled when any surfaces are selected)
        ui.separator();
        let has_sel = !state.selected_surfaces.is_empty();
        ui.add_enabled_ui(has_sel, |ui| {
            if ui.button("📋 Dup").on_hover_text("Duplicate selected (D)").clicked() {
                for uuid in &state.selected_surfaces {
                    actions.surface_actions.push(SurfaceAction::Duplicate { uuid: uuid.clone() });
                }
            }
            if ui.button("↔ Flip H").on_hover_text("Flip horizontal (H)").clicked() {
                for uuid in &state.selected_surfaces {
                    actions.surface_actions.push(SurfaceAction::FlipHorizontal { uuid: uuid.clone() });
                }
            }
            if ui.button("↕ Flip V").on_hover_text("Flip vertical (V)").clicked() {
                for uuid in &state.selected_surfaces {
                    actions.surface_actions.push(SurfaceAction::FlipVertical { uuid: uuid.clone() });
                }
            }
            if state.selected_surfaces.len() >= 2 {
                if ui.button("🔗 Combine").on_hover_text("Combine selected surfaces (G)").clicked() {
                    let uuids: Vec<String> = state.selected_surfaces.iter().cloned().collect();
                    actions.surface_actions.push(SurfaceAction::Combine { uuids });
                    state.selected_surfaces.clear();
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
                actions.surface_actions.push(SurfaceAction::ImportFromFile { path });
            }
        }

        // Camera detect button — 0 cameras: hidden; 1: direct click; N: dropdown
        let active_cameras = &data.cameras;
        if active_cameras.len() == 1 {
            if ui.button("📷 Detect").on_hover_text("Enter camera detection mode").clicked() {
                actions.camera_detect_actions.push(CameraDetectAction::Enter {
                    camera_id: active_cameras[0].1,
                });
            }
        } else if active_cameras.len() > 1 {
            let cam_btn = ui.button("📷 Detect ▼").on_hover_text("Enter camera detection mode");
            let cam_popup_id = cam_btn.id.with("cam_detect_popup");
            if cam_btn.clicked() {
                #[allow(deprecated)]
                ui.memory_mut(|mem| mem.toggle_popup(cam_popup_id));
            }
            #[allow(deprecated)]
            egui::popup_below_widget(ui, cam_popup_id, &cam_btn, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(150.0);
                for (name, cam_id) in active_cameras {
                    if ui.button(name).clicked() {
                        actions.camera_detect_actions.push(CameraDetectAction::Enter {
                            camera_id: *cam_id,
                        });
                    }
                }
            });
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("x Close Editor").clicked() {
                actions.toggle_stage_editor = true;
            }
            ui.separator();
            // Mode toggle: 2D Polygon / 3D Dome
            let mode = if data.dome_mode_active { StageEditorMode::Dome3D } else { StageEditorMode::Polygon2D };
            if ui.selectable_label(mode == StageEditorMode::Polygon2D, "⬡ 2D").on_hover_text("2D Polygon mode").clicked() {
                actions.dome_actions.push(DomeAction::SetMode(false));
            }
            if ui.selectable_label(mode == StageEditorMode::Dome3D, "🔮 3D Dome").on_hover_text("3D Dome mode").clicked() {
                actions.dome_actions.push(DomeAction::SetMode(true));
            }
        });
    });

    // ── Camera detection mode: takes over the entire canvas ──
    match &data.camera_detect_mode {
        CameraDetectMode::Live { .. } => {
            render_camera_detect_live(ui, data, actions);
            ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
            return;
        }
        CameraDetectMode::Preview { .. } => {
            render_camera_detect_preview(ui, data, actions);
            ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
            return;
        }
        CameraDetectMode::Off => {} // continue normal rendering
    }

    let mode = if data.dome_mode_active { StageEditorMode::Dome3D } else { StageEditorMode::Polygon2D };

    // Dome config toolbar (second row, only in Dome3D mode)
    if mode == StageEditorMode::Dome3D {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🔮 Dome:").strong());

            // Preset dropdown
            let presets = [
                DomePreset::Single, DomePreset::Dual, DomePreset::Triple,
                DomePreset::Quad, DomePreset::Penta, DomePreset::Hexa, DomePreset::Octa,
            ];
            let mut current_preset = data.dome_preset;
            egui::ComboBox::from_id_salt("dome_preset")
                .selected_text(format!("{}", current_preset))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for preset in &presets {
                        if ui.selectable_value(&mut current_preset, *preset, format!("{}", preset)).clicked() {
                            actions.dome_actions.push(DomeAction::SetPreset(*preset));
                        }
                    }
                });

            ui.separator();

            // Radius slider
            let mut radius = data.dome_geometry.radius;
            ui.label("R:");
            if ui.add(egui::DragValue::new(&mut radius).range(0.5..=5.0).speed(0.01)).changed() {
                actions.dome_actions.push(DomeAction::SetRadius(radius));
            }

            // Truncation angle slider
            let mut trunc = data.dome_geometry.truncation_degrees;
            ui.label("Trunc:");
            if ui.add(egui::DragValue::new(&mut trunc).range(30.0..=90.0).speed(0.5).suffix("°")).changed() {
                actions.dome_actions.push(DomeAction::SetTruncation(trunc));
            }

            // Tilt slider
            let mut tilt = data.dome_geometry.tilt_degrees;
            ui.label("Tilt:");
            if ui.add(egui::DragValue::new(&mut tilt).range(0.0..=45.0).speed(0.5).suffix("°")).changed() {
                actions.dome_actions.push(DomeAction::SetTilt(tilt));
            }

            ui.separator();

            // Content rotation controls
            let mut c_az = data.dome_geometry.content_azimuth_degrees;
            ui.label("Content Az:");
            if ui.add(egui::DragValue::new(&mut c_az).range(-180.0..=180.0).speed(1.0).suffix("°")).changed() {
                actions.dome_actions.push(DomeAction::SetContentAzimuth(c_az));
            }

            let mut c_el = data.dome_geometry.content_elevation_degrees;
            ui.label("Content El:");
            if ui.add(egui::DragValue::new(&mut c_el).range(-90.0..=90.0).speed(1.0).suffix("°")).changed() {
                actions.dome_actions.push(DomeAction::SetContentElevation(c_el));
            }

            let mut c_roll = data.dome_geometry.content_roll_degrees;
            ui.label("Content Roll:");
            if ui.add(egui::DragValue::new(&mut c_roll).range(-180.0..=180.0).speed(1.0).suffix("°")).changed() {
                actions.dome_actions.push(DomeAction::SetContentRoll(c_roll));
            }

            ui.separator();

            // Generate Slices button
            if ui.button("🎯 Generate Slices").on_hover_text("Create per-projector surfaces with warp meshes").clicked() {
                let setup = current_preset.to_setup_with_geometry(data.dome_geometry);
                actions.surface_actions.push(SurfaceAction::GenerateDomeSlices { setup });
            }
        });
    }

    ui.add_space(4.0);

    // ── Dome 3D mode: full-canvas interactive dome view ──
    if mode == StageEditorMode::Dome3D {
        render_dome_canvas(ui, data, actions);
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

    let painter = ui.painter_at(canvas_rect);

    // Canvas background
    painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_rgb(10, 10, 18));

    // Grid lines
    let grid_size = data.stage_editor_grid_size;
    if grid_size > 0.001 {
        let steps = (1.0 / grid_size).round() as usize;
        for i in 1..steps {
            let t = i as f32 * grid_size;
            let x = canvas_rect.left() + t * canvas_width;
            let y = canvas_rect.top() + t * canvas_height;
            if x < canvas_rect.right() {
                painter.line_segment(
                    [egui::pos2(x, canvas_rect.top()), egui::pos2(x, canvas_rect.bottom())],
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(25, 25, 38)),
                );
            }
            if y < canvas_rect.bottom() {
                painter.line_segment(
                    [egui::pos2(canvas_rect.left(), y), egui::pos2(canvas_rect.right(), y)],
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(25, 25, 38)),
                );
            }
        }
    }

    // Draw surfaces
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
        let is_selected = state.selected_surfaces.contains(&surface.uuid);
        let fill_alpha = if is_selected { 120 } else { 60 };
        let fill = egui::Color32::from_rgba_premultiplied(color.r() / 3, color.g() / 3, color.b() / 3, fill_alpha);
        let stroke_width = if is_selected { 2.5 } else { 1.5 };

        let pixel_verts: Vec<egui::Pos2> = surface.vertices.iter().map(|v| {
            egui::pos2(
                canvas_rect.left() + v[0] * canvas_width,
                canvas_rect.top() + v[1] * canvas_height,
            )
        }).collect();

        if pixel_verts.len() >= 3 {
            painter.add(polygon_shape(&pixel_verts, fill, egui::Stroke::new(stroke_width, color)));
        }
        // Draw extra contours (combined non-overlapping surfaces)
        for ec in &surface.extra_contours {
            let ec_verts: Vec<egui::Pos2> = ec.iter().map(|v| {
                egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
            }).collect();
            if ec_verts.len() >= 3 {
                painter.add(polygon_shape(&ec_verts, fill, egui::Stroke::new(stroke_width, color)));
            }
        }

        // Label
        let n = surface.vertices.len().max(1) as f32;
        let center = surface.vertices.iter().fold([0.0f32, 0.0], |acc, v| {
            [acc[0] + v[0] / n, acc[1] + v[1] / n]
        });
        let center_px = egui::pos2(
            canvas_rect.left() + center[0] * canvas_width,
            canvas_rect.top() + center[1] * canvas_height,
        );
        painter.text(center_px, egui::Align2::CENTER_CENTER, &surface.name, egui::FontId::proportional(13.0), egui::Color32::WHITE);

        // For circles: render radius handle instead of vertex handles
        if is_selected && surface.circle_hint.is_some() {
            let Some(hint) = surface.circle_hint else { continue; };
            let cx_px = canvas_rect.left() + hint.center[0] * canvas_width;
            let cy_px = canvas_rect.top() + hint.center[1] * canvas_height;
            let center_pos = egui::pos2(cx_px, cy_px);
            // Radius ring — compute the pixel radius at angle=0
            let radius_px_x = hint.radius * canvas_width;
            let radius_px_y = hint.radius * hint.aspect_ratio * canvas_height;
            let avg_radius_px = (radius_px_x + radius_px_y) / 2.0;
            // Center dot (white)
            painter.circle_filled(center_pos, 4.0, egui::Color32::WHITE);
            // Radius ring (yellow, dashed look via stroke)
            painter.circle_stroke(center_pos, avg_radius_px, egui::Stroke::new(1.0, egui::Color32::YELLOW));
            // Radius handle at angle=0 (yellow dot on the right)
            let handle_pos = egui::pos2(cx_px + radius_px_x, cy_px);
            painter.circle_filled(handle_pos, 6.0, egui::Color32::YELLOW);
            painter.circle_stroke(handle_pos, 6.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
        } else {
            // Regular vertex handles (primary + extra contours)
            let handle_size = if is_selected { 10.0 } else { 7.0 };
            let handle_color = if is_selected { egui::Color32::WHITE } else { color };
            let draw_handles = |verts: &[egui::Pos2]| {
                for v in verts {
                    let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(handle_size, handle_size));
                    painter.rect_filled(handle_rect, 2.0, handle_color);
                    painter.rect_stroke(handle_rect, 2.0, egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Outside);
                }
            };
            draw_handles(&pixel_verts);
            for ec in &surface.extra_contours {
                let ec_px: Vec<egui::Pos2> = ec.iter().map(|v| {
                    egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
                }).collect();
                draw_handles(&ec_px);
            }
        }
    }

    // Draw in-progress polygon
    if !state.polygon_verts.is_empty() && state.tool == DrawingTool::Polygon {
        let pixel_verts: Vec<egui::Pos2> = state.polygon_verts.iter().map(|v| {
            egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
        }).collect();
        for i in 0..pixel_verts.len() - 1 {
            painter.line_segment([pixel_verts[i], pixel_verts[i + 1]], egui::Stroke::new(2.0, egui::Color32::YELLOW));
        }
        // Draw line from last vertex to cursor
        if let Some(pos) = canvas_response.hover_pos() {
            if let Some(last) = pixel_verts.last() {
                painter.line_segment([*last, pos], egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 255, 0, 128)));
            }
        }
        for v in &pixel_verts {
            let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(8.0, 8.0));
            painter.rect_filled(handle_rect, 2.0, egui::Color32::YELLOW);
        }
    }

    // Draw in-progress rectangle preview
    if let Some(start) = state.rect_start {
        if state.tool == DrawingTool::Rectangle {
            if let Some(pos) = canvas_response.hover_pos() {
                let end_x = (pos.x - canvas_rect.left()) / canvas_width;
                let end_y = (pos.y - canvas_rect.top()) / canvas_height;
                let (sx, sy) = (start[0], start[1]);
                let preview_rect = egui::Rect::from_two_pos(
                    egui::pos2(canvas_rect.left() + sx * canvas_width, canvas_rect.top() + sy * canvas_height),
                    egui::pos2(canvas_rect.left() + end_x * canvas_width, canvas_rect.top() + end_y * canvas_height),
                );
                painter.rect_stroke(preview_rect, 0.0, egui::Stroke::new(2.0, egui::Color32::YELLOW), egui::StrokeKind::Outside);
            }
        }
    }

    // Draw in-progress circle preview
    if let Some(center) = state.circle_center {
        if state.tool == DrawingTool::Circle {
            if let Some(pos) = canvas_response.hover_pos() {
                let cx_px = canvas_rect.left() + center[0] * canvas_width;
                let cy_px = canvas_rect.top() + center[1] * canvas_height;
                let radius = ((pos.x - cx_px).powi(2) + (pos.y - cy_px).powi(2)).sqrt();
                painter.circle_stroke(egui::pos2(cx_px, cy_px), radius, egui::Stroke::new(2.0, egui::Color32::YELLOW));
            }
        }
    }

    // --- Interaction handling ---
    let snap = |v: f32| -> f32 {
        if data.stage_editor_snap && grid_size > 0.001 {
            (v / grid_size).round() * grid_size
        } else {
            v
        }
    };

    let to_norm = |pos: egui::Pos2| -> [f32; 2] {
        let nx = ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0);
        let ny = ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0);
        [snap(nx), snap(ny)]
    };

    // Helper: check if a point is inside any existing surface (returns surface UUID)
    let point_in_any_surface = |nx: f32, ny: f32| -> Option<String> {
        for surface in data.surfaces.iter().rev() {
            let verts = &surface.vertices;
            let n = verts.len();
            if n >= 3 {
                let mut inside = false;
                let mut j = n - 1;
                for k in 0..n {
                    let (xi, yi) = (verts[k][0], verts[k][1]);
                    let (xj, yj) = (verts[j][0], verts[j][1]);
                    if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
                        inside = !inside;
                    }
                    j = k;
                }
                if inside { return Some(surface.uuid.clone()); }
            }
        }
        None
    };

    match state.tool {
        DrawingTool::Select => {
            // Helper: pixel-space distance between a normalized point and a vertex
            let pixel_dist = |nx: f32, ny: f32, vx: f32, vy: f32| -> f32 {
                let dx_px = (nx - vx) * canvas_width;
                let dy_px = (ny - vy) * canvas_height;
                (dx_px * dx_px + dy_px * dy_px).sqrt()
            };

            // Helper: find the closest edge of a specific surface within a threshold.
            // Returns (contour_idx, edge_start_idx, projected_point, distance_px).
            let find_closest_edge = |nx: f32, ny: f32, surface: &super::super::SurfaceUI, threshold: f32|
                -> Option<(usize, usize, [f32; 2], f32)>
            {
                let contours: Vec<&Vec<[f32; 2]>> = std::iter::once(&surface.vertices)
                    .chain(surface.extra_contours.iter()).collect();
                let mut best: Option<(usize, usize, [f32; 2], f32)> = None;
                for (ci, verts) in contours.iter().enumerate() {
                    let n = verts.len();
                    for ei in 0..n {
                        let ej = (ei + 1) % n;
                        let (ax, ay) = (verts[ei][0], verts[ei][1]);
                        let (bx, by) = (verts[ej][0], verts[ej][1]);
                        let dx = (bx - ax) * canvas_width;
                        let dy = (by - ay) * canvas_height;
                        let len_sq = dx * dx + dy * dy;
                        if len_sq < 1e-6 { continue; }
                        let px_nx = (nx - ax) * canvas_width;
                        let px_ny = (ny - ay) * canvas_height;
                        let t = ((px_nx * dx + px_ny * dy) / len_sq).clamp(0.0, 1.0);
                        let proj_x = ax + t * (bx - ax);
                        let proj_y = ay + t * (by - ay);
                        let d = pixel_dist(nx, ny, proj_x, proj_y);
                        if d < threshold {
                            if best.as_ref().map_or(true, |b| d < b.3) {
                                best = Some((ci, ei, [proj_x, proj_y], d));
                            }
                        }
                    }
                }
                best
            };

            // Helper: find what's under the cursor
            // vertex: (surface_uuid, contour_idx, vertex_idx)
            // edge: (surface_uuid, contour_idx, edge_start_idx, projected_point)
            // surface: (surface_uuid, nx, ny)
            let hit_test = |nx: f32, ny: f32| -> (Option<(String, usize, usize)>, Option<(String, usize, usize, [f32; 2])>, Option<(String, f32, f32)>) {
                let vertex_threshold_px = 14.0;
                let edge_threshold_px = 10.0;
                // Wider threshold for edges when cursor is inside the surface.
                // This ensures top/right edges are grabbable from inside.
                let edge_inner_threshold_px = 24.0;
                let mut found_vertex = None;
                let mut found_edge = None;
                let mut found_surface = None;

                for surface in data.surfaces.iter().rev() {
                    let uid = &surface.uuid;
                    // Check all contours for vertex/edge hits
                    let contours: Vec<&Vec<[f32; 2]>> = std::iter::once(&surface.vertices)
                        .chain(surface.extra_contours.iter()).collect();
                    for (ci, verts) in contours.iter().enumerate() {
                        for (vi, v) in verts.iter().enumerate() {
                            if pixel_dist(nx, ny, v[0], v[1]) < vertex_threshold_px {
                                found_vertex = Some((uid.clone(), ci, vi));
                                return (found_vertex, None, None);
                            }
                        }
                    }

                    // Standard edge detection (narrow threshold, works from outside)
                    if found_edge.is_none() {
                        if let Some((ci, ei, proj, _d)) = find_closest_edge(nx, ny, surface, edge_threshold_px) {
                            found_edge = Some((uid.clone(), ci, ei, proj));
                        }
                    }

                    // Point-in-polygon (any contour)
                    if found_surface.is_none() {
                        let point_in = |verts: &[[f32; 2]]| -> bool {
                            let n = verts.len();
                            if n < 3 { return false; }
                            let mut inside = false;
                            let mut j = n - 1;
                            for k in 0..n {
                                let (xi, yi) = (verts[k][0], verts[k][1]);
                                let (xj, yj) = (verts[j][0], verts[j][1]);
                                if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
                                    inside = !inside;
                                }
                                j = k;
                            }
                            inside
                        };
                        if point_in(&surface.vertices) || surface.extra_contours.iter().any(|c| point_in(c)) {
                            found_surface = Some((uid.clone(), nx, ny));
                            // If cursor is inside the surface but no edge found yet,
                            // try again with a wider threshold to catch edges from inside.
                            if found_edge.is_none() {
                                if let Some((ci, ei, proj, _d)) = find_closest_edge(nx, ny, surface, edge_inner_threshold_px) {
                                    found_edge = Some((uid.clone(), ci, ei, proj));
                                }
                            }
                        }
                    }
                }
                (found_vertex, found_edge, found_surface)
            };

            // Hover feedback: change cursor when over interactive elements
            if let Some(pos) = canvas_response.hover_pos() {
                let [nx, ny] = to_norm(pos);
                let (found_vertex, found_edge, found_surface) = hit_test(nx, ny);
                if found_vertex.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if found_edge.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                } else if found_surface.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }

            let shift_held = ui.input(|i| i.modifiers.shift);

            // Click to select (without drag)
            if canvas_response.clicked() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    let (found_vertex, _found_edge, found_surface) = hit_test(nx, ny);
                    if let Some((si, _ci, _vi)) = found_vertex {
                        if shift_held {
                            // Toggle selection with shift
                            if !state.selected_surfaces.remove(&si) {
                                state.selected_surfaces.insert(si);
                            }
                        } else {
                            state.selected_surfaces.clear();
                            state.selected_surfaces.insert(si);
                        }
                    } else if let Some((si, _lx, _ly)) = found_surface {
                        if shift_held {
                            if !state.selected_surfaces.remove(&si) {
                                state.selected_surfaces.insert(si);
                            }
                        } else {
                            state.selected_surfaces.clear();
                            state.selected_surfaces.insert(si);
                        }
                    } else if !shift_held {
                        state.selected_surfaces.clear();
                    }
                }
            }

            // Double-click on edge to insert vertex
            if canvas_response.double_clicked() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    let (_found_vertex, found_edge, _found_surface) = hit_test(nx, ny);
                    if let Some((uuid, _ci, ei, snap_pos)) = found_edge {
                        let snapped = [snap(snap_pos[0]), snap(snap_pos[1])];
                        actions.surface_actions.push(SurfaceAction::InsertVertex {
                            uuid: uuid.clone(),
                            after_vert_idx: ei,
                            position: snapped,
                        });
                        state.selected_surfaces.clear();
                        state.selected_surfaces.insert(uuid);
                    }
                }
            }

            // Drag start: begin radius drag, vertex drag, surface move, or marquee selection
            if canvas_response.drag_started() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);

                    // Check for radius handle hit on selected circles first
                    let mut found_radius_handle = None;
                    for sel_uuid in &state.selected_surfaces {
                        if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *sel_uuid) {
                            if let Some(hint) = &surface.circle_hint {
                                let hx = hint.center[0] + hint.radius;
                                let hy = hint.center[1];
                                if pixel_dist(nx, ny, hx, hy) < 14.0 {
                                    found_radius_handle = Some(sel_uuid.clone());
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(uuid) = found_radius_handle {
                        state.dragging_radius = Some(uuid);
                        state.dragging_vertex = None;
                        state.moving_surface = None;
                        state.selection_rect_start = None;
                        state.dragging_edge = None;
                    } else {
                        let (found_vertex, found_edge, found_surface) = hit_test(nx, ny);

                        if let Some((uuid, ci, vi)) = found_vertex {
                            // If vertex drag on a circle, auto-convert to polygon first
                            if data.surfaces.iter().find(|s| s.uuid == uuid).map_or(false, |s| s.circle_hint.is_some()) {
                                actions.surface_actions.push(SurfaceAction::ConvertToPolygon { uuid: uuid.clone() });
                            }
                            if !shift_held {
                                state.selected_surfaces.clear();
                            }
                            state.selected_surfaces.insert(uuid.clone());
                            state.dragging_vertex = Some((uuid, ci, vi));
                            state.moving_surface = None;
                            state.selection_rect_start = None;
                            state.dragging_edge = None;
                        } else if let Some((uuid, ci, ei, grab_pt)) = found_edge {
                            // Edge drag: store original edge endpoints + grab point
                            if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == uuid) {
                                let verts = if ci == 0 { &surface.vertices } else { &surface.extra_contours[ci - 1] };
                                let ej = (ei + 1) % verts.len();
                                let v0 = verts[ei];
                                let v1 = verts[ej];
                                // Auto-convert circle to polygon before edge drag
                                if surface.circle_hint.is_some() {
                                    actions.surface_actions.push(SurfaceAction::ConvertToPolygon { uuid: uuid.clone() });
                                }
                                if !shift_held {
                                    state.selected_surfaces.clear();
                                }
                                state.selected_surfaces.insert(uuid.clone());
                                state.dragging_edge = Some((uuid, ci, ei, v0, v1, grab_pt));
                                state.dragging_vertex = None;
                                state.moving_surface = None;
                                state.selection_rect_start = None;
                            }
                        } else if let Some((uuid, lx, ly)) = found_surface {
                            if !shift_held && !state.selected_surfaces.contains(&uuid) {
                                state.selected_surfaces.clear();
                            }
                            state.selected_surfaces.insert(uuid.clone());
                            state.moving_surface = Some((uuid, lx, ly));
                            state.dragging_vertex = None;
                            state.selection_rect_start = None;
                            state.dragging_edge = None;
                        } else {
                            if !shift_held {
                                state.selected_surfaces.clear();
                            }
                            state.selection_rect_start = Some([nx, ny]);
                            state.dragging_vertex = None;
                            state.moving_surface = None;
                            state.dragging_edge = None;
                        }
                    }
                }
            }

            if canvas_response.dragged() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);

                    if let Some(ref uuid) = state.dragging_radius {
                        // Compute new radius from cursor distance to circle center
                        if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                            if let Some(hint) = &surface.circle_hint {
                                let dx = nx - hint.center[0];
                                let dy = ny - hint.center[1];
                                let new_radius = (dx * dx + dy * dy).sqrt().max(0.01);
                                actions.surface_actions.push(SurfaceAction::SetCircleRadius {
                                    uuid: uuid.clone(), radius: new_radius,
                                });
                            }
                        }
                    } else if let Some((ref uuid, ci, vi)) = state.dragging_vertex {
                        if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                            let contour_verts = if ci == 0 { Some(&surface.vertices) } else { surface.extra_contours.get(ci - 1) };
                            if let Some(verts) = contour_verts {
                                let mut new_verts = verts.clone();
                                if vi < new_verts.len() {
                                    new_verts[vi] = [nx, ny];
                                    actions.surface_actions.push(SurfaceAction::UpdateVertices {
                                        uuid: uuid.clone(), contour: ci, vertices: new_verts,
                                    });
                                }
                            }
                        }
                    } else if let Some((ref uuid, ci, ei, orig_v0, orig_v1, grab_pt)) = state.dragging_edge {
                        // Edge drag: move both edge endpoints by the cursor displacement
                        // relative to where the user first grabbed the edge.
                        let dx = nx - grab_pt[0];
                        let dy = ny - grab_pt[1];
                        if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                            let contour_verts = if ci == 0 { Some(&surface.vertices) } else { surface.extra_contours.get(ci - 1) };
                            if let Some(verts) = contour_verts {
                                let mut new_verts = verts.clone();
                                let ej = (ei + 1) % new_verts.len();
                                new_verts[ei] = [
                                    (orig_v0[0] + dx).clamp(0.0, 1.0),
                                    (orig_v0[1] + dy).clamp(0.0, 1.0),
                                ];
                                new_verts[ej] = [
                                    (orig_v1[0] + dx).clamp(0.0, 1.0),
                                    (orig_v1[1] + dy).clamp(0.0, 1.0),
                                ];
                                actions.surface_actions.push(SurfaceAction::UpdateVertices {
                                    uuid: uuid.clone(), contour: ci, vertices: new_verts,
                                });
                            }
                        }
                    } else if let Some((ref _uuid, lx, ly)) = state.moving_surface {
                        let dx = nx - lx;
                        let dy = ny - ly;
                        // Move ALL selected surfaces by the same delta
                        for surf_uuid in &state.selected_surfaces {
                            if data.surfaces.iter().any(|s| s.uuid == *surf_uuid) {
                                actions.surface_actions.push(SurfaceAction::MoveDelta {
                                    uuid: surf_uuid.clone(), dx, dy,
                                });
                            }
                        }
                        state.moving_surface = Some((_uuid.clone(), nx, ny));
                    } else if let Some(start) = state.selection_rect_start {
                        // Draw marquee selection rectangle
                        let x0 = canvas_rect.left() + start[0] * canvas_width;
                        let y0 = canvas_rect.top() + start[1] * canvas_height;
                        let x1 = canvas_rect.left() + nx * canvas_width;
                        let y1 = canvas_rect.top() + ny * canvas_height;
                        let sel_rect = egui::Rect::from_two_pos(
                            egui::pos2(x0, y0),
                            egui::pos2(x1, y1),
                        );
                        painter.rect_filled(sel_rect, 0.0, egui::Color32::from_rgba_premultiplied(80, 130, 255, 40));
                        painter.rect_stroke(sel_rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 130, 255)), egui::StrokeKind::Outside);
                    }
                }
            }

            if canvas_response.drag_stopped() {
                // Finish marquee selection: select all surfaces that intersect the rect
                if let Some(start) = state.selection_rect_start {
                    if let Some(pos) = canvas_response.interact_pointer_pos() {
                        let [nx, ny] = to_norm(pos);
                        let sel_min_x = start[0].min(nx);
                        let sel_max_x = start[0].max(nx);
                        let sel_min_y = start[1].min(ny);
                        let sel_max_y = start[1].max(ny);

                        for surface in data.surfaces.iter() {
                            // Compute bounding box of surface vertices
                            let (mut bb_min_x, mut bb_min_y) = (f32::MAX, f32::MAX);
                            let (mut bb_max_x, mut bb_max_y) = (f32::MIN, f32::MIN);
                            for v in &surface.vertices {
                                bb_min_x = bb_min_x.min(v[0]);
                                bb_min_y = bb_min_y.min(v[1]);
                                bb_max_x = bb_max_x.max(v[0]);
                                bb_max_y = bb_max_y.max(v[1]);
                            }
                            // Check if surface bounding box overlaps the selection rect
                            let intersects = bb_min_x < sel_max_x && bb_max_x > sel_min_x &&
                                bb_min_y < sel_max_y && bb_max_y > sel_min_y;
                            if intersects {
                                state.selected_surfaces.insert(surface.uuid.clone());
                            }
                        }
                    }
                }
                state.selection_rect_start = None;
                state.dragging_vertex = None;
                state.moving_surface = None;
                state.dragging_radius = None;
                state.dragging_edge = None;
            }

            // Delete selected surfaces (handled below via keymap)
        }

        DrawingTool::Rectangle => {
            if canvas_response.drag_started() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    if let Some(uuid) = point_in_any_surface(nx, ny) {
                        state.selected_surfaces.clear();
                        state.selected_surfaces.insert(uuid.clone());
                        state.moving_surface = Some((uuid, nx, ny));
                        state.tool = DrawingTool::Select;
                    } else {
                        state.rect_start = Some([nx, ny]);
                    }
                }
            }
            if canvas_response.drag_stopped() {
                if let Some(start) = state.rect_start.take() {
                    if let Some(pos) = canvas_response.interact_pointer_pos() {
                        let end = to_norm(pos);
                        let x0 = start[0].min(end[0]);
                        let y0 = start[1].min(end[1]);
                        let x1 = start[0].max(end[0]);
                        let y1 = start[1].max(end[1]);
                        if (x1 - x0) > 0.01 && (y1 - y0) > 0.01 {
                            let idx = data.surfaces.len() + 1;
                            actions.surface_actions.push(SurfaceAction::AddPolygon {
                                name: format!("Surface {}", idx),
                                vertices: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                                source: OutputSource::Master,
                            });
                        }
                    }
                }
            }
        }

        DrawingTool::Polygon => {
            if canvas_response.clicked() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let pt = to_norm(pos);

                    // If no polygon in progress and clicking inside existing surface, select it
                    let mut handled = false;
                    if state.polygon_verts.is_empty() {
                        if let Some(uuid) = point_in_any_surface(pt[0], pt[1]) {
                            state.selected_surfaces.clear();
                            state.selected_surfaces.insert(uuid);
                            state.tool = DrawingTool::Select;
                            handled = true;
                        }
                    }

                    // Check if clicking near first vertex to close
                    if !handled && state.polygon_verts.len() >= 3 {
                        let first = state.polygon_verts[0];
                        let dx = pt[0] - first[0];
                        let dy = pt[1] - first[1];
                        let close_threshold = 15.0 / canvas_width;
                        if (dx * dx + dy * dy).sqrt() < close_threshold {
                            // Close polygon
                            let idx = data.surfaces.len() + 1;
                            actions.surface_actions.push(SurfaceAction::AddPolygon {
                                name: format!("Surface {}", idx),
                                vertices: state.polygon_verts.clone(),
                                source: OutputSource::Master,
                            });
                            state.polygon_verts.clear();
                        } else {
                            state.polygon_verts.push(pt);
                        }
                    } else if !handled {
                        state.polygon_verts.push(pt);
                    }
                }
            }
            if canvas_response.double_clicked() {
                // Finish polygon on double-click
                if state.polygon_verts.len() >= 3 {
                    let idx = data.surfaces.len() + 1;
                    actions.surface_actions.push(SurfaceAction::AddPolygon {
                        name: format!("Surface {}", idx),
                        vertices: state.polygon_verts.clone(),
                        source: OutputSource::Master,
                    });
                }
                state.polygon_verts.clear();
            }
        }

        DrawingTool::Circle => {
            if canvas_response.drag_started() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    if let Some(uuid) = point_in_any_surface(nx, ny) {
                        state.selected_surfaces.clear();
                        state.selected_surfaces.insert(uuid.clone());
                        state.moving_surface = Some((uuid, nx, ny));
                        state.tool = DrawingTool::Select;
                    } else {
                        state.circle_center = Some([nx, ny]);
                    }
                }
            }
            if canvas_response.drag_stopped() {
                if let Some(center) = state.circle_center.take() {
                    if let Some(pos) = canvas_response.interact_pointer_pos() {
                        let end = to_norm(pos);
                        let rx = (end[0] - center[0]).abs();
                        let ry = (end[1] - center[1]).abs();
                        let radius = (rx.max(ry)).max(0.02);
                        let sides = state.circle_sides.max(3);
                        let aspect_ratio = canvas_width / canvas_height;
                        let idx = data.surfaces.len() + 1;
                        actions.surface_actions.push(SurfaceAction::AddCircle {
                            name: format!("Surface {}", idx),
                            hint: CircleHint {
                                center,
                                radius,
                                sides,
                                aspect_ratio,
                            },
                            source: OutputSource::Master,
                        });
                    }
                }
            }
        }
    }

    // Keyboard shortcuts (data-driven via keymap)
    if !data.keyboard_learn_active {
        use crate::keymap::{KeyCombo, KeyTarget, ActionId, collect_pressed_keys};
        let pressed = collect_pressed_keys(ui.ctx());
        for (key, mods) in &pressed {
            let combo = KeyCombo::from_egui(*key, mods);
            if let Some(target) = data.keymap_bindings.get(&combo) {
                match target {
                    KeyTarget::Action(ActionId::ToolSelect) => state.tool = DrawingTool::Select,
                    KeyTarget::Action(ActionId::ToolRectangle) => state.tool = DrawingTool::Rectangle,
                    KeyTarget::Action(ActionId::ToolPolygon) => state.tool = DrawingTool::Polygon,
                    KeyTarget::Action(ActionId::ToolCircle) => state.tool = DrawingTool::Circle,
                    KeyTarget::Action(ActionId::ClearDrawing) => {
                        state.polygon_verts.clear();
                        state.rect_start = None;
                        state.circle_center = None;
                    }
                    KeyTarget::Action(ActionId::DeleteSurface) => {
                        if !state.selected_surfaces.is_empty() {
                            let uuids: Vec<String> = state.selected_surfaces.iter().cloned().collect();
                            for uuid in uuids {
                                actions.surface_actions.push(SurfaceAction::Remove { uuid });
                            }
                            state.selected_surfaces.clear();
                        }
                    }
                    KeyTarget::Action(ActionId::DuplicateSurface) => {
                        for uuid in &state.selected_surfaces {
                            actions.surface_actions.push(SurfaceAction::Duplicate { uuid: uuid.clone() });
                        }
                    }
                    KeyTarget::Action(ActionId::FlipHorizontal) => {
                        for uuid in &state.selected_surfaces {
                            actions.surface_actions.push(SurfaceAction::FlipHorizontal { uuid: uuid.clone() });
                        }
                    }
                    KeyTarget::Action(ActionId::FlipVertical) => {
                        for uuid in &state.selected_surfaces {
                            actions.surface_actions.push(SurfaceAction::FlipVertical { uuid: uuid.clone() });
                        }
                    }
                    KeyTarget::Action(ActionId::CombineSurfaces) => {
                        if state.selected_surfaces.len() >= 2 {
                            let uuids: Vec<String> = state.selected_surfaces.iter().cloned().collect();
                            actions.surface_actions.push(SurfaceAction::Combine { uuids });
                            state.selected_surfaces.clear();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Persist state
    ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
}


/// Fixed 8-color palette for dome projector slices.
const SLICE_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(230, 57, 70),    // Red
    egui::Color32::from_rgb(42, 157, 143),    // Green/Teal
    egui::Color32::from_rgb(69, 123, 157),    // Blue
    egui::Color32::from_rgb(241, 196, 15),    // Yellow
    egui::Color32::from_rgb(230, 126, 34),    // Orange
    egui::Color32::from_rgb(155, 89, 182),    // Purple
    egui::Color32::from_rgb(26, 188, 156),    // Cyan
    egui::Color32::from_rgb(232, 67, 147),    // Pink
];

/// Render the 3D dome canvas (Dome3D mode).
fn render_dome_canvas(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let available_width = ui.available_width();
    let available_height = ui.available_height().max(200.0);
    // Square, centered in available space
    let dome_size = available_width.min(available_height);
    let padding_x = (available_width - dome_size) * 0.5;

    if padding_x > 0.0 {
        ui.add_space(0.0); // ensure horizontal layout
    }

    ui.horizontal(|ui| {
        if padding_x > 1.0 {
            ui.add_space(padding_x);
        }
        if let Some(tex_id) = data.dome_preview_texture {
            let img = egui::Image::new(egui::load::SizedTexture::new(
                tex_id, egui::vec2(dome_size, dome_size),
            ));
            let response = ui.add(img.sense(egui::Sense::click_and_drag()));

            // Mouse interaction: orbit camera
            if response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                actions.dome_actions.push(DomeAction::RotateCamera {
                    delta_x: delta.x,
                    delta_y: delta.y,
                });
            }

            // Scroll to zoom
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.1 {
                    actions.dome_actions.push(DomeAction::ZoomCamera { delta: scroll });
                }
            }

            // Right-click to reset camera
            if response.clicked_by(egui::PointerButton::Secondary) {
                actions.dome_actions.push(DomeAction::ResetCamera);
            }

            // Projector labels overlay
            let rect = response.rect;
            let painter = ui.painter_at(rect);
            let setup = data.dome_preset.to_setup_with_geometry(data.dome_geometry);
            for (i, proj) in setup.projectors.iter().enumerate() {
                let color = SLICE_COLORS[i % SLICE_COLORS.len()];
                let label = format!("P{}", i + 1);
                // Position label at projector azimuth around the dome edge
                let az = proj.azimuth_degrees.to_radians();
                let label_r = dome_size * 0.42;
                let cx = rect.center().x + label_r * az.sin();
                let cy = rect.center().y - label_r * az.cos();
                painter.text(
                    egui::pos2(cx, cy),
                    egui::Align2::CENTER_CENTER,
                    &label,
                    egui::FontId::proportional(12.0),
                    color,
                );
            }
        } else {
            ui.label(egui::RichText::new("3D dome: waiting for renderer…").weak().italics());
        }
    });
}

/// Live camera detection mode: camera feed with contour overlay and parameter sliders.
fn render_camera_detect_live(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let CameraDetectMode::Live { params, camera_id } = &data.camera_detect_mode else {
        return;
    };

    // Detection param toolbar
    ui.add_space(2.0);
    let mut new_params = params.clone();
    let mut params_changed = false;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("📷 Camera Detection").strong());
        ui.separator();

        // Detection method toggle
        ui.label("Method:");
        if ui.selectable_label(matches!(new_params.detection_method, DetectionMethod::Canny), "Canny").clicked() {
            new_params.detection_method = DetectionMethod::Canny;
            params_changed = true;
        }
        if ui.selectable_label(matches!(new_params.detection_method, DetectionMethod::Threshold), "Threshold").clicked() {
            new_params.detection_method = DetectionMethod::Threshold;
            params_changed = true;
        }

        ui.separator();

        // Conditional controls based on detection method
        match new_params.detection_method {
            DetectionMethod::Canny => {
                ui.label("Canny Lo:");
                let mut canny_low = new_params.canny_low as f32;
                if ui.add(egui::DragValue::new(&mut canny_low).range(0.0..=255.0).speed(1.0)).changed() {
                    new_params.canny_low = canny_low as u8;
                    params_changed = true;
                }

                ui.label("Hi:");
                let mut canny_high = new_params.canny_high as f32;
                if ui.add(egui::DragValue::new(&mut canny_high).range(0.0..=255.0).speed(1.0)).changed() {
                    new_params.canny_high = canny_high as u8;
                    params_changed = true;
                }
            }
            DetectionMethod::Threshold => {
                ui.label("Thresh:");
                let mut thresh = new_params.threshold as f32;
                if ui.add(egui::DragValue::new(&mut thresh).range(0.0..=255.0).speed(1.0)).changed() {
                    new_params.threshold = thresh as u8;
                    params_changed = true;
                }

                if ui.checkbox(&mut new_params.invert, "Invert").changed() {
                    params_changed = true;
                }
            }
        }

        ui.label("Blur:");
        let mut blur = new_params.blur_radius as f32;
        if ui.add(egui::DragValue::new(&mut blur).range(0.0..=10.0).speed(0.1)).changed() {
            new_params.blur_radius = blur as u32;
            params_changed = true;
        }

        ui.label("Morph:");
        let mut morph = new_params.morph_size as f32;
        if ui.add(egui::DragValue::new(&mut morph).range(0.0..=10.0).speed(0.5)).changed() {
            new_params.morph_size = morph as u32;
            params_changed = true;
        }

        ui.label("Min Area:");
        if ui.add(egui::DragValue::new(&mut new_params.min_area).range(0.0001..=0.1).speed(0.001)).changed() {
            params_changed = true;
        }

        ui.label("Simplify:");
        if ui.add(egui::DragValue::new(&mut new_params.simplify_tolerance).range(0.001..=0.05).speed(0.001)).changed() {
            params_changed = true;
        }

        ui.label("Min Verts:");
        let mut min_verts = new_params.min_vertices as f32;
        if ui.add(egui::DragValue::new(&mut min_verts).range(3.0..=20.0).speed(0.5)).changed() {
            new_params.min_vertices = min_verts as usize;
            params_changed = true;
        }

        ui.separator();

        // Hull mode toggle
        ui.label("Hull:");
        if ui.selectable_label(matches!(new_params.hull_mode, HullMode::None), "None").clicked() {
            new_params.hull_mode = HullMode::None;
            params_changed = true;
        }
        if ui.selectable_label(matches!(new_params.hull_mode, HullMode::ConvexHull), "Convex").clicked() {
            new_params.hull_mode = HullMode::ConvexHull;
            params_changed = true;
        }

        ui.separator();

        // Camera picker (if multiple cameras)
        if data.cameras.len() > 1 {
            let current_name = data.cameras.iter()
                .find(|(_, id)| *id == *camera_id)
                .map(|(n, _)| n.as_str())
                .unwrap_or("Unknown");
            egui::ComboBox::from_id_salt("cam_detect_picker")
                .selected_text(current_name)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (name, cam_id) in &data.cameras {
                        if ui.selectable_label(*cam_id == *camera_id, name).clicked() {
                            actions.camera_detect_actions.push(CameraDetectAction::Exit);
                            actions.camera_detect_actions.push(CameraDetectAction::Enter { camera_id: *cam_id });
                        }
                    }
                });
            ui.separator();
        }

        if ui.button("📸 Capture").on_hover_text("Freeze frame and select contours").clicked() {
            actions.camera_detect_actions.push(CameraDetectAction::Capture);
        }
        if ui.button("✕ Cancel").on_hover_text("Exit detection mode").clicked() {
            actions.camera_detect_actions.push(CameraDetectAction::Exit);
        }
    });

    if params_changed {
        actions.camera_detect_actions.push(CameraDetectAction::UpdateParams(new_params));
    }

    ui.add_space(4.0);

    // Canvas: camera feed + contour overlay
    let canvas_width = ui.available_width();
    let canvas_height = (canvas_width * 9.0 / 16.0).min(ui.available_height().max(200.0));
    let (canvas_rect, _canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::hover(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Draw camera feed texture
    if let Some(tex_id) = data.camera_detect_texture {
        painter.image(
            tex_id,
            canvas_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_gray(30));
        painter.text(
            canvas_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Waiting for camera…",
            egui::FontId::proportional(16.0),
            egui::Color32::GRAY,
        );
    }

    // Overlay contours
    let contour_colors = [
        egui::Color32::from_rgb(0, 255, 128),
        egui::Color32::from_rgb(255, 200, 0),
        egui::Color32::from_rgb(0, 200, 255),
        egui::Color32::from_rgb(255, 100, 200),
        egui::Color32::from_rgb(200, 100, 255),
    ];
    for (i, contour) in data.camera_detect_contours.iter().enumerate() {
        let color = contour_colors[i % contour_colors.len()];
        let points: Vec<egui::Pos2> = contour.vertices.iter().map(|v| {
            egui::pos2(
                canvas_rect.min.x + v[0] * canvas_rect.width(),
                canvas_rect.min.y + v[1] * canvas_rect.height(),
            )
        }).collect();
        if points.len() >= 2 {
            let mut closed_points = points.clone();
            closed_points.push(points[0]);
            painter.add(egui::Shape::line(closed_points, egui::Stroke::new(2.0, color)));
        }
    }

    // Info bar
    ui.horizontal(|ui| {
        ui.label(format!("{} contours detected", data.camera_detect_contours.len()));
    });
}

/// Preview mode: frozen frame with selectable contours.
fn render_camera_detect_preview(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let CameraDetectMode::Preview { contours, selected, .. } = &data.camera_detect_mode else {
        return;
    };

    let selected_count = selected.iter().filter(|&&s| s).count();
    let total = contours.len();

    // Toolbar
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("📷 Preview — Select Contours").strong());
        ui.separator();

        if ui.button(format!("✓ Accept ({}/{})", selected_count, total))
            .on_hover_text("Create surfaces from selected contours")
            .clicked()
        {
            actions.camera_detect_actions.push(CameraDetectAction::Accept);
        }
        if ui.button("✕ Cancel").on_hover_text("Return to live view").clicked() {
            actions.camera_detect_actions.push(CameraDetectAction::Exit);
        }
        ui.separator();

        let all_selected = selected.iter().all(|&s| s);
        if all_selected {
            if ui.button("Deselect All").clicked() {
                actions.camera_detect_actions.push(CameraDetectAction::SelectAll(false));
            }
        } else {
            if ui.button("Select All").clicked() {
                actions.camera_detect_actions.push(CameraDetectAction::SelectAll(true));
            }
        }
    });

    ui.add_space(4.0);

    // Canvas: frozen frame + contour overlay
    let canvas_width = ui.available_width();
    let list_height = 120.0_f32; // reserve for contour list
    let canvas_height = (canvas_width * 9.0 / 16.0).min((ui.available_height() - list_height).max(200.0));
    let (canvas_rect, _canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::hover(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Draw frozen frame texture
    if let Some(tex_id) = data.camera_detect_texture {
        painter.image(
            tex_id,
            canvas_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_gray(30));
    }

    // Overlay contours: green=selected, gray=deselected
    for (i, contour) in contours.iter().enumerate() {
        let is_selected = selected.get(i).copied().unwrap_or(false);
        let color = if is_selected {
            egui::Color32::from_rgb(0, 255, 128)
        } else {
            egui::Color32::from_gray(100)
        };
        let stroke_width = if is_selected { 2.5 } else { 1.5 };
        let points: Vec<egui::Pos2> = contour.vertices.iter().map(|v| {
            egui::pos2(
                canvas_rect.min.x + v[0] * canvas_rect.width(),
                canvas_rect.min.y + v[1] * canvas_rect.height(),
            )
        }).collect();
        if points.len() >= 2 {
            let mut closed_points = points.clone();
            closed_points.push(points[0]);
            painter.add(egui::Shape::line(closed_points, egui::Stroke::new(stroke_width, color)));
        }
    }

    // Contour list with checkboxes
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(list_height).show(ui, |ui| {
        for (i, contour) in contours.iter().enumerate() {
            let mut is_sel = selected.get(i).copied().unwrap_or(false);
            let label = format!(
                "{} — area: {:.4} ({} verts)",
                contour.suggested_name,
                contour.area,
                contour.vertices.len(),
            );
            if ui.checkbox(&mut is_sel, label).changed() {
                actions.camera_detect_actions.push(CameraDetectAction::ToggleContour(i));
            }
        }
    });
}