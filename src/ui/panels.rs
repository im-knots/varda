use crate::mixer::CrossfadeEasing;
use crate::params::ParamValue;
use crate::modulation::{LFOWaveform, StepInterpolation};
use crate::{BlendMode, ScalingMode};
use super::{UIData, UIActions, ParamUpdate, ModulationAction, CrossfaderAction, ModSourceUI, NotificationUI, modulator_color};
use super::widgets;

/// Render the complete UI and return all collected actions/intents.
pub fn render_ui(ctx: &egui::Context, data: &UIData) -> UIActions {
    let mut actions = UIActions::new();

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
            render_bottom_panel(ui, data, &mut actions);
        });

    // === CENTRAL AREA: Decks as columns ===
    egui::CentralPanel::default().show(ctx, |ui| {
        render_central_panel(ui, data, &mut actions);
    });

    // === NOTIFICATION OVERLAY (rendered last, on top of everything) ===
    render_notifications(ctx, &data.notifications, &mut actions);

    actions
}

/// Render toast notifications as an overlay in the top-right corner
fn render_notifications(ctx: &egui::Context, notifications: &[NotificationUI], actions: &mut UIActions) {
    if notifications.is_empty() {
        return;
    }

    let screen_rect = ctx.content_rect();
    let toast_width = 360.0;
    let toast_height = 48.0;
    let margin = 12.0;
    let spacing = 6.0;

    for (i, notif) in notifications.iter().enumerate() {
        let x = screen_rect.right() - toast_width - margin;
        let y = screen_rect.top() + margin + (toast_height + spacing) * i as f32;

        let toast_rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(toast_width, toast_height),
        );

        let layer_id = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(format!("notif_{}", i)));
        let painter = ctx.layer_painter(layer_id);

        // Background
        painter.rect_filled(toast_rect, 6.0, egui::Color32::from_rgba_unmultiplied(20, 20, 30, 230));

        // Left accent bar
        let accent_color = notif.level.color();
        let bar_rect = egui::Rect::from_min_size(
            toast_rect.min,
            egui::vec2(4.0, toast_height),
        );
        painter.rect_filled(bar_rect, egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, accent_color);

        // Level label
        painter.text(
            egui::pos2(toast_rect.left() + 12.0, toast_rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            notif.level.label(),
            egui::FontId::proportional(10.0),
            accent_color,
        );

        // Message text (truncated)
        let max_msg_width = toast_width - 50.0;
        let msg = if notif.message.len() > 60 {
            format!("{}…", &notif.message[..59])
        } else {
            notif.message.clone()
        };
        painter.text(
            egui::pos2(toast_rect.left() + 12.0, toast_rect.top() + 22.0),
            egui::Align2::LEFT_TOP,
            &msg,
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(220),
        );

        // Progress bar (fade out indicator)
        let progress_width = toast_width * (1.0 - notif.progress);
        let progress_rect = egui::Rect::from_min_size(
            egui::pos2(toast_rect.left(), toast_rect.bottom() - 2.0),
            egui::vec2(progress_width, 2.0),
        );
        painter.rect_filled(progress_rect, 0.0, accent_color.linear_multiply(0.5));

        // Dismiss button ("✕") — use an Area so it's clickable
        let dismiss_id = egui::Id::new(format!("dismiss_notif_{}", i));
        egui::Area::new(dismiss_id)
            .fixed_pos(egui::pos2(toast_rect.right() - 24.0, toast_rect.top() + 4.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if ui.add(egui::Button::new(egui::RichText::new("✕").size(12.0).color(egui::Color32::GRAY))
                    .frame(false)
                ).clicked() {
                    actions.notifications_to_dismiss.push(i);
                }
            });
    }

    // Request repaint to animate progress bars
    ctx.request_repaint();
}

/// Render the right side panel (main output + master effects only)
fn render_right_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Clickable heading — selects master for bottom bar
        let heading_response = ui.add(
            egui::Label::new(egui::RichText::new("🎬 Main Output").heading())
                .sense(egui::Sense::click()),
        );
        if heading_response.clicked() {
            actions.select_master = true;
        }

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
        ui.heading("🔮 Master Effects");
        ui.label("(Apply to final composite)");

        ui.add_space(10.0);
        ui.separator();

        // Modulation sources (moved from bottom bar)
        render_modulation_section(ui, data, actions);

        ui.add_space(10.0);
        ui.separator();

        // Shader browser (moved from bottom bar)
        render_shader_browser(ui, data, actions);
    });
}

/// Render the bottom panel (crossfader strip + audio, modulation, shader browser)
fn render_bottom_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // MIDI learn status indicator
    if data.midi_learn_active {
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(4.0)
            .fill(egui::Color32::from_rgb(180, 80, 220))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🎹 MIDI LEARN — Move a control to map it")
                        .strong().color(egui::Color32::WHITE));
                    if let Some(target) = &data.midi_learn_target {
                        ui.label(egui::RichText::new(format!("→ {}", target))
                            .color(egui::Color32::from_rgb(255, 255, 200)));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕ Cancel").clicked() {
                            actions.midi_learn_cancel = true;
                        }
                    });
                });
            });
    }

    // Context-sensitive bottom bar: master effects, channel effects, or deck detail
    if data.selected_master {
        render_master_effect_detail(ui, data, actions);
    } else if let Some(ch_idx) = data.selected_channel {
        render_channel_effect_detail(ui, ch_idx, data, actions);
    } else {
        render_selected_deck_detail(ui, data, actions);
    }
}

