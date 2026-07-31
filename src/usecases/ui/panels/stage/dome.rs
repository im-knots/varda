//! Dome 3D mode: full-canvas interactive dome preview.
//!
//! See spec/dome-projection.md.

use super::super::super::{DomeAction, UIActions, UIData};

/// Fixed 8-color palette for dome projector slices.
const SLICE_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(230, 57, 70),  // Red
    egui::Color32::from_rgb(42, 157, 143), // Green/Teal
    egui::Color32::from_rgb(69, 123, 157), // Blue
    egui::Color32::from_rgb(241, 196, 15), // Yellow
    egui::Color32::from_rgb(230, 126, 34), // Orange
    egui::Color32::from_rgb(155, 89, 182), // Purple
    egui::Color32::from_rgb(26, 188, 156), // Cyan
    egui::Color32::from_rgb(232, 67, 147), // Pink
];

/// Render the 3D dome canvas (`Dome3D` mode).
pub(super) fn render_dome_canvas(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
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
                tex_id,
                egui::vec2(dome_size, dome_size),
            ));
            let response = ui.add(img.sense(egui::Sense::click_and_drag()));

            // Mouse interaction: orbit camera
            if response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                actions.session.dome_actions.push(DomeAction::RotateCamera {
                    delta_x: delta.x,
                    delta_y: delta.y,
                });
            }

            // Scroll to zoom
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.1 {
                    actions
                        .session
                        .dome_actions
                        .push(DomeAction::ZoomCamera { delta: scroll });
                }
            }

            // Right-click to reset camera
            if response.clicked_by(egui::PointerButton::Secondary) {
                actions.session.dome_actions.push(DomeAction::ResetCamera);
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
            ui.label(
                egui::RichText::new("3D dome: waiting for renderer…")
                    .weak()
                    .italics(),
            );
        }
    });
}
