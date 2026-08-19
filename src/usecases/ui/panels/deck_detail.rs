//! Deck detail: the bottom-bar mode shown when a deck is selected.

use super::super::{
    widgets, DeckUIInfo, DepthPreproUI, EffectDrag, LibraryDrag, PointCloudUI, ScreenCaptureUI,
    TapUI, UIActions, UIData,
};
use super::utils::{
    channel_color, format_time, render_collapsed_column, render_effect_drag_ghost,
    render_effect_drag_handle, render_effect_drop_zone,
};
use crate::channel::DeckRenderFps;
use crate::engine::EngineCommand;
use crate::modulation::DEFAULT_ASSIGNMENT_AMOUNT;
use crate::params::ParamValue;
use crate::{BlendMode, ScalingMode};

/// Apply MIDI + keyboard learn affordances (glow + click-to-select) to a just-drawn
/// control. `path` is the parameter-router path the control binds to. The two learn
/// modes are mutually exclusive, so at most one overlay is active at a time.
fn learn_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: String,
    data: &UIData,
    actions: &mut UIActions,
) {
    if data.midi_learn_active {
        if data.midi_learn_target.as_deref() == Some(path.as_str()) {
            widgets::draw_midi_learn_selected(ui, rect);
        } else {
            widgets::draw_midi_learn_glow(ui, rect);
        }
        let id = ui.id().with(("midi_learn", path.as_str()));
        if ui.interact(rect, id, egui::Sense::click()).clicked() {
            actions.session.midi_learn_select = Some(path);
        }
    } else if data.keyboard_learn_active {
        if data.keyboard_learn_target.as_deref() == Some(path.as_str()) {
            widgets::draw_keyboard_learn_selected(ui, rect);
        } else {
            widgets::draw_keyboard_learn_glow(ui, rect);
        }
        let id = ui.id().with(("kb_learn", path.as_str()));
        if ui.interact(rect, id, egui::Sense::click()).clicked() {
            actions.session.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(path));
        }
    }
}

/// Render point-cloud controls for a depth-sensor deck. Values are sent
/// normalized (0.0–1.0) through the generic `deck/<uuid>/depth/<name>` param
/// path, matching the router in `src/internal/param_router.rs`. See
/// spec/depth-sensors.md.
fn render_depth_controls(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    pc: &PointCloudUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.separator();
    ui.label(egui::RichText::new("🛰 Point Cloud").strong().size(12.0));

    // (label, param name, current normalized value)
    let sliders: [(&str, &str, f32); 9] = [
        ("Yaw", "orbit_yaw", pc.orbit_yaw),
        ("Pitch", "orbit_pitch", pc.orbit_pitch),
        ("Zoom", "zoom", pc.zoom),
        ("Points", "point_size", pc.point_size),
        ("Near", "depth_min", pc.depth_min),
        ("Far", "depth_max", pc.depth_max),
        ("Seed", "seed", pc.seed),
        ("Drift", "drift", pc.drift),
        ("Disruption", "disruption", pc.disruption),
    ];
    for (label, name, current) in sliders {
        let mut v = current;
        ui.horizontal(|ui| {
            ui.label(label);
            let resp = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
            if resp.changed() {
                actions.commands.push(EngineCommand::SetParam {
                    path: format!("deck/{}/depth/{}", deck.uuid, name),
                    value: ParamValue::Float(v),
                });
            }
            learn_overlay(
                ui,
                resp.rect,
                format!("deck/{}/depth/{}", deck.uuid, name),
                data,
                actions,
            );
        });
    }

    // Color mode: 3 buckets mapped into the normalized 0..1 range.
    ui.horizontal(|ui| {
        ui.label("Color:");
        let modes = ["RGB", "Depth", "Solid"];
        let mut idx = usize::from(pc.color_mode).min(2);
        let before = idx;
        egui::ComboBox::from_id_salt("depth_color_combo")
            .selected_text(modes[idx])
            .width(70.0)
            .show_ui(ui, |ui| {
                for (i, m) in modes.iter().enumerate() {
                    ui.selectable_value(&mut idx, i, *m);
                }
            });
        if idx != before {
            // Map bucket index to a normalized value that lands in that bucket.
            let norm = (idx as f32 + 0.5) / 3.0;
            actions.commands.push(EngineCommand::SetParam {
                path: format!("deck/{}/depth/color_mode", deck.uuid),
                value: ParamValue::Float(norm),
            });
        }
    });
}

