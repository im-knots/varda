//! Deck detail: the bottom-bar mode shown when a deck is selected.

use super::super::{
    DeckUIInfo, DepthPreproUI, EffectDrag, LibraryDrag, ParamUIInfo, UIActions, UIData, widgets,
};
use super::utils::{
    channel_color, format_time, render_collapsed_column, render_effect_drag_ghost,
    render_effect_drag_handle, render_effect_drop_zone,
};
use crate::BlendMode;
use crate::channel::DeckRenderFps;
use crate::engine::EngineCommand;
use crate::engine::value::param::{DeckTarget, ParamAddress};
use crate::engine::value::source::{SourceParamKind, SourceParamSpec, SourceValue, WidgetHint};
use crate::modulation::DEFAULT_ASSIGNMENT_AMOUNT;
use crate::params::ParamValue;

/// The `〰` dropdown for one of a deck source's modulatable controls.
///
/// Source controls are deck built-ins (`deck/<uuid>/video/speed`, ...) rather
/// than `ParamUIInfo` rows, so they are addressed the way a channel fader is.
/// See /spec/video-playback-modulation.md § Key naming.
fn playback_mod_menu(
    ui: &mut egui::Ui,
    deck_uuid: &str,
    target: DeckTarget,
    data: &UIData,
    actions: &mut UIActions,
) {
    widgets::modulation_menu_for_key(
        ui,
        format!("vidmod-{deck_uuid}-{target}"),
        &ParamAddress::deck(deck_uuid, target).to_string(),
        &data.modulation_sources,
        &data.modulation_assignments,
        &mut actions.commands,
    );
}

/// The colour of the first modulator driving the parameter at `key`, or
/// `None` when nothing is assigned to it.
///
/// The colour has to match the card in the modulation panel, so the index comes
/// from the unfiltered source list.
fn mod_color_for_key(key: &str, data: &UIData) -> Option<egui::Color32> {
    let first = data.modulation_assignments.get(key)?.first()?;
    let idx = data
        .modulation_sources
        .iter()
        .position(|s| s.uuid == first.source_id)?;
    Some(super::super::modulator_color(idx))
}

/// The track of a slider, excluding any value box drawn beside it.
///
/// egui returns one rect for the whole widget, so measuring a ghost against it
/// stretches the scale across the number as well and pushes every reading
/// right. A slider built with `show_value(false)` is already all track.
fn slider_track(ui: &egui::Ui, rect: egui::Rect, shows_value: bool) -> egui::Rect {
    if !shows_value {
        return rect;
    }
    let width = ui.spacing().slider_width.min(rect.width());
    egui::Rect::from_min_size(rect.min, egui::vec2(width, rect.height()))
}

/// Draw a ghost line at `value` on a slider whose track spans `range`.
///
/// Which of the two positions the ghost carries depends on the control: where
/// the handle shows the performer's set point the ghost shows the live value,
/// and where the handle already rides the live value it shows the set point the
/// modulator is working from. Either way the pair is set point and actual.
fn draw_slider_ghost(
    ui: &egui::Ui,
    rect: egui::Rect,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    color: egui::Color32,
) {
    let span = range.end() - range.start();
    if span <= 0.0 {
        return;
    }
    let t = ((value - range.start()) / span).clamp(0.0, 1.0);
    let x = rect.left() + t * rect.width();
    ui.painter().line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(2.0_f32, color),
    );
}

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
            actions
                .commands
                .push(EngineCommand::MidiLearnSelect { path });
        }
    } else if data.keyboard_learn_active {
        if data.keyboard_learn_target.as_deref() == Some(path.as_str()) {
            widgets::draw_keyboard_learn_selected(ui, rect);
        } else {
            widgets::draw_keyboard_learn_glow(ui, rect);
        }
        let id = ui.id().with(("kb_learn", path.as_str()));
        if ui.interact(rect, id, egui::Sense::click()).clicked() {
            actions.commands.push(EngineCommand::KeyboardLearnSelect {
                target: crate::keymap::KeyTarget::ParamPath(path),
            });
        }
    }
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
                    path: ParamAddress::deck(&deck.uuid, DeckTarget::depth_preprocess(name))
                        .to_string(),
                    value: ParamValue::Float(v),
                });
            }
            learn_overlay(
                ui,
                resp.rect,
                ParamAddress::deck(&deck.uuid, DeckTarget::depth_preprocess(name)).to_string(),
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
                path: ParamAddress::deck(&deck.uuid, DeckTarget::depth_preprocess("mirror"))
                    .to_string(),
                value: ParamValue::Float(f32::from(u8::from(mirror))),
            });
        }
        learn_overlay(
            ui,
            resp.rect,
            ParamAddress::deck(&deck.uuid, DeckTarget::depth_preprocess("mirror")).to_string(),
            data,
            actions,
        );
    });
}

