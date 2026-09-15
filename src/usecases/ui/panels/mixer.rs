//! Central panel, mixer box, channel columns, deck thumbnails.

use super::super::{ChannelUIInfo, DeckDrag, DeckUIInfo, LibraryDrag, UIActions, UIData, widgets};
use super::clipboard_menu;
use super::dnd::{publish_channel_surface_fx, publish_deck_surface_fx};
use super::macros::render_macro_column;
use super::sequence::render_sequence_builder;
use super::stage::render_stage_editor;
use super::utils::channel_color;
use crate::BlendMode;
use crate::engine::EngineCommand;
use crate::mixer::CrossfadeEasing;

fn effect_drag_active(ctx: &egui::Context) -> bool {
    egui::DragAndDrop::payload::<LibraryDrag>(ctx)
        .is_some_and(|p| matches!(&*p, LibraryDrag::Effect(_)))
}

fn fx_accent() -> egui::Color32 {
    egui::Color32::from_rgb(100, 200, 255)
}

pub(super) fn render_central_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    if data.stage_editor_open {
        render_stage_editor(ui, data, actions);
        return;
    }

    // Arrangement is a view of the same decks, so it replaces the central area
    // and nothing else. The stage editor wins when both are open: it is a modal
    // task, while arrangement is a way of looking at the scene.
    if data.arrangement_mode_open {
        super::arrangement::render_arrangement(ui, data, actions);
        return;
    }

    let available = ui.available_width();
    let panel_height = ui.available_height();
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
                    // The scrolling layout carries the same two bands as the radiating one.
                    // Dropping them here would mean a rig with enough channels to overflow
                    // silently loses its lighting half.
                    let side = ch_card_width * left_count as f32;
                    ui.allocate_ui(egui::vec2(side, panel_height), |ui| {
                        render_side_bands(
                            ui,
                            side,
                            panel_height,
                            0..left_count,
                            0.0,
                            preset_hint_threshold,
                            ch_card_width,
                            true,
                            data,
                            actions,
                        );
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(center_width);
                        render_center_stack(ui, data, actions);
                    });
                    ui.separator();
                    let right_side = ch_card_width * right_count as f32;
                    ui.allocate_ui(egui::vec2(right_side, panel_height), |ui| {
                        render_side_bands(
                            ui,
                            right_side,
                            panel_height,
                            left_count..num_channels,
                            0.0,
                            preset_hint_threshold,
                            ch_card_width,
                            false,
                            data,
                            actions,
                        );
                    });
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

            // Left side: hint on far left, channels adjacent to mixer. The side is a vertical
            // stack of bands so the channel lane crosses both, while the mixer column beside it
            // spans the full height as one constant.
            // See /spec/lighting-routing.md § Two bands, crossed by the channel lanes.
            ui.allocate_ui(egui::vec2(side_width, panel_height), |ui| {
                render_side_bands(
                    ui,
                    side_width,
                    panel_height,
                    0..left_count,
                    empty_each,
                    preset_hint_threshold,
                    ch_card_width,
                    true,
                    data,
                    actions,
                );
            });

            // Center column — force vertical layout (parent is horizontal_top)
            ui.allocate_ui_with_layout(
                egui::vec2(center_width, panel_height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.separator();
                    render_center_stack(ui, data, actions);
                },
            );

            // Right side: channels adjacent to mixer, hint on far right
            ui.allocate_ui(egui::vec2(side_width, panel_height), |ui| {
                ui.separator();
                render_side_bands(
                    ui,
                    side_width,
                    panel_height,
                    left_count..num_channels,
                    empty_each,
                    preset_hint_threshold,
                    ch_card_width,
                    false,
                    data,
                    actions,
                );
            });
        });
    }
}

