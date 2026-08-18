//! Right side panel.

use super::super::{UIActions, UIData};
use super::dnd::publish_master_surface_fx;
use super::midi::render_midi_section;
use super::modulation::render_modulation_section;
use super::monitoring::render_monitoring_section;
use super::outputs::render_output_section;
use super::stage::render_surface_editor;
use super::tonemap::{render_tonemap_section, tonemap_name};

fn effect_drag_active(ctx: &egui::Context) -> bool {
    use super::super::LibraryDrag;
    egui::DragAndDrop::payload::<LibraryDrag>(ctx)
        .is_some_and(|p| matches!(&*p, LibraryDrag::Effect(_)))
}

fn fx_accent() -> egui::Color32 {
    egui::Color32::from_rgb(100, 200, 255)
}

pub(super) fn render_right_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // Bottom-up so the telemetry cluster is pinned to the panel floor rather
    // than riding the end of the scrolled content, where it would drift with
    // whichever sections happen to be expanded.
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(4.0);
        render_monitoring_section(ui, data, actions);
        ui.separator();

        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            render_right_panel_body(ui, data, actions);
        });
    });
}

fn render_right_panel_body(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Header row: collapse button on left, heading on right (mirror of library panel)
        ui.horizontal(|ui| {
            if ui
                .small_button("»")
                .on_hover_text("Collapse panel")
                .clicked()
            {
                actions.session.toggle_right_panel = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let heading_response = ui.add(
                    egui::Label::new(egui::RichText::new("🎬 Main Output").heading())
                        .sense(egui::Sense::click()),
                );
                if heading_response.clicked() {
                    actions.session.select_master = true;
                }
            });
        });

        // Main output preview (clickable to select master). The height is
        // capped at the panel width so a portrait project does not push the
        // MIDI, modulation and output sections off the bottom of the panel.
        let width = ui.available_width() - 10.0;
        let preview_size = super::utils::preview_size(
            egui::vec2(width, width),
            data.render_width,
            data.render_height,
        );

        if let Some(texture_id) = data.main_output_texture {
            let img_response = ui.add(
                egui::Image::new(egui::load::SizedTexture::new(texture_id, preview_size))
                    .corner_radius(4.0)
                    .sense(egui::Sense::click()),
            );
            publish_master_surface_fx(ui.ctx(), img_response.rect);
            if effect_drag_active(ui.ctx()) && ui.rect_contains_pointer(img_response.rect) {
                ui.painter().rect_stroke(
                    img_response.rect,
                    4.0,
                    egui::Stroke::new(2.0_f32, fx_accent()),
                    egui::StrokeKind::Outside,
                );
            }
            if img_response.clicked() {
                actions.session.select_master = true;
            }
        } else {
            ui.allocate_ui(preview_size, |ui| {
                let (rect, response) = ui.allocate_exact_size(preview_size, egui::Sense::click());
                publish_master_surface_fx(ui.ctx(), rect);
                ui.painter()
                    .rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 20, 30));
                if effect_drag_active(ui.ctx()) && ui.rect_contains_pointer(rect) {
                    ui.painter().rect_stroke(
                        rect,
                        4.0,
                        egui::Stroke::new(2.0_f32, fx_accent()),
                        egui::StrokeKind::Outside,
                    );
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No Output",
                    egui::FontId::proportional(14.0),
                    egui::Color32::GRAY,
                );
                if response.clicked() {
                    actions.session.select_master = true;
                }
            });
        }

        // Hint: click preview to see master effect chain
        let hint_resp = ui.add(
            egui::Label::new(
                egui::RichText::new("Click preview to edit master effects")
                    .small()
                    .weak(),
            )
            .sense(egui::Sense::click()),
        );
        if hint_resp.clicked() {
            actions.session.select_master = true;
        }

        ui.add_space(6.0);

        // === Collapsible sections ===

        // Directly under the preview it grades, and named in full on the header
        // so the active curve is legible without opening anything.
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("🎨 Tonemap — {}", tonemap_name(data.tonemap_mode)))
                .strong(),
        )
        .default_open(false)
        .show(ui, |ui| {
            render_tonemap_section(ui, data, actions);
        });

        ui.add_space(4.0);

        egui::CollapsingHeader::new(egui::RichText::new("〰 Modulation").strong())
            .default_open(false)
            .show(ui, |ui| {
                render_modulation_section(ui, data, actions);
            });

        ui.add_space(4.0);

        // Library panel toggle (if closed, show a button to reopen)
        if !data.library_panel_open {
            if ui.button("📚 Open Library (L)").clicked() {
                actions.session.toggle_library_panel = true;
            }
            ui.add_space(4.0);
        }

        egui::CollapsingHeader::new(egui::RichText::new("🎹 MIDI").strong())
            .default_open(false)
            .show(ui, |ui| {
                render_midi_section(ui, data, actions);
            });

        ui.add_space(4.0);

        egui::CollapsingHeader::new(egui::RichText::new("🗺 Stage Layout").strong())
            .default_open(false)
            .show(ui, |ui| {
                render_surface_editor(ui, data, actions);
            });

        ui.add_space(4.0);

        egui::CollapsingHeader::new(egui::RichText::new("📺 Outputs").strong())
            .default_open(false)
            .show(ui, |ui| {
                render_output_section(ui, data, actions);
            });

        // Loading indicator for background deck loads
        if data.pending_deck_loads > 0 {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            let label = if data.pending_deck_loads == 1 {
                "Loading 1 deck…".to_string()
            } else {
                format!("Loading {} decks…", data.pending_deck_loads)
            };
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new(label)
                        .small()
                        .color(egui::Color32::from_rgb(180, 180, 255)),
                );
            });
            ui.ctx().request_repaint();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_right_panel_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_right_panel(ui, &data, &mut actions);
        });
    }
}