/// Render the selected deck's full details (params, effects, blend, scaling) in the bottom bar
fn render_selected_deck_detail(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎛 Selected Deck");

    let Some((ch_idx, deck_idx)) = data.selected_deck else {
        ui.label(egui::RichText::new("Click a deck thumbnail to see its controls here").weak().small());
        return;
    };

    // Find the deck data
    let Some(ch) = data.channels.get(ch_idx) else {
        ui.label(egui::RichText::new("Channel not found").weak());
        return;
    };
    let Some(deck) = ch.decks.iter().find(|d| d.deck_idx == deck_idx) else {
        ui.label(egui::RichText::new("Deck not found").weak());
        return;
    };

    let accent = channel_color(ch_idx);
    ui.label(egui::RichText::new(format!("Ch {} / Deck {} — {}", ch.name, deck_idx + 1, deck.name))
        .strong().color(accent));

    // Horizontal columns: Preview | Generator | Effect 1 | Effect 2 | ... | Add Effect
    egui::ScrollArea::horizontal().id_salt("selected_deck_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            // Column 0: Deck preview — scales with bottom bar height
            if let Some(tex_id) = data.deck_preview_textures.get(&(ch_idx, deck_idx)) {
                let available_height = ui.available_height() - 12.0; // margin
                let preview_height = available_height.max(60.0);
                let preview_width = preview_height * 16.0 / 9.0;
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(preview_width + 12.0);
                        ui.set_max_width(preview_width + 12.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.image(egui::load::SizedTexture::new(*tex_id, egui::vec2(preview_width, preview_height)));
                            ui.label(egui::RichText::new(&deck.name).small().color(accent));
                        });
                    });
                ui.separator();
            }

            // Column 1: Generator parameters + blend/scale
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(ui.visuals().faint_bg_color)
                .show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.set_max_width(280.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    egui::ScrollArea::vertical().id_salt("deck_gen_scroll").show(ui, |ui| {
                        // Blend mode
                        let blend_modes = ["Norm", "Add", "Mult", "Scrn", "Ovly", "Diff"];
                        let current_blend = match deck.blend_mode {
                            BlendMode::Normal => 0, BlendMode::Add => 1, BlendMode::Multiply => 2,
                            BlendMode::Screen => 3, BlendMode::Overlay => 4, BlendMode::Difference => 5,
                        };
                        let mut selected = current_blend;
                        ui.horizontal(|ui| {
                            ui.label("Blend:");
                            egui::ComboBox::from_id_salt("sel_deck_blend")
                                .selected_text(blend_modes[selected])
                                .width(60.0)
                                .show_ui(ui, |ui| {
                                    for (i, mode_name) in blend_modes.iter().enumerate() {
                                        ui.selectable_value(&mut selected, i, *mode_name);
                                    }
                                });
                        });
                        if selected != current_blend {
                            let new_blend = match selected {
                                1 => BlendMode::Add, 2 => BlendMode::Multiply, 3 => BlendMode::Screen,
                                4 => BlendMode::Overlay, 5 => BlendMode::Difference, _ => BlendMode::Normal,
                            };
                            actions.deck_updates.push((ch_idx, deck_idx, deck.opacity, new_blend, deck.solo, deck.mute));
                        }

                        // Scaling mode
                        if let Some(current_scaling) = deck.scaling_mode {
                            let scaling_modes = ["Fill", "Fit", "Stretch", "Center"];
                            let current_idx = match current_scaling {
                                ScalingMode::Fill => 0, ScalingMode::Fit => 1,
                                ScalingMode::Stretch => 2, ScalingMode::Center => 3,
                            };
                            let mut selected_scaling = current_idx;
                            ui.horizontal(|ui| {
                                ui.label("Scale:");
                                egui::ComboBox::from_id_salt("sel_deck_scale")
                                    .selected_text(scaling_modes[selected_scaling])
                                    .width(60.0)
                                    .show_ui(ui, |ui| {
                                        for (i, mode_name) in scaling_modes.iter().enumerate() {
                                            ui.selectable_value(&mut selected_scaling, i, *mode_name);
                                        }
                                    });
                            });
                            if selected_scaling != current_idx {
                                let new_scaling = match selected_scaling {
                                    1 => ScalingMode::Fit, 2 => ScalingMode::Stretch,
                                    3 => ScalingMode::Center, _ => ScalingMode::Fill,
                                };
                                actions.scaling_mode_updates.push((ch_idx, deck_idx, new_scaling));
                            }
                        }

                        // Generator parameters
                        let gen_params = &deck.generator;
                        if !gen_params.params.is_empty() {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("🎨 {}", gen_params.shader_name)).strong());
                            let midi_path_prefix = format!("ch/{}/deck/{}", ch_idx, deck_idx);
                            widgets::render_params(
                                ui,
                                &gen_params.params,
                                &data.modulation_sources,
                                &|name: &str, val: ParamValue| match val {
                                    ParamValue::Float(v) => ParamUpdate::GeneratorFloat { ch_idx, deck_idx, name: name.to_string(), value: v },
                                    ParamValue::Bool(v) => ParamUpdate::GeneratorBool { ch_idx, deck_idx, name: name.to_string(), value: v },
                                    ParamValue::Color(v) => ParamUpdate::GeneratorColor { ch_idx, deck_idx, name: name.to_string(), value: v },
                                    _ => unreachable!(),
                                },
                                Some(&|name: &str, src_idx: usize| ModulationAction::AssignModulation {
                                    ch_idx, deck_idx, param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                }),
                                Some(&|name: &str| ModulationAction::RemoveAssignment {
                                    ch_idx, deck_idx, param_name: name.to_string(), source_idx: 0,
                                }),
                                &mut actions.param_updates,
                                &mut actions.modulation_actions,
                                &format!("sel_{}_{}", ch_idx, deck_idx),
                                Some(&midi_path_prefix),
                                &mut actions.midi_learn_start,
                                &data.modulation_assignments,
                                &data.modulation_current_values,
                                &format!("ch{}_deck{}", ch_idx, deck_idx),
                            );
                            ui.add_space(4.0);
                            if ui.button("🔄 Reset").clicked() {
                                actions.param_updates.push(ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx });
                            }
                        }
                    });
                    });
                });

            ui.separator();

            // One column per effect
            for (eff_idx, (eff_name, eff_enabled, eff_params)) in deck.effects.iter().enumerate() {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(180.0);
                        ui.set_max_width(250.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        egui::ScrollArea::vertical().id_salt(format!("deck_fx_scroll_{}_{}_{}",ch_idx,deck_idx,eff_idx)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let mut enabled = *eff_enabled;
                                if ui.checkbox(&mut enabled, "").changed() {
                                    actions.effect_to_toggle = Some((ch_idx, deck_idx, eff_idx));
                                }
                                ui.label(egui::RichText::new(format!("🔮 {}", eff_name)).strong());
                                if ui.small_button("✕").clicked() {
                                    actions.effect_to_remove = Some((ch_idx, deck_idx, eff_idx));
                                }
                            });

                            if !eff_params.params.is_empty() {
                                let ch_copy = ch_idx;
                                let deck_copy = deck_idx;
                                let eff_idx_copy = eff_idx;
                                let eff_midi_prefix = format!("ch/{}/deck/{}/effect/{}", ch_copy, deck_copy, eff_idx_copy);
                                widgets::render_effect_params(
                                    ui,
                                    &eff_params.params,
                                    &data.modulation_sources,
                                    &|name: &str, val: ParamValue| match val {
                                        ParamValue::Float(v) => ParamUpdate::EffectFloat { ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                        ParamValue::Bool(v) => ParamUpdate::EffectBool { ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                        ParamValue::Color(v) => ParamUpdate::EffectColor { ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                        _ => unreachable!(),
                                    },
                                    Some(&|name: &str, src_idx: usize| ModulationAction::AssignEffectModulation {
                                        ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy,
                                        param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                    }),
                                    Some(&|name: &str| ModulationAction::RemoveEffectAssignment {
                                        ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy,
                                        param_name: name.to_string(),
                                    }),
                                    &mut actions.param_updates,
                                    &mut actions.modulation_actions,
                                    &format!("fx_{}_{}_{}", ch_copy, deck_copy, eff_idx_copy),
                                    Some(&eff_midi_prefix),
                                    &mut actions.midi_learn_start,
                                    &data.modulation_assignments,
                                    &data.modulation_current_values,
                                    &format!("ch{}_deck{}_fx{}", ch_copy, deck_copy, eff_idx_copy),
                                );
                            }
                        });
                        });
                    });
                ui.separator();
            }

            // Add effect column
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(ui.visuals().faint_bg_color)
                .show(ui, |ui| {
                    ui.set_min_width(100.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.label(egui::RichText::new("➕ Add Effect").strong());
                    for (filter_name, filter_idx) in &data.filters {
                        if ui.button(filter_name).clicked() {
                            actions.effect_to_add = Some((ch_idx, deck_idx, *filter_idx));
                        }
                    }
                    });
                });
        });
    });
}

