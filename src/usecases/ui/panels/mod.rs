//! UI panel rendering — decomposed into focused modules.
//!
//! Each sub-module renders a specific panel or UI section.
//! The `render_ui` function orchestrates the top-level layout.

mod geometry;
mod utils;
mod library;
mod notifications_overlay;
mod right_panel;
mod stage;
mod outputs;
mod deck_detail;
mod effects;
mod modulation;
mod midi;
mod mixer;
mod sequence;

use super::{UIData, UIActions, LibraryDrag, EffectDrag};
use library::render_library_panel;
use right_panel::render_right_panel;
use deck_detail::render_bottom_panel;
use mixer::render_central_panel;
use notifications_overlay::render_notifications;

/// Top-level UI rendering entry point. Orchestrates all panels.
pub fn render_ui(ctx: &egui::Context, data: &UIData) -> UIActions {
    let mut actions = UIActions::new();

    // Disable all egui animations — instant panel/widget transitions
    ctx.style_mut(|style| {
        style.animation_time = 0.0;
    });

    // === LEFT PANEL: Library (collapsible) ===
    if data.library_panel_open {
        egui::SidePanel::left("library_panel")
            .min_width(180.0)
            .default_width(220.0)
            .resizable(true)
            .show(ctx, |ui| {
                render_library_panel(ui, data, &mut actions);
            });
    } else {
        egui::SidePanel::left("library_collapsed")
            .exact_width(36.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    if ui.small_button("▶").on_hover_text("Open library (L)").clicked() {
                        actions.toggle_library_panel = true;
                    }
                });
            });
    }

    // === RIGHT PANEL: Main Output + Master Effects ===
    egui::SidePanel::right("master_panel")
        .min_width(280.0)
        .default_width(320.0)
        .show(ctx, |ui| {
            render_right_panel(ui, data, &mut actions);
        });

    // === BOTTOM PANEL: Audio, Modulation, Shader Browser ===
    egui::TopBottomPanel::bottom("bottom_panel")
        .min_height(80.0)
        .max_height(400.0)
        .default_height(180.0)
        .resizable(true)
        .show_separator_line(true)
        .show(ctx, |ui| {
            ui.set_min_height(ui.max_rect().height());
            render_bottom_panel(ui, data, &mut actions);
        });

    // === TOP BAR: Save button + FPS/BPM status ===
    egui::TopBottomPanel::top("top_bar")
        .exact_height(28.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if ui.button("💾 Save").on_hover_text("Save workspace (⌘S)").clicked() {
                    actions.save_requested = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // BPM from unified clock (MIDI > OSC > Audio > --)
                    let bpm_text = if let Some(bpm) = data.clock_bpm {
                        format!("{:.0} BPM", bpm)
                    } else {
                        "-- BPM".to_string()
                    };
                    if let Some(dev) = &data.clock_device_name {
                        ui.label(egui::RichText::new(format!("({})", dev)).weak().small());
                    }
                    // Clickable BPM label → opens clock source popover
                    let bpm_response = ui.add(
                        egui::Label::new(egui::RichText::new(&bpm_text).monospace())
                            .sense(egui::Sense::click()),
                    ).on_hover_text("Click to select clock source");
                    egui::Popup::from_toggle_button_response(&bpm_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_clock_popover(ui, data, &mut actions);
                        });

                    ui.separator();

                    let fps = data.fps;
                    let fps_color = if fps > 55.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if fps > 30.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    let fps_response = ui.add(
                        egui::Label::new(egui::RichText::new(format!("{:.0} FPS", fps)).color(fps_color).monospace())
                            .sense(egui::Sense::click()),
                    ).on_hover_text("Click for render timing details");
                    egui::Popup::from_toggle_button_response(&fps_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_fps_popover(ui, data);
                        });

                    ui.separator();

                    // GPU load — render budget % (total render time / 16.67ms target)
                    let total_render_ms: f32 = data.channel_render_stats.iter().map(|s| s.render_time_ms).sum();
                    let gpu_load_pct = (total_render_ms / 16.67) * 100.0;
                    let gpu_color = if gpu_load_pct < 50.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if gpu_load_pct < 80.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    let gpu_response = ui.add(
                        egui::Label::new(egui::RichText::new(format!("🖥 {:.0}%", gpu_load_pct))
                            .color(gpu_color).monospace())
                            .sense(egui::Sense::click()),
                    ).on_hover_text("GPU render load — click for details");
                    egui::Popup::from_toggle_button_response(&gpu_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_gpu_popover(ui, data);
                        });

                    ui.separator();

                    // CPU & RAM usage
                    let cpu_color = if data.cpu_usage < 50.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if data.cpu_usage < 80.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    ui.label(egui::RichText::new(format!("CPU {:.0}%", data.cpu_usage))
                        .color(cpu_color).monospace().small());

                    let ram_gb = data.ram_used as f64 / (1024.0 * 1024.0 * 1024.0);
                    let ram_total_gb = data.ram_total as f64 / (1024.0 * 1024.0 * 1024.0);
                    let ram_pct = if data.ram_total > 0 { data.ram_used as f32 / data.ram_total as f32 * 100.0 } else { 0.0 };
                    let ram_color = if ram_pct < 50.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if ram_pct < 80.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    ui.label(egui::RichText::new(format!("RAM {:.1}/{:.0}G", ram_gb, ram_total_gb))
                        .color(ram_color).monospace().small());

                    ui.separator();

                    // Resolution selector
                    let res_label = format!("📐 {}×{}", data.render_width, data.render_height);
                    let res_response = ui.add(
                        egui::Label::new(egui::RichText::new(&res_label).monospace())
                            .sense(egui::Sense::click()),
                    ).on_hover_text("Click to change render resolution");
                    egui::Popup::from_toggle_button_response(&res_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_resolution_popover(ui, data, &mut actions);
                        });
                });
            });
        });

    // === CENTRAL AREA: Decks as columns ===
    egui::CentralPanel::default().show(ctx, |ui| {
        render_central_panel(ui, data, &mut actions);
    });

    // === LIBRARY DnD: deferred drop handler ===
    handle_library_dnd(ctx, data, &mut actions);

    // === EFFECT REORDER DnD: deferred drop handler ===
    handle_effect_dnd(ctx, data, &mut actions);

    // === NOTIFICATION OVERLAY ===
    render_notifications(ctx, &data.notifications, &mut actions);

    // === GLOBAL RIGHT-CLICK: Toggle MIDI Learn Mode ===
    handle_midi_learn_popup(ctx, data, &mut actions);

    // === KEYBOARD SHORTCUTS (global) ===
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
        actions.save_requested = true;
    }
    if !ctx.wants_keyboard_input() {
        if ctx.input(|i| i.key_pressed(egui::Key::L)) {
            actions.toggle_library_panel = true;
        }
    }

    actions
}

