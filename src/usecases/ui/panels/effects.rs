//! Master and channel effect detail panels.

use super::super::{EffectDrag, LibraryDrag, UIActions, UIData, widgets};
use super::clipboard_menu;
use super::utils::{
    channel_color, render_effect_drag_ghost, render_effect_drag_handle, render_effect_drop_zone,
};
use crate::engine::EngineCommand;
use crate::modulation::DEFAULT_ASSIGNMENT_AMOUNT;
use crate::params::ParamValue;

/// Copy, duplicate, paste, and remove, on any effect card in any chain.
///
/// `card` must be the response of a *scope* wrapping the card rather than the
/// card's own `Frame` response. A `Ui` registers itself before its contents, so
/// its rect loses hit-test ties to the checkboxes and sliders inside it; a
/// `Frame` registers after, so making that one sense clicks would swallow every
/// click meant for a parameter.
pub(super) fn effect_context_menu(
    card: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    uuid: &str,
    name: &str,
) {
    card.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{name} effect card"),
        )
    });
    card.context_menu(|ui| {
        clipboard_menu::items(
            ui,
            data,
            actions,
            &clipboard_menu::Subject::effect(uuid, name),
        );
        ui.separator();
        if ui.button("Remove effect").clicked() {
            actions.commands.push(EngineCommand::RemoveEffect {
                effect_uuid: uuid.to_string(),
            });
            ui.close();
        }
    });
}