/// Render one side of the radiating layout as a vertical stack of bands.
///
/// The channel lanes run through both bands while the mixer column beside them spans the whole
/// height, which is what makes `Ch A` read as one channel rather than two stacked containers.
/// A collapsed band gives its height to the other, so with LIGHTS collapsed this is byte for
/// byte the layout Varda had before lighting existed.
///
/// The margin at the outer edge is **one** drop zone spanning both bands, drawn beside the band
/// stack rather than inside each band. It is universal: a shader or a lighting deck released
/// anywhere in it creates a channel and lands in the correct half. Drawing it per-band made two
/// stacked zones with a seam between them, which is not what an empty margin is.
/// See /spec/lighting-routing.md § Two bands, crossed by the channel lanes.
#[allow(clippy::too_many_arguments)]
fn render_side_bands(
    ui: &mut egui::Ui,
    side_width: f32,
    panel_height: f32,
    channels: std::ops::Range<usize>,
    empty_each: f32,
    preset_hint_threshold: f32,
    ch_card_width: f32,
    hint_first: bool,
    data: &UIData,
    actions: &mut UIActions,
) {
    use super::lighting::{
        band_heights, lights_band_expanded, lights_band_offered, render_collapsed_band,
    };

    let (video_height, lights_height) = band_heights(ui.ctx(), data, panel_height);
    let lights_expanded = lights_band_expanded(ui.ctx(), data);
    // Offered as soon as there is anything lighting-related in the scene, so the collapsed strip
    // is always there to click once a rig or a lighting deck exists.
    let offer_lights = lights_band_offered(ui.ctx(), data);

    // `side` is the margin's identity, and there is exactly one per side: 0 left, 1 right.
    let side = usize::from(!hint_first);

    ui.horizontal_top(|ui| {
        if hint_first {
            render_side_hint(
                ui,
                empty_each,
                preset_hint_threshold,
                panel_height,
                data,
                actions,
                side,
            );
        }

        ui.vertical(|ui| {
            // ── VIDEO band ──────────────────────────────────────────────
            if data.video_band_open {
                ui.allocate_ui(egui::vec2(side_width, video_height), |ui| {
                    ui.horizontal_top(|ui| {
                        for i in channels.clone() {
                            if let Some(ch) = data.channels.get(i) {
                                render_channel_column_scrolled(
                                    ui,
                                    ch,
                                    data,
                                    actions,
                                    ch_card_width,
                                    video_height,
                                );
                            }
                        }
                    });
                });
            } else if hint_first {
                let mut open = data.video_band_open;
                if render_collapsed_band(ui, "VIDEO", false, &mut open) {
                    actions.session.toggle_video_band = true;
                }
            }

            // ── LIGHTS band ─────────────────────────────────────────────
            if !offer_lights {
                return;
            }
            if lights_expanded {
                ui.allocate_ui(egui::vec2(side_width, lights_height), |ui| {
                    ui.horizontal_top(|ui| {
                        for i in channels.clone() {
                            if let Some(ch) = data.channels.get(i) {
                                super::lighting::render_lighting_column(
                                    ui,
                                    &ch.uuid,
                                    ch.ch_idx,
                                    ch_card_width,
                                    lights_height,
                                    data,
                                    actions,
                                );
                            }
                        }
                    });
                });
            } else if hint_first {
                let mut open = data.lights_band_open;
                if render_collapsed_band(ui, "LIGHTS", true, &mut open) {
                    actions.session.toggle_lights_band = true;
                }
            }
        });

        if !hint_first {
            render_side_hint(
                ui,
                empty_each,
                preset_hint_threshold,
                panel_height,
                data,
                actions,
                side,
            );
        }
    });
}

/// The empty-space hint at the outer edge of a side.
///
/// `height` is the whole panel's, not one band's: this is a single zone spanning both bands.
fn render_side_hint(
    ui: &mut egui::Ui,
    empty_each: f32,
    threshold: f32,
    height: f32,
    data: &UIData,
    actions: &mut UIActions,
    side: usize,
) {
    if empty_each > threshold {
        render_new_channel_drop_zone(ui, empty_each, height, data, actions, side);
    } else if empty_each > 1.0 {
        ui.add_space(empty_each);
    }
}

/// A single channel column, wrapped in a vertical scroll area so a tall deck
/// stack scrolls independently instead of overflowing the panel.
fn render_channel_column_scrolled(
    ui: &mut egui::Ui,
    ch: &ChannelUIInfo,
    data: &UIData,
    actions: &mut UIActions,
    width: f32,
    height: f32,
) {
    ui.vertical(|ui| {
        ui.set_width(width);
        egui::ScrollArea::vertical()
            .id_salt(("ch_col_scroll", ch.ch_idx))
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                render_channel_column(ui, ch, data, actions);
            });
    });
}

