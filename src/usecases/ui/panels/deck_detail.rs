//! Bottom panel and deck detail.

use crate::params::ParamValue;
use crate::{BlendMode, ScalingMode};
use super::super::{UIData, UIActions, ParamUpdate, ModulationAction, VideoAction, AutoTransitionAction, LibraryDrag, widgets, EffectDrag};
use super::utils::{format_time, channel_color, render_collapsed_column, render_effect_drop_zone, render_effect_drag_handle, render_effect_drag_ghost};
use super::effects::{render_master_effect_detail, render_channel_effect_detail};

pub(super) fn render_bottom_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // MIDI learn status indicator
    if data.midi_learn_active {
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(4.0)
            .fill(egui::Color32::from_rgb(180, 80, 220))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(target) = &data.midi_learn_target {
                        ui.label(egui::RichText::new(format!("🎹 MIDI LEARN — Move a control to map: {}", target))
                            .strong().color(egui::Color32::WHITE));
                    } else {
                        ui.label(egui::RichText::new("🎹 MIDI LEARN — Click a parameter to select it")
                            .strong().color(egui::Color32::WHITE));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("x Exit MIDI Learn").clicked() {
                            actions.midi_learn_toggle = true;
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
pub(super) fn render_selected_deck_detail(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
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
    ui.label(egui::RichText::new(format!("{} / Deck {} — {}", ch.name, deck_idx + 1, deck.name))
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

            // Column: Video playback controls (only for video decks)
            if let Some(ref vp) = deck.video_playback {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(220.0);
                        ui.set_max_width(280.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.label(egui::RichText::new("▶ Playback").strong());

                            // Play/Pause button
                            let play_label = if vp.playing { "⏸ Pause" } else { "▶ Play" };
                            if ui.button(play_label).clicked() {
                                actions.video_actions.push((ch_idx, deck_idx, VideoAction::TogglePlay));
                            }

                            // Position scrub bar
                            let duration = vp.duration.max(0.001);
                            let mut pos = vp.position as f32;
                            ui.horizontal(|ui| {
                                ui.label(format_time(vp.position));
                                let slider = egui::Slider::new(&mut pos, 0.0..=duration as f32)
                                    .show_value(false)
                                    .trailing_fill(true);
                                if ui.add(slider).changed() {
                                    actions.video_actions.push((ch_idx, deck_idx, VideoAction::Seek(pos as f64)));
                                }
                                ui.label(format_time(duration));
                            });

                            // Speed control
                            let mut speed = vp.speed as f32;
                            ui.horizontal(|ui| {
                                ui.label("Speed:");
                                if ui.add(egui::Slider::new(&mut speed, 0.1..=4.0).step_by(0.05).suffix("x")).changed() {
                                    actions.video_actions.push((ch_idx, deck_idx, VideoAction::SetSpeed(speed as f64)));
                                }
                            });

                            // Loop mode
                            ui.horizontal(|ui| {
                                ui.label("Loop:");
                                let modes = [
                                    ("🔁", crate::video::LoopMode::Loop, "Loop"),
                                    ("🔄", crate::video::LoopMode::PingPong, "Ping-Pong"),
                                    ("1️⃣", crate::video::LoopMode::OneShot, "One Shot"),
                                    ("⏹", crate::video::LoopMode::HoldLast, "Hold Last"),
                                ];
                                for (icon, mode, tooltip) in &modes {
                                    let selected = vp.loop_mode == *mode;
                                    let btn = egui::Button::new(*icon).selected(selected);
                                    if ui.add(btn).on_hover_text(*tooltip).clicked() && !selected {
                                        actions.video_actions.push((ch_idx, deck_idx, VideoAction::SetLoopMode(*mode)));
                                    }
                                }
                            });

                            // In/Out points (bookshelf)
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("📐 In/Out Points").strong());
                            let effective_out = if vp.out_point > 0.0 { vp.out_point } else { duration };
                            let has_range = vp.in_point > 0.0 || vp.out_point > 0.0;

                            // In-point
                            let mut in_pt = vp.in_point as f32;
                            ui.horizontal(|ui| {
                                ui.label("In:");
                                if ui.add(egui::Slider::new(&mut in_pt, 0.0..=duration as f32)
                                    .show_value(false).trailing_fill(true)).changed()
                                {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetInPoint(in_pt as f64)));
                                }
                                ui.label(format_time(in_pt as f64));
                            });

                            // Out-point
                            let mut out_pt = effective_out as f32;
                            ui.horizontal(|ui| {
                                ui.label("Out:");
                                if ui.add(egui::Slider::new(&mut out_pt, 0.0..=duration as f32)
                                    .show_value(false).trailing_fill(true)).changed()
                                {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetOutPoint(out_pt as f64)));
                                }
                                ui.label(format_time(out_pt as f64));
                            });

                            // Set from current / clear buttons
                            ui.horizontal(|ui| {
                                if ui.small_button("[ Set In").on_hover_text("Set in-point to current position").clicked() {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetInPoint(vp.position)));
                                }
                                if ui.small_button("Set Out ]").on_hover_text("Set out-point to current position").clicked() {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetOutPoint(vp.position)));
                                }
                                if has_range {
                                    if ui.small_button("x Clear").on_hover_text("Reset to full clip").clicked() {
                                        actions.video_actions.push((ch_idx, deck_idx,
                                            VideoAction::ClearInOutPoints));
                                    }
                                }
                            });

                            if has_range {
                                ui.label(egui::RichText::new(format!(
                                    "Range: {} → {} ({})",
                                    format_time(vp.in_point),
                                    format_time(effective_out),
                                    format_time(effective_out - vp.in_point),
                                )).small().weak());
                            }

                            // Info line
                            ui.label(egui::RichText::new(format!(
                                "{:.0} fps • {}", vp.frame_rate, format_time(duration)
                            )).small().weak());
                        });
                    });
                ui.separator();
            }

            // Column: Auto-Transition controls (collapsible column, default closed)
            {
                let at_open_id = egui::Id::new("at_col_open").with((ch_idx, deck_idx));
                let at_open = ui.ctx().memory(|mem| mem.data.get_temp::<bool>(at_open_id).unwrap_or(false));
                if at_open {
                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(200.0);
                            ui.set_max_width(260.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                // Clickable full-width header to collapse
                                let header_rect = ui.available_rect_before_wrap();
                                let header_rect = egui::Rect::from_min_size(header_rect.min, egui::vec2(ui.available_width(), 20.0));
                                let header_resp = ui.allocate_rect(header_rect, egui::Sense::click());
                                ui.painter().text(header_rect.left_center(), egui::Align2::LEFT_CENTER, "Auto Transition", egui::FontId::proportional(13.0), ui.visuals().strong_text_color());
                                if header_resp.clicked() {
                                    ui.ctx().memory_mut(|mem| mem.data.insert_temp(at_open_id, false));
                                }
                                if header_resp.hovered() {
                                    ui.painter().rect_filled(header_rect, 2.0, ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.3));
                                }
                                ui.separator();
                                // Enable toggle
                                ui.horizontal(|ui| {
                                    let enabled = deck.auto_transition.as_ref().map_or(false, |at| at.enabled);
                                    let mut en = enabled;
                                    ui.checkbox(&mut en, "Enabled");
                                    if en != enabled {
                                        actions.auto_transition_actions.push((ch_idx, deck_idx,
                                            AutoTransitionAction::SetEnabled(en)));
                                    }
                                });

                                if let Some(ref at) = deck.auto_transition {
                                    if at.enabled {
                                        ui.horizontal(|ui| {
                                            ui.label("Trigger:");
                                            let mut clip_end = at.trigger_is_clip_end;
                                            if ui.selectable_label(!clip_end, "Timer").clicked() && clip_end {
                                                clip_end = false;
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::SetTrigger(false)));
                                            }
                                            if ui.selectable_label(clip_end, "Clip End").clicked() && !clip_end {
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::SetTrigger(true)));
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Play:");
                                            let mut val = at.play_duration_value as f32;
                                            let max = if at.play_duration_is_beats { 128.0 } else { 300.0 };
                                            if ui.add(egui::Slider::new(&mut val, 0.5..=max)
                                                .logarithmic(true)
                                                .suffix(if at.play_duration_is_beats { " beats" } else { " sec" })
                                            ).changed() {
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::SetPlayDuration(val as f64)));
                                            }
                                            if ui.small_button(if at.play_duration_is_beats { "♩" } else { "⏱" })
                                                .on_hover_text("Toggle beats/seconds").clicked()
                                            {
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::TogglePlayDurationUnit));
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Trans:");
                                            let mut val = at.transition_duration_value as f32;
                                            let max = if at.transition_duration_is_beats { 32.0 } else { 30.0 };
                                            if ui.add(egui::Slider::new(&mut val, 0.1..=max)
                                                .logarithmic(true)
                                                .suffix(if at.transition_duration_is_beats { " beats" } else { " sec" })
                                            ).changed() {
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::SetTransitionDuration(val as f64)));
                                            }
                                            if ui.small_button(if at.transition_duration_is_beats { "♩" } else { "⏱" })
                                                .on_hover_text("Toggle beats/seconds").clicked()
                                            {
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::ToggleTransitionDurationUnit));
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Shader:");
                                            let current = at.transition_shader_name.as_deref().unwrap_or("(fade)");
                                            egui::ComboBox::from_id_salt(format!("at_shader_{}_{}", ch_idx, deck_idx))
                                                .selected_text(current)
                                                .width(120.0)
                                                .show_ui(ui, |ui| {
                                                    if ui.selectable_label(at.transition_shader_name.is_none(), "(fade)").clicked() {
                                                        actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                            AutoTransitionAction::SetTransitionShader(None)));
                                                    }
                                                    for name in &data.transition_names {
                                                        let selected = at.transition_shader_name.as_deref() == Some(name.as_str());
                                                        if ui.selectable_label(selected, name).clicked() && !selected {
                                                            actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                                AutoTransitionAction::SetTransitionShader(Some(name.clone()))));
                                                        }
                                                    }
                                                });
                                        });
                                    }
                                }
                            });
                        });
                } else {
                    // Collapsed: narrow vertical strip with vertical text
                    render_collapsed_column(ui, "Auto Transition", at_open_id);
                }
                ui.separator();
            }

            // Column: Generator parameters + blend/scale (collapsible column, default open)
            {
                let params_open_id = egui::Id::new("params_col_open").with((ch_idx, deck_idx));
                let params_open = ui.ctx().memory(|mem| mem.data.get_temp::<bool>(params_open_id).unwrap_or(true));
                if params_open {
                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(200.0);
                            ui.set_max_width(280.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                // Clickable full-width header to collapse
                                let header_rect = ui.available_rect_before_wrap();
                                let header_rect = egui::Rect::from_min_size(header_rect.min, egui::vec2(ui.available_width(), 20.0));
                                let header_resp = ui.allocate_rect(header_rect, egui::Sense::click());
                                let params_label = format!("Params: {}", deck.generator.shader_name);
                                ui.painter().text(header_rect.left_center(), egui::Align2::LEFT_CENTER, &params_label, egui::FontId::proportional(13.0), ui.visuals().strong_text_color());
                                if header_resp.clicked() {
                                    ui.ctx().memory_mut(|mem| mem.data.insert_temp(params_open_id, false));
                                }
                                if header_resp.hovered() {
                                    ui.painter().rect_filled(header_rect, 2.0, ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.3));
                                }
                                ui.separator();
                            let max_h = (ui.available_height() - 8.0).max(100.0);
                            egui::ScrollArea::vertical().id_salt("deck_gen_scroll").max_height(max_h).show(ui, |ui| {
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
                                    ui.label(egui::RichText::new(&gen_params.shader_name).strong());
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
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("ch{}_deck{}", ch_idx, deck_idx),
                                    );
                                    ui.add_space(4.0);
                                    if ui.button("Reset").clicked() {
                                        actions.param_updates.push(ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx });
                                    }
                                }
                            });
                            });
                        });
                } else {
                    // Collapsed: narrow vertical strip with vertical text
                    render_collapsed_column(ui, &format!("Params: {}", deck.generator.shader_name), params_open_id);
                }
            }

            ui.separator();

            // Effect chain: drag-and-drop reordering + library drops
            {
                for (eff_idx, (eff_name, eff_enabled, eff_params)) in deck.effects.iter().enumerate() {
                    // Drop zone before this effect (for reordering)
                    render_effect_drop_zone(ui, &format!("deck_{}_{}", ch_idx, deck_idx), eff_idx);

                    // Effect card with drag handle in header only
                    let card_resp = egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            let max_h = (ui.available_height() - 8.0).max(100.0);
                            egui::ScrollArea::vertical().id_salt(format!("deck_fx_scroll_{}_{}_{}",ch_idx,deck_idx,eff_idx)).max_height(max_h).scroll_source(egui::scroll_area::ScrollSource { drag: false, scroll_bar: true, mouse_wheel: true }).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    render_effect_drag_handle(ui, EffectDrag::Deck(ch_idx, deck_idx, eff_idx));
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.effect_to_toggle = Some((ch_idx, deck_idx, eff_idx));
                                    }
                                    ui.label(egui::RichText::new(eff_name).strong());
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
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("ch{}_deck{}_fx{}", ch_copy, deck_copy, eff_idx_copy),
                                    );
                                }
                            });
                            });
                        });
                    // X button overlay at top-right of card
                    {
                        let card_rect = card_resp.response.rect;
                        let btn_size = egui::vec2(16.0, 16.0);
                        let btn_pos = egui::pos2(card_rect.right() - btn_size.x - 4.0, card_rect.top() + 4.0);
                        let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                        let btn_resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                        let color = if btn_resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                        ui.painter().text(btn_rect.center(), egui::Align2::CENTER_CENTER, "x", egui::FontId::proportional(12.0), color);
                        if btn_resp.clicked() {
                            actions.effect_to_remove = Some((ch_idx, deck_idx, eff_idx));
                        }
                    }
                    render_effect_drag_ghost(
                        ui,
                        egui::Id::new(("eff_ghost", ch_idx, deck_idx, eff_idx)),
                        EffectDrag::Deck(ch_idx, deck_idx, eff_idx),
                        eff_name,
                    );
                    ui.separator();
                }

                // Drop zone after last effect (for reordering to end)
                if !deck.effects.is_empty() {
                    let num_effects = deck.effects.len();
                    render_effect_drop_zone(ui, &format!("deck_{}_{}", ch_idx, deck_idx), num_effects);
                }

                // Remaining space: always present drop target that fills remaining width
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
                        if deck.effects.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                            });
                        }
                    });
            }

            // Store the entire horizontal_top area as the drop rect for deferred library effect drops
            let chain_rect = ui.min_rect();
            let deck_chain_key = format!("deck_{}_{}", ch_idx, deck_idx);
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("deck_fx_drop_rect").with((ch_idx, deck_idx)), chain_rect);
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with(deck_chain_key), deck.effects.len() + 1);
            });
        });
    });
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_bottom_panel_smoke_deck_selected() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_channel_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = Some(0);
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_master_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_master = true;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_nothing_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = None;
        data.selected_master = false;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }
}