pub(super) fn render_master_effect_detail(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.heading("🎬 Master Effects");

    egui::ScrollArea::horizontal()
        .id_salt("master_fx_hscroll")
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                {
                    for (eff_idx, (eff_uuid, eff_name, eff_enabled, eff_params)) in
                        data.master_effect_info.iter().enumerate()
                    {
                        let eff_uuid_master = eff_uuid.clone();
                        let eff_uuid_master_unassign = eff_uuid.clone();
                        let eff_uuid_master_remove = eff_uuid.clone();
                        let eff_uuid_master_automate = eff_uuid.clone();
                        render_effect_drop_zone(ui, "master", eff_idx);

                        // A scope, not the Frame, because a `Ui` registers itself
                        // before its contents and so loses hit-test ties to the
                        // parameter widgets inside the card.
                        let card = egui::UiBuilder::new().sense(egui::Sense::click());
                        let card_scope = ui.scope_builder(card, |ui| {
                            egui::Frame::default()
                            .inner_margin(6.0)
                            .corner_radius(4.0)
                            .fill(ui.visuals().faint_bg_color)
                            .show(ui, |ui| {
                                ui.set_min_width(180.0);
                                ui.set_max_width(250.0);
                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                    let max_h = (ui.available_height() - 8.0).max(100.0);
                                    egui::ScrollArea::vertical()
                                        .id_salt(format!("master_fx_scroll_{eff_idx}"))
                                        .max_height(max_h)
                                        .scroll_source(egui::scroll_area::ScrollSource {
                                            drag: egui::scroll_area::DragScroll::Never,
                                            scroll_bar: true,
                                            mouse_wheel: true,
                                        })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                // Drag handle
                                                render_effect_drag_handle(
                                                    ui,
                                                    EffectDrag::Master(eff_idx),
                                                );
                                                let mut enabled = *eff_enabled;
                                                if ui.checkbox(&mut enabled, "").changed() {
                                                    actions.commands.push(
                                                        EngineCommand::ToggleEffect {
                                                            effect_uuid: eff_uuid.clone(),
                                                        },
                                                    );
                                                }
                                                ui.label(egui::RichText::new(eff_name).strong());
                                            });

                                            if !eff_params.params.is_empty() {
                                                let eff_idx_copy = eff_idx;
                                                let eff_uuid_param = eff_uuid.clone();
                                                let midi_prefix =
                                                    format!("master/effect/{eff_uuid}");
                                                widgets::render_effect_params(
                                                    ui,
                                                    &eff_params.params,
                                                    &data.modulation_sources,
                                                    &|name: &str, val: ParamValue| {
                                                        EngineCommand::SetEffectParam {
                                                            effect_uuid: eff_uuid_param.clone(),
                                                            name: name.to_string(),
                                                            value: val,
                                                        }
                                                    },
                                                    Some(&|name: &str, source_uuid: &str| {
                                                        EngineCommand::AssignModulation {
                                                            target: format!(
                                                                "fx_{eff_uuid_master}:{name}"
                                                            ),
                                                            source_id: source_uuid.to_string(),
                                                            amount: DEFAULT_ASSIGNMENT_AMOUNT,
                                                        }
                                                    }),
                                                    Some(&|name: &str, source_uuid: &str| {
                                                        EngineCommand::ClearModulationSource {
                                                            target: format!(
                                                                "fx_{eff_uuid_master_unassign}:{name}"
                                                            ),
                                                            source_id: source_uuid.to_string(),
                                                        }
                                                    }),
                                                    Some(&|name: &str| {
                                                        EngineCommand::ClearModulation {
                                                            target: format!(
                                                                "fx_{eff_uuid_master_remove}:{name}"
                                                            ),
                                                        }
                                                    }),
                                                    Some(&|name: &str| {
                                                        EngineCommand::AddAutomationLane {
                                                            target: format!(
                                                                "fx_{eff_uuid_master_automate}:{name}"
                                                            ),
                                                            timebase:
                                                                crate::timebase::Timebase::Transport,
                                                        }
                                                    }),
                                                    &mut actions.commands,
                                                    &mut actions.session.gesture_active,
                                                    &format!("master_fx_{eff_idx_copy}"),
                                                    Some(&midi_prefix),
                                                    data.midi_learn_active,
                                                    &mut actions.session.midi_learn_select,
                                                    data.midi_learn_target.as_deref(),
                                                    &data.modulation_assignments,
                                                    &data.modulation_current_values,
                                                    &format!("fx_{eff_uuid}"),
                                                    data.keyboard_learn_active,
                                                    &mut actions.session.keyboard_learn_select,
                                                    data.keyboard_learn_target.as_deref(),
                                                );
                                            }
                                        });
                                });
                            })
                        });
                        let card_resp = card_scope.inner;
                        effect_context_menu(&card_scope.response, data, actions, eff_uuid, eff_name);
                        {
                            let card_rect = card_resp.response.rect;
                            let btn_size = egui::vec2(16.0, 16.0);
                            let btn_pos = egui::pos2(
                                card_rect.right() - btn_size.x - 4.0,
                                card_rect.top() + 4.0,
                            );
                            let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                            let btn_resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                            let color = if btn_resp.hovered() {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().text_color()
                            };
                            ui.painter().text(
                                btn_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "x",
                                egui::FontId::proportional(12.0),
                                color,
                            );
                            if btn_resp.clicked() {
                                actions.commands.push(EngineCommand::RemoveEffect {
                                    effect_uuid: eff_uuid.clone(),
                                });
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
                        .is_some_and(|p| matches!(&*p, LibraryDrag::Effect(_)));
                    let remaining_w = ui.available_width().max(80.0);
                    let remaining_h = ui.available_height().max(40.0);
                    let stroke = if has_fx_drag {
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 200, 255))
                    } else {
                        egui::Stroke::NONE
                    };
                    let fill = if has_fx_drag {
                        egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
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

                // Master effect chain takes deferred library drops
                let chain_rect = ui.min_rect();
                super::dnd::publish_master_surface_fx(ui.ctx(), chain_rect);
                ui.ctx().memory_mut(|mem| {
                    mem.data.insert_temp(
                        egui::Id::new("eff_dz_count").with("master".to_string()),
                        data.master_effect_info.len() + 1,
                    );
                });
            });
        });
}

