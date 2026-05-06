//! Library panel.

use super::super::{UIData, UIActions, LibraryDrag};

pub(super) fn render_library_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.horizontal(|ui| {
        ui.heading("📚 Library");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("◀").on_hover_text("Close library (L)").clicked() {
                actions.toggle_library_panel = true;
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical().scroll_source(egui::scroll_area::ScrollSource {
        scroll_bar: true,
        drag: false,
        mouse_wheel: true,
    }).show(ui, |ui| {
        // === GENERATORS ===
        let gen_header = egui::RichText::new(format!("🎨 Generators ({})", data.generators.len())).strong();
        egui::CollapsingHeader::new(gen_header).default_open(false).show(ui, |ui| {
            for (name, gen_idx) in &data.generators {
                let item_id = egui::Id::new(("lib_gen", *gen_idx));
                let resp = ui.dnd_drag_source(item_id, LibraryDrag::Generator(*gen_idx), |ui| {
                    ui.label(egui::RichText::new(format!("  ◆ {}", name)).size(12.0));
                }).response;
                // Store generator index in temp memory so the deferred drop handler can use it
                if ui.ctx().is_being_dragged(item_id) {
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(egui::Id::new("__lib_dnd_gen_idx"), *gen_idx);
                    });
                }
                // Fallback: double-click adds to first channel
                if resp.double_clicked() {
                    actions.shader_to_add = Some((0, *gen_idx));
                }
                resp.on_hover_text("Drag to a channel to create a deck, or double-click to add to Ch 0");
            }
        });

        ui.add_space(4.0);

        // === EFFECTS ===
        let fx_header = egui::RichText::new(format!("🔮 Effects ({})", data.filters.len())).strong();
        egui::CollapsingHeader::new(fx_header).default_open(false).show(ui, |ui| {
            for (name, filter_idx) in &data.filters {
                let item_id = egui::Id::new(("lib_fx", *filter_idx));
                ui.dnd_drag_source(item_id, LibraryDrag::Effect(*filter_idx), |ui| {
                    ui.label(egui::RichText::new(format!("  ◇ {}", name)).size(12.0));
                });
                // Store effect filter index in temp memory for deferred drop handler
                if ui.ctx().is_being_dragged(item_id) {
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(egui::Id::new("__lib_dnd_fx_idx"), *filter_idx);
                    });
                }
            }
        });

        ui.add_space(4.0);

        // === IMAGES ===
        let img_header = egui::RichText::new("🖼 Images").strong();
        egui::CollapsingHeader::new(img_header).default_open(false).show(ui, |ui| {
            ui.label(egui::RichText::new("Load image files as deck sources").small().weak());
            for ch in &data.channels {
                if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                    actions.open_image_dialog_for_channel = Some(ch.ch_idx);
                }
            }
        });

        ui.add_space(4.0);

        // === VIDEO ===
        let vid_header = egui::RichText::new("🎬 Video").strong();
        egui::CollapsingHeader::new(vid_header).default_open(false).show(ui, |ui| {
            ui.label(egui::RichText::new("Load video files as deck sources").small().weak());
            for ch in &data.channels {
                if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                    actions.open_video_dialog_for_channel = Some(ch.ch_idx);
                }
            }
        });

        ui.add_space(4.0);

        // === SOLID COLOR ===
        let color_header = egui::RichText::new("🎨 Solid Color").strong();
        egui::CollapsingHeader::new(color_header).default_open(false).show(ui, |ui| {
            for ch in &data.channels {
                if ui.button(format!("Add to {}", ch.name)).clicked() {
                    actions.solid_color_to_add = Some((ch.ch_idx, [0.0, 0.0, 0.0, 1.0]));
                }
            }
        });

        ui.add_space(4.0);

        // === CAMERAS ===
        let cam_header = egui::RichText::new(format!("📹 Cameras ({})", data.cameras.len())).strong();
        egui::CollapsingHeader::new(cam_header).default_open(false).show(ui, |ui| {
            if ui.small_button("🔄 Rescan").clicked() {
                actions.camera_rescan = true;
            }
            if data.cameras.is_empty() {
                ui.label(egui::RichText::new("No cameras detected").small().weak());
            }
            for (name, cam_id) in &data.cameras {
                let item_id = egui::Id::new(("lib_cam", *cam_id));
                ui.dnd_drag_source(item_id, LibraryDrag::Camera(*cam_id), |ui| {
                    ui.label(egui::RichText::new(format!("  📹 {}", name)).size(12.0));
                }).response;
                if ui.ctx().is_being_dragged(item_id) {
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(egui::Id::new("__lib_dnd_cam_id"), *cam_id);
                    });
                }
            }
        });

    });
}