/// Render master effect chain detail in the bottom bar (when master is selected)
fn render_master_effect_detail(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎬 Master Effects");

    // Horizontal columns: one per effect + add column
    egui::ScrollArea::horizontal().id_salt("master_fx_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (eff_idx, (eff_name, eff_enabled, eff_params)) in data.master_effect_info.iter().enumerate() {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(180.0);
                        ui.set_max_width(250.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        egui::ScrollArea::vertical().id_salt(format!("master_fx_scroll_{}", eff_idx)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let mut enabled = *eff_enabled;
                                if ui.checkbox(&mut enabled, "").changed() {
                                    actions.master_effect_to_toggle = Some(eff_idx);
                                }
                                ui.label(egui::RichText::new(format!("🔮 {}", eff_name)).strong());
                                if ui.small_button("✕").clicked() {
                                    actions.master_effect_to_remove = Some(eff_idx);
                                }
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
                                    Some(&|name: &str, src_idx: usize| ModulationAction::AssignMasterEffectModulation {
                                        effect_idx: eff_idx_copy,
                                        param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                    }),
                                    Some(&|name: &str| ModulationAction::RemoveMasterEffectAssignment {
                                        effect_idx: eff_idx_copy,
                                        param_name: name.to_string(),
                                    }),
                                    &mut actions.param_updates,
                                    &mut actions.modulation_actions,
                                    &format!("master_fx_{}", eff_idx_copy),
                                    Some(&midi_prefix),
                                    &mut actions.midi_learn_start,
                                    &data.modulation_assignments,
                                    &data.modulation_current_values,
                                    &format!("master_fx{}", eff_idx_copy),
                                );
                            }
                        });
                        });
                    });
                ui.separator();
            }

            // Add effect column
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(ui.visuals().faint_bg_color)
                .show(ui, |ui| {
                    ui.set_min_width(100.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.label(egui::RichText::new("➕ Add Effect").strong());
                    for (filter_name, filter_idx) in &data.filters {
                        if ui.button(filter_name).clicked() {
                            actions.master_effect_to_add = Some(*filter_idx);
                        }
                    }
                    });
                });
        });
    });
}


/// Render channel effect chain detail in the bottom bar
fn render_channel_effect_detail(ui: &mut egui::Ui, ch_idx: usize, data: &UIData, actions: &mut UIActions) {
    let Some(ch) = data.channels.get(ch_idx) else {
        ui.label(egui::RichText::new("Channel not found").weak());
        return;
    };

    let accent = channel_color(ch_idx);
    ui.heading(egui::RichText::new(format!("🔮 Channel {} Effects", ch.name)).color(accent));

    // Horizontal columns: one per effect + add column
    egui::ScrollArea::horizontal().id_salt("channel_fx_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (eff_idx, (eff_name, eff_enabled, eff_params)) in ch.effects.iter().enumerate() {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(180.0);
                        ui.set_max_width(250.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        egui::ScrollArea::vertical().id_salt(format!("ch_fx_scroll_{}_{}", ch_idx, eff_idx)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let mut enabled = *eff_enabled;
                                if ui.checkbox(&mut enabled, "").changed() {
                                    actions.ch_effect_to_toggle = Some((ch_idx, eff_idx));
                                }
                                ui.label(egui::RichText::new(format!("🔮 {}", eff_name)).strong().color(accent));
                                if ui.small_button("✕").clicked() {
                                    actions.ch_effect_to_remove = Some((ch_idx, eff_idx));
                                }
                            });

                            if !eff_params.params.is_empty() {
                                let ch_copy = ch_idx;
                                let eff_idx_copy = eff_idx;
                                let midi_prefix = format!("ch/{}/effect/{}", ch_copy, eff_idx_copy);
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
                                    Some(&|name: &str, src_idx: usize| ModulationAction::AssignChannelEffectModulation {
                                        ch_idx: ch_copy, effect_idx: eff_idx_copy,
                                        param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                    }),
                                    Some(&|name: &str| ModulationAction::RemoveChannelEffectAssignment {
                                        ch_idx: ch_copy, effect_idx: eff_idx_copy,
                                        param_name: name.to_string(),
                                    }),
                                    &mut actions.param_updates,
                                    &mut actions.modulation_actions,
                                    &format!("ch_fx_{}_{}", ch_copy, eff_idx_copy),
                                    Some(&midi_prefix),
                                    &mut actions.midi_learn_start,
                                    &data.modulation_assignments,
                                    &data.modulation_current_values,
                                    &format!("ch{}_fx{}", ch_copy, eff_idx_copy),
                                );
                            }
                        });
                        });
                    });
                ui.separator();
            }

            // Add effect column
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(ui.visuals().faint_bg_color)
                .show(ui, |ui| {
                    ui.set_min_width(100.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.label(egui::RichText::new("➕ Add Effect").strong());
                    for (filter_name, filter_idx) in &data.filters {
                        if ui.button(filter_name).clicked() {
                            actions.ch_effect_to_add = Some((ch_idx, *filter_idx));
                        }
                    }
                    });
                });
        });
    });
}