/// Render depth-preprocessor controls for a deck whose shader declared a
/// `depth_sensor` PREPROCESSOR. Values are sent normalized (0.0–1.0) through the
/// generic `deck/<uuid>/depth_prepro/<name>` param path, matching the router in
/// `src/internal/param_router.rs`. See spec/depth-sensor-preprocessor.md.
fn render_depth_prepro_controls(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    prepro: &DepthPreproUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.separator();
    ui.label(
        egui::RichText::new(format!("🛰 Depth Sensor — {}", prepro.sensor_name))
            .strong()
            .size(12.0),
    );

    // (label, param name, current normalized value)
    let sliders: [(&str, &str, f32); 6] = [
        ("Near", "near", prepro.near),
        ("Far", "far", prepro.far),
        ("Smoothing", "smoothing", prepro.smoothing),
        ("Hole Fill", "hole_fill", prepro.hole_fill),
        ("Mask Feather", "mask_feather", prepro.mask_feather),
        ("Motion Gain", "motion_gain", prepro.motion_gain),
    ];
    for (label, name, current) in sliders {
        let mut v = current;
        ui.horizontal(|ui| {
            ui.label(label);
            let resp = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
            if resp.changed() {
                actions.commands.push(EngineCommand::SetParam {
                    path: format!("deck/{}/depth_prepro/{}", deck.uuid, name),
                    value: ParamValue::Float(v),
                });
            }
            learn_overlay(
                ui,
                resp.rect,
                format!("deck/{}/depth_prepro/{}", deck.uuid, name),
                data,
                actions,
            );
        });
    }

    // Mirror is a fader-bucketed bool on the router; send the bucket centre.
    ui.horizontal(|ui| {
        let mut mirror = prepro.mirror;
        let resp = ui.checkbox(&mut mirror, "Mirror");
        if resp.changed() {
            actions.commands.push(EngineCommand::SetParam {
                path: format!("deck/{}/depth_prepro/mirror", deck.uuid),
                value: ParamValue::Float(f32::from(u8::from(mirror))),
            });
        }
        learn_overlay(
            ui,
            resp.rect,
            format!("deck/{}/depth_prepro/mirror", deck.uuid),
            data,
            actions,
        );
    });
}

/// Tap controls for the selected deck: which internal output it re-enters,
/// plus the two things a performer has to know about feedback.
/// See spec/program-tap.md.
fn render_tap_controls(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    tap: &TapUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.separator();
    ui.label(egui::RichText::new("🔁 Tap").strong().size(12.0));

    ui.horizontal(|ui| {
        ui.label("Source:");
        let selected = if tap.bound {
            tap.label.clone()
        } else {
            format!("{} (missing)", tap.label)
        };
        let mut chosen: Option<crate::scene::TapSourceConfig> = None;
        egui::ComboBox::from_id_salt("sel_deck_tap_source")
            .selected_text(selected)
            .width(160.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(tap.kind == "master_program", "Master Program")
                    .clicked()
                {
                    chosen = Some(crate::scene::TapSourceConfig::MasterProgram);
                }
                for ch in &data.channels {
                    let is_current = tap.channel_uuid.as_deref() == Some(ch.uuid.as_str());
                    if ui
                        .selectable_label(is_current, format!("{} ({})", ch.name, ch.uuid))
                        .clicked()
                    {
                        chosen = Some(crate::scene::TapSourceConfig::Channel {
                            uuid: ch.uuid.clone(),
                        });
                    }
                }
            });
        if let Some(source) = chosen {
            actions.commands.push(EngineCommand::SetTapSource {
                deck_uuid: deck.uuid.clone(),
                source,
            });
        }
    });

    if !tap.bound {
        ui.colored_label(
            egui::Color32::from_rgb(220, 160, 60),
            "Tapped channel no longer exists — showing black.",
        );
    }
    ui.label(
        egui::RichText::new(
            "Shows the previous frame. Feedback above 1.0 opacity on an additive \
             blend grows without limit — the tonemap rolls it off, it is not clamped.",
        )
        .small()
        .weak(),
    );
}

