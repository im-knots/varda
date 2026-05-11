//! Right side panel.

use super::super::{UIData, UIActions};
use super::modulation::render_modulation_section;
use super::midi::render_midi_section;
use super::stage::render_surface_editor;
use super::outputs::render_output_section;

pub(super) fn render_right_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Header row: collapse button on left, heading on right (mirror of library panel)
        ui.horizontal(|ui| {
            if ui.small_button("»").on_hover_text("Collapse panel").clicked() {
                actions.toggle_right_panel = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let heading_response = ui.add(
                    egui::Label::new(egui::RichText::new("🎬 Main Output").heading())
                        .sense(egui::Sense::click()),
                );
                if heading_response.clicked() {
                    actions.select_master = true;
                }
            });
        });

        // Main output preview (clickable to select master)
        let preview_width = ui.available_width() - 10.0;
        let preview_height = preview_width * 0.5625;
        let preview_size = egui::vec2(preview_width, preview_height);

        if let Some(texture_id) = data.main_output_texture {
            let img_response = ui.add(egui::Image::new(egui::load::SizedTexture::new(texture_id, preview_size))
                .corner_radius(4.0)
                .sense(egui::Sense::click()));
            if img_response.clicked() {
                actions.select_master = true;
            }
        } else {
            ui.allocate_ui(preview_size, |ui| {
                let (rect, response) = ui.allocate_exact_size(preview_size, egui::Sense::click());
                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 20, 30));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No Output",
                    egui::FontId::proportional(14.0),
                    egui::Color32::GRAY,
                );
                if response.clicked() {
                    actions.select_master = true;
                }
            });
        }

        ui.add_space(10.0);

        // === Collapsible sections (like the library panel) ===

        egui::CollapsingHeader::new(egui::RichText::new("🔮 Master Effects").strong())
            .default_open(false)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("(Apply to final composite)").small().weak());
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
                actions.toggle_library_panel = true;
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
                ui.label(egui::RichText::new(label).small().color(egui::Color32::from_rgb(180, 180, 255)));
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