/// Render channel effect chain detail in the bottom bar
pub(super) fn render_channel_effect_detail(
    ui: &mut egui::Ui,
    ch_idx: usize,
    data: &UIData,
    actions: &mut UIActions,
) {
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
            let mut name: String = ui
                .data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| ch.name.clone());
            let response = ui.text_edit_singleline(&mut name);
            if response.gained_focus() && !was_cleared {
                name.clear();
                ui.data_mut(|d| d.insert_temp(cleared_id, true));
            }
            if ui.small_button("✓ Save").clicked() && !name.is_empty() {
                actions.commands.push(EngineCommand::SaveChannelPreset {
                    channel_uuid: ch.uuid.clone(),
                    name: name.clone(),
                });
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

    egui::ScrollArea::horizontal()
        .id_salt("channel_fx_hscroll")
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Channel composite preview (first column before effect cards)
                if let Some(&tex_id) = data.channel_preview_textures.get(&ch_idx) {
                    let available_height = ui.available_height() - 12.0;
                    let preview = super::utils::preview_size(
                        egui::vec2(ui.available_width(), available_height.max(60.0)),
                        data.render_width,
                        data.render_height,
                    );
                    let preview_width = preview.x;
                    let preview_height = preview.y;
                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(preview_width + 12.0);
                            ui.set_max_width(preview_width + 12.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                ui.image(egui::load::SizedTexture::new(
                                    tex_id,
                                    egui::vec2(preview_width, preview_height),
                                ));
                                ui.label(egui::RichText::new(&ch.name).small().color(accent));
                            });
                        });
                    ui.separator();
                }
                let ch_chain_key = format!("ch_{}", ch.uuid);
                {
                    for (eff_idx, (eff_uuid, eff_name, eff_enabled, eff_params)) in
                        ch.effects.iter().enumerate()
                    {
                        let eff_uuid_ch_assign = eff_uuid.clone();
                        let eff_uuid_ch_unassign = eff_uuid.clone();
                        let eff_uuid_ch_remove = eff_uuid.clone();
                        let eff_uuid_ch_automate = eff_uuid.clone();
                        render_effect_drop_zone(ui, &ch_chain_key, eff_idx);

                        // A scope, not the Frame, because a `Ui` registers itself
                        // before its contents and so loses hit-test ties to the
                        // parameter widgets inside the card.
                        let card = egui::UiBuilder::new().sense(egui::Sense::click());
                        let card_scope = ui.scope_builder(card, |ui| {
                            egui::Frame::default()
                            .inner_margin(6.0)
                            .corner_radius(4.0)
                            .fill(ui.visuals().faint_bg_color)
                            .show(ui, |ui| {
                                ui.set_min_width(180.0);
                                ui.set_max_width(250.0);
                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                    let max_h = (ui.available_height() - 8.0).max(100.0);
                                    egui::ScrollArea::vertical()
                                        .id_salt(format!("ch_fx_scroll_{ch_idx}_{eff_idx}"))
                                        .max_height(max_h)
                                        .scroll_source(egui::scroll_area::ScrollSource {
                                            drag: egui::scroll_area::DragScroll::Never,
                                            scroll_bar: true,
                                            mouse_wheel: true,
                                        })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                render_effect_drag_handle(
                                                    ui,
                                                    EffectDrag::Channel(ch.uuid.clone(), eff_idx),
                                                );
                                                let mut enabled = *eff_enabled;
                                                if ui.checkbox(&mut enabled, "").changed() {
                                                    actions.commands.push(
                                                        EngineCommand::ToggleEffect {
                                                            effect_uuid: eff_uuid.clone(),
                                                        },
                                                    );
                                                }
                                                ui.label(
                                                    egui::RichText::new(eff_name)
                                                        .strong()
                                                        .color(accent),
                                                );
                                            });

                                            if !eff_params.params.is_empty() {
                                                let ch_copy = ch_idx;
                                                let eff_idx_copy = eff_idx;
                                                let ch_uuid = ch.uuid.clone();
                                                let eff_uuid_param = eff_uuid.clone();
                                                let midi_prefix =
                                                    format!("ch/{ch_uuid}/effect/{eff_uuid}");
                                                widgets::render_effect_params(
                                                    ui,
                                                    &eff_params.params,
                                                    &data.modulation_sources,
                                                    &|name: &str, val: ParamValue| {
                                                        EngineCommand::SetEffectParam {
                                                            effect_uuid: eff_uuid_param.clone(),
                                                            name: name.to_string(),
                                                            value: val,
                                                        }
                                                    },
                                                    Some(&|name: &str, source_uuid: &str| {
                                                        EngineCommand::AssignModulation {
                                                            target: format!(
                                                                "fx_{eff_uuid_ch_assign}:{name}"
                                                            ),
                                                            source_id: source_uuid.to_string(),
                                                            amount: DEFAULT_ASSIGNMENT_AMOUNT,
                                                        }
                                                    }),
                                                    Some(&|name: &str, source_uuid: &str| {
                                                        EngineCommand::ClearModulationSource {
                                                            target: format!(
                                                                "fx_{eff_uuid_ch_unassign}:{name}"
                                                            ),
                                                            source_id: source_uuid.to_string(),
                                                        }
                                                    }),
                                                    Some(&|name: &str| {
                                                        EngineCommand::ClearModulation {
                                                            target: format!(
                                                                "fx_{eff_uuid_ch_remove}:{name}"
                                                            ),
                                                        }
                                                    }),
                                                    Some(&|name: &str| {
                                                        EngineCommand::AddAutomationLane {
                                                            target: format!(
                                                                "fx_{eff_uuid_ch_automate}:{name}"
                                                            ),
                                                            timebase:
                                                                crate::timebase::Timebase::Transport,
                                                        }
                                                    }),
                                                    &mut actions.commands,
                                                    &mut actions.session.gesture_active,
                                                    &format!("ch_fx_{ch_copy}_{eff_idx_copy}"),
                                                    Some(&midi_prefix),
                                                    data.midi_learn_active,
                                                    &mut actions.session.midi_learn_select,
                                                    data.midi_learn_target.as_deref(),
                                                    &data.modulation_assignments,
                                                    &data.modulation_current_values,
                                                    &format!("fx_{eff_uuid}"),
                                                    data.keyboard_learn_active,
                                                    &mut actions.session.keyboard_learn_select,
                                                    data.keyboard_learn_target.as_deref(),
                                                );
                                            }
                                        });
                                });
                            })
                        });
                        let card_resp = card_scope.inner;
                        effect_context_menu(&card_scope.response, data, actions, eff_uuid, eff_name);
                        {
                            let card_rect = card_resp.response.rect;
                            let btn_size = egui::vec2(16.0, 16.0);
                            let btn_pos = egui::pos2(
                                card_rect.right() - btn_size.x - 4.0,
                                card_rect.top() + 4.0,
                            );
                            let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                            let btn_resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                            let color = if btn_resp.hovered() {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().text_color()
                            };
                            ui.painter().text(
                                btn_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "x",
                                egui::FontId::proportional(12.0),
                                color,
                            );
                            if btn_resp.clicked() {
                                actions.commands.push(EngineCommand::RemoveEffect {
                                    effect_uuid: eff_uuid.clone(),
                                });
                            }
                        }
                        render_effect_drag_ghost(
                            ui,
                            egui::Id::new(("eff_ghost_ch", ch_idx, eff_idx)),
                            EffectDrag::Channel(ch.uuid.clone(), eff_idx),
                            eff_name,
                        );
                        ui.separator();
                    }

                    // Drop zone after last effect (for reordering)
                    if !ch.effects.is_empty() {
                        let num_effects = ch.effects.len();
                        render_effect_drop_zone(ui, &ch_chain_key, num_effects);
                    }

                    // Remaining space: always present drop target
                    let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                        .is_some_and(|p| matches!(&*p, LibraryDrag::Effect(_)));
                    let remaining_w = ui.available_width().max(80.0);
                    let remaining_h = ui.available_height().max(40.0);
                    let stroke = if has_fx_drag {
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 200, 255))
                    } else {
                        egui::Stroke::NONE
                    };
                    let fill = if has_fx_drag {
                        egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
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

                // Channel effect chain takes deferred library drops
                let chain_rect = ui.min_rect();
                super::dnd::publish_channel_surface_fx(ui.ctx(), &ch.uuid, ch_idx, chain_rect);
                ui.ctx().memory_mut(|mem| {
                    mem.data.insert_temp(
                        egui::Id::new("eff_dz_count").with(ch_chain_key),
                        ch.effects.len() + 1,
                    );
                });
            });
        });
}