/// Screen-capture controls for the selected deck: rate, crop, cursor, and
/// (for display targets) Varda-window exclusion. See spec/screen-capture.md.
fn render_capture_controls(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    capture: &ScreenCaptureUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    let send = |actions: &mut UIActions, name: &str, value: f32| {
        actions.commands.push(EngineCommand::SetParam {
            path: format!("deck/{}/capture/{}", deck.uuid, name),
            value: ParamValue::Float(value),
        });
    };

    ui.separator();
    ui.label(
        egui::RichText::new(format!("🖥 Screen Capture — {}", capture.target_label))
            .strong()
            .size(12.0),
    );
    if !capture.bound {
        ui.colored_label(
            egui::Color32::from_rgb(220, 160, 60),
            "Target not found — showing black. Reopen the target and rescan.",
        );
    } else if !capture.connected {
        ui.colored_label(egui::Color32::GRAY, "Waiting for frames…");
    }

    ui.horizontal(|ui| {
        ui.label("Rate:");
        let mut rate = capture.rate;
        let resp = ui.add(egui::Slider::new(&mut rate, 0.0..=1.0).show_value(false));
        if resp.changed() {
            send(actions, "rate", rate);
        }
        ui.label(format!("{:.0} fps", capture.rate_fps));
        learn_overlay(
            ui,
            resp.rect,
            format!("deck/{}/capture/rate", deck.uuid),
            data,
            actions,
        );
    });

    // Crop is a sub-section of its own: the params column is only 200–280px, too
    // narrow to hold four sliders side by side, so they get a row each. Labels stay
    // single-character so every slider starts at the same x.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Crop").strong());
        if ui.small_button("Reset").clicked() {
            send(actions, "crop_x", 0.0);
            send(actions, "crop_y", 0.0);
            send(actions, "crop_w", 1.0);
            send(actions, "crop_h", 1.0);
        }
    });

    let crop_sliders: [(&str, &str, f32); 4] = [
        ("X", "crop_x", capture.crop[0]),
        ("Y", "crop_y", capture.crop[1]),
        ("W", "crop_w", capture.crop[2]),
        ("H", "crop_h", capture.crop[3]),
    ];
    for (label, name, current) in crop_sliders {
        let mut v = current;
        ui.horizontal(|ui| {
            ui.label(label);
            let resp = ui.add(
                egui::Slider::new(&mut v, 0.0..=1.0)
                    .show_value(false)
                    .fixed_decimals(2),
            );
            if resp.changed() {
                send(actions, name, v);
            }
            learn_overlay(
                ui,
                resp.rect,
                format!("deck/{}/capture/{}", deck.uuid, name),
                data,
                actions,
            );
        });
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut cursor = capture.show_cursor;
        let resp = ui.checkbox(&mut cursor, "Cursor");
        if resp.changed() {
            send(actions, "cursor", f32::from(u8::from(cursor)));
        }
        learn_overlay(
            ui,
            resp.rect,
            format!("deck/{}/capture/cursor", deck.uuid),
            data,
            actions,
        );

        // Only displays can contain Varda's own windows; for a window target
        // the toggle would be a no-op, so it is not offered.
        if capture.is_display {
            let mut exclude = capture.exclude_varda;
            let resp = ui
                .checkbox(&mut exclude, "Exclude Varda")
                .on_hover_text("Omit Varda's own windows from this display capture");
            if resp.changed() {
                send(actions, "exclude_varda", f32::from(u8::from(exclude)));
            }
            learn_overlay(
                ui,
                resp.rect,
                format!("deck/{}/capture/exclude_varda", deck.uuid),
                data,
                actions,
            );
        }
    });
}