/// Render modulation sources section
fn render_modulation_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎛 Modulation");

    // Audio levels (integrated into modulation)
    ui.collapsing("🎵 Audio Input", |ui| {
        widgets::render_audio_levels(ui, &data.audio);
    });
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        if ui.button("➕ LFO").clicked() {
            actions.modulation_actions.push(ModulationAction::AddLFO {
                waveform: LFOWaveform::Sine,
                frequency: 1.0,
            });
        }
        if ui.button("➕ Audio").clicked() {
            actions.modulation_actions.push(ModulationAction::AddAudioBand {
                band: crate::modulation::AudioBand::Bass,
            });
        }
        if ui.button("➕ ADSR").clicked() {
            actions.modulation_actions.push(ModulationAction::AddADSR {
                attack: 0.1, decay: 0.3, sustain: 0.7, release: 0.5,
            });
        }
        if ui.button("➕ StepSeq").clicked() {
            actions.modulation_actions.push(ModulationAction::AddStepSequencer {
                num_steps: 8, rate: 2.0,
            });
        }
    });


    if data.modulation_sources.is_empty() {
        ui.label(egui::RichText::new("No modulation sources").small().weak());
    } else {
        egui::ScrollArea::horizontal().id_salt("mod_sources_hscroll").max_height(220.0).show(ui, |ui| {
            ui.horizontal_top(|ui| {
            for (idx, src) in data.modulation_sources.iter().enumerate() {
                let mod_color = modulator_color(idx);
                let dim_color = egui::Color32::from_rgba_premultiplied(
                    mod_color.r() / 4, mod_color.g() / 4, mod_color.b() / 4, 40
                );
                egui::Frame::default()
                    .inner_margin(4.0)
                    .corner_radius(4.0)
                    .fill(dim_color)
                    .stroke(egui::Stroke::new(1.0, mod_color))
                    .show(ui, |ui| {
                        ui.set_min_width(140.0);
                        ui.set_max_width(190.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        match src {
                            ModSourceUI::LFO { waveform, frequency, phase, amplitude, bipolar } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("LFO {}", idx + 1)).strong().color(mod_color));
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                let waveforms = ["Sine", "Square", "Triangle", "Saw", "Random"];
                                let current_wf = match waveform {
                                    LFOWaveform::Sine => 0,
                                    LFOWaveform::Square => 1,
                                    LFOWaveform::Triangle => 2,
                                    LFOWaveform::Sawtooth => 3,
                                    LFOWaveform::Random => 4,
                                };
                                let mut selected_wf = current_wf;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Wave:").small());
                                    egui::ComboBox::from_id_salt(format!("wf_{}", idx))
                                        .selected_text(waveforms[selected_wf])
                                        .width(70.0)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in waveforms.iter().enumerate() {
                                                if ui.selectable_value(&mut selected_wf, i, *name).changed() {
                                                    let new_wf = match i {
                                                        0 => LFOWaveform::Sine,
                                                        1 => LFOWaveform::Square,
                                                        2 => LFOWaveform::Triangle,
                                                        3 => LFOWaveform::Sawtooth,
                                                        _ => LFOWaveform::Random,
                                                    };
                                                    actions.modulation_actions.push(ModulationAction::UpdateLFOWaveform { idx, waveform: new_wf });
                                                }
                                            }
                                        });
                                });
                                let mut freq = *frequency;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Freq:").small());
                                    if ui.add(egui::Slider::new(&mut freq, 0.01..=10.0).logarithmic(true).show_value(true).suffix("Hz")).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOFrequency { idx, frequency: freq });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "frequency");
                                });
                                let mut amp = *amplitude;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Amp:").small());
                                    if ui.add(egui::Slider::new(&mut amp, 0.0..=1.0).show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOAmplitude { idx, amplitude: amp });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "amplitude");
                                });
                                let mut ph = *phase;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Phase:").small());
                                    if ui.add(egui::Slider::new(&mut ph, 0.0..=1.0).show_value(false)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOPhase { idx, phase: ph });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "phase");
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar (-1 to 1)").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateLFOBipolar { idx, bipolar: bp });
                                }
                                // LFO waveform visualization
                                let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(180.0), 30.0), egui::Sense::hover());
                                let rect = response.rect;
                                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                                let n_points = 60;
                                let points: Vec<egui::Pos2> = (0..=n_points).map(|i| {
                                    let t = i as f32 / n_points as f32;
                                    let raw = match waveform {
                                        LFOWaveform::Sine => (t * std::f32::consts::TAU).sin(),
                                        LFOWaveform::Square => if t < 0.5 { 1.0 } else { -1.0 },
                                        LFOWaveform::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                                        LFOWaveform::Sawtooth => 2.0 * t - 1.0,
                                        LFOWaveform::Random => {
                                            let step = (t * 8.0).floor();
                                            let seed = step as u32;
                                            let hash = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                                            (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
                                        }
                                    };
                                    let y = rect.center().y - raw * *amplitude * rect.height() * 0.4;
                                    egui::pos2(rect.left() + t * rect.width(), y)
                                }).collect();
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    let y = rect.center().y - cur_val * rect.height() * 0.4;
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::Audio { band, smoothing } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("Audio {}", idx + 1)).strong().color(mod_color));
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                let band_name = match band {
                                    crate::modulation::AudioBand::Level => "Level",
                                    crate::modulation::AudioBand::Bass => "Bass",
                                    crate::modulation::AudioBand::Mid => "Mid",
                                    crate::modulation::AudioBand::Treble => "Treble",
                                };
                                ui.label(egui::RichText::new(format!("Band: {}", band_name)).small());
                                let mut sm = *smoothing;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Smooth:").small());
                                    if ui.add(egui::Slider::new(&mut sm, 0.0..=0.99).show_value(false)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioSmoothing { idx, smoothing: sm });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "smoothing");
                                });
                                // Audio level bar
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    ui.add(egui::ProgressBar::new(cur_val).desired_width(140.0).fill(mod_color));
                                }
                            }
                            ModSourceUI::ADSR { attack, decay, sustain, release, stage } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("ADSR {}", idx + 1)).strong().color(mod_color));
                                    let stage_text = match stage {
                                        crate::modulation::ADSRStage::Idle => "⏹",
                                        crate::modulation::ADSRStage::Attack => "▲",
                                        crate::modulation::ADSRStage::Decay => "▼",
                                        crate::modulation::ADSRStage::Sustain => "━",
                                        crate::modulation::ADSRStage::Release => "↘",
                                    };
                                    ui.label(egui::RichText::new(stage_text).small());
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                ui.horizontal(|ui| {
                                    if ui.button(egui::RichText::new("▶ Gate").small()).clicked() {
                                        actions.modulation_actions.push(ModulationAction::TriggerADSR { idx });
                                    }
                                    if ui.button(egui::RichText::new("⏹ Release").small()).clicked() {
                                        actions.modulation_actions.push(ModulationAction::ReleaseADSR { idx });
                                    }
                                });
                                let mut a = *attack;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("A:").small());
                                    if ui.add(egui::Slider::new(&mut a, 0.001..=5.0).logarithmic(true).suffix("s").show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRAttack { idx, attack: a });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "attack");
                                });
                                let mut d = *decay;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("D:").small());
                                    if ui.add(egui::Slider::new(&mut d, 0.001..=5.0).logarithmic(true).suffix("s").show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRDecay { idx, decay: d });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "decay");
                                });
                                let mut s = *sustain;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("S:").small());
                                    if ui.add(egui::Slider::new(&mut s, 0.0..=1.0).show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRSustain { idx, sustain: s });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "sustain");
                                });
                                let mut r = *release;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("R:").small());
                                    if ui.add(egui::Slider::new(&mut r, 0.001..=5.0).logarithmic(true).suffix("s").show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRRelease { idx, release: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "release");
                                });
                                // ADSR envelope visualization
                                let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(180.0), 30.0), egui::Sense::hover());
                                let rect = response.rect;
                                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                                let total_time = a + d + 0.3 + r; // sustain shown as ~0.3 segment
                                let ax = rect.left() + (a / total_time) * rect.width();
                                let dx = ax + (d / total_time) * rect.width();
                                let sx = dx + (0.3 / total_time) * rect.width();
                                let rx = sx + (r / total_time) * rect.width();
                                let top = rect.top() + 2.0;
                                let bot = rect.bottom() - 2.0;
                                let sus_y = top + (1.0 - s) * (bot - top);
                                let points = vec![
                                    egui::pos2(rect.left(), bot),  // start at 0
                                    egui::pos2(ax, top),           // attack peak
                                    egui::pos2(dx, sus_y),         // decay to sustain
                                    egui::pos2(sx, sus_y),         // sustain hold
                                    egui::pos2(rx, bot),           // release to 0
                                ];
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    let y = bot - cur_val * (bot - top);
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::StepSequencer { steps, rate, interpolation, bipolar } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("StepSeq {}", idx + 1)).strong().color(mod_color));
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                let mut r = *rate;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Rate:").small());
                                    if ui.add(egui::Slider::new(&mut r, 0.1..=20.0).logarithmic(true).suffix("Hz").show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateStepRate { idx, rate: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "rate");
                                });
                                // Interpolation mode
                                let interp_names = ["None", "Linear", "Smooth"];
                                let current_interp = match interpolation {
                                    StepInterpolation::None => 0,
                                    StepInterpolation::Linear => 1,
                                    StepInterpolation::Smooth => 2,
                                };
                                let mut selected_interp = current_interp;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Interp:").small());
                                    egui::ComboBox::from_id_salt(format!("step_interp_{}", idx))
                                        .selected_text(interp_names[selected_interp])
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in interp_names.iter().enumerate() {
                                                if ui.selectable_value(&mut selected_interp, i, *name).changed() {
                                                    let new_interp = match i {
                                                        0 => StepInterpolation::None,
                                                        1 => StepInterpolation::Linear,
                                                        _ => StepInterpolation::Smooth,
                                                    };
                                                    actions.modulation_actions.push(ModulationAction::UpdateStepInterpolation { idx, interpolation: new_interp });
                                                }
                                            }
                                        });
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateStepBipolar { idx, bipolar: bp });
                                }
                                // Step value sliders (compact)
                                ui.horizontal_wrapped(|ui| {
                                    for (step_idx, step_val) in steps.iter().enumerate() {
                                        let mut val = *step_val;
                                        let slider = egui::Slider::new(&mut val, 0.0..=1.0)
                                            .vertical()
                                            .show_value(false);
                                        if ui.add_sized([12.0, 30.0], slider).on_hover_text(format!("Step {}", step_idx + 1)).changed() {
                                            actions.modulation_actions.push(ModulationAction::UpdateStepValue { idx, step_idx, value: val });
                                        }
                                    }
                                });
                            }
                        }
                        });
                    });
                ui.separator();
            }
            });
        });
    }
}