/// A source control's router path on this deck, when it has one.
fn source_path(deck_uuid: &str, spec: &SourceParamSpec) -> Option<String> {
    spec.route
        .as_deref()
        .map(|route| ParamAddress::deck(deck_uuid, DeckTarget::source(route)).to_string())
}

fn set_source(actions: &mut UIActions, deck_uuid: &str, name: &str, value: SourceValue) {
    actions.commands.push(EngineCommand::SetSourceParam {
        deck_uuid: deck_uuid.to_string(),
        name: name.to_string(),
        value,
    });
}

/// The learn glow and the `〰` modulation menu a routed control gets.
fn source_affordances(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    deck_uuid: &str,
    spec: &SourceParamSpec,
    data: &UIData,
    actions: &mut UIActions,
) {
    let Some(route) = spec.route.as_deref() else {
        return;
    };
    learn_overlay(
        ui,
        rect,
        ParamAddress::deck(deck_uuid, DeckTarget::source(route)).to_string(),
        data,
        actions,
    );
    if spec.modulatable {
        playback_mod_menu(ui, deck_uuid, DeckTarget::source(route), data, actions);
    }
}

/// The current normalized value of a numeric control.
fn norm(deck: &DeckUIInfo, name: &str) -> f32 {
    deck.source
        .status
        .params
        .get(name)
        .and_then(SourceValue::as_f32)
        .unwrap_or_default()
}

/// One source control, drawn from its kind.
fn source_param(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    spec: &SourceParamSpec,
    data: &UIData,
    actions: &mut UIActions,
) {
    let uuid = deck.uuid.as_str();
    let current = deck.source.status.params.get(&spec.name);
    match &spec.kind {
        SourceParamKind::Float {
            display_min,
            display_max,
            unit,
        } => {
            ui.horizontal(|ui| {
                ui.label(&spec.label);
                let mut v = norm(deck, &spec.name);
                let resp = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                if resp.changed() {
                    set_source(actions, uuid, &spec.name, SourceValue::Float(v));
                }
                let shown = deck
                    .source
                    .status
                    .display
                    .get(&spec.name)
                    .cloned()
                    .unwrap_or_else(|| {
                        let value = display_min + v * (display_max - display_min);
                        match unit {
                            Some(unit) => format!("{value:.2} {unit}"),
                            None => format!("{value:.2}"),
                        }
                    });
                ui.label(egui::RichText::new(shown).small().weak());
                source_affordances(ui, resp.rect, uuid, spec, data, actions);
            });
        }
        SourceParamKind::Toggle => {
            ui.horizontal(|ui| {
                let mut on = current.and_then(SourceValue::as_f32).unwrap_or_default() > 0.5;
                let resp = ui.checkbox(&mut on, &spec.label);
                if resp.changed() {
                    set_source(actions, uuid, &spec.name, SourceValue::Bool(on));
                }
                source_affordances(ui, resp.rect, uuid, spec, data, actions);
            });
        }
        SourceParamKind::Choice { options } => {
            ui.horizontal(|ui| {
                ui.label(format!("{}:", spec.label));
                let n = options.len().max(1);
                let index = ((norm(deck, &spec.name) * n as f32).floor() as usize).min(n - 1);
                let mut chosen = index;
                let combo = egui::ComboBox::from_id_salt(("source_choice", uuid, &spec.name))
                    .selected_text(options.get(index).cloned().unwrap_or_default())
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for (i, option) in options.iter().enumerate() {
                            ui.selectable_value(&mut chosen, i, option);
                        }
                    });
                if chosen != index {
                    let value = (chosen as f32 + 0.5) / n as f32;
                    set_source(actions, uuid, &spec.name, SourceValue::Float(value));
                }
                source_affordances(ui, combo.response.rect, uuid, spec, data, actions);
            });
        }
        SourceParamKind::Color => {
            ui.horizontal(|ui| {
                ui.label(&spec.label);
                let mut rgba = match current {
                    Some(SourceValue::Color(c)) => *c,
                    _ => [0.0, 0.0, 0.0, 1.0],
                };
                if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                    set_source(actions, uuid, &spec.name, SourceValue::Color(rgba));
                }
            });
        }
        SourceParamKind::Text => {
            ui.horizontal(|ui| {
                ui.label(format!("{}:", spec.label));
                let id = ui.id().with(("source_text", uuid, &spec.name));
                let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| {
                    current
                        .and_then(SourceValue::as_str)
                        .unwrap_or_default()
                        .to_string()
                });
                let resp = ui.text_edit_singleline(&mut text);
                if resp.lost_focus() {
                    set_source(actions, uuid, &spec.name, SourceValue::Text(text.clone()));
                    ui.data_mut(|d| d.remove::<String>(id));
                } else {
                    ui.data_mut(|d| d.insert_temp(id, text));
                }
            });
        }
        SourceParamKind::Number { unit, step } => {
            ui.horizontal(|ui| {
                ui.label(format!("{}:", spec.label));
                let mut v = current.and_then(SourceValue::as_f32).unwrap_or_default();
                let mut drag = egui::DragValue::new(&mut v).speed(*step);
                if let Some(unit) = unit {
                    drag = drag.suffix(format!(" {unit}"));
                }
                if ui.add(drag).changed() {
                    set_source(actions, uuid, &spec.name, SourceValue::Float(v));
                }
            });
        }
        SourceParamKind::Action => {
            let label = if spec.name == "interactive" && deck.is_interactive {
                format!("Exit {}", spec.label)
            } else {
                spec.label.clone()
            };
            let resp = ui.button(label);
            if resp.clicked() {
                actions.commands.push(EngineCommand::TriggerSourceAction {
                    deck_uuid: uuid.to_string(),
                    action: spec.name.clone(),
                });
            }
            source_affordances(ui, resp.rect, uuid, spec, data, actions);
        }
    }
}

