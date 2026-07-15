//! Central panel, mixer box, channel columns, deck thumbnails.

use super::super::{
    widgets, ChannelUIInfo, CrossfaderAction, DeckUIInfo, LibraryDrag, SequenceAction, UIActions,
    UIData,
};
use super::sequence::render_sequence_builder;
use super::stage::render_stage_editor;
use super::utils::channel_color;
use crate::mixer::CrossfadeEasing;
use crate::BlendMode;

pub(super) fn render_central_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    if data.stage_editor_open {
        render_stage_editor(ui, data, actions);
        return;
    }

    let available = ui.available_width();
    let panel_height = ui.available_height();
    let has_sequences = !data.sequences.is_empty();
    let num_channels = data.channels.len();
    let left_count = num_channels.div_ceil(2); // ceil(N/2)
    let right_count = num_channels / 2; // floor(N/2)

    // Fixed widths — channels and mixer never scale with window resize
    let ch_card_width = 150.0_f32;
    // Mixer width scales with channel count (30px per fader + 4px spacing + frame padding)
    let per_fader = 30.0_f32;
    let fader_spacing = 4.0_f32;
    let frame_pad = 6.0 * 2.0; // inner_margin on each side
    let mixer_width = (num_channels as f32 * per_fader
        + (num_channels.saturating_sub(1)) as f32 * fader_spacing
        + frame_pad)
        .max(160.0); // minimum 160px for header/crossfader controls
    let center_width = mixer_width;
    let preset_hint_threshold = 80.0;

    // Channels always take priority — compute how much space they need
    let left_channels_total = left_count as f32 * ch_card_width;
    let right_channels_total = right_count as f32 * ch_card_width;
    let max_channels_side = left_channels_total.max(right_channels_total);
    let all_channels_and_center = max_channels_side * 2.0 + center_width;

    // Does the wider channel side overflow the available space?
    let channels_overflow = all_channels_and_center > available;

    if channels_overflow {
        // Too many channels — horizontal scroll across full width
        egui::ScrollArea::horizontal()
            .id_salt("central_channel_scroll")
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for i in 0..left_count {
                        if let Some(ch) = data.channels.get(i) {
                            ui.vertical(|ui| {
                                ui.set_width(ch_card_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                        }
                    }
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(center_width);
                        render_mixer_box(ui, data, actions);
                        // Sequence builder — same width as mixer
                        if has_sequences {
                            ui.add_space(4.0);
                            render_sequence_builder(ui, data, actions);
                        }
                        // + Sequence button — centered below mixer
                        if data.channel_count >= 2 {
                            ui.add_space(4.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                if ui
                                    .small_button("+ Sequence")
                                    .on_hover_text("Create a new transition sequence")
                                    .clicked()
                                {
                                    actions.sequence_actions.push(SequenceAction::Create);
                                }
                            });
                        }
                    });
                    ui.separator();
                    for i in 0..right_count {
                        let ch_idx = left_count + i;
                        if let Some(ch) = data.channels.get(ch_idx) {
                            ui.vertical(|ui| {
                                ui.set_width(ch_card_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                        }
                    }
                });
            });
    } else {
        // Channels fit — compute equal empty hint space on each side.
        // Use the LARGER channel side to determine side_width so both sides are equal.
        // Empty space = side_width - that side's channels. Both sides get the same side_width.
        let side_width = ((available - center_width) / 2.0).max(0.0);
        // Empty space is limited by the side with MORE channels (so it doesn't overflow)
        let empty_each = (side_width - max_channels_side).max(0.0);

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;

            // Left side: hint on far left, channels adjacent to mixer
            ui.allocate_ui(egui::vec2(side_width, panel_height), |ui| {
                ui.horizontal_top(|ui| {
                    if empty_each > preset_hint_threshold {
                        render_new_channel_drop_zone(ui, empty_each, data, actions, 0);
                    } else if empty_each > 1.0 {
                        ui.add_space(empty_each);
                    }
                    for i in 0..left_count {
                        if let Some(ch) = data.channels.get(i) {
                            ui.vertical(|ui| {
                                ui.set_width(ch_card_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                        }
                    }
                });
            });

            // Center column — force vertical layout (parent is horizontal_top)
            ui.allocate_ui_with_layout(
                egui::vec2(center_width, panel_height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.separator();
                    render_mixer_box(ui, data, actions);
                    // Sequence builder — same width as mixer
                    if has_sequences {
                        ui.add_space(4.0);
                        render_sequence_builder(ui, data, actions);
                    }
                    // + Sequence button — centered below mixer
                    if data.channel_count >= 2 {
                        ui.add_space(4.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            if ui
                                .small_button("+ Sequence")
                                .on_hover_text("Create a new transition sequence")
                                .clicked()
                            {
                                actions.sequence_actions.push(SequenceAction::Create);
                            }
                        });
                    }
                },
            );

            // Right side: channels adjacent to mixer, hint on far right
            ui.allocate_ui(egui::vec2(side_width, panel_height), |ui| {
                ui.separator();
                ui.horizontal_top(|ui| {
                    for i in 0..right_count {
                        let ch_idx = left_count + i;
                        if let Some(ch) = data.channels.get(ch_idx) {
                            ui.vertical(|ui| {
                                ui.set_width(ch_card_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                        }
                    }
                    if empty_each > preset_hint_threshold {
                        render_new_channel_drop_zone(ui, empty_each, data, actions, 1);
                    }
                });
            });
        });
    }
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
        .stroke(egui::Stroke::new(
            1.0_f32,
            egui::Color32::from_rgb(60, 60, 80),
        ))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🎚 Mixer").strong().size(13.0));
                if ui
                    .small_button("➕ Ch")
                    .on_hover_text("Add a new channel")
                    .clicked()
                {
                    actions.add_channel = true;
                }
            });
            ui.add_space(4.0);

            // Channel volume faders (vertical, side by side) — N channels
            let fader_height = 100.0;
            let mut opacities: Vec<f32> = data.channels.iter().map(|c| c.opacity).collect();
            let blend_modes_orig: Vec<BlendMode> =
                data.channels.iter().map(|c| c.blend_mode).collect();

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
                            ui.label(
                                egui::RichText::new(&ch.name)
                                    .strong()
                                    .color(color)
                                    .size(11.0),
                            );
                            // Show remove button only if more than 2 channels
                            if num_channels > 2
                                && ui
                                    .small_button("x")
                                    .on_hover_text(format!("Remove channel {}", ch.name))
                                    .clicked()
                            {
                                actions.remove_channel = Some(ch_idx);
                            }
                        });
                        // Render slider — disabled in learn mode
                        let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                        let slider_rect = if any_learn {
                            let inner = ui.scope(|ui| {
                                ui.disable();
                                let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                                    .vertical()
                                    .show_value(false);
                                ui.add_sized([18.0, fader_height], slider)
                            });
                            inner.inner.rect
                        } else {
                            let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                                .vertical()
                                .show_value(false);
                            let resp = ui.add_sized([18.0, fader_height], slider);
                            resp.rect
                        };
                        // MIDI learn: glow + click overlay
                        if data.midi_learn_active {
                            let path = format!("ch/{}/opacity", ch.uuid);
                            let is_target =
                                data.midi_learn_target.as_deref() == Some(path.as_str());
                            if is_target {
                                widgets::draw_midi_learn_selected(ui, slider_rect);
                            } else {
                                widgets::draw_midi_learn_glow(ui, slider_rect);
                            }
                            let click_id = ui.id().with(("midi_learn_ch_opacity", ch_idx));
                            let click_resp =
                                ui.interact(slider_rect, click_id, egui::Sense::click());
                            if click_resp.clicked() {
                                actions.midi_learn_select = Some(path);
                            }
                        }
                        // Keyboard learn: orange glow + click overlay
                        if data.keyboard_learn_active {
                            let path = format!("ch/{}/opacity", ch.uuid);
                            let is_target =
                                data.keyboard_learn_target.as_deref() == Some(path.as_str());
                            if is_target {
                                widgets::draw_keyboard_learn_selected(ui, slider_rect);
                            } else {
                                widgets::draw_keyboard_learn_glow(ui, slider_rect);
                            }
                            let click_id = ui.id().with(("kb_learn_ch_opacity", ch_idx));
                            let click_resp =
                                ui.interact(slider_rect, click_id, egui::Sense::click());
                            if click_resp.clicked() {
                                actions.keyboard_learn_select =
                                    Some(crate::keymap::KeyTarget::ParamPath(path));
                            }
                        }
                    });
                }
            });

            // Only emit channel updates when opacity actually changed (via the fader)
            // This avoids overwriting blend mode changes made by the blend mode selector below
            for (ch_idx, ch) in data.channels.iter().enumerate() {
                if (opacities[ch_idx] - ch.opacity).abs() > f32::EPSILON {
                    actions.channel_updates.push((
                        ch_idx,
                        opacities[ch_idx],
                        blend_modes_orig[ch_idx],
                    ));
                }
            }

            ui.add_space(4.0);
            ui.separator();

            // Crossfader — only shown for exactly 2 channels
            if use_crossfader {
                let color_a = channel_color(0);
                let color_b = channel_color(1);
                ui.label(egui::RichText::new("Crossfader").small());
                let name_a = data
                    .channels
                    .first()
                    .map(|c| c.name.as_str())
                    .unwrap_or("A");
                let name_b = data.channels.get(1).map(|c| c.name.as_str()).unwrap_or("B");
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name_a).small().color(color_a));
                    let mut crossfader = data.crossfader;
                    let slider_rect;
                    let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                    if any_learn {
                        let inner = ui.scope(|ui| {
                            ui.disable();
                            let slider =
                                egui::Slider::new(&mut crossfader, 0.0..=1.0).show_value(false);
                            ui.add_sized([ui.available_width() - 16.0, 18.0], slider)
                        });
                        slider_rect = inner.inner.rect;
                    } else {
                        let slider =
                            egui::Slider::new(&mut crossfader, 0.0..=1.0).show_value(false);
                        let resp = ui.add_sized([ui.available_width() - 16.0, 18.0], slider);
                        if resp.changed() {
                            actions.crossfader_action =
                                Some(CrossfaderAction::SetPosition(crossfader));
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
                    if data.keyboard_learn_active {
                        let is_target = data.keyboard_learn_target.as_deref() == Some("crossfader");
                        if is_target {
                            widgets::draw_keyboard_learn_selected(ui, slider_rect);
                        } else {
                            widgets::draw_keyboard_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with("kb_learn_crossfader");
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            actions.keyboard_learn_select = Some(
                                crate::keymap::KeyTarget::ParamPath("crossfader".to_string()),
                            );
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
                let auto_label = if data.crossfader < 0.5 {
                    format!("→{}", name_b)
                } else {
                    format!("→{}", name_a)
                };

                if data.auto_crossfade_active {
                    ui.add(
                        egui::ProgressBar::new(data.auto_crossfade_progress)
                            .text("Transitioning..."),
                    );
                } else {
                    // Toggle state: beats vs seconds (stored in egui memory)
                    let mode_id = egui::Id::new("crossfade_duration_is_beats");
                    let has_bpm = data.clock_bpm.is_some();
                    let is_beats =
                        has_bpm && ui.data(|d| d.get_temp::<bool>(mode_id).unwrap_or(false));

                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.label(egui::RichText::new(&auto_label).small().strong());
                        if is_beats {
                            for &beats in &[1.0f32, 2.0, 4.0, 8.0, 16.0] {
                                if ui.small_button(format!("{}", beats as u32)).clicked() {
                                    actions.crossfader_action =
                                        Some(CrossfaderAction::BeatTransition {
                                            target: auto_target,
                                            beats,
                                        });
                                }
                            }
                        } else {
                            for &secs in &[1.0f32, 2.0, 4.0, 8.0, 16.0] {
                                if ui.small_button(format!("{}", secs as u32)).clicked() {
                                    actions.crossfader_action =
                                        Some(CrossfaderAction::AutoTransition {
                                            target: auto_target,
                                            duration_secs: secs,
                                            easing: CrossfadeEasing::EaseInOut,
                                        });
                                }
                            }
                        }
                        // Unit toggle button (only show when BPM is available)
                        if has_bpm {
                            let toggle_label = if is_beats { "♩" } else { "s" };
                            if ui
                                .small_button(toggle_label)
                                .on_hover_text("Toggle beats/seconds")
                                .clicked()
                            {
                                ui.data_mut(|d| d.insert_temp(mode_id, !is_beats));
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
                        if ui
                            .selectable_label(is_opacity, "Opacity (default)")
                            .clicked()
                        {
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
        });
}

/// Render a channel column with header and its decks.
/// Always shown in a bordered box. Clicking anywhere (except deck thumbnails) selects the channel.
pub(super) fn render_channel_column(
    ui: &mut egui::Ui,
    ch: &ChannelUIInfo,
    data: &UIData,
    actions: &mut UIActions,
) {
    let accent = channel_color(ch.ch_idx);
    let ch_idx = ch.ch_idx;

    ui.push_id(format!("ch_{}", ch_idx), |ui| {
        // Detect relevant drags: library sources (not Effect) or deck moves
        let has_source_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
            .map(|p| !matches!(&*p, LibraryDrag::Effect(_)))
            .unwrap_or(false);
        let has_deck_drag = egui::DragAndDrop::has_payload_of_type::<(usize, usize)>(ui.ctx());
        let has_relevant_drag = has_source_drag || has_deck_drag;
        let is_hovering = has_relevant_drag && ui.rect_contains_pointer(ui.max_rect());
        let is_ch_selected = data.selected_channel == Some(ch_idx);

        // Always show a bordered box — glow when a relevant drag is active, intensify on hover
        let frame = if is_hovering {
            // Direct hover — strong highlight
            egui::Frame::default()
                .fill(accent.linear_multiply(0.15))
                .stroke(egui::Stroke::new(2.0_f32, accent))
                .corner_radius(4.0)
                .inner_margin(2.0)
        } else if has_relevant_drag {
            // Drag active but not hovering this channel — subtle glow
            egui::Frame::default()
                .fill(accent.linear_multiply(0.08))
                .stroke(egui::Stroke::new(1.5_f32, accent.linear_multiply(0.5)))
                .corner_radius(4.0)
                .inner_margin(2.0)
        } else if is_ch_selected {
            egui::Frame::default()
                .stroke(egui::Stroke::new(2.0_f32, accent.linear_multiply(0.6)))
                .corner_radius(4.0)
                .inner_margin(2.0)
        } else {
            egui::Frame::default()
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgb(50, 50, 60),
                ))
                .corner_radius(4.0)
                .inner_margin(2.0)
        };

        frame.show(ui, |ui| {
            ui.vertical(|ui| {
                // Channel header — clickable to select channel
                let header_frame = if is_ch_selected {
                    egui::Frame::default()
                        .fill(accent.linear_multiply(0.15))
                        .corner_radius(3.0)
                        .inner_margin(2.0)
                } else {
                    egui::Frame::default().inner_margin(2.0)
                };
                header_frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let header_resp = ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("▌ {}", ch.name))
                                    .strong()
                                    .color(accent)
                                    .size(16.0),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if header_resp.clicked() {
                            actions.select_channel = Some(ch_idx);
                        }

                        // Blend mode dropdown — right-aligned in header
                        let all_modes = BlendMode::all();
                        let current = all_modes
                            .iter()
                            .position(|m| *m == ch.blend_mode)
                            .unwrap_or(0);
                        let mut selected = current;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::ComboBox::from_id_salt(format!("ch_blend_{}", ch_idx))
                                .selected_text(all_modes[selected].short_name())
                                .width(50.0)
                                .show_ui(ui, |ui| {
                                    for (i, mode) in all_modes.iter().enumerate() {
                                        ui.selectable_value(&mut selected, i, mode.short_name());
                                    }
                                });
                        });
                        if selected != current {
                            let new_blend = all_modes[selected];
                            if let Some(entry) =
                                actions.channel_updates.iter_mut().find(|e| e.0 == ch_idx)
                            {
                                entry.2 = new_blend;
                            } else {
                                actions
                                    .channel_updates
                                    .push((ch_idx, ch.opacity, new_blend));
                            }
                        }
                    });
                });

                let sep_resp = ui.separator();
                if sep_resp.interact(egui::Sense::click()).clicked() {
                    actions.select_channel = Some(ch_idx);
                }

                // Deck stack (single column, vertical)
                egui::ScrollArea::vertical()
                    .id_salt(format!("ch_scroll_{}", ch_idx))
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: false,
                        scroll_bar: true,
                        mouse_wheel: true,
                    })
                    .show(ui, |ui| {
                        if ch.decks.is_empty() {
                            let hint = if is_hovering {
                                "➕ Drop here"
                            } else {
                                "No decks — drag source here"
                            };
                            let hint_color = if is_hovering {
                                accent
                            } else {
                                egui::Color32::from_rgb(120, 120, 130)
                            };
                            let empty_resp = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(hint).weak().small().color(hint_color),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if empty_resp.clicked() {
                                actions.select_channel = Some(ch_idx);
                            }
                        }
                        let is_deck_drag_active =
                            egui::DragAndDrop::has_payload_of_type::<(usize, usize)>(ui.ctx());
                        let drag_is_same_ch =
                            egui::DragAndDrop::payload::<(usize, usize)>(ui.ctx())
                                .map(|p| p.0 == ch_idx)
                                .unwrap_or(false);

                        for (i, deck) in ch.decks.iter().enumerate() {
                            // Drop zone BEFORE each deck (for reordering within channel)
                            if is_deck_drag_active && drag_is_same_ch {
                                let drop_zone = ui.allocate_response(
                                    egui::vec2(ui.available_width(), 6.0),
                                    egui::Sense::click(),
                                );
                                if drop_zone.contains_pointer() {
                                    ui.painter().rect_filled(
                                        drop_zone.rect,
                                        2.0,
                                        accent.linear_multiply(0.5),
                                    );
                                }
                                if let Some(payload) =
                                    drop_zone.dnd_release_payload::<(usize, usize)>()
                                {
                                    let (src_ch, src_deck) = *payload;
                                    if src_ch == ch_idx && src_deck != i {
                                        let to = if src_deck < i { i - 1 } else { i };
                                        if to != src_deck {
                                            actions.deck_to_reorder = Some((ch_idx, src_deck, to));
                                        }
                                    }
                                }
                            }

                            render_deck_thumbnail(ui, ch_idx, deck, accent, data, actions);
                            ui.add_space(2.0);
                        }

                        // Drop zone AFTER last deck (for moving to end)
                        if is_deck_drag_active && drag_is_same_ch && !ch.decks.is_empty() {
                            let drop_zone = ui.allocate_response(
                                egui::vec2(ui.available_width(), 6.0),
                                egui::Sense::click(),
                            );
                            if drop_zone.contains_pointer() {
                                ui.painter().rect_filled(
                                    drop_zone.rect,
                                    2.0,
                                    accent.linear_multiply(0.5),
                                );
                            }
                            if let Some(payload) = drop_zone.dnd_release_payload::<(usize, usize)>()
                            {
                                let (src_ch, src_deck) = *payload;
                                let last = ch.decks.len() - 1;
                                if src_ch == ch_idx && src_deck != last {
                                    actions.deck_to_reorder = Some((ch_idx, src_deck, last));
                                }
                            }
                        }

                        // Drop hint when dragging over a channel with existing decks
                        if is_hovering && !ch.decks.is_empty() {
                            ui.label(
                                egui::RichText::new("➕ Drop to add deck")
                                    .small()
                                    .color(accent),
                            );
                        }

                        // Remaining space — clickable to select channel + drop zone for deck moves
                        let drop_resp = ui.allocate_response(
                            egui::vec2(ui.available_width(), 20.0_f32.max(ui.available_height())),
                            egui::Sense::click() | egui::Sense::hover(),
                        );
                        if drop_resp.clicked() {
                            actions.select_channel = Some(ch_idx);
                        }
                        if let Some(payload) = drop_resp.dnd_release_payload::<(usize, usize)>() {
                            let (src_ch, src_deck) = *payload;
                            if src_ch != ch_idx && src_ch < data.channels.len() {
                                actions.deck_to_move = Some((src_ch, src_deck, ch_idx));
                            }
                        }

                        // Channel FX chain (compact) — clickable to select channel
                        if !ch.effects.is_empty() {
                            ui.add_space(4.0);
                            let fx_resp = ui.add(
                                egui::Label::new(
                                    egui::RichText::new("🔮 Ch FX").small().color(accent),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if fx_resp.clicked() {
                                actions.select_channel = Some(ch_idx);
                            }
                            for (_uuid, name, enabled, _) in ch.effects.iter() {
                                ui.horizontal(|ui| {
                                    let label = if *enabled {
                                        egui::RichText::new(name).small()
                                    } else {
                                        egui::RichText::new(name).small().strikethrough().weak()
                                    };
                                    let fx_item =
                                        ui.add(egui::Label::new(label).sense(egui::Sense::click()));
                                    if fx_item.clicked() {
                                        actions.select_channel = Some(ch_idx);
                                    }
                                });
                            }
                        }
                    });
            });
        }); // frame.show

        // Store this channel's screen rect for the deferred DnD drop handler
        let ch_rect = ui.min_rect();
        ui.ctx().memory_mut(|mem| {
            mem.data
                .insert_temp(egui::Id::new("ch_drop_rect").with(ch_idx), ch_rect);
        });
    });
}