/// Render a mod-on-mod assignment dropdown for a modulator's parameter.
/// `target_idx` is the modulator whose parameter is being targeted.
/// `param_name` is the parameter name (e.g., "frequency", "amplitude", "phase").
/// Shows a 🎛 combo listing all other modulators that can modulate this parameter.
fn render_mod_on_mod_dropdown(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    target_idx: usize,
    param_name: &str,
) {
    let key = format!("mod:{}:{}", target_idx, param_name);
    let has_assignment = data.modulation_assignments.get(&key).map_or(false, |v| !v.is_empty());
    let btn_text = if has_assignment { "🎛" } else { "🎛" };
    let btn_color = if has_assignment {
        modulator_color(data.modulation_assignments.get(&key)
            .and_then(|v| v.first())
            .map(|a| a.source_idx)
            .unwrap_or(0))
    } else {
        egui::Color32::GRAY
    };

    egui::ComboBox::from_id_salt(format!("mom_{}_{}", target_idx, param_name))
        .selected_text(egui::RichText::new(btn_text).color(btn_color).small())
        .width(30.0)
        .show_ui(ui, |ui| {
            ui.label(egui::RichText::new(format!("Modulate {}", param_name)).small().strong());
            for (src_idx, src) in data.modulation_sources.iter().enumerate() {
                if src_idx == target_idx { continue; } // can't modulate yourself
                let color = modulator_color(src_idx);
                let src_name = match src {
                    ModSourceUI::LFO { .. } => format!("LFO {}", src_idx + 1),
                    ModSourceUI::Audio { band, .. } => format!("Audio {:?}", band),
                    ModSourceUI::ADSR { .. } => format!("ADSR {}", src_idx + 1),
                    ModSourceUI::StepSequencer { .. } => format!("StepSeq {}", src_idx + 1),
                };
                if ui.button(egui::RichText::new(format!("+ {}", src_name)).color(color).small()).clicked() {
                    actions.modulation_actions.push(ModulationAction::AssignModOnMod {
                        target_source_idx: target_idx,
                        param_name: param_name.to_string(),
                        modulator_idx: src_idx,
                        amount: 1.0,
                    });
                }
            }
            if has_assignment {
                ui.separator();
                if ui.button(egui::RichText::new("✕ Remove").small().color(egui::Color32::from_rgb(255, 100, 100))).clicked() {
                    actions.modulation_actions.push(ModulationAction::RemoveModOnMod {
                        target_source_idx: target_idx,
                        param_name: param_name.to_string(),
                    });
                }
            }
        });
}