/// The crop rectangle a `CropRect` hint groups: a row per edge (the column is
/// too narrow for four sliders side by side) and a reset to the full frame.
fn crop_widget(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    specs: &[&SourceParamSpec],
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Crop").strong());
        if ui.small_button("Reset").clicked() {
            for spec in specs {
                // A crop's size resets to the whole frame, its origin to zero.
                let full = spec.name.ends_with("_w") || spec.name.ends_with("_h");
                set_source(
                    actions,
                    &deck.uuid,
                    &spec.name,
                    SourceValue::Float(if full { 1.0 } else { 0.0 }),
                );
            }
        }
    });
    for spec in specs {
        source_param(ui, deck, spec, data, actions);
    }
}

/// A clip transport: play, scrub, speed and loop, the show-transport chase,
/// and the in/out range. Reads the info keys `WidgetHint::Transport` names.
fn transport_widget(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    specs: &[&SourceParamSpec],
    data: &UIData,
    actions: &mut UIActions,
) {
    let info = &deck.source.status.info;
    let num = |key: &str| {
        info.get(key)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
    };
    let spec = |name: &str| specs.iter().copied().find(|s| s.name == name);
    let uuid = deck.uuid.as_str();
    let duration = num("duration").max(0.001);
    let position = num("position");
    let playing = info
        .get("playing")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let sync_mode = info
        .get("transport_sync")
        .and_then(|s| s.get("mode"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Auto");
    let chasing_now = match sync_mode {
        "Always" => true,
        "Never" => false,
        _ => data.transport.running,
    };

    ui.label(egui::RichText::new("▶ Playback").strong());
    if let Some(play) = spec("play") {
        ui.horizontal(|ui| {
            let resp = ui.button(if playing { "⏸ Pause" } else { "▶ Play" });
            if resp.clicked() {
                set_source(actions, uuid, &play.name, SourceValue::Bool(!playing));
            }
            source_affordances(ui, resp.rect, uuid, play, data, actions);
        });
    }

    // The handle rides the live playhead, so the ghost marks the opposite
    // thing: the point the modulator is swinging around.
    if let Some(pos_spec) = spec("position") {
        let mut pos = position as f32;
        ui.horizontal(|ui| {
            ui.label(format_time(position));
            let resp = ui.add(
                egui::Slider::new(&mut pos, 0.0..=duration as f32)
                    .show_value(false)
                    .trailing_fill(true),
            );
            if resp.changed() {
                set_source(
                    actions,
                    uuid,
                    &pos_spec.name,
                    SourceValue::Float((f64::from(pos) / duration) as f32),
                );
            }
            if let Some(path) = source_path(uuid, pos_spec)
                && let Some(color) = mod_color_for_key(&path, data)
            {
                let anchor = position - num("position_offset");
                let track = slider_track(ui, resp.rect, false);
                draw_slider_ghost(ui, track, anchor as f32, 0.0..=duration as f32, color);
            }
            ui.label(format_time(duration));
            source_affordances(ui, resp.rect, uuid, pos_spec, data, actions);
        });
    }

    // The slider stays on the set point, so the ghost is the only thing
    // showing the live rate.
    if let Some(speed_spec) = spec("speed")
        && let SourceParamKind::Float {
            display_min,
            display_max,
            ..
        } = speed_spec.kind
    {
        let mut speed = num("speed") as f32;
        ui.horizontal(|ui| {
            ui.label("Speed:");
            let resp = ui.add(
                egui::Slider::new(&mut speed, display_min..=display_max)
                    .step_by(0.05)
                    .suffix("x"),
            );
            if resp.changed() {
                let v = (speed - display_min) / (display_max - display_min);
                set_source(actions, uuid, &speed_spec.name, SourceValue::Float(v));
            }
            if let Some(path) = source_path(uuid, speed_spec)
                && let Some(color) = mod_color_for_key(&path, data)
            {
                let track = slider_track(ui, resp.rect, true);
                draw_slider_ghost(
                    ui,
                    track,
                    num("effective_speed") as f32,
                    display_min..=display_max,
                    color,
                );
            }
            source_affordances(ui, resp.rect, uuid, speed_spec, data, actions);
        });
    }

    if let Some(loop_spec) = spec("loop_mode") {
        source_param(ui, deck, loop_spec, data, actions);
    }

    if chasing_now {
        ui.label(
            egui::RichText::new("Loop is ignored while chasing the transport")
                .small()
                .weak(),
        );
        // Two authorities on one value is the seek storm this design avoids,
        // so say which one wins rather than letting it look broken.
        let held = [("Playhead", spec("position")), ("Speed", spec("speed"))]
            .into_iter()
            .filter(|(_, s)| {
                s.and_then(|s| source_path(uuid, s))
                    .is_some_and(|path| mod_color_for_key(&path, data).is_some())
            })
            .map(|(label, _)| label)
            .collect::<Vec<_>>();
        if !held.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} modulation is ignored while chasing — set Chase to Never to use it",
                    held.join(" and ")
                ))
                .small()
                .weak(),
            );
        }
    }

    ui.add_space(4.0);
    ui.label(egui::RichText::new("⏱ Transport").strong());
    for name in ["chase", "chase_offset", "chase_delay"] {
        if let Some(s) = spec(name) {
            source_param(ui, deck, s, data, actions);
        }
    }

    ui.add_space(4.0);
    ui.label(egui::RichText::new("📐 In/Out Points").strong());
    let in_point = num("in_point");
    let out_point = num("out_point");
    let effective_out = if out_point > 0.0 { out_point } else { duration };
    let has_range = in_point > 0.0 || out_point > 0.0;
    for (name, label, secs) in [
        ("in_point", "In:", in_point),
        ("out_point", "Out:", effective_out),
    ] {
        let Some(point) = spec(name) else { continue };
        let mut v = secs as f32;
        ui.horizontal(|ui| {
            ui.label(label);
            let resp = ui.add(
                egui::Slider::new(&mut v, 0.0..=duration as f32)
                    .show_value(false)
                    .trailing_fill(true),
            );
            if resp.changed() {
                set_source(
                    actions,
                    uuid,
                    &point.name,
                    SourceValue::Float((f64::from(v) / duration) as f32),
                );
            }
            source_affordances(ui, resp.rect, uuid, point, data, actions);
            ui.label(format_time(f64::from(v)));
        });
    }
    ui.horizontal(|ui| {
        let here = SourceValue::Float((position / duration) as f32);
        if let Some(point) = spec("in_point")
            && ui
                .small_button("[ Set In")
                .on_hover_text("Set in-point to current position")
                .clicked()
        {
            set_source(actions, uuid, &point.name, here.clone());
        }
        if let Some(point) = spec("out_point")
            && ui
                .small_button("Set Out ]")
                .on_hover_text("Set out-point to current position")
                .clicked()
        {
            set_source(actions, uuid, &point.name, here);
        }
        // Clear is always shown (disabled when no range) so it stays MIDI/keyboard-mappable.
        if let Some(clear) = spec("clear") {
            let resp = ui
                .add_enabled(has_range, egui::Button::new("x Clear").small())
                .on_hover_text("Reset to full clip");
            if resp.clicked() {
                actions.commands.push(EngineCommand::TriggerSourceAction {
                    deck_uuid: uuid.to_string(),
                    action: clear.name.clone(),
                });
            }
            source_affordances(ui, resp.rect, uuid, clear, data, actions);
        }
    });
    if has_range {
        ui.label(
            egui::RichText::new(format!(
                "Range: {} → {} ({})",
                format_time(in_point),
                format_time(effective_out),
                format_time(effective_out - in_point),
            ))
            .small()
            .weak(),
        );
    }
    ui.label(
        egui::RichText::new(format!(
            "{:.0} fps • {}",
            num("frame_rate"),
            format_time(duration)
        ))
        .small()
        .weak(),
    );
}