/// Render a drop zone in empty mixer side space that creates a new channel on drop.
/// Accepts library drags (except Effect) and deck drags from existing channels.
/// `side` distinguishes left (0) vs right (1) so both zones can coexist.
fn render_new_channel_drop_zone(
    ui: &mut egui::Ui,
    max_width: f32,
    data: &UIData,
    actions: &mut UIActions,
    side: usize,
) {
    let has_library_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
        .map(|p| !matches!(&*p, LibraryDrag::Effect(_)))
        .unwrap_or(false);
    let has_deck_drag = egui::DragAndDrop::has_payload_of_type::<(usize, usize)>(ui.ctx());
    let relevant_drag = has_library_drag || has_deck_drag;
    // Pre-compute the zone rect from the cursor — ui.max_rect() is too broad
    // (it spans the full remaining horizontal space including adjacent channels)
    let hint_height = ui.available_height().max(60.0);
    let zone_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(max_width, hint_height));
    let is_hovering = relevant_drag
        && ui
            .ctx()
            .input(|i| i.pointer.hover_pos().is_some_and(|p| zone_rect.contains(p)));

    let accent = egui::Color32::from_rgb(100, 200, 255);
    let (stroke, fill, label_text, label_color) = if is_hovering {
        // Direct hover — strong highlight (matches channel hover intensity)
        (
            egui::Stroke::new(2.0_f32, accent),
            accent.linear_multiply(0.15),
            "➕ Drop to create channel",
            accent,
        )
    } else if relevant_drag {
        // Drag active, not hovering — subtle glow (matches channel ambient glow)
        (
            egui::Stroke::new(1.5_f32, accent.linear_multiply(0.5)),
            accent.linear_multiply(0.08),
            "➕ Drop to create channel",
            accent.linear_multiply(0.5),
        )
    } else {
        // Idle
        (
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 60, 80)),
            egui::Color32::TRANSPARENT,
            "➕ Drop here to add channel",
            egui::Color32::from_rgb(100, 100, 120),
        )
    };
    let resp = ui.allocate_ui(egui::vec2(max_width, hint_height), |ui| {
        let frame_resp = egui::Frame::default()
            .inner_margin(8.0)
            .corner_radius(6.0)
            .fill(fill)
            .stroke(stroke)
            .show(ui, |ui| {
                ui.set_min_height(hint_height - 16.0);
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(label_text)
                            .weak()
                            .size(12.0)
                            .color(label_color),
                    );
                });
            });
        frame_resp.response
    });

    // Store rect for the deferred library DnD handler (indexed by side)
    let zone_rect = resp.response.rect;
    ui.ctx().memory_mut(|mem| {
        mem.data
            .insert_temp(egui::Id::new("new_ch_drop_rect").with(side), zone_rect);
    });

    // Handle immediate deck drag (not deferred — egui native DnD)
    if let Some(payload) = resp.response.dnd_release_payload::<(usize, usize)>() {
        let (src_ch, src_deck) = *payload;
        let new_ch_idx = data.channels.len();
        actions.add_channel = true;
        actions.deck_to_move = Some((src_ch, src_deck, new_ch_idx));
        log::info!(
            "Deck drag -> new channel: ch{} deck{} -> new ch{}",
            src_ch,
            src_deck,
            new_ch_idx
        );
    }
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
        let border_color = if is_selected {
            accent
        } else {
            accent.linear_multiply(0.3)
        };
        let border_width = if is_selected { 2.0_f32 } else { 1.0_f32 };

        // Use manual rect-based painting to avoid egui layout overlap issues.
        // Total card height = preview_height + name_row(16) + button_row(20) + spacing(8) + padding(8)
        let name_row_h = 16.0;
        let button_row_h = 20.0;
        let spacing = 4.0;
        let padding = 4.0;
        let total_h =
            padding + preview_height + spacing + name_row_h + spacing + button_row_h + padding;

        let card_size = egui::vec2(card_width + padding * 2.0, total_h);
        let (card_rect, card_resp) =
            ui.allocate_exact_size(card_size, egui::Sense::click_and_drag());

        // MIDI learn mode: glow on deck card, click to select trigger
        if data.midi_learn_active {
            let trigger_path = format!("deck/{}/trigger", deck.uuid);
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
        // Keyboard learn mode: orange glow on deck card
        if data.keyboard_learn_active {
            let trigger_path = format!("deck/{}/trigger", deck.uuid);
            let is_target = data.keyboard_learn_target.as_deref() == Some(trigger_path.as_str());
            if is_target {
                widgets::draw_keyboard_learn_selected(ui, card_rect);
            } else {
                widgets::draw_keyboard_learn_glow(ui, card_rect);
            }
            if card_resp.clicked() {
                actions.keyboard_learn_select =
                    Some(crate::keymap::KeyTarget::ParamPath(trigger_path));
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
                let layer =
                    egui::LayerId::new(egui::Order::Tooltip, ui.id().with("deck_drag_ghost"));
                let painter = ui.ctx().layer_painter(layer);
                painter.rect_filled(
                    ghost_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(80, 120, 200, 120),
                );
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
        ui.painter().rect_filled(
            card_rect,
            4.0,
            egui::Color32::from_rgba_unmultiplied(25, 25, 35, bg_alpha),
        );
        ui.painter().rect_stroke(
            card_rect,
            4.0,
            egui::Stroke::new(border_width, border_color),
            egui::StrokeKind::Outside,
        );

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
            ui.painter()
                .rect_filled(preview_rect, 3.0, egui::Color32::from_rgb(30, 30, 40));
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
                    crate::channel::DeckTransitionPhase::Playing { .. } => {
                        ("▶", egui::Color32::from_rgb(80, 200, 80))
                    }
                    crate::channel::DeckTransitionPhase::Transitioning { .. } => {
                        ("🔄", egui::Color32::from_rgb(200, 160, 40))
                    }
                    crate::channel::DeckTransitionPhase::Done => {
                        ("✓", egui::Color32::from_rgb(100, 100, 100))
                    }
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
                    ui.painter()
                        .rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(200, 160, 40));
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
                        ui.painter().rect_filled(
                            bar_rect,
                            0.0,
                            egui::Color32::from_rgb(80, 200, 80),
                        );
                    }
                }
            }
        }

        // Click on card to select deck (only when not in learn mode — learn mode click selects trigger)
        if !data.midi_learn_active && !data.keyboard_learn_active && card_resp.clicked() {
            actions.select_deck = Some((ch_idx, idx));
        }

        // Vertical opacity slider — use a child ui placed at the slider rect
        let mut slider_ui = ui.new_child(egui::UiBuilder::new().max_rect(slider_rect));
        let any_learn = data.midi_learn_active || data.keyboard_learn_active;
        let op_slider_rect = if any_learn {
            let inner = slider_ui.scope(|ui| {
                ui.disable();
                let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
                    .vertical()
                    .show_value(false);
                ui.add_sized([slider_width, preview_height], slider)
            });
            inner.inner.rect
        } else {
            let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
                .vertical()
                .show_value(false);
            let resp = slider_ui.add_sized([slider_width, preview_height], slider);
            resp.rect
        };
        if data.midi_learn_active {
            let opacity_path = format!("deck/{}/opacity", deck.uuid);
            let is_target = data.midi_learn_target.as_deref() == Some(opacity_path.as_str());
            if is_target {
                widgets::draw_midi_learn_selected(&slider_ui, op_slider_rect);
            } else {
                widgets::draw_midi_learn_glow(&slider_ui, op_slider_rect);
            }
            let click_id = slider_ui
                .id()
                .with(("midi_learn_deck_opacity", ch_idx, idx));
            let click_resp = slider_ui.interact(op_slider_rect, click_id, egui::Sense::click());
            if click_resp.clicked() {
                actions.midi_learn_select = Some(opacity_path);
            }
        }
        if data.keyboard_learn_active {
            let opacity_path = format!("deck/{}/opacity", deck.uuid);
            let is_target = data.keyboard_learn_target.as_deref() == Some(opacity_path.as_str());
            if is_target {
                widgets::draw_keyboard_learn_selected(&slider_ui, op_slider_rect);
            } else {
                widgets::draw_keyboard_learn_glow(&slider_ui, op_slider_rect);
            }
            let click_id = slider_ui.id().with(("kb_learn_deck_opacity", ch_idx, idx));
            let click_resp = slider_ui.interact(op_slider_rect, click_id, egui::Sense::click());
            if click_resp.clicked() {
                actions.keyboard_learn_select =
                    Some(crate::keymap::KeyTarget::ParamPath(opacity_path));
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
            ui.painter().rect_filled(
                bar_rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(200, 60, 60, 80),
            );
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
        let any_learn = data.midi_learn_active || data.keyboard_learn_active;
        btn_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mute_resp = ui.selectable_label(mute, egui::RichText::new("M").small());
            if any_learn {
                let mute_path = format!("deck/{}/mute", deck.uuid);
                if data.midi_learn_active {
                    let is_target = data.midi_learn_target.as_deref() == Some(mute_path.as_str());
                    if is_target {
                        widgets::draw_midi_learn_selected(ui, mute_resp.rect);
                    } else {
                        widgets::draw_midi_learn_glow(ui, mute_resp.rect);
                    }
                    if mute_resp.clicked() {
                        actions.midi_learn_select = Some(mute_path.clone());
                    }
                }
                if data.keyboard_learn_active {
                    let is_target =
                        data.keyboard_learn_target.as_deref() == Some(mute_path.as_str());
                    if is_target {
                        widgets::draw_keyboard_learn_selected(ui, mute_resp.rect);
                    } else {
                        widgets::draw_keyboard_learn_glow(ui, mute_resp.rect);
                    }
                    if mute_resp.clicked() {
                        actions.keyboard_learn_select =
                            Some(crate::keymap::KeyTarget::ParamPath(mute_path));
                    }
                }
            } else if mute_resp.clicked() {
                mute = !mute;
            }
            let solo_resp = ui.selectable_label(solo, egui::RichText::new("S").small());
            if any_learn {
                let solo_path = format!("deck/{}/solo", deck.uuid);
                if data.midi_learn_active {
                    let is_target = data.midi_learn_target.as_deref() == Some(solo_path.as_str());
                    if is_target {
                        widgets::draw_midi_learn_selected(ui, solo_resp.rect);
                    } else {
                        widgets::draw_midi_learn_glow(ui, solo_resp.rect);
                    }
                    if solo_resp.clicked() {
                        actions.midi_learn_select = Some(solo_path.clone());
                    }
                }
                if data.keyboard_learn_active {
                    let is_target =
                        data.keyboard_learn_target.as_deref() == Some(solo_path.as_str());
                    if is_target {
                        widgets::draw_keyboard_learn_selected(ui, solo_resp.rect);
                    } else {
                        widgets::draw_keyboard_learn_glow(ui, solo_resp.rect);
                    }
                    if solo_resp.clicked() {
                        actions.keyboard_learn_select =
                            Some(crate::keymap::KeyTarget::ParamPath(solo_path));
                    }
                }
            } else if solo_resp.clicked() {
                solo = !solo;
            }
            if !any_learn && ui.small_button(egui::RichText::new("x").small()).clicked() {
                actions.deck_to_remove = Some((ch_idx, idx));
            }
        });

        // Only push deck updates when something actually changed from the UI controls here
        // (opacity slider, solo/mute buttons). This avoids overwriting blend mode changes
        // made in the detail panel's blend mode selector.
        if (opacity - deck.opacity).abs() > f32::EPSILON || solo != deck.solo || mute != deck.mute {
            actions
                .deck_updates
                .push((ch_idx, idx, opacity, deck.blend_mode, solo, mute));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_central_panel_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_central_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_central_panel_smoke_stage_editor() {
        let mut data = UIData::test_fixture();
        data.stage_editor_open = true;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_central_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_mixer_box_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_mixer_box(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_channel_column_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_channel_column(ui, &data.channels[0], &data, &mut actions);
        });
    }
}