/// Deferred library drag-and-drop handler.
/// Each frame while a LibraryDrag payload is active, find which drop target the pointer is over.
/// When the payload disappears (mouse released), apply the drop action.
fn handle_library_dnd(ctx: &egui::Context, data: &UIData, actions: &mut UIActions) {
    let had_payload_id = egui::Id::new("__lib_dnd_had_payload");
    let hover_ch_id = egui::Id::new("__lib_dnd_hover_ch");
    let hover_fx_target_id = egui::Id::new("__lib_dnd_hover_fx_target");
    let has_payload = egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx);

    if has_payload {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            let mut found_ch: Option<usize> = None;
            for ch_idx in 0..data.channels.len() {
                let key = egui::Id::new("ch_drop_rect").with(ch_idx);
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                    if rect.contains(pos) {
                        found_ch = Some(ch_idx);
                        break;
                    }
                }
            }

            let mut found_fx: Option<(String, usize, usize)> = None;
            if data.selected_master {
                let master_key = egui::Id::new("master_fx_drop_rect");
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(master_key)) {
                    if rect.contains(pos) {
                        found_fx = Some(("master".to_string(), 0, 0));
                    }
                }
            } else if let Some(ch_idx) = data.selected_channel {
                let key = egui::Id::new("ch_fx_drop_rect").with(ch_idx);
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                    if rect.contains(pos) {
                        found_fx = Some(("channel".to_string(), ch_idx, 0));
                    }
                }
            } else if let Some((sel_ch, sel_dk)) = data.selected_deck {
                let key = egui::Id::new("deck_fx_drop_rect").with((sel_ch, sel_dk));
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                    if rect.contains(pos) {
                        found_fx = Some(("deck".to_string(), sel_ch, sel_dk));
                    }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.insert_temp(hover_ch_id, found_ch);
                mem.data.insert_temp(hover_fx_target_id, found_fx);
                mem.data.insert_temp::<bool>(had_payload_id, true);
            });
        }
    } else {
        let had_payload: bool = ctx.memory(|mem| mem.data.get_temp(had_payload_id).unwrap_or(false));
        if had_payload {
            let hover_ch: Option<usize> = ctx.memory(|mem| mem.data.get_temp(hover_ch_id).unwrap_or(None));
            let hover_fx: Option<(String, usize, usize)> = ctx.memory(|mem| mem.data.get_temp(hover_fx_target_id).unwrap_or(None));

            if let Some(ch_idx) = hover_ch {
                let gen_key = egui::Id::new("__lib_dnd_gen_idx");
                let gen_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(gen_key));
                if let Some(gen_idx) = gen_idx {
                    log::info!("Library drop (deferred): generator {} -> ch{}", gen_idx, ch_idx);
                    actions.shader_to_add = Some((ch_idx, gen_idx));
                }

                let cam_key = egui::Id::new("__lib_dnd_cam_id");
                let cam_id: Option<crate::camera::CameraId> = ctx.memory(|mem| mem.data.get_temp(cam_key));
                if let Some(cam_id) = cam_id {
                    log::info!("Library drop (deferred): camera {} -> ch{}", cam_id, ch_idx);
                    actions.camera_to_add = Some((ch_idx, cam_id));
                }
            }

            if let Some((target_type, ch_idx, deck_idx)) = hover_fx {
                let fx_key = egui::Id::new("__lib_dnd_fx_idx");
                let filter_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(fx_key));
                if let Some(filter_idx) = filter_idx {
                    match target_type.as_str() {
                        "deck" => {
                            log::info!("Library drop (deferred): effect {} -> ch{} deck{}", filter_idx, ch_idx, deck_idx);
                            actions.effect_to_add = Some((ch_idx, deck_idx, filter_idx));
                        }
                        "channel" => {
                            log::info!("Library drop (deferred): effect {} -> ch{} channel fx", filter_idx, ch_idx);
                            actions.ch_effect_to_add = Some((ch_idx, filter_idx));
                        }
                        "master" => {
                            log::info!("Library drop (deferred): effect {} -> master fx", filter_idx);
                            actions.master_effect_to_add = Some(filter_idx);
                        }
                        _ => {}
                    }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.remove::<bool>(had_payload_id);
                mem.data.remove::<Option<usize>>(hover_ch_id);
                mem.data.remove::<Option<(String, usize, usize)>>(hover_fx_target_id);
                mem.data.remove::<usize>(egui::Id::new("__lib_dnd_gen_idx"));
                mem.data.remove::<usize>(egui::Id::new("__lib_dnd_fx_idx"));
                mem.data.remove::<crate::camera::CameraId>(egui::Id::new("__lib_dnd_cam_id"));
            });
        }
    }
}


