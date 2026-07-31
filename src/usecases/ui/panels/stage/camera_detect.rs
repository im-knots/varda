//! Camera-based surface auto-detection: live capture and frozen-frame preview.
//!
//! Two full-canvas modes that take over the stage editor while detection is
//! running. See spec/2d-projection-mapping.md.

use super::super::super::{CameraDetectAction, CameraDetectMode, UIActions, UIData};
use crate::surface::detect::{DetectionMethod, HullMode};

/// Live camera detection mode: camera feed with contour overlay and parameter sliders.
pub(super) fn render_camera_detect_live(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
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
        if ui
            .selectable_label(
                matches!(new_params.detection_method, DetectionMethod::Canny),
                "Canny",
            )
            .clicked()
        {
            new_params.detection_method = DetectionMethod::Canny;
            params_changed = true;
        }
        if ui
            .selectable_label(
                matches!(new_params.detection_method, DetectionMethod::Threshold),
                "Threshold",
            )
            .clicked()
        {
            new_params.detection_method = DetectionMethod::Threshold;
            params_changed = true;
        }

        ui.separator();

        // Conditional controls based on detection method
        match new_params.detection_method {
            DetectionMethod::Canny => {
                ui.label("Canny Lo:");
                let mut canny_low = f32::from(new_params.canny_low);
                if ui
                    .add(
                        egui::DragValue::new(&mut canny_low)
                            .range(0.0..=255.0)
                            .speed(1.0),
                    )
                    .changed()
                {
                    new_params.canny_low = canny_low as u8;
                    params_changed = true;
                }

                ui.label("Hi:");
                let mut canny_high = f32::from(new_params.canny_high);
                if ui
                    .add(
                        egui::DragValue::new(&mut canny_high)
                            .range(0.0..=255.0)
                            .speed(1.0),
                    )
                    .changed()
                {
                    new_params.canny_high = canny_high as u8;
                    params_changed = true;
                }
            }
            DetectionMethod::Threshold => {
                ui.label("Thresh:");
                let mut thresh = f32::from(new_params.threshold);
                if ui
                    .add(
                        egui::DragValue::new(&mut thresh)
                            .range(0.0..=255.0)
                            .speed(1.0),
                    )
                    .changed()
                {
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
        if ui
            .add(egui::DragValue::new(&mut blur).range(0.0..=10.0).speed(0.1))
            .changed()
        {
            new_params.blur_radius = blur as u32;
            params_changed = true;
        }

        ui.label("Morph:");
        let mut morph = new_params.morph_size as f32;
        if ui
            .add(
                egui::DragValue::new(&mut morph)
                    .range(0.0..=10.0)
                    .speed(0.5),
            )
            .changed()
        {
            new_params.morph_size = morph as u32;
            params_changed = true;
        }

        ui.label("Min Area:");
        if ui
            .add(
                egui::DragValue::new(&mut new_params.min_area)
                    .range(0.0001..=0.1)
                    .speed(0.001),
            )
            .changed()
        {
            params_changed = true;
        }

        ui.label("Simplify:");
        if ui
            .add(
                egui::DragValue::new(&mut new_params.simplify_tolerance)
                    .range(0.001..=0.05)
                    .speed(0.001),
            )
            .changed()
        {
            params_changed = true;
        }

        ui.label("Min Verts:");
        let mut min_verts = new_params.min_vertices as f32;
        if ui
            .add(
                egui::DragValue::new(&mut min_verts)
                    .range(3.0..=20.0)
                    .speed(0.5),
            )
            .changed()
        {
            new_params.min_vertices = min_verts as usize;
            params_changed = true;
        }

        ui.separator();

        // Hull mode toggle
        ui.label("Hull:");
        if ui
            .selectable_label(matches!(new_params.hull_mode, HullMode::None), "None")
            .clicked()
        {
            new_params.hull_mode = HullMode::None;
            params_changed = true;
        }
        if ui
            .selectable_label(
                matches!(new_params.hull_mode, HullMode::ConvexHull),
                "Convex",
            )
            .clicked()
        {
            new_params.hull_mode = HullMode::ConvexHull;
            params_changed = true;
        }

        ui.separator();

        // Camera picker (if multiple cameras)
        if data.cameras.len() > 1 {
            let current_name = data
                .cameras
                .iter()
                .find(|(_, id)| *id == *camera_id)
                .map_or("Unknown", |(n, _)| n.as_str());
            egui::ComboBox::from_id_salt("cam_detect_picker")
                .selected_text(current_name)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (name, cam_id) in &data.cameras {
                        if ui.selectable_label(*cam_id == *camera_id, name).clicked() {
                            actions
                                .session
                                .camera_detect_actions
                                .push(CameraDetectAction::Exit);
                            actions
                                .session
                                .camera_detect_actions
                                .push(CameraDetectAction::Enter { camera_id: *cam_id });
                        }
                    }
                });
            ui.separator();
        }

        if ui
            .button("📸 Capture")
            .on_hover_text("Freeze frame and select contours")
            .clicked()
        {
            actions
                .session
                .camera_detect_actions
                .push(CameraDetectAction::Capture);
        }
        if ui
            .button("✕ Cancel")
            .on_hover_text("Exit detection mode")
            .clicked()
        {
            actions
                .session
                .camera_detect_actions
                .push(CameraDetectAction::Exit);
        }
    });

    if params_changed {
        actions
            .session
            .camera_detect_actions
            .push(CameraDetectAction::UpdateParams(new_params));
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
        let points: Vec<egui::Pos2> = contour
            .vertices
            .iter()
            .map(|v| {
                egui::pos2(
                    canvas_rect.min.x + v[0] * canvas_rect.width(),
                    canvas_rect.min.y + v[1] * canvas_rect.height(),
                )
            })
            .collect();
        if points.len() >= 2 {
            let mut closed_points = points.clone();
            closed_points.push(points[0]);
            painter.add(egui::Shape::line(
                closed_points,
                egui::Stroke::new(2.0_f32, color),
            ));
        }
    }

    // Info bar
    ui.horizontal(|ui| {
        ui.label(format!(
            "{} contours detected",
            data.camera_detect_contours.len()
        ));
    });
}