/// Render shader browser section
fn render_shader_browser(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("📚 Shader Library");
    ui.label(format!("{} shaders loaded", data.shader_count));

    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
        ui.collapsing(format!("Generators ({})", data.generators.len()), |ui| {
            for (name, gen_idx) in &data.generators {
                // Show which channel to add to
                ui.horizontal(|ui| {
                    for ch in &data.channels {
                        if ui.button(format!("{}+", ch.name)).on_hover_text(format!("Add {} to Channel {}", name, ch.name)).clicked() {
                            actions.shader_to_add = Some((ch.ch_idx, *gen_idx));
                        }
                    }
                    ui.label(name);
                });
            }
        });

        ui.collapsing(format!("Filters ({})", data.filters.len()), |ui| {
            for (name, _) in &data.filters {
                ui.label(format!("  🔮 {}", name));
            }
        });

        ui.collapsing("🖼 Image / Solid Color", |ui| {
            ui.horizontal(|ui| {
                ui.label("📁 Image file:");
                for ch in &data.channels {
                    if ui.button(format!("{}+", ch.name)).on_hover_text(format!("Load image to Channel {}", ch.name)).clicked() {
                        // Open file dialog
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp"])
                            .pick_file()
                        {
                            actions.image_to_add = Some((ch.ch_idx, path));
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("🎨 Solid color:");
                for ch in &data.channels {
                    if ui.button(format!("{}+", ch.name)).on_hover_text(format!("Add solid color to Channel {}", ch.name)).clicked() {
                        actions.solid_color_to_add = Some((ch.ch_idx, [0.0, 0.0, 0.0, 1.0]));
                    }
                }
            });
        });
    });
}

/// Channel accent colors
fn channel_color(ch_idx: usize) -> egui::Color32 {
    match ch_idx {
        0 => egui::Color32::from_rgb(160, 100, 255), // Purple — Channel A
        1 => egui::Color32::from_rgb(100, 160, 255), // Blue — Channel B
        2 => egui::Color32::from_rgb(255, 160, 60),  // Orange
        3 => egui::Color32::from_rgb(80, 200, 120),   // Green
        _ => egui::Color32::from_rgb(180, 180, 180),  // Gray for extras
    }
}

/// Render the central panel: Left channels | Mixer Box | Right channels
/// With 2 channels: A | Mixer | B
/// With 4 channels: A,B | Mixer | C,D
/// Generalizes to N channels split evenly across both sides.
fn render_central_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let available = ui.available_width();
    let mixer_width = 160.0;
    let num_channels = data.channels.len();
    let left_count = (num_channels + 1) / 2; // ceil(N/2)
    let right_count = num_channels / 2;       // floor(N/2)
    let side_width = ((available - mixer_width) / 2.0) - 8.0;

    ui.horizontal_top(|ui| {
        // Left side channels
        if left_count > 0 {
            ui.vertical(|ui| {
                ui.set_width(side_width);
                let per_ch_width = side_width / left_count as f32 - 4.0;
                ui.horizontal_top(|ui| {
                    for i in 0..left_count {
                        if let Some(ch) = data.channels.get(i) {
                            ui.vertical(|ui| {
                                ui.set_width(per_ch_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                            if i < left_count - 1 { ui.separator(); }
                        }
                    }
                });
            });
        }

        ui.separator();

        // Center mixer box
        ui.vertical(|ui| {
            ui.set_width(mixer_width);
            render_mixer_box(ui, data, actions);
        });

        ui.separator();

        // Right side channels
        if right_count > 0 {
            ui.vertical(|ui| {
                ui.set_width(side_width);
                let per_ch_width = side_width / right_count as f32 - 4.0;
                ui.horizontal_top(|ui| {
                    for i in 0..right_count {
                        let ch_idx = left_count + i;
                        if let Some(ch) = data.channels.get(ch_idx) {
                            ui.vertical(|ui| {
                                ui.set_width(per_ch_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                            if i < right_count - 1 { ui.separator(); }
                        }
                    }
                });
            });
        }
    });
}

/// Render the center mixer box (DJ console style)
/// Supports N channels: 2 channels = crossfader mode, 3+ = per-channel opacity mode
fn render_mixer_box(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let num_channels = data.channels.len();
    let use_crossfader = num_channels == 2;

    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(egui::Color32::from_rgb(20, 20, 30))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 80)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🎚 Mixer").strong().size(13.0));
                if ui.small_button("➕ Ch").on_hover_text("Add a new channel").clicked() {
                    actions.add_channel = true;
                }
            });
            ui.add_space(4.0);

            // Channel volume faders (vertical, side by side) — N channels
            let fader_height = 100.0;
            let mut opacities: Vec<f32> = data.channels.iter().map(|c| c.opacity).collect();
            let blend_modes_orig: Vec<BlendMode> = data.channels.iter().map(|c| c.blend_mode).collect();

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (ch_idx, ch) in data.channels.iter().enumerate() {
                    let color = channel_color(ch_idx);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&ch.name).strong().color(color).size(11.0));
                            // Show remove button only if more than 2 channels
                            if num_channels > 2 {
                                if ui.small_button("✕").on_hover_text(format!("Remove channel {}", ch.name)).clicked() {
                                    actions.remove_channel = Some(ch_idx);
                                }
                            }
                        });
                        let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                            .vertical()
                            .show_value(false);
                        let resp = ui.add_sized([18.0, fader_height], slider);
                        resp.context_menu(|ui| {
                            if ui.button("🎹 MIDI Learn").clicked() {
                                actions.midi_learn_start = Some(format!("ch/{}/opacity", ch_idx));
                                ui.close_menu();
                            }
                        });
                    });
                }
            });

            // Only emit channel updates when opacity actually changed (via the fader)
            // This avoids overwriting blend mode changes made by the blend mode selector below
            for (ch_idx, ch) in data.channels.iter().enumerate() {
                if (opacities[ch_idx] - ch.opacity).abs() > f32::EPSILON {
                    actions.channel_updates.push((ch_idx, opacities[ch_idx], blend_modes_orig[ch_idx]));
                }
            }

            ui.add_space(4.0);
            ui.separator();

            // Crossfader — only shown for exactly 2 channels
            if use_crossfader {
                let color_a = channel_color(0);
                let color_b = channel_color(1);
                ui.label(egui::RichText::new("Crossfader").small());
                let name_a = data.channels.first().map(|c| c.name.as_str()).unwrap_or("A");
                let name_b = data.channels.get(1).map(|c| c.name.as_str()).unwrap_or("B");
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name_a).small().color(color_a));
                    let mut crossfader = data.crossfader;
                    let slider = egui::Slider::new(&mut crossfader, 0.0..=1.0)
                        .show_value(false);
                    let resp = ui.add_sized([ui.available_width() - 16.0, 18.0], slider);
                    if resp.changed() {
                        actions.crossfader_action = Some(CrossfaderAction::SetPosition(crossfader));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("🎹 MIDI Learn").clicked() {
                            actions.midi_learn_start = Some("crossfader".to_string());
                            ui.close_menu();
                        }
                    });
                    ui.label(egui::RichText::new(name_b).small().color(color_b));
                });

                // Snap buttons
                ui.horizontal(|ui| {
                    if ui.small_button(format!("⏮ {}", name_a)).clicked() {
                        actions.crossfader_action = Some(CrossfaderAction::SnapA);
                    }
                    if ui.small_button(format!("{} ⏭", name_b)).clicked() {
                        actions.crossfader_action = Some(CrossfaderAction::SnapB);
                    }
                });

                ui.add_space(2.0);
                ui.separator();

                // Auto-transition
                let auto_target = if data.crossfader < 0.5 { 1.0 } else { 0.0 };
                let auto_label = if data.crossfader < 0.5 { format!("→{}", name_b) } else { format!("→{}", name_a) };

                if data.auto_crossfade_active {
                    ui.add(egui::ProgressBar::new(data.auto_crossfade_progress)
                        .text("Transitioning..."));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui.small_button(format!("{} 1s", auto_label)).clicked() {
                            actions.crossfader_action = Some(CrossfaderAction::AutoTransition {
                                target: auto_target, duration_secs: 1.0,
                                easing: CrossfadeEasing::EaseInOut,
                            });
                        }
                        if ui.small_button("2s").clicked() {
                            actions.crossfader_action = Some(CrossfaderAction::AutoTransition {
                                target: auto_target, duration_secs: 2.0,
                                easing: CrossfadeEasing::EaseInOut,
                            });
                        }
                        if ui.small_button("4s").clicked() {
                            actions.crossfader_action = Some(CrossfaderAction::AutoTransition {
                                target: auto_target, duration_secs: 4.0,
                                easing: CrossfadeEasing::EaseInOut,
                            });
                        }
                        if data.audio.bpm.is_some() {
                            if ui.small_button("4beat").clicked() {
                                actions.crossfader_action = Some(CrossfaderAction::BeatTransition {
                                    target: auto_target, beats: 4.0,
                                });
                            }
                        }
                    });
                }

                ui.add_space(2.0);
                ui.separator();

                // Transition shader selector
                let current_label = data.active_transition_name.as_deref().unwrap_or("Opacity");
                egui::ComboBox::from_id_salt("transition_selector")
                    .selected_text(egui::RichText::new(format!("🔀 {}", current_label)).small())
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        let is_opacity = data.active_transition_name.is_none();
                        if ui.selectable_label(is_opacity, "Opacity (default)").clicked() {
                            actions.set_transition = Some(None);
                        }
                        ui.separator();
                        for name in &data.transition_names {
                            let selected = data.active_transition_name.as_ref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() {
                                actions.set_transition = Some(Some(name.clone()));
                            }
                        }
                    });
            }

            // Blend mode selectors for each channel
            ui.add_space(2.0);
            ui.separator();
            ui.label(egui::RichText::new("Blend").small());
            let blend_mode_names = ["Norm", "Add", "Mult", "Scrn", "Ovly", "Diff"];
            for (ch_idx, ch) in data.channels.iter().enumerate() {
                let color = channel_color(ch_idx);
                let current = match ch.blend_mode {
                    BlendMode::Normal => 0, BlendMode::Add => 1, BlendMode::Multiply => 2,
                    BlendMode::Screen => 3, BlendMode::Overlay => 4, BlendMode::Difference => 5,
                };
                let mut selected = current;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&ch.name).small().color(color));
                    egui::ComboBox::from_id_salt(format!("mix_blend_{}", ch_idx))
                        .selected_text(blend_mode_names[selected])
                        .width(55.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in blend_mode_names.iter().enumerate() {
                                ui.selectable_value(&mut selected, i, *name);
                            }
                        });
                });
                if selected != current {
                    let new_blend = match selected {
                        1 => BlendMode::Add, 2 => BlendMode::Multiply, 3 => BlendMode::Screen,
                        4 => BlendMode::Overlay, 5 => BlendMode::Difference, _ => BlendMode::Normal,
                    };
                    // Either update an existing entry (if opacity also changed this frame)
                    // or push a new one for blend-mode-only changes
                    if let Some(entry) = actions.channel_updates.iter_mut().find(|e| e.0 == ch_idx) {
                        entry.2 = new_blend;
                    } else {
                        actions.channel_updates.push((ch_idx, ch.opacity, new_blend));
                    }
                }
            }
        });
}