/// Deferred effect reorder drag-and-drop handler.
/// Same pattern as library drops — tracks which drop zone the pointer is over,
/// then applies the move when the payload disappears.
fn handle_effect_dnd(ctx: &egui::Context, data: &UIData, actions: &mut UIActions) {
    let had_eff_id = egui::Id::new("__eff_dnd_had_payload");
    let hover_dz_id = egui::Id::new("__eff_dnd_hover_dz");
    let has_eff_payload = egui::DragAndDrop::has_payload_of_type::<EffectDrag>(ctx);

    if has_eff_payload {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            let mut found_dz: Option<(String, usize)> = None;

            let check_chain = |chain_key: &str, ctx: &egui::Context, pos: egui::Pos2| -> Option<(String, usize)> {
                let count_key = egui::Id::new("eff_dz_count").with(chain_key.to_string());
                let count: usize = ctx.memory(|mem| mem.data.get_temp(count_key).unwrap_or(0));
                for p in 0..count {
                    let rk = egui::Id::new("eff_dz_rect").with((chain_key.to_string(), p));
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(rk)) {
                        if rect.contains(pos) {
                            return Some((chain_key.to_string(), p));
                        }
                    }
                }
                None
            };

            if found_dz.is_none() {
                if let Some((sel_ch, sel_dk)) = data.selected_deck {
                    found_dz = check_chain(&format!("deck_{}_{}", sel_ch, sel_dk), ctx, pos);
                }
            }
            if found_dz.is_none() {
                found_dz = check_chain("master", ctx, pos);
            }
            if found_dz.is_none() {
                for ch_idx in 0..data.channels.len() {
                    found_dz = check_chain(&format!("ch_{}", ch_idx), ctx, pos);
                    if found_dz.is_some() { break; }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.insert_temp(hover_dz_id, found_dz);
                mem.data.insert_temp::<bool>(had_eff_id, true);
            });
        }
    } else {
        let had: bool = ctx.memory(|mem| mem.data.get_temp(had_eff_id).unwrap_or(false));
        if had {
            let hover_dz: Option<(String, usize)> = ctx.memory(|mem| mem.data.get_temp(hover_dz_id).unwrap_or(None));
            let src_key = egui::Id::new("__eff_dnd_src");
            let src: Option<EffectDrag> = ctx.memory(|mem| mem.data.get_temp(src_key));

            if let (Some((chain_key, target_pos)), Some(src_drag)) = (hover_dz, src) {
                match src_drag {
                    EffectDrag::Deck(src_ch, src_dk, src_eff) => {
                        let expected_key = format!("deck_{}_{}", src_ch, src_dk);
                        if chain_key == expected_key {
                            let to = if src_eff < target_pos { target_pos - 1 } else { target_pos };
                            if to != src_eff {
                                log::info!("Effect reorder (deferred): deck {}/{} effect {} -> {}", src_ch, src_dk, src_eff, to);
                                actions.effect_to_move = Some((src_ch, src_dk, src_eff, to));
                            }
                        }
                    }
                    EffectDrag::Channel(src_ch, src_eff) => {
                        let expected_key = format!("ch_{}", src_ch);
                        if chain_key == expected_key {
                            let to = if src_eff < target_pos { target_pos - 1 } else { target_pos };
                            if to != src_eff {
                                log::info!("Effect reorder (deferred): ch{} effect {} -> {}", src_ch, src_eff, to);
                                actions.ch_effect_to_move = Some((src_ch, src_eff, to));
                            }
                        }
                    }
                    EffectDrag::Master(src_eff) => {
                        if chain_key == "master" {
                            let to = if src_eff < target_pos { target_pos - 1 } else { target_pos };
                            if to != src_eff {
                                log::info!("Effect reorder (deferred): master effect {} -> {}", src_eff, to);
                                actions.master_effect_to_move = Some((src_eff, to));
                            }
                        }
                    }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.remove::<bool>(had_eff_id);
                mem.data.remove::<Option<(String, usize)>>(hover_dz_id);
                mem.data.remove::<EffectDrag>(src_key);
            });
        }
    }
}