/// The center column stack (mixer box, sequence builder, macros), wrapped in a
/// vertical scroll area so many sequences/macros scroll instead of overflowing.
fn render_center_stack(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let height = ui.available_height();
    egui::ScrollArea::vertical()
        .id_salt("center_column_scroll")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            render_mixer_box(ui, data, actions);
            // Sequence builder — same width as mixer
            if !data.sequences.is_empty() {
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
                        actions.commands.push(EngineCommand::CreateSequence);
                    }
                });
            }
            // Macro controls — compact widgets in the center column
            render_macro_column(ui, data, actions);
            // Cue pads — the arrangement's marks, reachable from the desk
            super::cue_bank::render_cue_bank(ui, data, actions);
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
                    actions.commands.push(EngineCommand::AddChannel);
                }
            });
            ui.add_space(4.0);

            // Channel volume faders (vertical, side by side) — N channels
            let fader_height = 100.0;
            let mut opacities: Vec<f32> = data.channels.iter().map(|c| c.opacity).collect();

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
                                actions.session.remove_channel = Some(ch_idx);
                            }
                        });
                        widgets::modulation_menu_for_key(
                            ui,
                            format!("ch_mod_{ch_idx}"),
                            &crate::arrangement::channel_opacity_param_key(&ch.uuid),
                            &data.modulation_sources,
                            &data.modulation_assignments,
                            &mut actions.commands,
                        );
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
                            // A held fader drag is a single undo gesture.
                            if resp.dragged() {
                                actions.session.gesture_active = true;
                            }
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
                                actions.session.midi_learn_select = Some(path);
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
                                actions.session.keyboard_learn_select =
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
                    actions.commands.push(EngineCommand::SetChannelOpacity {
                        channel_uuid: ch.uuid.clone(),
                        opacity: opacities[ch_idx],
                    });
                }
            }

            ui.add_space(4.0);
            ui.separator();

            // Crossfader — only shown for exactly 2 channels
            if use_crossfader {
                let color_a = channel_color(0);
                let color_b = channel_color(1);
                ui.label(egui::RichText::new("Crossfader").small());
                let name_a = data.channels.first().map_or("A", |c| c.name.as_str());
                let name_b = data.channels.get(1).map_or("B", |c| c.name.as_str());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name_a).small().color(color_a));
                    let mut crossfader = data.crossfader;
                    let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                    let slider_rect = if any_learn {
                        let inner = ui.scope(|ui| {
                            ui.disable();
                            let slider =
                                egui::Slider::new(&mut crossfader, 0.0..=1.0).show_value(false);
                            ui.add_sized([ui.available_width() - 16.0, 18.0], slider)
                        });
                        inner.inner.rect
                    } else {
                        let slider =
                            egui::Slider::new(&mut crossfader, 0.0..=1.0).show_value(false);
                        let resp = ui.add_sized([ui.available_width() - 16.0, 18.0], slider);
                        if resp.changed() {
                            actions
                                .commands
                                .push(EngineCommand::SetCrossfader(crossfader));
                        }
                        resp.rect
                    };
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
                            actions.session.midi_learn_select = Some("crossfader".to_string());
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
                            actions.session.keyboard_learn_select = Some(
                                crate::keymap::KeyTarget::ParamPath("crossfader".to_string()),
                            );
                        }
                    }
                    ui.label(egui::RichText::new(name_b).small().color(color_b));
                });

                // Snap buttons
                ui.horizontal(|ui| {
                    if ui.small_button(format!("⏮ {name_a}")).clicked() {
                        actions.commands.push(EngineCommand::SetCrossfader(0.0));
                    }
                    if ui.small_button(format!("{name_b} ⏭")).clicked() {
                        actions.commands.push(EngineCommand::SetCrossfader(1.0));
                    }
                });

                ui.add_space(2.0);
                ui.separator();

                // Auto-transition
                let auto_target = if data.crossfader < 0.5 { 1.0 } else { 0.0 };
                let auto_label = if data.crossfader < 0.5 {
                    format!("→{name_b}")
                } else {
                    format!("→{name_a}")
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
                                    actions.commands.push(EngineCommand::BeatCrossfade {
                                        target: auto_target,
                                        beats,
                                    });
                                }
                            }
                        } else {
                            for &secs in &[1.0f32, 2.0, 4.0, 8.0, 16.0] {
                                if ui.small_button(format!("{}", secs as u32)).clicked() {
                                    actions.commands.push(EngineCommand::AutoCrossfade {
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
                    .selected_text(egui::RichText::new(format!("🔀 {current_label}")).small())
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        let is_opacity = data.active_transition_name.is_none();
                        if ui
                            .selectable_label(is_opacity, "Opacity (default)")
                            .clicked()
                        {
                            actions
                                .commands
                                .push(EngineCommand::SetTransition { shader_name: None });
                        }
                        ui.separator();
                        for name in &data.transition_names {
                            let selected = data.active_transition_name.as_ref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() {
                                actions.commands.push(EngineCommand::SetTransition {
                                    shader_name: Some(name.clone()),
                                });
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

    ui.push_id(format!("ch_{ch_idx}"), |ui| {
        // Detect relevant drags: library sources (not Effect) or deck moves
        let has_source_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
            .is_some_and(|p| !matches!(&*p, LibraryDrag::Effect(_)));
        let has_deck_drag = egui::DragAndDrop::has_payload_of_type::<DeckDrag>(ui.ctx());
        let has_fx_drag = effect_drag_active(ui.ctx());
        let has_relevant_drag = has_source_drag || has_deck_drag;
        let is_hovering = has_relevant_drag && ui.rect_contains_pointer(ui.max_rect());
        let fx_hovering = has_fx_drag && ui.rect_contains_pointer(ui.max_rect());
        let is_ch_selected = data.selected_channel == Some(ch_idx);

        // Always show a bordered box — glow when a relevant drag is active, intensify on hover.
        // The resting hairline is the vertical rule between channels; the LIGHTS band draws the
        // same one from the same helper so a lane reads as one column across both bands.
        // The effect surface wins the accent only when it is the one actually engaged.
        let fx_lane = (has_fx_drag && !has_relevant_drag) || (fx_hovering && !is_hovering);
        let frame = super::utils::channel_lane_frame(
            if fx_lane { fx_accent() } else { accent },
            super::utils::LaneBorder {
                hovered: is_hovering || fx_hovering,
                drag_active: has_relevant_drag || has_fx_drag,
                selected: is_ch_selected,
            },
        );

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
                            actions.session.select_channel = Some(ch_idx);
                        }
                        header_resp.context_menu(|ui| channel_context_menu(ui, data, actions, ch));

                        // Blend mode dropdown — right-aligned in header
                        let all_modes = BlendMode::all();
                        let current = all_modes
                            .iter()
                            .position(|m| *m == ch.blend_mode)
                            .unwrap_or(0);
                        let mut selected = current;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::ComboBox::from_id_salt(format!("ch_blend_{ch_idx}"))
                                .selected_text(all_modes[selected].short_name())
                                .width(50.0)
                                .show_ui(ui, |ui| {
                                    for (i, mode) in all_modes.iter().enumerate() {
                                        ui.selectable_value(&mut selected, i, mode.short_name());
                                    }
                                });
                        });
                        if selected != current {
                            actions.commands.push(EngineCommand::SetChannelBlendMode {
                                channel_uuid: ch.uuid.clone(),
                                mode: all_modes[selected],
                            });
                        }
                    });
                });

                let sep_resp = ui.separator();
                if sep_resp.interact(egui::Sense::click()).clicked() {
                    actions.session.select_channel = Some(ch_idx);
                }

                // Deck stack (single column, vertical)
                egui::ScrollArea::vertical()
                    .id_salt(format!("ch_scroll_{ch_idx}"))
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: egui::scroll_area::DragScroll::Never,
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
                                actions.session.select_channel = Some(ch_idx);
                            }
                        }
                        let is_deck_drag_active =
                            egui::DragAndDrop::has_payload_of_type::<DeckDrag>(ui.ctx());
                        let drag_is_same_ch = egui::DragAndDrop::payload::<DeckDrag>(ui.ctx())
                            .is_some_and(|p| ch.decks.iter().any(|d| d.uuid == p.deck_uuid));

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
                                if let Some(payload) = drop_zone.dnd_release_payload::<DeckDrag>() {
                                    // The ordinal is read here, not at drag
                                    // start, so a reorder mid-drag can't send
                                    // the wrong deck.
                                    let from_idx =
                                        ch.decks.iter().position(|d| d.uuid == payload.deck_uuid);
                                    if let Some(from_idx) = from_idx {
                                        let to = if from_idx < i { i - 1 } else { i };
                                        if to != from_idx {
                                            actions.commands.push(EngineCommand::ReorderDeck {
                                                channel_uuid: ch.uuid.clone(),
                                                from_idx,
                                                to_idx: to,
                                            });
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
                            if let Some(payload) = drop_zone.dnd_release_payload::<DeckDrag>() {
                                let last = ch.decks.len() - 1;
                                let from_idx =
                                    ch.decks.iter().position(|d| d.uuid == payload.deck_uuid);
                                if let Some(from_idx) = from_idx
                                    && from_idx != last
                                {
                                    actions.commands.push(EngineCommand::ReorderDeck {
                                        channel_uuid: ch.uuid.clone(),
                                        from_idx,
                                        to_idx: last,
                                    });
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
                            actions.session.select_channel = Some(ch_idx);
                        }
                        // The body of a column is the obvious place to aim a
                        // paste, and the only one an empty channel has.
                        drop_resp.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("{} deck area", ch.name),
                            )
                        });
                        drop_resp.context_menu(|ui| channel_context_menu(ui, data, actions, ch));
                        if let Some(payload) = drop_resp.dnd_release_payload::<DeckDrag>()
                            && !ch.decks.iter().any(|d| d.uuid == payload.deck_uuid)
                        {
                            actions.commands.push(EngineCommand::MoveDeck {
                                deck_uuid: payload.deck_uuid.clone(),
                                dst_channel_uuid: ch.uuid.clone(),
                            });
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
                                actions.session.select_channel = Some(ch_idx);
                            }
                            for (_uuid, name, enabled, _) in &ch.effects {
                                ui.horizontal(|ui| {
                                    let label = if *enabled {
                                        egui::RichText::new(name).small()
                                    } else {
                                        egui::RichText::new(name).small().strikethrough().weak()
                                    };
                                    let fx_item =
                                        ui.add(egui::Label::new(label).sense(egui::Sense::click()));
                                    if fx_item.clicked() {
                                        actions.session.select_channel = Some(ch_idx);
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
        // Effect drops resolve channel ownership from this surface (deck cards
        // publish their own rects and win by hit priority).
        publish_channel_surface_fx(ui.ctx(), &ch.uuid, ch_idx, ch_rect);
    });
}

/// Copy, duplicate, and paste for a channel, on every part of its column that
/// is the channel rather than a deck inside it.
fn channel_context_menu(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    ch: &ChannelUIInfo,
) {
    let subject = clipboard_menu::Subject::channel(&ch.uuid, &ch.name);
    clipboard_menu::items(ui, data, actions, &subject);
}

/// Render a drop zone in empty mixer side space that creates a new channel on drop.
/// Accepts library drags (except Effect) and deck drags from existing channels.
/// `side` distinguishes left (0) vs right (1) so both zones can coexist.
fn render_new_channel_drop_zone(
    ui: &mut egui::Ui,
    max_width: f32,
    height: f32,
    data: &UIData,
    actions: &mut UIActions,
    side: usize,
) {
    let has_library_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
        .is_some_and(|p| !matches!(&*p, LibraryDrag::Effect(_)));
    let has_deck_drag = egui::DragAndDrop::has_payload_of_type::<DeckDrag>(ui.ctx());
    let relevant_drag = has_library_drag || has_deck_drag;
    // A deck dropped here needs a channel that does not exist yet, so the move
    // is held until `AddChannel` has been applied and the new channel has a
    // UUID to address. See `/spec/api-addressing.md`.
    let pending_move_id = egui::Id::new("__new_ch_pending_deck_move");
    if let Some((deck_uuid, expected_len)) = ui
        .ctx()
        .memory(|mem| mem.data.get_temp::<(String, usize)>(pending_move_id))
    {
        ui.ctx()
            .memory_mut(|mem| mem.data.remove::<(String, usize)>(pending_move_id));
        match data.channels.last() {
            Some(ch) if data.channels.len() >= expected_len => {
                actions.commands.push(EngineCommand::MoveDeck {
                    deck_uuid,
                    dst_channel_uuid: ch.uuid.clone(),
                });
            }
            _ => log::warn!("Dropping deck move: the new channel was never created"),
        }
    }
    // Pre-compute the zone rect from the cursor — ui.max_rect() is too broad
    // (it spans the full remaining horizontal space including adjacent channels)
    // The panel's height, not the remaining height of one band: the margin is one zone from the
    // top of the VIDEO band to the bottom of the LIGHTS band.
    let hint_height = height.max(60.0);
    let zone_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(max_width, hint_height));
    let is_hovering = relevant_drag
        && ui
            .ctx()
            .input(|i| i.pointer.hover_pos().is_some_and(|p| zone_rect.contains(p)));

    // The zone wears the colour of what will land in it, so aiming a lighting deck at empty
    // space reads as "this will make a lighting deck" rather than as the video zone accepting it.
    let accent = if super::lighting::lighting_drag_in_flight(ui.ctx()) {
        super::lighting::lighting_accent()
    } else {
        egui::Color32::from_rgb(100, 200, 255)
    };
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

    if let Some(payload) = resp.response.dnd_release_payload::<DeckDrag>() {
        actions.commands.push(EngineCommand::AddChannel);
        ui.ctx().memory_mut(|mem| {
            mem.data.insert_temp(
                pending_move_id,
                (payload.deck_uuid.clone(), data.channels.len() + 1),
            );
        });
        log::info!("Deck drag -> new channel: deck {}", payload.deck_uuid);
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
    // The shared deck-card geometry. A lighting deck draws the same card from the same metrics.
    let card = super::utils::DeckCard::new(data.render_width, data.render_height);
    let preview_height = card.preview.y;
    let slider_width = card.slider_width;

    ui.push_id(format!("deck_{ch_idx}_{idx}"), |ui| {
        // Use manual rect-based painting to avoid egui layout overlap issues.
        let padding = card.padding;

        let card_size = card.size();
        let (card_rect, card_resp) =
            ui.allocate_exact_size(card_size, egui::Sense::click_and_drag());

        publish_deck_surface_fx(ui.ctx(), &deck.uuid, ch_idx, idx, card_rect);

        let fx_hover = effect_drag_active(ui.ctx()) && ui.rect_contains_pointer(card_rect);
        let border_color = if fx_hover {
            fx_accent()
        } else if is_selected {
            accent
        } else {
            accent.linear_multiply(0.3)
        };
        let border_width = if fx_hover || is_selected {
            2.0_f32
        } else {
            1.0_f32
        };

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
                actions.session.midi_learn_select = Some(trigger_path);
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
                actions.session.keyboard_learn_select =
                    Some(crate::keymap::KeyTarget::ParamPath(trigger_path));
            }
        }

        card_resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{} deck card", deck.name),
            )
        });
        card_resp.context_menu(|ui| {
            let subject = clipboard_menu::Subject::deck(&deck.uuid, &deck.name);
            clipboard_menu::items(ui, data, actions, &subject);
            ui.separator();
            if ui.button("Remove deck").clicked() {
                actions.commands.push(EngineCommand::RemoveDeck {
                    deck_uuid: deck.uuid.clone(),
                });
                ui.close();
            }
        });

        // Start drag: set payload for deck move between channels
        if card_resp.drag_started() {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                DeckDrag {
                    deck_uuid: deck.uuid.clone(),
                },
            );
        }

        // While dragging, show a translucent ghost at the cursor
        if card_resp.dragged()
            && let Some(pointer_pos) = ui.ctx().pointer_interact_pos()
        {
            let ghost_rect = egui::Rect::from_center_size(pointer_pos, card_size);
            let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("deck_drag_ghost"));
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

        // Draw card background + border
        let bg_alpha = if card_resp.dragged() { 100 } else { 255 };
        super::utils::paint_deck_card_frame(
            ui.painter(),
            card_rect,
            egui::Stroke::new(border_width, border_color),
            bg_alpha,
        );

        // Row 1: Preview image (left) + vertical opacity slider (right)
        let preview_rect = card.preview_rect(card_rect);
        let slider_rect = card.slider_rect(card_rect);

        // Draw preview
        if let Some(&texture_id) = data.deck_preview_textures.get(&deck.uuid) {
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

        // Arrangement state overlay. A performer looking at the mixer needs the
        // same answer the timeline gives: is this deck mine right now, or the
        // arrangement's? See /spec/arrangement.md § Live Override.
        if let Some(arrangement) = &data.arrangement {
            let held = arrangement
                .overridden_params
                .iter()
                .any(|p| *p == crate::arrangement::opacity_param_key(&deck.uuid));
            if held {
                ui.painter().circle_filled(
                    egui::pos2(preview_rect.min.x + 7.0, preview_rect.min.y + 7.0),
                    4.0,
                    egui::Color32::from_rgb(255, 170, 60),
                );
            } else if arrangement.engaged && arrangement.config.drives_deck(&deck.uuid) {
                ui.painter().text(
                    egui::pos2(preview_rect.min.x + 2.0, preview_rect.min.y + 2.0),
                    egui::Align2::LEFT_TOP,
                    "▤",
                    egui::FontId::proportional(10.0),
                    egui::Color32::from_rgb(120, 180, 255),
                );
            }
        }

        // Auto-transition indicator overlay on preview
        if let Some(ref at) = deck.auto_transition
            && at.enabled
        {
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
                    ui.painter()
                        .rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(80, 200, 80));
                }
            }
        }

        // Click on card to select deck (only when not in learn mode — learn mode click selects trigger)
        if !data.midi_learn_active && !data.keyboard_learn_active && card_resp.clicked() {
            actions.session.select_deck = Some((ch_idx, idx));
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
            // A held fader drag is a single undo gesture.
            if resp.dragged() {
                actions.session.gesture_active = true;
            }
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
                actions.session.midi_learn_select = Some(opacity_path);
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
                actions.session.keyboard_learn_select =
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
        let name_y = card.name_pos(card_rect).y;
        let display_name = super::utils::truncate_chars(&deck.name, 16);
        ui.painter().text(
            egui::pos2(card_rect.min.x + padding, name_y),
            egui::Align2::LEFT_TOP,
            &display_name,
            egui::FontId::proportional(11.0),
            accent,
        );

        // Row 3: M S x buttons — use a child ui placed at the button row
        let btn_rect = card.button_rect(card_rect);
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
                        actions.session.midi_learn_select = Some(mute_path.clone());
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
                        actions.session.keyboard_learn_select =
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
                        actions.session.midi_learn_select = Some(solo_path.clone());
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
                        actions.session.keyboard_learn_select =
                            Some(crate::keymap::KeyTarget::ParamPath(solo_path));
                    }
                }
            } else if solo_resp.clicked() {
                solo = !solo;
            }
            if !any_learn && ui.small_button(egui::RichText::new("x").small()).clicked() {
                actions.commands.push(EngineCommand::RemoveDeck {
                    deck_uuid: deck.uuid.clone(),
                });
            }
        });

        // Push a command per control that actually changed (opacity slider,
        // solo/mute buttons). Separate commands mean a change here never clobbers
        // a blend-mode edit made in the detail panel.
        if (opacity - deck.opacity).abs() > f32::EPSILON {
            actions.commands.push(EngineCommand::SetDeckOpacity {
                deck_uuid: deck.uuid.clone(),
                opacity,
            });
        }
        if solo != deck.solo {
            actions.commands.push(EngineCommand::SetDeckSolo {
                deck_uuid: deck.uuid.clone(),
                solo,
            });
        }
        if mute != deck.mute {
            actions.commands.push(EngineCommand::SetDeckMute {
                deck_uuid: deck.uuid.clone(),
                mute,
            });
        }
    });
}