/// Render a channel column with header and its decks
fn render_channel_column(ui: &mut egui::Ui, ch: &super::ChannelUIInfo, data: &UIData, actions: &mut UIActions) {
    let accent = channel_color(ch.ch_idx);
    let ch_idx = ch.ch_idx;

    ui.push_id(format!("ch_{}", ch_idx), |ui| {
        // Check if a deck is being dragged over this channel
        let is_drag_hovering = egui::DragAndDrop::payload::<(usize, usize)>(ui.ctx())
            .map(|p| p.0 != ch_idx) // only highlight if from a different channel
            .unwrap_or(false)
            && ui.rect_contains_pointer(ui.max_rect());

        let frame = if is_drag_hovering {
            egui::Frame::default()
                .fill(accent.linear_multiply(0.08))
                .stroke(egui::Stroke::new(2.0, accent.linear_multiply(0.5)))
                .corner_radius(4.0)
        } else {
            egui::Frame::NONE
        };

        frame.show(ui, |ui| {
        ui.vertical(|ui| {
            // Channel header (clickable to select channel for bottom bar)
            let is_ch_selected = data.selected_channel == Some(ch_idx);
            let header_frame = if is_ch_selected {
                egui::Frame::default().fill(accent.linear_multiply(0.15)).corner_radius(3.0).inner_margin(2.0)
            } else {
                egui::Frame::default().inner_margin(2.0)
            };
            header_frame.show(ui, |ui| {
                let header_resp = ui.label(egui::RichText::new(format!("▌ Channel {}", ch.name)).strong().color(accent).size(16.0));
                if header_resp.interact(egui::Sense::click()).clicked() {
                    actions.select_channel = Some(ch_idx);
                }
            });

            ui.separator();

            // Deck grid (dynamically scaled columns based on available width)
            egui::ScrollArea::vertical().id_salt(format!("ch_scroll_{}", ch_idx)).show(ui, |ui| {
                if ch.decks.is_empty() {
                    ui.label(egui::RichText::new("No decks — drag from shader library").weak().small());
                }
                // Calculate how many deck cards fit per row
                let card_width = 134.0; // preview(100) + slider(18) + padding(8) + margin(8)
                let available_w = ui.available_width();
                let cols = ((available_w / card_width).floor() as usize).max(1);
                let mut deck_iter = ch.decks.iter().peekable();
                while deck_iter.peek().is_some() {
                    ui.horizontal(|ui| {
                        for _ in 0..cols {
                            if let Some(deck) = deck_iter.next() {
                                render_deck_thumbnail(ui, ch_idx, deck, accent, data, actions);
                            }
                        }
                    });
                    ui.add_space(2.0);
                }

                // Drop zone: accept deck drops into this channel
                let drop_resp = ui.allocate_response(
                    egui::vec2(ui.available_width(), 20.0_f32.max(ui.available_height())),
                    egui::Sense::hover(),
                );
                if let Some(payload) = drop_resp.dnd_release_payload::<(usize, usize)>() {
                    let (src_ch, src_deck) = *payload;
                    if src_ch != ch_idx {
                        actions.deck_to_move = Some((src_ch, src_deck, ch_idx));
                    }
                }

                // Channel FX chain (compact)
                if !ch.effects.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("🔮 Ch FX").small().color(accent));
                    for (_i, (name, enabled, _)) in ch.effects.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let label = if *enabled {
                                egui::RichText::new(name).small()
                            } else {
                                egui::RichText::new(name).small().strikethrough().weak()
                            };
                            ui.label(label);
                        });
                    }
                }
            });
        });
        }); // frame.show
    });
}