/// Global right-click popup for toggling MIDI learn mode.
fn handle_midi_learn_popup(ctx: &egui::Context, data: &UIData, actions: &mut UIActions) {
    let popup_id = egui::Id::new("global_midi_learn_popup");
    let popup_fresh_id = egui::Id::new("global_midi_learn_popup_fresh");

    let popup_pos: Option<egui::Pos2> = ctx.memory(|mem| mem.data.get_temp(popup_id));
    let popup_fresh: bool = ctx.memory(|mem| mem.data.get_temp(popup_fresh_id).unwrap_or(false));

    if ctx.input(|i| i.pointer.secondary_clicked()) {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            if popup_pos.is_some() {
                ctx.memory_mut(|mem| {
                    mem.data.remove::<egui::Pos2>(popup_id);
                    mem.data.remove::<bool>(popup_fresh_id);
                });
            } else {
                ctx.memory_mut(|mem| {
                    mem.data.insert_temp(popup_id, pos);
                    mem.data.insert_temp(popup_fresh_id, true);
                });
            }
        }
    }

    let popup_pos: Option<egui::Pos2> = ctx.memory(|mem| mem.data.get_temp(popup_id));
    if let Some(pos) = popup_pos {
        let label = if data.midi_learn_active { "🎹 Exit MIDI Learn" } else { "🎹 Enter MIDI Learn" };

        let area_resp = egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if ui.button(label).clicked() {
                        actions.midi_learn_toggle = true;
                        ctx.memory_mut(|mem| {
                            mem.data.remove::<egui::Pos2>(popup_id);
                            mem.data.remove::<bool>(popup_fresh_id);
                        });
                    }
                });
            });

        if !popup_fresh {
            if ctx.input(|i| i.pointer.primary_clicked()) {
                let popup_rect = area_resp.response.rect;
                let click_pos = ctx.input(|i| i.pointer.interact_pos());
                if let Some(click) = click_pos {
                    if !popup_rect.contains(click) {
                        ctx.memory_mut(|mem| {
                            mem.data.remove::<egui::Pos2>(popup_id);
                            mem.data.remove::<bool>(popup_fresh_id);
                        });
                    }
                }
            }
        } else {
            ctx.memory_mut(|mem| {
                mem.data.insert_temp(popup_fresh_id, false);
            });
        }
    }
}


