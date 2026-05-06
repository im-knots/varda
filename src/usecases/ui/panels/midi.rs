//! MIDI devices and mappings panel.

use super::super::{UIData, UIActions};

pub(super) fn render_midi_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎹 MIDI");

    ui.horizontal(|ui| {
        ui.label(format!("{} device(s)", data.midi_devices.len()));
        if ui.button("🔄 Rescan").clicked() {
            actions.midi_rescan = true;
        }
    });

    // Device list
    if !data.midi_devices.is_empty() {
        ui.collapsing(format!("Devices ({})", data.midi_devices.len()), |ui| {
            for dev in &data.midi_devices {
                ui.horizontal(|ui| {
                    let mut enabled = dev.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        actions.midi_device_toggles.push((dev.id, enabled));
                    }
                    let status = if dev.enabled { "●" } else { "○" };
                    let color = if dev.enabled {
                        egui::Color32::from_rgb(100, 255, 100)
                    } else {
                        egui::Color32::GRAY
                    };
                    ui.label(egui::RichText::new(status).color(color));
                    ui.label(&dev.name);
                    if dev.has_output {
                        ui.label(egui::RichText::new("⇄").small().color(egui::Color32::from_rgb(100, 200, 255)))
                            .on_hover_text("Has output (LED feedback)");
                    }
                    ui.label(egui::RichText::new(&dev.profile).small().weak());
                });
            }
        });
    }

    // Mappings list
    if !data.midi_mappings.is_empty() {
        ui.collapsing(format!("Mappings ({})", data.midi_mappings.len()), |ui| {
            ui.horizontal(|ui| {
                if ui.button("🗑 Clear All").clicked() {
                    actions.midi_clear_mappings = true;
                }
            });
            egui::ScrollArea::vertical().max_height(150.0).id_salt("midi_mappings_scroll").show(ui, |ui| {
                for mapping in &data.midi_mappings {
                    ui.horizontal(|ui| {
                        if ui.small_button("x").clicked() {
                            actions.midi_remove_mapping.push(mapping.key);
                        }
                        ui.label(egui::RichText::new(&mapping.device_name).small().color(egui::Color32::from_rgb(180, 180, 255)));
                        ui.label(egui::RichText::new(&mapping.key_display).small().strong());
                        ui.label(egui::RichText::new("→").small());
                        ui.label(egui::RichText::new(&mapping.param_path).small().color(egui::Color32::from_rgb(255, 200, 100)));
                    });
                }
            });
        });
    } else {
        ui.label(egui::RichText::new("No mappings. Right-click anywhere → Enter MIDI Learn.").small().weak());
    }
}