/// Preview mode: frozen frame with selectable contours.
pub(super) fn render_camera_detect_preview(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
) {
    let CameraDetectMode::Preview {
        contours, selected, ..
    } = &data.camera_detect_mode
    else {
        return;
    };

    let selected_count = selected.iter().filter(|&&s| s).count();
    let total = contours.len();

    // Toolbar
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("📷 Preview — Select Contours").strong());
        ui.separator();

        if ui
            .button(format!("✓ Accept ({selected_count}/{total})"))
            .on_hover_text("Create surfaces from selected contours")
            .clicked()
        {
            actions
                .session
                .camera_detect_actions
                .push(CameraDetectAction::Accept);
        }
        if ui
            .button("✕ Cancel")
            .on_hover_text("Return to live view")
            .clicked()
        {
            actions
                .session
                .camera_detect_actions
                .push(CameraDetectAction::Exit);
        }
        ui.separator();

        let all_selected = selected.iter().all(|&s| s);
        if all_selected {
            if ui.button("Deselect All").clicked() {
                actions
                    .session
                    .camera_detect_actions
                    .push(CameraDetectAction::SelectAll(false));
            }
        } else if ui.button("Select All").clicked() {
            actions
                .session
                .camera_detect_actions
                .push(CameraDetectAction::SelectAll(true));
        }
    });

    ui.add_space(4.0);

    // Canvas: frozen frame + contour overlay
    let canvas_width = ui.available_width();
    let list_height = 120.0_f32; // reserve for contour list
    let canvas_height =
        (canvas_width * 9.0 / 16.0).min((ui.available_height() - list_height).max(200.0));
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
        let stroke_width = if is_selected { 2.5_f32 } else { 1.5_f32 };
        let points: Vec<egui::Pos2> = contour
            .vertices
            .iter()
            .map(|v| {
                egui::pos2(
                    canvas_rect.min.x + v[0] * canvas_rect.width(),
                    canvas_rect.min.y + v[1] * canvas_rect.height(),
                )
            })
            .collect();
        if points.len() >= 2 {
            let mut closed_points = points.clone();
            closed_points.push(points[0]);
            painter.add(egui::Shape::line(
                closed_points,
                egui::Stroke::new(stroke_width, color),
            ));
        }
    }

    // Contour list with checkboxes
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .max_height(list_height)
        .show(ui, |ui| {
            for (i, contour) in contours.iter().enumerate() {
                let mut is_sel = selected.get(i).copied().unwrap_or(false);
                let label = format!(
                    "{} — area: {:.4} ({} verts)",
                    contour.suggested_name,
                    contour.area,
                    contour.vertices.len(),
                );
                if ui.checkbox(&mut is_sel, label).changed() {
                    actions
                        .session
                        .camera_detect_actions
                        .push(CameraDetectAction::ToggleContour(i));
                }
            }
        });
}