/// The deck's source controls: whatever its type declares, grouped by the
/// widget hints it uses. See /spec/deck-source-providers.md Decision 8.
fn render_source_column(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    data: &UIData,
    actions: &mut UIActions,
) {
    let Some(ty) = data.source_type(&deck.source.source_type) else {
        return;
    };
    let status = &deck.source.status;
    // A type with no controls of its own (a shader, whose inputs are in the
    // params column) and nothing to report gets no column, so the effect
    // chain stays where it was on a narrow screen.
    let has_status =
        !deck.source.available || status.bound == Some(false) || status.connected == Some(false);
    if ty.params.is_empty() && !has_status {
        return;
    }
    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_min_width(200.0);
            ui.set_max_width(280.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.label(egui::RichText::new(format!("{} {}", ty.icon, ty.label)).strong());
                if !deck.source.available {
                    let reason = status
                        .info
                        .get("unavailable_reason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("This source cannot run here");
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 160, 60),
                        format!("{reason} — showing black. Its settings are kept."),
                    );
                    return;
                }
                if status.bound == Some(false) {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 160, 60),
                        "Not bound — showing black until what it reads comes back.",
                    );
                } else if status.connected == Some(false) {
                    ui.colored_label(egui::Color32::GRAY, "Waiting for frames…");
                }

                // Repoint the deck at another entry of its own type (another
                // camera, another tap point), keeping its effects and mappings.
                if ty.library.entries.len() > 1 {
                    ui.horizontal(|ui| {
                        ui.label("Source:");
                        let mut chosen = None;
                        egui::ComboBox::from_id_salt(("source_repoint", &deck.uuid))
                            .selected_text(&deck.name)
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                for entry in &ty.library.entries {
                                    if ui.selectable_label(false, &entry.label).clicked() {
                                        chosen = Some(entry.config.clone());
                                    }
                                }
                            });
                        if let Some(source) = chosen {
                            actions.commands.push(EngineCommand::ReplaceDeckSource {
                                deck_uuid: deck.uuid.clone(),
                                source,
                            });
                        }
                    });
                }

                let mut transport = Vec::new();
                let mut crop = Vec::new();
                for spec in &ty.params {
                    match spec.widget {
                        Some(WidgetHint::Transport) => transport.push(spec),
                        Some(WidgetHint::CropRect) => crop.push(spec),
                        Some(WidgetHint::Orbit) | None => {}
                    }
                }
                if !transport.is_empty() {
                    transport_widget(ui, deck, &transport, data, actions);
                    ui.add_space(4.0);
                }
                for spec in ty.params.iter().filter(|s| {
                    !matches!(s.widget, Some(WidgetHint::Transport | WidgetHint::CropRect))
                }) {
                    source_param(ui, deck, spec, data, actions);
                }
                if !crop.is_empty() {
                    crop_widget(ui, deck, &crop, data, actions);
                }

                let mut transparent = deck.transparent;
                let resp = ui.checkbox(&mut transparent, "Transparent BG");
                if resp.changed() {
                    actions.commands.push(EngineCommand::SetDeckTransparent {
                        deck_uuid: deck.uuid.clone(),
                        transparent,
                    });
                }
                learn_overlay(
                    ui,
                    resp.rect,
                    ParamAddress::deck(&deck.uuid, DeckTarget::Transparent).to_string(),
                    data,
                    actions,
                );
            });
        });
    ui.separator();
}