#[cfg(test)]
mod tests {

    /// A channel owns two drop surfaces: its video column and its lighting zone. The lighting
    /// one published nothing at first, so a drop there resolved to no channel and was discarded
    /// in silence. Asserted by rendering and reading the published rect, because reasoning about
    /// this is exactly what went wrong.
    #[test]
    fn the_lighting_zone_publishes_a_drop_rect_for_its_channel() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_central_panel(ui, &data, &mut actions);
        });
        harness.run();

        let ctx = harness.ctx.clone();
        for ch in &data.channels {
            let key = egui::Id::new(super::super::lighting::LIGHT_DROP_RECT_KEY).with(ch.ch_idx);
            let rect = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key));
            let rect = rect
                .unwrap_or_else(|| panic!("channel {} published no lighting drop rect", ch.ch_idx));
            assert!(
                rect.width() > 0.0 && rect.height() > 0.0,
                "channel {} lighting rect is empty: {rect:?}",
                ch.ch_idx
            );
        }
    }

    /// The video column's rect must survive the lighting zone publishing its own, or dropping a
    /// shader onto a channel would stop working.
    #[test]
    fn both_drop_surfaces_coexist() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_central_panel(ui, &data, &mut actions);
        });
        harness.run();

        let ctx = harness.ctx.clone();
        let ch_idx = data.channels[0].ch_idx;
        let video = ctx.memory(|mem| {
            mem.data
                .get_temp::<egui::Rect>(egui::Id::new("ch_drop_rect").with(ch_idx))
        });
        let lights = ctx.memory(|mem| {
            mem.data.get_temp::<egui::Rect>(
                egui::Id::new(super::super::lighting::LIGHT_DROP_RECT_KEY).with(ch_idx),
            )
        });
        assert!(video.is_some(), "video column must still publish its rect");
        assert!(lights.is_some(), "lighting zone must publish its own");
    }

    /// The lighting section is part of a channel, present whether or not anything is in it.
    ///
    /// Asserted by rendering rather than by reasoning about the gating, because two earlier
    /// versions of this were logically fine and still showed nothing on screen.
    #[test]
    fn the_lighting_band_renders_in_an_empty_scene() {
        let data = UIData::test_fixture();
        let ctx = egui::Context::default();
        assert!(
            super::super::lighting::lights_band_offered(&ctx, &data),
            "a channel's lighting section is always offered"
        );
        assert!(
            super::super::lighting::lights_band_expanded(&ctx, &data),
            "and open by default, since the layout default is open"
        );

        // And it must actually render on an empty scene, not merely be permitted to.
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_central_panel(ui, &data, &mut actions);
        });
        harness.run();
    }

    /// The scrolling layout carries the same bands, so a rig with enough channels to overflow
    /// does not silently lose its lighting half.
    #[test]
    fn the_overflow_layout_renders_the_lighting_band_too() {
        let mut data = UIData::test_fixture();
        // Enough channels that the radiating layout gives way to horizontal scroll.
        while data.channels.len() < 12 {
            let mut extra = data.channels[0].clone();
            extra.ch_idx = data.channels.len();
            extra.uuid = format!("ch{:06}", data.channels.len());
            extra.name = format!("Ch {}", data.channels.len());
            data.channels.push(extra);
        }
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            ui.set_max_width(600.0);
            render_central_panel(ui, &data, &mut actions);
        });
        harness.run();
    }
    use super::*;

    /// Render the mixer band to a PNG so the layout can be looked at rather than reasoned about.
    ///
    /// Ignored by default: a development aid, not a contract. Run with
    /// `SHOT_DIR=/tmp/shots cargo test --lib -- --ignored render_band_to_png --nocapture`.
    #[test]
    #[ignore = "development aid: writes PNGs to $SHOT_DIR rather than asserting"]
    fn render_band_to_png() {
        for dragging in [false, true] {
            let mut data = UIData::test_fixture();
            data.lights_band_open = true;
            data.video_band_open = true;

            let mut actions = UIActions::new();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1360.0, 520.0))
                .build_ui(|ui| {
                    if dragging {
                        egui::DragAndDrop::set_payload(ui.ctx(), LibraryDrag::LightingDeck);
                    }
                    render_central_panel(ui, &data, &mut actions);
                });
            harness.run();
            harness.run();
            let name = if dragging { "dragging" } else { "idle" };
            let path = format!("{}/band_{name}.png", std::env::var("SHOT_DIR").unwrap());
            harness.render().expect("render").save(&path).expect("save");
            println!("wrote {path}");
        }
    }

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