/// Render the FPS details popover (shown when clicking FPS in the top bar).
fn render_fps_popover(ui: &mut egui::Ui, data: &UIData) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("⏱ Render Pipeline").strong());
    ui.separator();

    let fps_color = |fps: f32| {
        if fps > 55.0 { egui::Color32::from_rgb(100, 220, 100) }
        else if fps > 30.0 { egui::Color32::from_rgb(220, 200, 60) }
        else { egui::Color32::from_rgb(220, 60, 60) }
    };

    ui.horizontal(|ui| {
        ui.label("Avg pipeline FPS:");
        ui.label(egui::RichText::new(format!("{:.0}", data.fps)).color(fps_color(data.fps)).monospace().strong());
    });
    ui.add_space(4.0);

    if data.channel_render_stats.is_empty() {
        ui.label(egui::RichText::new("No channels").weak());
    } else {
        egui::Grid::new("fps_channel_grid").striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Channel").strong().small());
            ui.label(egui::RichText::new("Avg FPS").strong().small());
            ui.label(egui::RichText::new("Decks").strong().small());
            ui.label(egui::RichText::new("Time").strong().small());
            ui.end_row();

            for stat in &data.channel_render_stats {
                ui.label(&stat.name);
                if stat.avg_deck_fps > 0.0 {
                    ui.label(egui::RichText::new(format!("{:.0}", stat.avg_deck_fps))
                        .color(fps_color(stat.avg_deck_fps)).monospace());
                } else {
                    ui.label(egui::RichText::new("--").weak());
                }
                ui.label(format!("{}", stat.active_deck_count));
                ui.label(egui::RichText::new(format!("{:.1}ms", stat.render_time_ms)).monospace());
                ui.end_row();
            }
        });

        let total_active: u32 = data.channel_render_stats.iter().map(|s| s.active_deck_count).sum();
        let total_ms: f32 = data.channel_render_stats.iter().map(|s| s.render_time_ms).sum();
        ui.add_space(4.0);
        ui.separator();
        ui.label(format!("{} active decks · {:.1}ms total render", total_active, total_ms));
    }
}