/// A seed nobody has to choose.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

/// Reset, randomize, and mutate for a deck's generator parameters.
///
/// Randomize and mutate only produce candidates. Deck presets are what name and
/// keep one, so the loop is: mutate, look, save a preset if it is good, undo if it
/// is not. See spec/parameter-exploration.md.
fn render_exploration_controls(
    ui: &mut egui::Ui,
    deck_uuid: &str,
    params: &[ParamUIInfo],
    commands: &mut Vec<EngineCommand>,
) {
    let amount_id = ui.id().with(("mutate_amount", deck_uuid));
    let scope_id = ui.id().with(("explore_scope", deck_uuid));
    let mut amount = ui.data_mut(|d| d.get_temp::<f32>(amount_id)).unwrap_or(0.1);

    // A group that has since gone (a different shader, or a `_mode` toggle that
    // hid its section) must not leave a scope selected that no longer exists.
    let groups = widgets::param_groups(params);
    let mut scope = ui
        .data_mut(|d| d.get_temp::<String>(scope_id))
        .filter(|s| groups.contains(&s.as_str()));

    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            commands.push(EngineCommand::ResetGeneratorParamsToDefaults {
                deck_uuid: deck_uuid.to_string(),
            });
        }
        if ui
            .button("Random")
            .on_hover_text("Draw the parameters in scope afresh from their ranges")
            .clicked()
        {
            commands.push(EngineCommand::RandomizeGeneratorParams {
                deck_uuid: deck_uuid.to_string(),
                group: scope.clone(),
                seed: fresh_seed(),
            });
        }
        if ui
            .button("Mutate")
            .on_hover_text("Nudge the parameters in scope, keeping the current look")
            .clicked()
        {
            commands.push(EngineCommand::MutateGeneratorParams {
                deck_uuid: deck_uuid.to_string(),
                group: scope.clone(),
                amount,
                seed: fresh_seed(),
            });
        }
    });

    ui.horizontal(|ui| {
        if ui
            .add(
                egui::DragValue::new(&mut amount)
                    .speed(0.01)
                    .range(0.01..=1.0)
                    .prefix("by "),
            )
            .on_hover_text("Mutation size, as a fraction of each parameter's range")
            .changed()
        {
            ui.data_mut(|d| d.insert_temp(amount_id, amount));
        }
        // Scoping is what makes exploration usable on a shader with fifty
        // parameters: hunt a formula without disturbing a grade that already works.
        if !groups.is_empty() {
            let before = scope.clone();
            egui::ComboBox::from_id_salt(("explore_scope_combo", deck_uuid))
                .selected_text(
                    egui::RichText::new(scope.as_deref().unwrap_or("Everything")).small(),
                )
                .width(100.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut scope,
                        None,
                        egui::RichText::new("Everything").small(),
                    );
                    for name in &groups {
                        ui.selectable_value(
                            &mut scope,
                            Some((*name).to_string()),
                            egui::RichText::new(*name).small(),
                        );
                    }
                })
                .response
                .on_hover_text("Which section a random or a mutation touches");
            if scope != before {
                ui.data_mut(|d| match &scope {
                    Some(name) => {
                        d.insert_temp(scope_id, name.clone());
                    }
                    None => {
                        d.remove_temp::<String>(scope_id);
                    }
                });
            }
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

            // Column: the source's own controls, from its type's schema.
            render_source_column(ui, deck, data, actions);

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

                                if let Some(ref at) = deck.auto_transition
                                    && at.enabled {
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
                                            let play_path = ParamAddress::deck(&deck.uuid, DeckTarget::AutoTransitionPlay).to_string();
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
                                                    actions.commands.push(EngineCommand::MidiLearnSelect { path: play_path.clone() });
                                                }
                                            }
                                            if data.keyboard_learn_active {
                                                let is_target = data.keyboard_learn_target.as_deref() == Some(play_path.as_str());
                                                if is_target { widgets::draw_keyboard_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_keyboard_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("kb_learn_at_play", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.commands.push(EngineCommand::KeyboardLearnSelect { target: crate::keymap::KeyTarget::ParamPath(play_path) });
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
                                            let trans_path = ParamAddress::deck(&deck.uuid, DeckTarget::AutoTransitionFade).to_string();
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
                                                    actions.commands.push(EngineCommand::MidiLearnSelect { path: trans_path.clone() });
                                                }
                                            }
                                            if data.keyboard_learn_active {
                                                let is_target = data.keyboard_learn_target.as_deref() == Some(trans_path.as_str());
                                                if is_target { widgets::draw_keyboard_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_keyboard_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("kb_learn_at_trans", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.commands.push(EngineCommand::KeyboardLearnSelect { target: crate::keymap::KeyTarget::ParamPath(trans_path) });
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

                                // Depth-sensor shader preprocessor controls
                                if let Some(prepro) = &deck.depth_prepro {
                                    render_depth_prepro_controls(
                                        ui, deck, prepro, data, actions,
                                    );
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
                                    let deck_uuid_unassign = deck_uuid.clone();
                                    let deck_uuid_remove = deck_uuid.clone();
                                    let deck_uuid_automate = deck_uuid.clone();
                                    widgets::render_params(
                                        ui,
                                        &gen_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| EngineCommand::SetGeneratorParam { deck_uuid: deck.uuid.clone(), name: name.to_string(), value: val },
                                        Some(&|name: &str, source_uuid: &str| EngineCommand::AssignModulation {
                                            target: ParamAddress::deck_param(&deck_uuid_assign, name).to_string(), source_id: source_uuid.to_string(), amount: DEFAULT_ASSIGNMENT_AMOUNT,
                                        }),
                                        Some(&|name: &str, source_uuid: &str| EngineCommand::ClearModulationSource {
                                            target: ParamAddress::deck_param(&deck_uuid_unassign, name).to_string(), source_id: source_uuid.to_string(),
                                        }),
                                        Some(&|name: &str| EngineCommand::ClearModulation {
                                            target: ParamAddress::deck_param(&deck_uuid_remove, name).to_string(),
                                        }),
                                        Some(&|name: &str| EngineCommand::AddAutomationLane {
                                            target: ParamAddress::deck_param(&deck_uuid_automate, name).to_string(),
                                            timebase: crate::timebase::Timebase::Transport,
                                        }),
                                        &mut actions.commands,
                                        &mut actions.session.gesture_active,
                                        &format!("sel_{ch_idx}_{deck_idx}"),
                                        Some(&midi_path_prefix),
                                        data.midi_learn_active,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &crate::engine::value::param::deck_param_prefix(&deck_uuid),
                                        data.keyboard_learn_active,
                                        data.keyboard_learn_target.as_deref(),
                                    );
                                    ui.add_space(4.0);
                                    render_exploration_controls(ui, &deck.uuid, &gen_params.params, &mut actions.commands);
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
                            egui::ScrollArea::vertical().id_salt(format!("deck_fx_scroll_{}_{}", deck.uuid, eff_uuid)).max_height(max_h).scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, scroll_bar: true, mouse_wheel: true }).show(ui, |ui| {
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
                                    let eff_uuid_unassign = eff_uuid.clone();
                                    let eff_uuid_remove = eff_uuid.clone();
                                    let eff_uuid_automate = eff_uuid.clone();
                                    let eff_midi_prefix = format!("effect/{eff_uuid}");
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| EngineCommand::SetEffectParam { effect_uuid: eff_uuid_param.clone(), name: name.to_string(), value: val },
                                        Some(&|name: &str, source_uuid: &str| EngineCommand::AssignModulation {
                                            target: ParamAddress::effect_param(&eff_uuid_assign, name).to_string(), source_id: source_uuid.to_string(), amount: DEFAULT_ASSIGNMENT_AMOUNT,
                                        }),
                                        Some(&|name: &str, source_uuid: &str| EngineCommand::ClearModulationSource {
                                            target: ParamAddress::effect_param(&eff_uuid_unassign, name).to_string(), source_id: source_uuid.to_string(),
                                        }),
                                        Some(&|name: &str| EngineCommand::ClearModulation {
                                            target: ParamAddress::effect_param(&eff_uuid_remove, name).to_string(),
                                        }),
                                        Some(&|name: &str| EngineCommand::AddAutomationLane {
                                            target: ParamAddress::effect_param(&eff_uuid_automate, name).to_string(),
                                            timebase: crate::timebase::Timebase::Transport,
                                        }),
                                        &mut actions.commands,
                                        &mut actions.session.gesture_active,
                                        &format!("fx_{deck_uuid_eff}_{eff_uuid}"),
                                        Some(&eff_midi_prefix),
                                        data.midi_learn_active,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &crate::engine::value::param::effect_param_prefix(eff_uuid),
                                        data.keyboard_learn_active,
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
