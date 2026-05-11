//! Master and channel effect detail panels.

use crate::params::ParamValue;
use super::super::{UIData, UIActions, ParamUpdate, ModulationAction, LibraryDrag, widgets, EffectDrag};
use super::utils::{channel_color, render_effect_drop_zone, render_effect_drag_handle, render_effect_drag_ghost};

pub(super) fn render_master_effect_detail(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎬 Master Effects");

    egui::ScrollArea::horizontal().id_salt("master_fx_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            {
                for (eff_idx, (eff_uuid, eff_name, eff_enabled, eff_params)) in data.master_effect_info.iter().enumerate() {
                    let eff_uuid_master = eff_uuid.clone();
                    let eff_uuid_master_remove = eff_uuid.clone();
                    render_effect_drop_zone(ui, "master", eff_idx);

                    let card_resp = egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            let max_h = (ui.available_height() - 8.0).max(100.0);
                            egui::ScrollArea::vertical().id_salt(format!("master_fx_scroll_{}", eff_idx)).max_height(max_h).scroll_source(egui::scroll_area::ScrollSource { drag: false, scroll_bar: true, mouse_wheel: true }).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Drag handle
                                    render_effect_drag_handle(ui, EffectDrag::Master(eff_idx));
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.master_effect_to_toggle = Some(eff_idx);
                                    }
                                    ui.label(egui::RichText::new(eff_name).strong());
                                });

                                if !eff_params.params.is_empty() {
                                    let eff_idx_copy = eff_idx;
                                    let midi_prefix = format!("master/effect/{}", eff_idx_copy);
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| match val {
                                            ParamValue::Float(v) => ParamUpdate::MasterEffectFloat { effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Bool(v) => ParamUpdate::MasterEffectBool { effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Color(v) => ParamUpdate::MasterEffectColor { effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            _ => unreachable!(),
                                        },
                                        Some(&|name: &str, source_uuid: &str| ModulationAction::AssignEffectModulation {
                                            effect_uuid: eff_uuid_master.clone(),
                                            param_name: name.to_string(), source_id: source_uuid.to_string(), amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveEffectAssignment {
                                            effect_uuid: eff_uuid_master_remove.clone(),
                                            param_name: name.to_string(),
                                        }),
                                        &mut actions.param_updates,
                                        &mut actions.modulation_actions,
                                        &format!("master_fx_{}", eff_idx_copy),
                                        Some(&midi_prefix),
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("fx_{}", eff_uuid),
                                        data.keyboard_learn_active,
                                        &mut actions.keyboard_learn_select,
                                        data.keyboard_learn_target.as_deref(),
                                    );
                                }
                            });
                            });
                        });
                    {
                        let card_rect = card_resp.response.rect;
                        let btn_size = egui::vec2(16.0, 16.0);
                        let btn_pos = egui::pos2(card_rect.right() - btn_size.x - 4.0, card_rect.top() + 4.0);
                        let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                        let btn_resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                        let color = if btn_resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                        ui.painter().text(btn_rect.center(), egui::Align2::CENTER_CENTER, "x", egui::FontId::proportional(12.0), color);
                        if btn_resp.clicked() {
                            actions.master_effect_to_remove = Some(eff_idx);
                        }
                    }
                    render_effect_drag_ghost(
                        ui,
                        egui::Id::new(("eff_ghost_master", eff_idx)),
                        EffectDrag::Master(eff_idx),
                        eff_name,
                    );
                    ui.separator();
                }

                // Drop zone after last effect (for reordering)
                if !data.master_effect_info.is_empty() {
                    let num_effects = data.master_effect_info.len();
                    render_effect_drop_zone(ui, "master", num_effects);
                }

                // Remaining space: always present drop target
                let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                    .map(|p| matches!(&*p, LibraryDrag::Effect(_))).unwrap_or(false);
                let remaining_w = ui.available_width().max(80.0);
                let remaining_h = ui.available_height().max(40.0);
                let stroke = if has_fx_drag { egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
                let fill = if has_fx_drag { egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20) } else { egui::Color32::TRANSPARENT };
                egui::Frame::default()
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(remaining_w - 16.0, remaining_h - 16.0));
                        ui.centered_and_justified(|ui| {
                            ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                        });
                    });
            }

            // Store master effect chain rect for deferred library drops
            let chain_rect = ui.min_rect();
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("master_fx_drop_rect"), chain_rect);
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with("master".to_string()), data.master_effect_info.len() + 1);
            });
        });
    });
}