/// Render the GPU details popover (shown when clicking GPU device in the top bar).
fn render_gpu_popover(ui: &mut egui::Ui, data: &UIData) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("🖥 GPU Details").strong());
    ui.separator();

    egui::Grid::new("gpu_details_grid").show(ui, |ui| {
        ui.label(egui::RichText::new("Device").strong().small());
        ui.label(&data.gpu_device_name);
        ui.end_row();

        ui.label(egui::RichText::new("Type").strong().small());
        ui.label(&data.gpu_device_type);
        ui.end_row();

        ui.label(egui::RichText::new("Backend").strong().small());
        ui.label(&data.gpu_backend);
        ui.end_row();

        if !data.gpu_driver.is_empty() {
            ui.label(egui::RichText::new("Driver").strong().small());
            ui.label(&data.gpu_driver);
            ui.end_row();
        }

        if !data.gpu_driver_info.is_empty() {
            ui.label(egui::RichText::new("Driver info").strong().small());
            ui.label(&data.gpu_driver_info);
            ui.end_row();
        }

        let total_render_ms: f32 = data.channel_render_stats.iter().map(|s| s.render_time_ms).sum();
        let load_pct = (total_render_ms / 16.67) * 100.0;
        ui.label(egui::RichText::new("Render load").strong().small());
        let load_color = if load_pct < 50.0 {
            egui::Color32::from_rgb(100, 220, 100)
        } else if load_pct < 80.0 {
            egui::Color32::from_rgb(220, 200, 60)
        } else {
            egui::Color32::from_rgb(220, 60, 60)
        };
        ui.label(egui::RichText::new(format!("{:.1}ms / 16.7ms ({:.0}%)", total_render_ms, load_pct))
            .color(load_color).monospace());
        ui.end_row();
    });
}


/// Render the clock source popover (shown when clicking BPM in the top bar).
fn render_clock_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("🕐 Clock Source").strong());
    ui.separator();

    let is_auto = data.clock_preference == "Auto";

    // Auto option
    if ui.radio(is_auto, "Auto (recommended)").clicked() && !is_auto {
        actions.clock_preference = Some(crate::clock::ClockPreference::Auto);
    }

    // Detected MIDI devices
    for src in &data.clock_detected_midi {
        let is_selected = data.clock_preference_force_device_id == Some(src.device_id);
        let bpm_str = src.bpm.map_or("--".to_string(), |b| format!("{:.0}", b));
        let label = format!("🟣 {}  {} BPM", src.device_name, bpm_str);
        if ui.radio(is_selected, label).clicked() && !is_selected {
            actions.clock_preference = Some(crate::clock::ClockPreference::ForceMidi {
                device_id: src.device_id,
            });
        }
    }

    // OSC option (only shown if OSC is active)
    if data.clock_osc_active {
        let is_osc = data.clock_preference == "ForceOsc";
        let bpm_str = data.clock_osc_bpm.map_or("--".to_string(), |b| format!("{:.0}", b));
        let label = format!("🔵 OSC  {} BPM", bpm_str);
        if ui.radio(is_osc, label).clicked() && !is_osc {
            actions.clock_preference = Some(crate::clock::ClockPreference::ForceOsc);
        }
    }

    // Audio only option
    let is_audio = data.clock_preference == "ForceAudio";
    let audio_bpm_str = data.clock_audio_bpm.map_or("--".to_string(), |b| format!("{:.0}", b));
    let label = format!("🟢 Audio only  {} BPM", audio_bpm_str);
    if ui.radio(is_audio, label).clicked() && !is_audio {
        actions.clock_preference = Some(crate::clock::ClockPreference::ForceAudio);
    }

    // Manual BPM option
    let is_manual = data.clock_preference == "ForceManual";
    let mut manual_bpm = data.clock_manual_bpm.unwrap_or(120.0);
    ui.horizontal(|ui| {
        if ui.radio(is_manual, "🟠 Manual").clicked() && !is_manual {
            actions.clock_preference = Some(crate::clock::ClockPreference::ForceManual { bpm: manual_bpm });
        }
        if is_manual {
            let drag = ui.add(
                egui::DragValue::new(&mut manual_bpm)
                    .range(20.0..=300.0)
                    .speed(0.5)
                    .suffix(" BPM"),
            );
            if drag.changed() {
                actions.manual_bpm = Some(manual_bpm);
            }
        }
    });

    // Current status line
    ui.separator();
    let status = match data.clock_source.as_str() {
        "MIDI" => {
            let dev = data.clock_device_name.as_deref().unwrap_or("Unknown");
            format!("Currently: {} ({})", dev, if is_auto { "auto" } else { "forced" })
        }
        "OSC" => format!("Currently: OSC ({})", if is_auto { "auto" } else { "forced" }),
        "Audio" => format!("Currently: Audio ({})", if is_auto { "auto" } else { "forced" }),
        "Manual" => format!("Currently: Manual ({:.0} BPM)", manual_bpm),
        _ => "Currently: No clock".to_string(),
    };
    ui.label(egui::RichText::new(status).weak().small());
}