/// Render the selected deck's full details (params, effects, blend, scaling) in the bottom bar
pub(super) fn render_selected_deck_detail(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.heading("🎛 Selected Deck");

    let Some((ch_idx, deck_idx)) = data.selected_deck else {
        ui.label(
            egui::RichText::new("Click a deck thumbnail to see its controls here")
                .weak()
                .small(),
        );
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
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} / Deck {} — {}",
                ch.name,
                deck_idx + 1,
                deck.name
            ))
            .strong()
            .color(accent),
        );

        // Save as preset — inline name prompt
        let prompt_id = egui::Id::new("deck_preset_name_prompt");
        let name_id = egui::Id::new("deck_preset_name_input");
        let is_prompting: bool = ui.data(|d| d.get_temp(prompt_id)).unwrap_or(false);

        if is_prompting {
            let cleared_id = egui::Id::new("deck_preset_name_cleared");
            let was_cleared: bool = ui.data(|d| d.get_temp(cleared_id)).unwrap_or(false);
            let mut name: String = ui
                .data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| deck.name.clone());
            let response = ui.text_edit_singleline(&mut name);
            if response.gained_focus() && !was_cleared {
                name.clear();
                ui.data_mut(|d| d.insert_temp(cleared_id, true));
            }
            if ui.small_button("✓ Save").clicked() && !name.is_empty() {
                actions.commands.push(EngineCommand::SaveDeckPreset {
                    deck_uuid: deck.uuid.clone(),
                    name: name.clone(),
                });
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            if ui.small_button("✕").clicked() {
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            ui.data_mut(|d| d.insert_temp(name_id, name));
        } else if ui.small_button("💾 Save Preset").clicked() {
            ui.data_mut(|d| {
                d.insert_temp(prompt_id, true);
                d.remove_temp::<String>(name_id);
                d.insert_temp(egui::Id::new("deck_preset_name_cleared"), false);
            });
        }
    });

    // Horizontal columns: Preview | Generator | Effect 1 | Effect 2 | ... | Add Effect
    egui::ScrollArea::horizontal().id_salt("selected_deck_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            // Column 0: Deck preview — scales with bottom bar height
            if let Some(tex_id) = data.deck_preview_textures.get(&deck.uuid) {
                // Height-driven from the bottom bar, with the visible panel
                // width as the other bound so an ultra-wide project cannot
                // produce a column wider than the bar it sits in.
                let available_height = ui.available_height() - 12.0; // margin
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
                            ui.image(egui::load::SizedTexture::new(*tex_id, egui::vec2(preview_width, preview_height)));
                            ui.label(egui::RichText::new(&deck.name).small().color(accent));
                        });
                    });
                ui.separator();
            }

            // Column: HTML source controls (only for HTML decks)
            if deck.is_html {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(140.0);
                        ui.set_max_width(200.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.label(egui::RichText::new("🌐 HTML").strong());
                            let reload_resp = ui.button("⟳ Reload");
                            if reload_resp.clicked() {
                                actions.commands.push(EngineCommand::ReloadHtmlDeck {
                                    deck_uuid: deck.uuid.clone(),
                                });
                            }
                            learn_overlay(
                                ui,
                                reload_resp.rect,
                                format!("deck/{}/html/reload", deck.uuid),
                                data,
                                actions,
                            );
                            let interactive_label = if deck.is_html_interactive {
                                "🖱 Exit Interactive"
                            } else {
                                "🖱 Interactive"
                            };
                            let interactive_resp = ui.button(interactive_label);
                            if interactive_resp.clicked() {
                                let cmd = if deck.is_html_interactive {
                                    EngineCommand::CloseHtmlInteractive
                                } else {
                                    EngineCommand::OpenHtmlInteractive {
                                        deck_uuid: deck.uuid.clone(),
                                    }
                                };
                                actions.commands.push(cmd);
                            }
                            learn_overlay(
                                ui,
                                interactive_resp.rect,
                                format!("deck/{}/html/interactive", deck.uuid),
                                data,
                                actions,
                            );
                            let mut transparent = deck.transparent;
                            let transparent_resp =
                                ui.checkbox(&mut transparent, "Transparent BG");
                            if transparent_resp.changed() {
                                actions.commands.push(EngineCommand::SetDeckTransparent {
                                    deck_uuid: deck.uuid.clone(),
                                    transparent,
                                });
                            }
                            learn_overlay(
                                ui,
                                transparent_resp.rect,
                                format!("deck/{}/transparent", deck.uuid),
                                data,
                                actions,
                            );
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
                            let play_resp = ui.button(play_label);
                            if play_resp.clicked() {
                                actions.commands.push(EngineCommand::VideoTogglePlay { deck_uuid: deck.uuid.clone() });
                            }
                            learn_overlay(ui, play_resp.rect, format!("deck/{}/video/play", deck.uuid), data, actions);

                            // Position scrub bar
                            let duration = vp.duration.max(0.001);
                            let mut pos = vp.position as f32;
                            ui.horizontal(|ui| {
                                ui.label(format_time(vp.position));
                                let slider = egui::Slider::new(&mut pos, 0.0..=duration as f32)
                                    .show_value(false)
                                    .trailing_fill(true);
                                let resp = ui.add(slider);
                                if resp.changed() {
                                    actions.commands.push(EngineCommand::VideoSeek { deck_uuid: deck.uuid.clone(), position_secs: f64::from(pos) });
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/seek", deck.uuid), data, actions);
                                ui.label(format_time(duration));
                            });

                            // Speed control
                            let mut speed = vp.speed as f32;
                            ui.horizontal(|ui| {
                                ui.label("Speed:");
                                let resp = ui.add(egui::Slider::new(&mut speed, 0.1..=4.0).step_by(0.05).suffix("x"));
                                if resp.changed() {
                                    actions.commands.push(EngineCommand::VideoSetSpeed { deck_uuid: deck.uuid.clone(), speed: f64::from(speed) });
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/speed", deck.uuid), data, actions);
                            });

                            // Loop mode
                            let loop_resp = ui.horizontal(|ui| {
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
                                        actions.commands.push(EngineCommand::VideoSetLoopMode { deck_uuid: deck.uuid.clone(), mode: *mode });
                                    }
                                }
                            });
                            learn_overlay(ui, loop_resp.response.rect, format!("deck/{}/video/loop_mode", deck.uuid), data, actions);

                            let chasing_now = vp.transport_sync.mode.is_chasing(data.transport.running);
                            if chasing_now {
                                ui.label(
                                    egui::RichText::new("Loop is ignored while chasing the transport")
                                        .small()
                                        .weak(),
                                );
                            }

                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("⏱ Transport").strong());
                            let mut sync = vp.transport_sync;
                            ui.horizontal(|ui| {
                                ui.label("Chase:");
                                egui::ComboBox::from_id_salt(format!("deck-chase-{}", deck.uuid))
                                    .selected_text(sync.mode.label())
                                    .show_ui(ui, |ui| {
                                        for mode in [
                                            crate::video::TransportSyncMode::Auto,
                                            crate::video::TransportSyncMode::Always,
                                            crate::video::TransportSyncMode::Never,
                                        ] {
                                            if ui
                                                .selectable_label(sync.mode == mode, mode.label())
                                                .clicked()
                                            {
                                                sync.mode = mode;
                                                actions.commands.push(
                                                    EngineCommand::VideoSetTransportSync {
                                                        deck_uuid: deck.uuid.clone(),
                                                        sync,
                                                    },
                                                );
                                            }
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                ui.label("Offset:");
                                let resp = ui.add(
                                    egui::DragValue::new(&mut sync.offset)
                                        .speed(0.01)
                                        .suffix(" s"),
                                );
                                if resp.changed() {
                                    actions.commands.push(EngineCommand::VideoSetTransportSync {
                                        deck_uuid: deck.uuid.clone(),
                                        sync,
                                    });
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Delay:");
                                let resp = ui.add(
                                    egui::DragValue::new(&mut sync.delay_frames)
                                        .speed(1.0)
                                        .suffix(" f"),
                                );
                                if resp.changed() {
                                    actions.commands.push(EngineCommand::VideoSetTransportSync {
                                        deck_uuid: deck.uuid.clone(),
                                        sync,
                                    });
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
                                let resp = ui.add(egui::Slider::new(&mut in_pt, 0.0..=duration as f32)
                                    .show_value(false).trailing_fill(true));
                                if resp.changed()
                                {
                                    actions.commands.push(EngineCommand::VideoSetInPoint { deck_uuid: deck.uuid.clone(), secs: f64::from(in_pt) });
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/in_point", deck.uuid), data, actions);
                                ui.label(format_time(f64::from(in_pt)));
                            });

                            // Out-point
                            let mut out_pt = effective_out as f32;
                            ui.horizontal(|ui| {
                                ui.label("Out:");
                                let resp = ui.add(egui::Slider::new(&mut out_pt, 0.0..=duration as f32)
                                    .show_value(false).trailing_fill(true));
                                if resp.changed()
                                {
                                    actions.commands.push(EngineCommand::VideoSetOutPoint { deck_uuid: deck.uuid.clone(), secs: f64::from(out_pt) });
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/out_point", deck.uuid), data, actions);
                                ui.label(format_time(f64::from(out_pt)));
                            });

                            // Set from current / clear buttons
                            ui.horizontal(|ui| {
                                if ui.small_button("[ Set In").on_hover_text("Set in-point to current position").clicked() {
                                    actions.commands.push(EngineCommand::VideoSetInPoint { deck_uuid: deck.uuid.clone(), secs: vp.position });
                                }
                                if ui.small_button("Set Out ]").on_hover_text("Set out-point to current position").clicked() {
                                    actions.commands.push(EngineCommand::VideoSetOutPoint { deck_uuid: deck.uuid.clone(), secs: vp.position });
                                }
                                // Clear is always shown (disabled when no range) so it stays MIDI/keyboard-mappable.
                                let clear_resp = ui
                                    .add_enabled(has_range, egui::Button::new("x Clear").small())
                                    .on_hover_text("Reset to full clip");
                                if clear_resp.clicked() {
                                    actions.commands.push(EngineCommand::VideoClearInOutPoints { deck_uuid: deck.uuid.clone() });
                                }
                                learn_overlay(ui, clear_resp.rect, format!("deck/{}/video/clear", deck.uuid), data, actions);
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
                                    let enabled = deck.auto_transition.as_ref().is_some_and(|at| at.enabled);
                                    let mut en = enabled;
                                    ui.checkbox(&mut en, "Enabled");
                                    if en != enabled {
                                        actions.commands.push(EngineCommand::SetAutoTransitionEnabled { deck_uuid: deck.uuid.clone(), enabled: en });
                                    }
                                });

                                if let Some(ref at) = deck.auto_transition {
                                    if at.enabled {
                                        ui.horizontal(|ui| {
                                            ui.label("Trigger:");
                                            let mut clip_end = at.trigger_is_clip_end;
                                            if ui.selectable_label(!clip_end, "Timer").clicked() && clip_end {
                                                clip_end = false;
                                                actions.commands.push(EngineCommand::SetAutoTransitionTrigger { deck_uuid: deck.uuid.clone(), clip_end: false });
                                            }
                                            if ui.selectable_label(clip_end, "Clip End").clicked() && !clip_end {
                                                actions.commands.push(EngineCommand::SetAutoTransitionTrigger { deck_uuid: deck.uuid.clone(), clip_end: true });
                                            }
                                        });
                                        let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                                        ui.horizontal(|ui| {
                                            ui.label("Play:");
                                            let mut val = at.play_duration_value as f32;
                                            let max = if at.play_duration_is_beats { 128.0 } else { 300.0 };
                                            let play_path = format!("deck/{}/at/play_duration", deck.uuid);
                                            let slider_rect = if any_learn {
                                                let inner = ui.scope(|ui| {
                                                    ui.disable();
                                                    ui.add(egui::Slider::new(&mut val, 0.5..=max)
                                                        .logarithmic(true)
                                                        .suffix(if at.play_duration_is_beats { " beats" } else { " sec" }))
                                                });
                                                inner.inner.rect
                                            } else {
                                                let resp = ui.add(egui::Slider::new(&mut val, 0.5..=max)
                                                    .logarithmic(true)
                                                    .suffix(if at.play_duration_is_beats { " beats" } else { " sec" }));
                                                if resp.changed() {
                                                    actions.commands.push(EngineCommand::SetAutoTransitionPlayDurationValue { deck_uuid: deck.uuid.clone(), value: f64::from(val) });
                                                }
                                                resp.rect
                                            };
                                            if data.midi_learn_active {
                                                let is_target = data.midi_learn_target.as_deref() == Some(play_path.as_str());
                                                if is_target { widgets::draw_midi_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_midi_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("midi_learn_at_play", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.session.midi_learn_select = Some(play_path.clone());
                                                }
                                            }
                                            if data.keyboard_learn_active {
                                                let is_target = data.keyboard_learn_target.as_deref() == Some(play_path.as_str());
                                                if is_target { widgets::draw_keyboard_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_keyboard_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("kb_learn_at_play", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.session.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(play_path));
                                                }
                                            }
                                            if !any_learn
                                                && ui.small_button(if at.play_duration_is_beats { "♩" } else { "⏱" })
                                                    .on_hover_text("Toggle beats/seconds").clicked()
                                                {
                                                    actions.commands.push(EngineCommand::ToggleAutoTransitionPlayDurationUnit { deck_uuid: deck.uuid.clone() });
                                                }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Trans:");
                                            let mut val = at.transition_duration_value as f32;
                                            let max = if at.transition_duration_is_beats { 32.0 } else { 30.0 };
                                            let trans_path = format!("deck/{}/at/trans_duration", deck.uuid);
                                            let slider_rect = if any_learn {
                                                let inner = ui.scope(|ui| {
                                                    ui.disable();
                                                    ui.add(egui::Slider::new(&mut val, 0.1..=max)
                                                        .logarithmic(true)
                                                        .suffix(if at.transition_duration_is_beats { " beats" } else { " sec" }))
                                                });
                                                inner.inner.rect
                                            } else {
                                                let resp = ui.add(egui::Slider::new(&mut val, 0.1..=max)
                                                    .logarithmic(true)
                                                    .suffix(if at.transition_duration_is_beats { " beats" } else { " sec" }));
                                                if resp.changed() {
                                                    actions.commands.push(EngineCommand::SetAutoTransitionDurationValue { deck_uuid: deck.uuid.clone(), value: f64::from(val) });
                                                }
                                                resp.rect
                                            };
                                            if data.midi_learn_active {
                                                let is_target = data.midi_learn_target.as_deref() == Some(trans_path.as_str());
                                                if is_target { widgets::draw_midi_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_midi_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("midi_learn_at_trans", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.session.midi_learn_select = Some(trans_path.clone());
                                                }
                                            }
                                            if data.keyboard_learn_active {
                                                let is_target = data.keyboard_learn_target.as_deref() == Some(trans_path.as_str());
                                                if is_target { widgets::draw_keyboard_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_keyboard_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("kb_learn_at_trans", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.session.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(trans_path));
                                                }
                                            }
                                            if !any_learn
                                                && ui.small_button(if at.transition_duration_is_beats { "♩" } else { "⏱" })
                                                    .on_hover_text("Toggle beats/seconds").clicked()
                                                {
                                                    actions.commands.push(EngineCommand::ToggleAutoTransitionDurationUnit { deck_uuid: deck.uuid.clone() });
                                                }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Shader:");
                                            let current = at.transition_shader_name.as_deref().unwrap_or("(fade)");
                                            egui::ComboBox::from_id_salt(format!("at_shader_{ch_idx}_{deck_idx}"))
                                                .selected_text(current)
                                                .width(120.0)
                                                .show_ui(ui, |ui| {
                                                    if ui.selectable_label(at.transition_shader_name.is_none(), "(fade)").clicked() {
                                                        actions.commands.push(EngineCommand::SetAutoTransitionShader { deck_uuid: deck.uuid.clone(), shader_name: None });
                                                    }
                                                    for name in &data.transition_names {
                                                        let selected = at.transition_shader_name.as_deref() == Some(name.as_str());
                                                        if ui.selectable_label(selected, name).clicked() && !selected {
                                                            actions.commands.push(EngineCommand::SetAutoTransitionShader { deck_uuid: deck.uuid.clone(), shader_name: Some(name.clone()) });
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
                                let all_modes = BlendMode::all();
                                let current_blend = all_modes.iter().position(|m| *m == deck.blend_mode).unwrap_or(0);
                                let mut selected = current_blend;
                                ui.horizontal(|ui| {
                                    ui.label("Blend:");
                                    egui::ComboBox::from_id_salt("sel_deck_blend")
                                        .selected_text(all_modes[selected].short_name())
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for (i, mode) in all_modes.iter().enumerate() {
                                                ui.selectable_value(&mut selected, i, mode.short_name());
                                            }
                                        });
                                });
                                if selected != current_blend {
                                    actions.commands.push(EngineCommand::SetDeckBlendMode {
                                        deck_uuid: deck.uuid.clone(),
                                        mode: all_modes[selected],
                                    });
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
                                        let combo = egui::ComboBox::from_id_salt("sel_deck_scale")
                                            .selected_text(scaling_modes[selected_scaling])
                                            .width(60.0)
                                            .show_ui(ui, |ui| {
                                                for (i, mode_name) in scaling_modes.iter().enumerate() {
                                                    ui.selectable_value(&mut selected_scaling, i, *mode_name);
                                                }
                                            });
                                        learn_overlay(ui, combo.response.rect, format!("deck/{}/scaling_mode", deck.uuid), data, actions);
                                    });
                                    if selected_scaling != current_idx {
                                        let new_scaling = match selected_scaling {
                                            1 => ScalingMode::Fit, 2 => ScalingMode::Stretch,
                                            3 => ScalingMode::Center, _ => ScalingMode::Fill,
                                        };
                                        actions.commands.push(EngineCommand::SetDeckScalingMode {
                                            deck_uuid: deck.uuid.clone(),
                                            mode: new_scaling,
                                        });
                                    }
                                }

                                // Depth-sensor point-cloud controls
                                if let Some(pc) = &deck.point_cloud {
                                    render_depth_controls(ui, deck, pc, data, actions);
                                }

                                // Depth-sensor shader preprocessor controls
                                if let Some(prepro) = &deck.depth_prepro {
                                    render_depth_prepro_controls(
                                        ui, deck, prepro, data, actions,
                                    );
                                }

                                // Screen-capture controls
                                if let Some(capture) = &deck.screen_capture {
                                    render_capture_controls(ui, deck, capture, data, actions);
                                }

                                // Tap controls
                                if let Some(tap) = &deck.tap {
                                    render_tap_controls(ui, deck, tap, data, actions);
                                }

                                // Render FPS
                                ui.horizontal(|ui| {
                                    ui.label("Render:");
                                    let options = ["Auto", "60", "30", "15"];
                                    let current_idx = match deck.render_fps {
                                        DeckRenderFps::Fixed(60) => 1,
                                        DeckRenderFps::Fixed(30) => 2,
                                        DeckRenderFps::Fixed(15) => 3,
                                        // Auto and any other fixed rate fall back to "Auto"
                                        DeckRenderFps::Auto | DeckRenderFps::Fixed(_) => 0,
                                    };
                                    let mut selected = current_idx;
                                    egui::ComboBox::from_id_salt("sel_deck_render_fps")
                                        .selected_text(options[selected])
                                        .width(50.0)
                                        .show_ui(ui, |ui| {
                                            for (i, opt) in options.iter().enumerate() {
                                                ui.selectable_value(&mut selected, i, *opt);
                                            }
                                        });
                                    if selected != current_idx {
                                        let new_fps = match selected {
                                            1 => DeckRenderFps::Fixed(60),
                                            2 => DeckRenderFps::Fixed(30),
                                            3 => DeckRenderFps::Fixed(15),
                                            _ => DeckRenderFps::Auto,
                                        };
                                        actions.commands.push(EngineCommand::SetDeckRenderFps {
                                            deck_uuid: deck.uuid.clone(),
                                            render_fps: new_fps,
                                        });
                                    }
                                    // Show render cost
                                    if deck.gpu_render_cost_us > 0.0 {
                                        let ms = deck.gpu_render_cost_us / 1000.0;
                                        ui.label(egui::RichText::new(format!("⚡{ms:.1}ms GPU")).small().weak());
                                    } else if deck.render_cost_us > 0.0 {
                                        let ms = deck.render_cost_us / 1000.0;
                                        ui.label(egui::RichText::new(format!("⚡{ms:.1}ms")).small().weak());
                                    }
                                });

                                // Generator parameters
                                let gen_params = &deck.generator;
                                if !gen_params.params.is_empty() {
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(&gen_params.shader_name).strong());
                                    let deck_uuid = deck.uuid.clone();
                                    let midi_path_prefix = format!("deck/{deck_uuid}");
                                    let deck_uuid_assign = deck_uuid.clone();
                                    let deck_uuid_remove = deck_uuid.clone();
                                    let deck_uuid_automate = deck_uuid.clone();
                                    widgets::render_params(
                                        ui,
                                        &gen_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| EngineCommand::SetGeneratorParam { deck_uuid: deck.uuid.clone(), name: name.to_string(), value: val },
                                        Some(&|name: &str, source_uuid: &str| EngineCommand::AssignModulation {
                                            target: format!("deck_{deck_uuid_assign}:{name}"), source_id: source_uuid.to_string(), amount: DEFAULT_ASSIGNMENT_AMOUNT,
                                        }),
                                        Some(&|name: &str| EngineCommand::ClearModulation {
                                            target: format!("deck_{deck_uuid_remove}:{name}"),
                                        }),
                                        Some(&|name: &str| EngineCommand::AddAutomationLane {
                                            target: format!("deck_{deck_uuid_automate}:{name}"),
                                            timebase: crate::timebase::Timebase::Transport,
                                        }),
                                        &mut actions.commands,
                                        &mut actions.session.gesture_active,
                                        &format!("sel_{ch_idx}_{deck_idx}"),
                                        Some(&midi_path_prefix),
                                        data.midi_learn_active,
                                        &mut actions.session.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("deck_{deck_uuid}"),
                                        data.keyboard_learn_active,
                                        &mut actions.session.keyboard_learn_select,
                                        data.keyboard_learn_target.as_deref(),
                                    );
                                    ui.add_space(4.0);
                                    if ui.button("Reset").clicked() {
                                        actions.commands.push(EngineCommand::ResetGeneratorParamsToDefaults { deck_uuid: deck.uuid.clone() });
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
                for (eff_idx, (eff_uuid, eff_name, eff_enabled, eff_params)) in deck.effects.iter().enumerate() {
                    // Drop zone before this effect (for reordering)
                    render_effect_drop_zone(ui, &format!("deck_{}", deck.uuid), eff_idx);

                    // Effect card with drag handle in header only
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
                            egui::ScrollArea::vertical().id_salt(format!("deck_fx_scroll_{}_{}", deck.uuid, eff_uuid)).max_height(max_h).scroll_source(egui::scroll_area::ScrollSource { drag: false, scroll_bar: true, mouse_wheel: true }).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    render_effect_drag_handle(ui, EffectDrag::Deck(deck.uuid.clone(), eff_idx));
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.commands.push(EngineCommand::ToggleEffect {
                                            effect_uuid: eff_uuid.clone(),
                                        });
                                    }
                                    ui.label(egui::RichText::new(eff_name).strong());
                                });

                                if !eff_params.params.is_empty() {
                                    let deck_uuid_eff = deck.uuid.clone();
                                    let eff_uuid_param = eff_uuid.clone();
                                    let eff_uuid_assign = eff_uuid.clone();
                                    let eff_uuid_remove = eff_uuid.clone();
                                    let eff_uuid_automate = eff_uuid.clone();
                                    let eff_midi_prefix = format!("deck/{deck_uuid_eff}/effect/{eff_uuid}");
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| EngineCommand::SetEffectParam { effect_uuid: eff_uuid_param.clone(), name: name.to_string(), value: val },
                                        Some(&|name: &str, source_uuid: &str| EngineCommand::AssignModulation {
                                            target: format!("fx_{eff_uuid_assign}:{name}"), source_id: source_uuid.to_string(), amount: DEFAULT_ASSIGNMENT_AMOUNT,
                                        }),
                                        Some(&|name: &str| EngineCommand::ClearModulation {
                                            target: format!("fx_{eff_uuid_remove}:{name}"),
                                        }),
                                        Some(&|name: &str| EngineCommand::AddAutomationLane {
                                            target: format!("fx_{eff_uuid_automate}:{name}"),
                                            timebase: crate::timebase::Timebase::Transport,
                                        }),
                                        &mut actions.commands,
                                        &mut actions.session.gesture_active,
                                        &format!("fx_{deck_uuid_eff}_{eff_uuid}"),
                                        Some(&eff_midi_prefix),
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
                    super::effects::effect_context_menu(
                        &card_scope.response,
                        data,
                        actions,
                        eff_uuid,
                        eff_name,
                    );
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
                            actions.commands.push(EngineCommand::RemoveEffect {
                                effect_uuid: eff_uuid.clone(),
                            });
                        }
                    }
                    render_effect_drag_ghost(
                        ui,
                        egui::Id::new(("eff_ghost", &deck.uuid, eff_uuid)),
                        EffectDrag::Deck(deck.uuid.clone(), eff_idx),
                        eff_name,
                    );
                    ui.separator();
                }

                // Drop zone after last effect (for reordering to end)
                if !deck.effects.is_empty() {
                    let num_effects = deck.effects.len();
                    render_effect_drop_zone(ui, &format!("deck_{}", deck.uuid), num_effects);
                }

                // Remaining space: always present drop target that fills remaining width
                let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                    .is_some_and(|p| matches!(&*p, LibraryDrag::Effect(_)));
                let remaining_w = ui.available_width().max(80.0);
                let remaining_h = ui.available_height().max(40.0);
                let stroke = if has_fx_drag { egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
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

            // The entire horizontal_top area takes deferred library effect drops
            let chain_rect = ui.min_rect();
            super::dnd::publish_deck_surface_fx(ui.ctx(), &deck.uuid, ch_idx, deck_idx, chain_rect);
            let deck_chain_key = format!("deck_{}", deck.uuid);
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with(deck_chain_key), deck.effects.len() + 1);
            });
        });
    });
}