/// Render channel effect chain detail in the bottom bar
pub(super) fn render_channel_effect_detail(ui: &mut egui::Ui, ch_idx: usize, data: &UIData, actions: &mut UIActions) {
    let Some(ch) = data.channels.get(ch_idx) else {
        ui.label(egui::RichText::new("Channel not found").weak());
        return;
    };

    let accent = channel_color(ch_idx);
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new(format!("🔮 {} Effects", ch.name)).color(accent));

        // Save channel as preset — inline name prompt
        let prompt_id = egui::Id::new("ch_preset_name_prompt");
        let name_id = egui::Id::new("ch_preset_name_input");
        let is_prompting: bool = ui.data(|d| d.get_temp(prompt_id)).unwrap_or(false);

        if is_prompting {
            let cleared_id = egui::Id::new("ch_preset_name_cleared");
            let was_cleared: bool = ui.data(|d| d.get_temp(cleared_id)).unwrap_or(false);
            let mut name: String = ui.data(|d| d.get_temp(name_id)).unwrap_or_else(|| ch.name.clone());
            let response = ui.text_edit_singleline(&mut name);
            if response.gained_focus() && !was_cleared {
                name.clear();
                ui.data_mut(|d| d.insert_temp(cleared_id, true));
            }
            if ui.small_button("✓ Save").clicked() && !name.is_empty() {
                actions.save_channel_preset = Some((ch_idx, name.clone()));
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            if ui.small_button("✕").clicked() {
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            ui.data_mut(|d| d.insert_temp(name_id, name));
        } else if ui.small_button("💾 Save Channel Preset").clicked() {
            ui.data_mut(|d| {
                d.insert_temp(prompt_id, true);
                d.remove_temp::<String>(name_id);
                d.insert_temp(egui::Id::new("ch_preset_name_cleared"), false);
            });
        }
    });

    egui::ScrollArea::horizontal().id_salt("channel_fx_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            {
                for (eff_idx, (eff_uuid, eff_name, eff_enabled, eff_params)) in ch.effects.iter().enumerate() {
                    let eff_uuid_ch_assign = eff_uuid.clone();
                    let eff_uuid_ch_remove = eff_uuid.clone();
                    render_effect_drop_zone(ui, &format!("ch_{}", ch_idx), eff_idx);

                    let card_resp = egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            let max_h = (ui.available_height() - 8.0).max(100.0);
                            egui::ScrollArea::vertical().id_salt(format!("ch_fx_scroll_{}_{}", ch_idx, eff_idx)).max_height(max_h).scroll_source(egui::scroll_area::ScrollSource { drag: false, scroll_bar: true, mouse_wheel: true }).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    render_effect_drag_handle(ui, EffectDrag::Channel(ch_idx, eff_idx));
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.ch_effect_to_toggle = Some((ch_idx, eff_idx));
                                    }
                                    ui.label(egui::RichText::new(eff_name).strong().color(accent));
                                });

                                if !eff_params.params.is_empty() {
                                    let ch_copy = ch_idx;
                                    let eff_idx_copy = eff_idx;
                                    let ch_uuid = ch.uuid.clone();
                                    let _ch_uuid = ch_uuid.clone();
                                    let midi_prefix = format!("ch/{}/effect/{}", ch_uuid, eff_idx_copy);
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| match val {
                                            ParamValue::Float(v) => ParamUpdate::ChannelEffectFloat { ch_idx: ch_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Bool(v) => ParamUpdate::ChannelEffectBool { ch_idx: ch_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Color(v) => ParamUpdate::ChannelEffectColor { ch_idx: ch_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            _ => unreachable!(),
                                        },
                                        Some(&|name: &str, source_uuid: &str| ModulationAction::AssignEffectModulation {
                                            effect_uuid: eff_uuid_ch_assign.clone(),
                                            param_name: name.to_string(), source_id: source_uuid.to_string(), amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveEffectAssignment {
                                            effect_uuid: eff_uuid_ch_remove.clone(),
                                            param_name: name.to_string(),
                                        }),
                                        &mut actions.param_updates,
                                        &mut actions.modulation_actions,
                                        &format!("ch_fx_{}_{}", ch_copy, eff_idx_copy),
                                        Some(&midi_prefix),
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("fx_{}", eff_uuid),
                                        data.keyboard_learn_active,
                                        &mut actions.keyboard_learn_select,
                                        data.keyboard_learn_target.as_deref(),
                                    );
                                }
                            });
                            });
                        });
                    {
                        let card_rect = card_resp.response.rect;
                        let btn_size = egui::vec2(16.0, 16.0);
                        let btn_pos = egui::pos2(card_rect.right() - btn_size.x - 4.0, card_rect.top() + 4.0);
                        let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                        let btn_resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                        let color = if btn_resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                        ui.painter().text(btn_rect.center(), egui::Align2::CENTER_CENTER, "x", egui::FontId::proportional(12.0), color);
                        if btn_resp.clicked() {
                            actions.ch_effect_to_remove = Some((ch_idx, eff_idx));
                        }
                    }
                    render_effect_drag_ghost(
                        ui,
                        egui::Id::new(("eff_ghost_ch", ch_idx, eff_idx)),
                        EffectDrag::Channel(ch_idx, eff_idx),
                        eff_name,
                    );
                    ui.separator();
                }

                // Drop zone after last effect (for reordering)
                if !ch.effects.is_empty() {
                    let num_effects = ch.effects.len();
                    render_effect_drop_zone(ui, &format!("ch_{}", ch_idx), num_effects);
                }

                // Remaining space: always present drop target
                let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                    .map(|p| matches!(&*p, LibraryDrag::Effect(_))).unwrap_or(false);
                let remaining_w = ui.available_width().max(80.0);
                let remaining_h = ui.available_height().max(40.0);
                let stroke = if has_fx_drag { egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
                let fill = if has_fx_drag { egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20) } else { egui::Color32::TRANSPARENT };
                egui::Frame::default()
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(remaining_w - 16.0, remaining_h - 16.0));
                        ui.centered_and_justified(|ui| {
                            ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                        });
                    });
            }

            // Store channel effect chain rect for deferred library drops
            let chain_rect = ui.min_rect();
            let ch_chain_key = format!("ch_{}", ch_idx);
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("ch_fx_drop_rect").with(ch_idx), chain_rect);
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with(ch_chain_key), ch.effects.len() + 1);
            });
        });
    });
}