/// Render the resolution popover (shown when clicking resolution in the top bar).
fn render_resolution_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(200.0);
    ui.label(egui::RichText::new("📐 Render Resolution").strong());
    ui.separator();

    let current_w = data.render_width;
    let current_h = data.render_height;

    // Common presets
    let presets: &[(&str, u32, u32)] = &[
        ("720p", 1280, 720),
        ("1080p", 1920, 1080),
        ("1440p", 2560, 1440),
        ("4K", 3840, 2160),
    ];

    for &(label, w, h) in presets {
        let is_current = current_w == w && current_h == h;
        let text = format!("{} ({}×{})", label, w, h);
        if ui.radio(is_current, text).clicked() && !is_current {
            actions.resolution_change = Some((w, h));
        }
    }

    ui.separator();
    ui.label(egui::RichText::new("Custom").strong().small());

    // Custom W×H input — use persistent state via egui memory
    let custom_w_id = ui.id().with("custom_res_w");
    let custom_h_id = ui.id().with("custom_res_h");
    let mut custom_w: u32 = ui.data(|d| d.get_temp(custom_w_id)).unwrap_or(current_w);
    let mut custom_h: u32 = ui.data(|d| d.get_temp(custom_h_id)).unwrap_or(current_h);

    ui.horizontal(|ui| {
        ui.label("W:");
        ui.add(egui::DragValue::new(&mut custom_w).range(64..=7680).speed(16));
        ui.label("H:");
        ui.add(egui::DragValue::new(&mut custom_h).range(64..=4320).speed(16));
    });

    ui.data_mut(|d| {
        d.insert_temp(custom_w_id, custom_w);
        d.insert_temp(custom_h_id, custom_h);
    });

    let is_custom_different = custom_w != current_w || custom_h != current_h;
    if ui.add_enabled(is_custom_different && custom_w > 0 && custom_h > 0,
        egui::Button::new("Apply")).clicked()
    {
        actions.resolution_change = Some((custom_w, custom_h));
    }

    ui.separator();
    ui.label(egui::RichText::new(format!("Current: {}×{}", current_w, current_h)).weak().small());
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: render_ui doesn't panic with the test fixture.
    #[test]
    fn render_ui_smoke_default_fixture() {
        let data = UIData::test_fixture();
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        // Running the harness processes a frame — if render_ui panics, this test fails.
        let _ = harness;
    }

    /// Smoke test: render_ui with empty channels doesn't panic.
    #[test]
    fn render_ui_smoke_empty_channels() {
        let mut data = UIData::test_fixture();
        data.channels.clear();
        data.channel_count = 0;
        data.channel_names.clear();
        data.selected_deck = None;
        data.selected_channel = None;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        let _ = harness;
    }

    /// Smoke test: render_ui with library panel closed doesn't panic.
    #[test]
    fn render_ui_smoke_library_closed() {
        let mut data = UIData::test_fixture();
        data.library_panel_open = false;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        let _ = harness;
    }

    /// Smoke test: render_ui with stage editor open doesn't panic.
    #[test]
    fn render_ui_smoke_stage_editor_open() {
        let mut data = UIData::test_fixture();
        data.stage_editor_open = true;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        let _ = harness;
    }

    /// Smoke test: render_ui with master selected doesn't panic.
    #[test]
    fn render_ui_smoke_master_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_master = true;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        let _ = harness;
    }

    /// Smoke test: render_ui with channel selected doesn't panic.
    #[test]
    fn render_ui_smoke_channel_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = Some(0);
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        let _ = harness;
    }

    /// Smoke test: render_ui with MIDI learn active doesn't panic.
    #[test]
    fn render_ui_smoke_midi_learn() {
        let mut data = UIData::test_fixture();
        data.midi_learn_active = true;
        data.midi_learn_target = Some("crossfader".to_string());
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui.ctx(), &data);
        });
        let _ = harness;
    }
}