/// Render a compact deck thumbnail (clickable cell in the deck grid)
/// Layout: [ preview | opacity slider (vertical) ]
///         [ name                                 ]
///         [ M  S  ✕                              ]
fn render_deck_thumbnail(
    ui: &mut egui::Ui,
    ch_idx: usize,
    deck: &super::DeckUIInfo,
    accent: egui::Color32,
    data: &UIData,
    actions: &mut UIActions,
) {
    let idx = deck.deck_idx;
    let mut opacity = deck.opacity;
    let mut solo = deck.solo;
    let mut mute = deck.mute;
    let is_selected = data.selected_deck == Some((ch_idx, idx));
    let preview_width = 100.0;
    let preview_height = preview_width * 0.5625;
    let slider_width = 18.0;
    let card_width = preview_width + slider_width + 8.0; // preview + slider + padding

    ui.push_id(format!("deck_{}_{}", ch_idx, idx), |ui| {
        let border_color = if is_selected { accent } else { accent.linear_multiply(0.3) };
        let border_width = if is_selected { 2.0 } else { 1.0 };

        // Use manual rect-based painting to avoid egui layout overlap issues.
        // Total card height = preview_height + name_row(16) + button_row(20) + spacing(8) + padding(8)
        let name_row_h = 16.0;
        let button_row_h = 20.0;
        let spacing = 4.0;
        let padding = 4.0;
        let total_h = padding + preview_height + spacing + name_row_h + spacing + button_row_h + padding;

        let card_size = egui::vec2(card_width + padding * 2.0, total_h);
        let (card_rect, card_resp) = ui.allocate_exact_size(
            card_size,
            egui::Sense::click_and_drag(),
        );

        // Start drag: set payload for deck move between channels
        if card_resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), (ch_idx, idx));
        }

        // While dragging, show a translucent ghost at the cursor
        if card_resp.dragged() {
            if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                let ghost_rect = egui::Rect::from_center_size(pointer_pos, card_size);
                let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("deck_drag_ghost"));
                let painter = ui.ctx().layer_painter(layer);
                painter.rect_filled(ghost_rect, 4.0, egui::Color32::from_rgba_unmultiplied(80, 120, 200, 120));
                painter.text(
                    ghost_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &deck.name,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Draw card background + border
        let bg_alpha = if card_resp.dragged() { 100 } else { 255 };
        ui.painter().rect_filled(card_rect, 4.0, egui::Color32::from_rgba_unmultiplied(25, 25, 35, bg_alpha));
        ui.painter().rect_stroke(card_rect, 4.0, egui::Stroke::new(border_width, border_color), egui::StrokeKind::Outside);

        // Row 1: Preview image (left) + vertical opacity slider (right)
        let preview_rect = egui::Rect::from_min_size(
            card_rect.min + egui::vec2(padding, padding),
            egui::vec2(preview_width, preview_height),
        );
        let slider_rect = egui::Rect::from_min_size(
            egui::pos2(preview_rect.max.x + 2.0, card_rect.min.y + padding),
            egui::vec2(slider_width, preview_height),
        );

        // Draw preview
        if let Some(&texture_id) = data.deck_preview_textures.get(&(ch_idx, idx)) {
            ui.painter().image(
                texture_id,
                preview_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            ui.painter().rect_filled(preview_rect, 3.0, egui::Color32::from_rgb(30, 30, 40));
            ui.painter().text(
                preview_rect.center(),
                egui::Align2::CENTER_CENTER,
                "No Preview",
                egui::FontId::proportional(9.0),
                egui::Color32::GRAY,
            );
        }

        // Click on preview to select
        let preview_resp = ui.allocate_rect(preview_rect, egui::Sense::click());
        if preview_resp.clicked() {
            actions.select_deck = Some((ch_idx, idx));
        }

        // Vertical opacity slider — use a child ui placed at the slider rect
        let mut slider_ui = ui.new_child(egui::UiBuilder::new().max_rect(slider_rect));
        let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
            .vertical()
            .show_value(false);
        let op_resp = slider_ui.add_sized([slider_width, preview_height], slider);
        op_resp.context_menu(|ui| {
            let path = format!("ch/{}/deck/{}/opacity", ch_idx, idx);
            if ui.button("🎹 MIDI Learn").clicked() {
                actions.midi_learn_start = Some(path);
                ui.close_menu();
            }
        });

        // Row 2: Deck name
        let name_y = card_rect.min.y + padding + preview_height + spacing;
        let display_name = if deck.name.len() > 16 {
            format!("{}…", &deck.name[..15])
        } else {
            deck.name.clone()
        };
        ui.painter().text(
            egui::pos2(card_rect.min.x + padding, name_y),
            egui::Align2::LEFT_TOP,
            &display_name,
            egui::FontId::proportional(11.0),
            accent,
        );

        // Row 3: M S ✕ buttons — use a child ui placed at the button row
        let btn_y = name_y + name_row_h + spacing;
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(card_rect.min.x + padding, btn_y),
            egui::vec2(card_width, button_row_h),
        );
        let mut btn_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect));
        btn_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ui.selectable_label(mute, egui::RichText::new("M").small()).clicked() {
                mute = !mute;
            }
            if ui.selectable_label(solo, egui::RichText::new("S").small()).clicked() {
                solo = !solo;
            }
            if ui.small_button(egui::RichText::new("✕").small()).clicked() {
                actions.deck_to_remove = Some((ch_idx, idx));
            }
        });

        // Only push deck updates when something actually changed from the UI controls here
        // (opacity slider, solo/mute buttons). This avoids overwriting blend mode changes
        // made in the detail panel's blend mode selector.
        if (opacity - deck.opacity).abs() > f32::EPSILON || solo != deck.solo || mute != deck.mute {
            actions.deck_updates.push((ch_idx, idx, opacity, deck.blend_mode, solo, mute));
        }
    });
}

