//! Central panel, mixer box, channel columns, deck thumbnails.

use crate::mixer::CrossfadeEasing;
use crate::BlendMode;
use super::super::{UIData, UIActions, CrossfaderAction, LibraryDrag, ChannelUIInfo, DeckUIInfo, widgets};
use super::utils::channel_color;
use super::stage::render_stage_editor;
use super::sequence::render_sequence_builder;

pub(super) fn render_central_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    if data.stage_editor_open {
        render_stage_editor(ui, data, actions);
        return;
    }

    let available = ui.available_width();
    let has_sequences = !data.sequences.is_empty();
    // Widen the center column when sequences are present so steps don't wrap
    let center_width = if has_sequences { 400.0_f32.min(available * 0.45) } else { 160.0 };
    let num_channels = data.channels.len();
    let left_count = (num_channels + 1) / 2; // ceil(N/2)
    let right_count = num_channels / 2;       // floor(N/2)
    let side_width = ((available - center_width) / 2.0) - 8.0;

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

        // Center column: mixer box (centered) + sequence builder below
        ui.vertical(|ui| {
            ui.set_width(center_width);

            // Center the mixer box within the wider column
            let mixer_width = 160.0;
            let mixer_pad = ((center_width - mixer_width) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(mixer_pad);
                ui.vertical(|ui| {
                    ui.set_width(mixer_width);
                    render_mixer_box(ui, data, actions);
                });
            });

            // Sequence builder below mixer, uses full center_width
            if has_sequences || data.channel_count >= 2 {
                ui.add_space(4.0);
                render_sequence_builder(ui, data, actions);
            }
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
pub(super) fn render_mixer_box(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
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
                // Center faders: estimate total width and add leading space
                let per_fader_width = 30.0_f32; // label + slider column width
                let spacing = 4.0;
                let total_faders_width = num_channels as f32 * per_fader_width
                    + (num_channels.saturating_sub(1)) as f32 * spacing;
                let avail = ui.available_width();
                if avail > total_faders_width {
                    ui.add_space((avail - total_faders_width) / 2.0);
                }
                for (ch_idx, ch) in data.channels.iter().enumerate() {
                    let color = channel_color(ch_idx);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&ch.name).strong().color(color).size(11.0));
                            // Show remove button only if more than 2 channels
                            if num_channels > 2 {
                                if ui.small_button("x").on_hover_text(format!("Remove channel {}", ch.name)).clicked() {
                                    actions.remove_channel = Some(ch_idx);
                                }
                            }
                        });
                        // Render slider — disabled in learn mode
                        let slider_rect;
                        if data.midi_learn_active {
                            let inner = ui.scope(|ui| {
                                ui.disable();
                                let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                                    .vertical()
                                    .show_value(false);
                                ui.add_sized([18.0, fader_height], slider)
                            });
                            slider_rect = inner.inner.rect;
                        } else {
                            let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                                .vertical()
                                .show_value(false);
                            let resp = ui.add_sized([18.0, fader_height], slider);
                            slider_rect = resp.rect;
                        }
                        // MIDI learn: glow + click overlay
                        if data.midi_learn_active {
                            let path = format!("ch/{}/opacity", ch_idx);
                            let is_target = data.midi_learn_target.as_deref() == Some(path.as_str());
                            if is_target {
                                widgets::draw_midi_learn_selected(ui, slider_rect);
                            } else {
                                widgets::draw_midi_learn_glow(ui, slider_rect);
                            }
                            let click_id = ui.id().with(("midi_learn_ch_opacity", ch_idx));
                            let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                            if click_resp.clicked() {
                                actions.midi_learn_select = Some(path);
                            }
                        }
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
                    let slider_rect;
                    if data.midi_learn_active {
                        let inner = ui.scope(|ui| {
                            ui.disable();
                            let slider = egui::Slider::new(&mut crossfader, 0.0..=1.0)
                                .show_value(false);
                            ui.add_sized([ui.available_width() - 16.0, 18.0], slider)
                        });
                        slider_rect = inner.inner.rect;
                    } else {
                        let slider = egui::Slider::new(&mut crossfader, 0.0..=1.0)
                            .show_value(false);
                        let resp = ui.add_sized([ui.available_width() - 16.0, 18.0], slider);
                        if resp.changed() {
                            actions.crossfader_action = Some(CrossfaderAction::SetPosition(crossfader));
                        }
                        slider_rect = resp.rect;
                    }
                    if data.midi_learn_active {
                        let is_target = data.midi_learn_target.as_deref() == Some("crossfader");
                        if is_target {
                            widgets::draw_midi_learn_selected(ui, slider_rect);
                        } else {
                            widgets::draw_midi_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with("midi_learn_crossfader");
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            actions.midi_learn_select = Some("crossfader".to_string());
                        }
                    }
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
pub(super) fn render_channel_column(ui: &mut egui::Ui, ch: &ChannelUIInfo, data: &UIData, actions: &mut UIActions) {
    let accent = channel_color(ch.ch_idx);
    let ch_idx = ch.ch_idx;

    ui.push_id(format!("ch_{}", ch_idx), |ui| {
        // Visual feedback: highlight when a library drag hovers over this channel
        let has_library_drag = egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ui.ctx());
        let is_hovering = has_library_drag && ui.rect_contains_pointer(ui.max_rect());

        let frame = if is_hovering {
            egui::Frame::default()
                .fill(accent.linear_multiply(0.08))
                .stroke(egui::Stroke::new(2.0, accent.linear_multiply(0.5)))
                .corner_radius(4.0)
        } else {
            egui::Frame::NONE
        };

        frame.show(ui, |ui| {
        ui.vertical(|ui| {
            // Channel header
            let is_ch_selected = data.selected_channel == Some(ch_idx);
            let header_frame = if is_ch_selected {
                egui::Frame::default().fill(accent.linear_multiply(0.15)).corner_radius(3.0).inner_margin(2.0)
            } else {
                egui::Frame::default().inner_margin(2.0)
            };
            header_frame.show(ui, |ui| {
                let header_resp = ui.label(egui::RichText::new(format!("▌ {}", ch.name)).strong().color(accent).size(16.0));
                let header_resp = header_resp.interact(egui::Sense::click());
                if header_resp.clicked() {
                    actions.select_channel = Some(ch_idx);
                }
            });

            ui.separator();

            // Deck grid
            egui::ScrollArea::vertical()
                .id_salt(format!("ch_scroll_{}", ch_idx))
                .scroll_source(egui::scroll_area::ScrollSource { drag: false, scroll_bar: true, mouse_wheel: true })
                .show(ui, |ui| {
                if ch.decks.is_empty() {
                    ui.label(egui::RichText::new("No decks — drag generator here").weak().small());
                }
                let card_width = 134.0;
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

                // Remaining space for deck-to-deck moves
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

        // Store this channel's screen rect for the deferred DnD drop handler
        let ch_rect = ui.min_rect();
        ui.ctx().memory_mut(|mem| {
            mem.data.insert_temp(egui::Id::new("ch_drop_rect").with(ch_idx), ch_rect);
        });
    });
}

/// Render a compact deck thumbnail (clickable cell in the deck grid)
/// Layout: [ preview | opacity slider (vertical) ]
///         [ name                                 ]
///         [ M  S  x                              ]
pub(super) fn render_deck_thumbnail(
    ui: &mut egui::Ui,
    ch_idx: usize,
    deck: &DeckUIInfo,
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

        // MIDI learn mode: glow on deck card, click to select trigger
        if data.midi_learn_active {
            let trigger_path = format!("ch/{}/deck/{}/trigger", ch_idx, idx);
            let is_target = data.midi_learn_target.as_deref() == Some(trigger_path.as_str());
            if is_target {
                widgets::draw_midi_learn_selected(ui, card_rect);
            } else {
                widgets::draw_midi_learn_glow(ui, card_rect);
            }
            if card_resp.clicked() {
                actions.midi_learn_select = Some(trigger_path);
            }
        }

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

        // Auto-transition indicator overlay on preview
        if let Some(ref at) = deck.auto_transition {
            if at.enabled {
                let (icon, color) = match at.phase {
                    crate::channel::DeckTransitionPhase::Inactive => ("⏹", egui::Color32::GRAY),
                    crate::channel::DeckTransitionPhase::Playing { .. } => ("▶", egui::Color32::from_rgb(80, 200, 80)),
                    crate::channel::DeckTransitionPhase::Transitioning { .. } => ("🔄", egui::Color32::from_rgb(200, 160, 40)),
                    crate::channel::DeckTransitionPhase::Done => ("✓", egui::Color32::from_rgb(100, 100, 100)),
                };
                // Small badge in top-right of preview
                ui.painter().text(
                    egui::pos2(preview_rect.max.x - 2.0, preview_rect.min.y + 2.0),
                    egui::Align2::RIGHT_TOP,
                    icon,
                    egui::FontId::proportional(10.0),
                    color,
                );
                // Progress bar at bottom of preview during transition
                if let crate::channel::DeckTransitionPhase::Transitioning { progress } = at.phase {
                    let bar_h = 3.0;
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(preview_rect.min.x, preview_rect.max.y - bar_h),
                        egui::vec2(preview_rect.width() * progress as f32, bar_h),
                    );
                    ui.painter().rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(200, 160, 40));
                }
                // Countdown bar during playing phase
                if let crate::channel::DeckTransitionPhase::Playing { elapsed } = at.phase {
                    let total = at.play_duration_value;
                    if total > 0.0 {
                        let frac = (elapsed / total).min(1.0) as f32;
                        let bar_h = 3.0;
                        let bar_rect = egui::Rect::from_min_size(
                            egui::pos2(preview_rect.min.x, preview_rect.max.y - bar_h),
                            egui::vec2(preview_rect.width() * frac, bar_h),
                        );
                        ui.painter().rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(80, 200, 80));
                    }
                }
            }
        }

        // Click on card to select deck (only when not in learn mode — learn mode click selects trigger)
        if !data.midi_learn_active && card_resp.clicked() {
            actions.select_deck = Some((ch_idx, idx));
        }

        // Vertical opacity slider — use a child ui placed at the slider rect
        let mut slider_ui = ui.new_child(egui::UiBuilder::new().max_rect(slider_rect));
        let op_slider_rect;
        if data.midi_learn_active {
            let inner = slider_ui.scope(|ui| {
                ui.disable();
                let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
                    .vertical()
                    .show_value(false);
                ui.add_sized([slider_width, preview_height], slider)
            });
            op_slider_rect = inner.inner.rect;
        } else {
            let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
                .vertical()
                .show_value(false);
            let resp = slider_ui.add_sized([slider_width, preview_height], slider);
            op_slider_rect = resp.rect;
        }
        if data.midi_learn_active {
            let opacity_path = format!("ch/{}/deck/{}/opacity", ch_idx, idx);
            let is_target = data.midi_learn_target.as_deref() == Some(opacity_path.as_str());
            if is_target {
                widgets::draw_midi_learn_selected(&slider_ui, op_slider_rect);
            } else {
                widgets::draw_midi_learn_glow(&slider_ui, op_slider_rect);
            }
            let click_id = slider_ui.id().with(("midi_learn_deck_opacity", ch_idx, idx));
            let click_resp = slider_ui.interact(op_slider_rect, click_id, egui::Sense::click());
            if click_resp.clicked() {
                actions.midi_learn_select = Some(opacity_path);
            }
        }

        // Effective opacity overlay (shows auto-transition fading as a filled bar)
        if deck.effective_opacity < deck.opacity - 0.01 {
            let frac = deck.effective_opacity / deck.opacity.max(0.001);
            let bar_h = op_slider_rect.height() * (1.0 - frac);
            let bar_rect = egui::Rect::from_min_size(
                op_slider_rect.min,
                egui::vec2(op_slider_rect.width(), bar_h),
            );
            ui.painter().rect_filled(bar_rect, 2.0, egui::Color32::from_rgba_unmultiplied(200, 60, 60, 80));
        }

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

        // Row 3: M S x buttons — use a child ui placed at the button row
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
            if ui.small_button(egui::RichText::new("x").small()).clicked() {
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