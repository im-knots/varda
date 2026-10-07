//! Macro controls UI, following the transition-sequence pattern.
//!
//! - A compact column of macro widgets in the central mixer column
//!   (`render_macro_column`): the live control plus a click-to-select name.
//! - Selecting a macro shows its detail editor in the bottom bar
//!   (`render_macro_detail`): kind, value, per-target range/curve/invert, and
//!   button behavior/triggers.
//!
//! Reads `UIData.macros` and emits `EngineCommand`s and selection actions; the
//! engine does all state mutation.

use super::super::{ModSourceUI, UIActions, UIData, modulator_color, widgets};
use crate::engine::EngineCommand;
use crate::engine::value::param::{DeckTarget, ParamAddress};
use crate::macros::{ButtonBehavior, GlobalAction, Macro, MacroCurve, MacroKind, TriggerAction};
use crate::modulation::DEFAULT_ASSIGNMENT_AMOUNT;
use crate::params::ParamValue;

/// Compact macro column for the central mixer area: live controls plus add
/// buttons. Clicking a macro selects it for editing in the bottom bar.
pub(super) fn render_macro_column(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Macros").small().strong());
        if data.midi_learn_active {
            ui.colored_label(
                egui::Color32::from_rgb(180, 80, 220),
                egui::RichText::new("· click a control to map").small(),
            );
        }
    });

    for (idx, m) in data.macros.iter().enumerate() {
        render_macro_compact(ui, idx, m, data, actions);
    }

    ui.add_space(2.0);
    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("+").small());
            if ui
                .small_button("Knob")
                .on_hover_text("Add a knob macro")
                .clicked()
            {
                actions.commands.push(EngineCommand::AddMacro {
                    kind: MacroKind::Knob,
                });
            }
            if ui
                .small_button("Fader")
                .on_hover_text("Add a fader macro")
                .clicked()
            {
                actions.commands.push(EngineCommand::AddMacro {
                    kind: MacroKind::Fader,
                });
            }
            if ui
                .small_button("Button")
                .on_hover_text("Add a button macro")
                .clicked()
            {
                actions.commands.push(EngineCommand::AddMacro {
                    kind: MacroKind::Button,
                });
            }
        });
    });
}

/// One compact macro widget: a live control with its name. Clicking the card
/// around the control opens the macro's config in the bottom bar. As with deck
/// thumbnails, a background response senses card clicks while the controls
/// drawn on top capture their own.
fn render_macro_compact(
    ui: &mut egui::Ui,
    idx: usize,
    m: &Macro,
    data: &UIData,
    actions: &mut UIActions,
) {
    let accent = modulator_color(idx);
    let selected = data.selected_macro.as_deref() == Some(m.uuid.as_str());
    let border = if selected {
        accent
    } else {
        egui::Color32::from_rgb(60, 60, 80)
    };
    let border_w = if selected { 2.0_f32 } else { 1.0_f32 };

    let padding = 4.0_f32;
    let spacing = 4.0_f32;
    let header_h = 18.0_f32;
    let control_h = match m.kind {
        MacroKind::Knob => 42.0_f32,
        MacroKind::Fader => 20.0_f32,
        MacroKind::Button => 28.0_f32,
    };
    let card_w = ui.available_width();
    let total_h = padding + header_h + spacing + control_h + padding;

    // Background senses clicks off the control and opens the config.
    let (card_rect, card_resp) =
        ui.allocate_exact_size(egui::vec2(card_w, total_h), egui::Sense::click());
    let painter = ui.painter().clone();
    painter.rect_filled(card_rect, 4.0, egui::Color32::from_rgb(18, 18, 28));
    painter.rect_stroke(
        card_rect,
        4.0,
        egui::Stroke::new(border_w, border),
        egui::StrokeKind::Inside,
    );

    let inner = card_rect.shrink(padding);
    let header_rect = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), header_h));
    let control_rect = egui::Rect::from_min_max(
        egui::pos2(inner.min.x, header_rect.max.y + spacing),
        inner.max,
    );

    // Header: color dot and name; the name color follows selection.
    let mut header_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    color_dot(&mut header_ui, accent);
    let name_color = if selected {
        accent
    } else {
        header_ui.style().visuals.text_color()
    };
    header_ui
        .label(egui::RichText::new(super::utils::truncate_chars(&m.name, 12)).color(name_color));
    // Delete button, right-aligned like on sequence cards.
    header_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .small_button(egui::RichText::new("x").small())
            .on_hover_text("Delete macro")
            .clicked()
        {
            actions.commands.push(EngineCommand::RemoveMacro {
                uuid: m.uuid.clone(),
            });
            if selected {
                actions.session.deselect_macro = true;
            }
        }
    });

    let mut control_ui = ui.new_child(egui::UiBuilder::new().max_rect(control_rect));
    render_macro_value(&mut control_ui, m, accent, data, actions, 40.0);

    // A click on the card, not the control, selects the macro for editing.
    if !data.midi_learn_active && card_resp.clicked() {
        actions.session.select_macro = Some(m.uuid.clone());
    }
    card_resp.on_hover_text("Click to edit this macro in the bottom bar");

    ui.add_space(2.0);
}

/// Full editor for the selected macro, in the bottom bar.
pub(super) fn render_macro_detail(
    ui: &mut egui::Ui,
    uuid: &str,
    data: &UIData,
    actions: &mut UIActions,
) {
    let Some((idx, m)) = data.macros.iter().enumerate().find(|(_, m)| m.uuid == uuid) else {
        ui.weak("Macro not found — it may have been deleted.");
        return;
    };
    let accent = modulator_color(idx);
    let paths = collect_target_paths(data);

    ui.horizontal(|ui| {
        color_dot(ui, accent);

        let id = ui.id().with(("macro_detail_name", &m.uuid));
        let mut name = ui
            .data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| m.name.clone());
        let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(160.0));
        if resp.changed() {
            ui.data_mut(|d| d.insert_temp(id, name.clone()));
        }
        if resp.lost_focus() {
            if name != m.name {
                actions.commands.push(EngineCommand::RenameMacro {
                    uuid: m.uuid.clone(),
                    name: name.clone(),
                });
            }
            ui.data_mut(|d| d.insert_temp(id, m.name.clone()));
        }

        render_kind_selector(ui, m, actions);

        if ui.button("Delete").clicked() {
            actions.commands.push(EngineCommand::RemoveMacro {
                uuid: m.uuid.clone(),
            });
            actions.session.deselect_macro = true;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("x Close").clicked() {
                actions.session.deselect_macro = true;
            }
        });
    });
    ui.separator();

    // Value control on the left, target/trigger editor on the right.
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(200.0);
            render_macro_value(ui, m, accent, data, actions, 64.0);
            ui.add_space(4.0);
            if matches!(m.kind, MacroKind::Button) {
                render_button_behavior_selector(ui, m, actions);
                ui.add_space(2.0);
                ui.label(egui::RichText::new(format!("value {:.2}", m.value)).small());
            } else {
                render_macro_modulation(ui, m, data, actions);
                ui.add_space(2.0);
                ui.label(macro_value_text(m, data));
            }
        });

        ui.separator();

        ui.vertical(|ui| {
            let is_trigger = matches!(m.kind, MacroKind::Button)
                && m.button.as_ref().map(|b| b.behavior) == Some(ButtonBehavior::Trigger);
            egui::ScrollArea::vertical()
                .id_salt(("macro_detail_scroll", &m.uuid))
                .max_height(ui.available_height())
                .show(ui, |ui| {
                    if is_trigger {
                        render_trigger_editor(ui, m, &paths, actions);
                    } else {
                        render_targets_editor(ui, m, &paths, actions);
                    }
                });
        });
    });
}

/// Live macro control (knob, fader or button). `knob_diameter` sets the knob
/// size for the compact column (small) and the detail editor (large).
fn render_macro_value(
    ui: &mut egui::Ui,
    m: &Macro,
    accent: egui::Color32,
    data: &UIData,
    actions: &mut UIActions,
    knob_diameter: f32,
) {
    let path = ParamAddress::macro_value(&m.uuid).to_string();
    // For a modulated macro, the effective value and modulator color for the ghost
    // indicator, as on params.
    let ghost = macro_modulation_ghost(m, data);
    match m.kind {
        MacroKind::Knob => {
            let mut v = m.value;
            let knob_ghost = ghost.map(|(off, color)| ((v + off).clamp(0.0, 1.0), color));
            let resp = ui
                .vertical_centered(|ui| {
                    widgets::render_knob(ui, &mut v, knob_diameter, accent, knob_ghost)
                })
                .inner;
            if resp.changed() {
                actions.commands.push(EngineCommand::SetMacroValue {
                    uuid: m.uuid.clone(),
                    value: v,
                });
            }
            macro_learn_overlay(ui, resp.rect, path, data, actions);
        }
        MacroKind::Fader => {
            let mut v = m.value;
            let resp = ui
                .vertical_centered(|ui| {
                    ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(true))
                })
                .inner;
            if resp.changed() {
                actions.commands.push(EngineCommand::SetMacroValue {
                    uuid: m.uuid.clone(),
                    value: v,
                });
            }
            // Ghost line at the modulated value on the slider track.
            if let Some((off, color)) = ghost {
                let effective = (m.value + off).clamp(0.0, 1.0);
                let r = resp.rect;
                let x = r.left() + effective * r.width();
                ui.painter().line_segment(
                    [egui::pos2(x, r.top()), egui::pos2(x, r.bottom())],
                    egui::Stroke::new(2.0_f32, color),
                );
            }
            macro_learn_overlay(ui, resp.rect, path, data, actions);
        }
        MacroKind::Button => {
            let behavior = m.button.as_ref().map(|b| b.behavior).unwrap_or_default();
            let on = m.value > 0.5;
            let label = if on { "ON" } else { "OFF" };
            let fill = if on {
                egui::Color32::from_rgb(80, 160, 90)
            } else {
                ui.style().visuals.widgets.inactive.bg_fill
            };
            let width = ui.available_width().max(40.0);
            let resp = ui.add_sized([width, 28.0], egui::Button::new(label).fill(fill));
            macro_learn_overlay(ui, resp.rect, path, data, actions);

            if behavior == ButtonBehavior::Trigger {
                // Fire once: 1.0 then 0.0, so the next click is a new rising edge.
                if resp.clicked() {
                    actions.commands.push(EngineCommand::SetMacroValue {
                        uuid: m.uuid.clone(),
                        value: 1.0,
                    });
                    actions.commands.push(EngineCommand::SetMacroValue {
                        uuid: m.uuid.clone(),
                        value: 0.0,
                    });
                }
            } else {
                // Momentary/Toggle: emit on actual press and release transitions.
                let down = resp.is_pointer_button_down_on();
                let id = ui.id().with(("macro_btn_down", &m.uuid));
                let prev = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                if down != prev {
                    ui.data_mut(|d| d.insert_temp(id, down));
                    actions.commands.push(EngineCommand::SetMacroValue {
                        uuid: m.uuid.clone(),
                        value: if down { 1.0 } else { 0.0 },
                    });
                }
            }
        }
    }
}

fn render_kind_selector(ui: &mut egui::Ui, m: &Macro, actions: &mut UIActions) {
    egui::ComboBox::from_id_salt(("macro_kind", &m.uuid))
        .selected_text(kind_label(m.kind))
        .width(90.0)
        .show_ui(ui, |ui| {
            for kind in [MacroKind::Knob, MacroKind::Fader, MacroKind::Button] {
                if ui
                    .selectable_label(m.kind == kind, kind_label(kind))
                    .clicked()
                    && m.kind != kind
                {
                    actions.commands.push(EngineCommand::SetMacroKind {
                        uuid: m.uuid.clone(),
                        kind,
                    });
                }
            }
        });
}

fn render_button_behavior_selector(ui: &mut egui::Ui, m: &Macro, actions: &mut UIActions) {
    let behavior = m.button.as_ref().map(|b| b.behavior).unwrap_or_default();
    egui::ComboBox::from_id_salt(("macro_btn_behavior", &m.uuid))
        .selected_text(behavior_label(behavior))
        .width(110.0)
        .show_ui(ui, |ui| {
            for b in [
                ButtonBehavior::Momentary,
                ButtonBehavior::Toggle,
                ButtonBehavior::Trigger,
            ] {
                if ui
                    .selectable_label(behavior == b, behavior_label(b))
                    .clicked()
                    && behavior != b
                {
                    actions
                        .commands
                        .push(EngineCommand::SetMacroButtonBehavior {
                            uuid: m.uuid.clone(),
                            behavior: b,
                        });
                }
            }
        });
}

/// Assign or clear a modulation source on a Knob/Fader macro's value. The
/// modulator drives the whole macro (base + offset, to all targets) each frame.
fn render_macro_modulation(ui: &mut egui::Ui, m: &Macro, data: &UIData, actions: &mut UIActions) {
    let key = Macro::value_mod_key(&m.uuid);
    let assigns = data.modulation_assignments.get(&key);

    ui.label(egui::RichText::new("Mod").small());

    if data.modulation_sources.is_empty() {
        ui.weak(egui::RichText::new("(no modulators)").small());
        return;
    }

    // Current assignments, one per row with a delete button.
    if let Some(list) = assigns {
        for a in list {
            let idx = data
                .modulation_sources
                .iter()
                .position(|e| e.uuid == a.source_id);
            ui.horizontal(|ui| {
                if let Some(idx) = idx {
                    color_dot(ui, modulator_color(idx));
                    ui.label(
                        egui::RichText::new(data.modulation_sources[idx].label(idx))
                            .small()
                            .color(modulator_color(idx)),
                    );
                } else {
                    ui.weak(egui::RichText::new("(missing source)").small());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("x")
                        .on_hover_text("Remove this modulation")
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::ClearModulationSource {
                            target: Macro::value_mod_key(&m.uuid),
                            source_id: a.source_id.clone(),
                        });
                    }
                });
            });
        }
    }

    egui::ComboBox::from_id_salt(("macro_mod", &m.uuid))
        .selected_text("+ Modulate")
        .width(140.0)
        .show_ui(ui, |ui| {
            ui.label(egui::RichText::new("Assign Modulation").small().strong());
            for (idx, entry) in data.modulation_sources.iter().enumerate() {
                // An envelope drives the one parameter it was drawn for; a macro drives many.
                if matches!(entry.source, ModSourceUI::Envelope { .. }) {
                    continue;
                }
                if ui
                    .button(egui::RichText::new(entry.label(idx)).color(modulator_color(idx)))
                    .clicked()
                {
                    actions.commands.push(EngineCommand::AssignModulation {
                        target: Macro::value_mod_key(&m.uuid),
                        source_id: entry.uuid.clone(),
                        amount: DEFAULT_ASSIGNMENT_AMOUNT,
                    });
                }
            }
        });
}

/// Value label: the base and, when modulated, the effective value
/// (`base → effective`).
fn macro_value_text(m: &Macro, data: &UIData) -> egui::RichText {
    let key = Macro::value_mod_key(&m.uuid);
    if let Some(list) = data.modulation_assignments.get(&key)
        && !list.is_empty()
    {
        let offset: f32 = list
            .iter()
            .map(|a| {
                data.modulation_current_values
                    .get(&a.source_id)
                    .copied()
                    .unwrap_or(0.0)
                    * a.amount
            })
            .sum();
        let effective = (m.value + offset).clamp(0.0, 1.0);
        return egui::RichText::new(format!("value {:.2} → {:.2}", m.value, effective)).small();
    }
    egui::RichText::new(format!("value {:.2}", m.value)).small()
}

/// Live modulation offset on a Knob/Fader macro's value and the color of the
/// first driving source, or `None` when not modulated. Drawn as a ghost
/// indicator.
fn macro_modulation_ghost(m: &Macro, data: &UIData) -> Option<(f32, egui::Color32)> {
    if !matches!(m.kind, MacroKind::Knob | MacroKind::Fader) {
        return None;
    }
    let key = Macro::value_mod_key(&m.uuid);
    let list = data.modulation_assignments.get(&key)?;
    if list.is_empty() {
        return None;
    }
    let offset: f32 = list
        .iter()
        .map(|a| {
            data.modulation_current_values
                .get(&a.source_id)
                .copied()
                .unwrap_or(0.0)
                * a.amount
        })
        .sum();
    let color = data
        .modulation_sources
        .iter()
        .position(|e| e.uuid == list[0].source_id)
        .map_or(egui::Color32::YELLOW, modulator_color);
    Some((offset, color))
}

/// Paint a small color dot, vertically centered with adjacent text. Painted
/// rather than a "●" glyph so it renders with any bundled UI font.
fn color_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let d = 8.0_f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(d, d), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), d * 0.5, color);
}

fn render_targets_editor(
    ui: &mut egui::Ui,
    m: &Macro,
    paths: &[(String, String)],
    actions: &mut UIActions,
) {
    ui.label(egui::RichText::new(format!("Targets ({})", m.targets.len())).small());
    for (ti, t) in m.targets.iter().enumerate() {
        // Address and its range/curve/invert controls share one row.
        ui.horizontal(|ui| {
            let mut min = t.min;
            let mut max = t.max;
            let mut curve = t.curve;
            let mut invert = t.invert;
            let mut changed = false;

            // Fixed-width truncated address keeps the setting columns aligned; the full
            // path shows on hover.
            ui.add_sized(
                [150.0, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new(short_path(&t.path)).small().monospace())
                    .truncate(),
            )
            .on_hover_text(&t.path);

            ui.label("min");
            changed |= ui
                .add(egui::DragValue::new(&mut min).speed(0.01).range(0.0..=1.0))
                .changed();
            ui.label("max");
            changed |= ui
                .add(egui::DragValue::new(&mut max).speed(0.01).range(0.0..=1.0))
                .changed();
            changed |= ui.checkbox(&mut invert, "inv").changed();

            egui::ComboBox::from_id_salt(("macro_curve", &m.uuid, ti))
                .selected_text(curve_label(curve))
                .width(72.0)
                .show_ui(ui, |ui| {
                    for c in [
                        MacroCurve::Linear,
                        MacroCurve::Exponential,
                        MacroCurve::Logarithmic,
                        MacroCurve::SCurve,
                        MacroCurve::Stepped(4),
                    ] {
                        if ui.selectable_label(curve == c, curve_label(c)).clicked() && curve != c {
                            curve = c;
                            changed = true;
                        }
                    }
                });

            if changed {
                actions.commands.push(EngineCommand::UpdateMacroTarget {
                    uuid: m.uuid.clone(),
                    target_idx: ti,
                    min,
                    max,
                    curve,
                    invert,
                });
            }

            if ui
                .small_button("x")
                .on_hover_text("Remove target")
                .clicked()
            {
                actions.commands.push(EngineCommand::RemoveMacroTarget {
                    uuid: m.uuid.clone(),
                    target_idx: ti,
                });
            }
        });
    }
    add_target_combo(ui, &m.uuid, paths, actions, false);
}

fn render_trigger_editor(
    ui: &mut egui::Ui,
    m: &Macro,
    paths: &[(String, String)],
    actions: &mut UIActions,
) {
    let triggers = m
        .button
        .as_ref()
        .map(|b| b.trigger.clone())
        .unwrap_or_default();

    ui.label(egui::RichText::new("On press:").small());

    ui.horizontal(|ui| {
        for (ga, label) in [
            (GlobalAction::Undo, "Undo"),
            (GlobalAction::Redo, "Redo"),
            (GlobalAction::Save, "Save"),
        ] {
            let present = triggers
                .iter()
                .any(|t| matches!(t, TriggerAction::Global(g) if *g == ga));
            let mut checked = present;
            if ui.checkbox(&mut checked, label).changed() {
                let mut next = triggers.clone();
                if checked {
                    next.push(TriggerAction::Global(ga));
                } else {
                    next.retain(|t| !matches!(t, TriggerAction::Global(g) if *g == ga));
                }
                actions.commands.push(EngineCommand::SetMacroTriggers {
                    uuid: m.uuid.clone(),
                    actions: next,
                });
            }
        }
    });

    for (i, t) in triggers.iter().enumerate() {
        if let TriggerAction::Param { path, .. } = t {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(short_path(path)).small().monospace())
                    .on_hover_text(path);
                if ui.small_button("x").clicked() {
                    let mut next = triggers.clone();
                    next.remove(i);
                    actions.commands.push(EngineCommand::SetMacroTriggers {
                        uuid: m.uuid.clone(),
                        actions: next,
                    });
                }
            });
        }
    }

    if let Some(path) = target_picker_combo(ui, &m.uuid, paths, true) {
        let mut next = triggers.clone();
        next.push(TriggerAction::Param { path, value: 1.0 });
        actions.commands.push(EngineCommand::SetMacroTriggers {
            uuid: m.uuid.clone(),
            actions: next,
        });
    }
}

/// Combo that appends the selected target.
fn add_target_combo(
    ui: &mut egui::Ui,
    uuid: &str,
    paths: &[(String, String)],
    actions: &mut UIActions,
    trigger: bool,
) {
    if let Some(path) = target_picker_combo(ui, uuid, paths, trigger) {
        actions.commands.push(EngineCommand::AddMacroTarget {
            uuid: uuid.to_string(),
            path,
        });
    }
}

/// A "+ Add target" combo listing all mappable parameter paths. Returns the
/// path selected this frame, if any.
fn target_picker_combo(
    ui: &mut egui::Ui,
    uuid: &str,
    paths: &[(String, String)],
    trigger: bool,
) -> Option<String> {
    let mut chosen = None;
    let salt = if trigger {
        ("macro_add_trigger", uuid)
    } else {
        ("macro_add_target", uuid)
    };
    let text = if trigger {
        "+ Add param"
    } else {
        "+ Add target"
    };
    egui::ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(240.0)
        .show_ui(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    for (label, path) in paths {
                        if ui.selectable_label(false, label).clicked() {
                            chosen = Some(path.clone());
                        }
                    }
                });
        });
    chosen
}

/// MIDI-learn overlay for the macro value control, mapping to
/// `macro/<uuid>/value` through the shared learn state.
fn macro_learn_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: String,
    data: &UIData,
    actions: &mut UIActions,
) {
    if !data.midi_learn_active {
        return;
    }
    if data.midi_learn_target.as_deref() == Some(path.as_str()) {
        widgets::draw_midi_learn_selected(ui, rect);
    } else {
        widgets::draw_midi_learn_glow(ui, rect);
    }
    let id = ui.id().with(("macro_midi_learn", path.as_str()));
    if ui.interact(rect, id, egui::Sense::click()).clicked() {
        actions
            .commands
            .push(EngineCommand::MidiLearnSelect { path });
    }
}

/// All mappable scalar parameter paths in the UI snapshot, as `(label, path)`
/// pairs for the target picker.
fn collect_target_paths(data: &UIData) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    out.push(("Crossfader".to_string(), "crossfader".to_string()));

    for ch in &data.channels {
        out.push((
            format!("{} · opacity", ch.name),
            ParamAddress::channel_opacity(&ch.uuid).to_string(),
        ));
        for crate::usecases::ui::EffectInfo {
            uuid: fx_uuid,
            name: fx_name,
            params,
            ..
        } in &ch.effects
        {
            for p in &params.params {
                if matches!(p.value, ParamValue::Float(_)) {
                    out.push((
                        format!("{} · {} · {}", ch.name, fx_name, param_label(p)),
                        ParamAddress::effect_param(fx_uuid, &p.name).to_string(),
                    ));
                }
            }
        }
        for d in &ch.decks {
            out.push((
                format!("{} · {} · opacity", ch.name, d.name),
                ParamAddress::deck(&d.uuid, DeckTarget::Opacity).to_string(),
            ));
            for p in &d.generator.params {
                if matches!(p.value, ParamValue::Float(_)) {
                    out.push((
                        format!("{} · {}", d.name, param_label(p)),
                        ParamAddress::deck_param(&d.uuid, &p.name).to_string(),
                    ));
                }
            }
            for crate::usecases::ui::EffectInfo {
                uuid: fx_uuid,
                name: fx_name,
                params,
                ..
            } in &d.effects
            {
                for p in &params.params {
                    if matches!(p.value, ParamValue::Float(_)) {
                        out.push((
                            format!("{} · {} · {}", d.name, fx_name, param_label(p)),
                            ParamAddress::effect_param(fx_uuid, &p.name).to_string(),
                        ));
                    }
                }
            }
            // Every modulatable source control, as in the 〰 dropdowns. In and out points
            // are excluded because a position offset is measured against them.
            if let Some(ty) = data
                .sources
                .iter()
                .find(|t| t.type_id == d.source.source_type)
            {
                for spec in ty.params.iter().filter(|s| s.modulatable) {
                    if let Some(route) = spec.route.as_deref() {
                        out.push((
                            format!("{} · {} · {}", ch.name, d.name, spec.label.to_lowercase()),
                            ParamAddress::deck(&d.uuid, DeckTarget::source(route)).to_string(),
                        ));
                    }
                }
            }
        }
    }

    for crate::usecases::ui::EffectInfo {
        uuid: fx_uuid,
        name: fx_name,
        params,
        ..
    } in &data.master_effect_info
    {
        for p in &params.params {
            if matches!(p.value, ParamValue::Float(_)) {
                out.push((
                    format!("Master · {} · {}", fx_name, param_label(p)),
                    format!("master/effect/{}/param/{}", fx_uuid, p.name),
                ));
            }
        }
    }

    // Modulator params (LFO rate, ADSR stages, ...), routed through
    // `mod/<uuid>/<param>`. Keep these in sync with `param_router::apply_mod_param`.
    for (idx, entry) in data.modulation_sources.iter().enumerate() {
        let base = entry.label(idx);
        let params: &[&str] = match &entry.source {
            ModSourceUI::LFO { .. } => &["frequency", "amplitude", "phase"],
            ModSourceUI::Audio { .. } => &["gain", "smoothing", "freq_low", "freq_high"],
            ModSourceUI::ADSR { .. } => &["attack", "decay", "sustain", "release"],
            ModSourceUI::StepSequencer { .. } => &["rate"],
            ModSourceUI::Analyzer { .. } => &["smoothing"],
            // An envelope's shape is its breakpoints, not a scalar. Excluding it keeps
            // envelopes out of the mod-on-mod graph.
            ModSourceUI::Envelope { .. } => &[],
        };
        for p in params {
            out.push((
                format!("{base} · {p}"),
                ParamAddress::modulator_param(&entry.uuid, p).to_string(),
            ));
        }
    }

    out
}

fn param_label(p: &super::super::ParamUIInfo) -> String {
    p.label.clone().unwrap_or_else(|| p.name.clone())
}

/// Shorten a router path for display to its last two segments.
fn short_path(path: &str) -> String {
    let segs: Vec<&str> = path.split('/').collect();
    if segs.len() <= 2 {
        path.to_string()
    } else {
        format!("…/{}/{}", segs[segs.len() - 2], segs[segs.len() - 1])
    }
}

fn kind_label(kind: MacroKind) -> &'static str {
    match kind {
        MacroKind::Knob => "Knob",
        MacroKind::Fader => "Fader",
        MacroKind::Button => "Button",
    }
}

fn behavior_label(b: ButtonBehavior) -> &'static str {
    match b {
        ButtonBehavior::Momentary => "Momentary",
        ButtonBehavior::Toggle => "Toggle",
        ButtonBehavior::Trigger => "Trigger",
    }
}

fn curve_label(c: MacroCurve) -> &'static str {
    match c {
        MacroCurve::Linear => "Linear",
        MacroCurve::Exponential => "Exp",
        MacroCurve::Logarithmic => "Log",
        MacroCurve::SCurve => "S-Curve",
        MacroCurve::Stepped(_) => "Stepped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::{ButtonSpec, MacroTarget};

    fn fixture_with_macros() -> UIData {
        let mut data = UIData::test_fixture();

        let mut knob = Macro::new(MacroKind::Knob, "Sweep");
        knob.targets.push(MacroTarget::new("crossfader"));
        data.macros.push(knob);

        data.macros.push(Macro::new(MacroKind::Fader, "Blend"));

        let mut trigger = Macro::new(MacroKind::Button, "Panic");
        trigger.button = Some(ButtonSpec {
            behavior: ButtonBehavior::Trigger,
            trigger: vec![TriggerAction::Global(GlobalAction::Undo)],
        });
        data.macros.push(trigger);

        data
    }

    #[test]
    fn render_macro_column_smoke_empty() {
        let mut data = UIData::test_fixture();
        data.macros.clear();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_column(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_macro_column_smoke_with_macros() {
        let data = fixture_with_macros();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_column(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_macro_column_smoke_midi_learn() {
        let mut data = fixture_with_macros();
        data.midi_learn_active = true;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_column(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_macro_detail_smoke_knob() {
        let data = fixture_with_macros();
        let uuid = data.macros[0].uuid.clone();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_detail(ui, &uuid, &data, &mut actions);
        });
    }

    /// An envelope drives the one parameter it was drawn for, so macros cannot
    /// target it.
    #[test]
    fn a_curve_is_not_offered_to_a_macro() {
        use crate::usecases::ui::ModSourceUIEntry;
        use egui_kittest::kittest::Queryable;

        let mut data = fixture_with_macros();
        let lfo_idx = data.modulation_sources.len();
        data.modulation_sources.push(ModSourceUIEntry {
            uuid: "lfo1".to_string(),
            source: ModSourceUI::LFO {
                waveform: crate::modulation::LFOWaveform::Sine,
                frequency: 1.0,
                phase: 0.0,
                amplitude: 1.0,
                bipolar: false,
            },
            timebase: crate::timebase::Timebase::FreeRun,
        });
        let curve_idx = data.modulation_sources.len();
        data.modulation_sources.push(ModSourceUIEntry {
            uuid: "env1".to_string(),
            source: ModSourceUI::Envelope {
                breakpoints: Vec::new(),
            },
            timebase: crate::timebase::Timebase::Transport,
        });
        let lfo = data.modulation_sources[lfo_idx].label(lfo_idx);
        let curve = data.modulation_sources[curve_idx].label(curve_idx);

        let uuid = data.macros[0].uuid.clone();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_detail(ui, &uuid, &data, &mut actions);
        });
        harness.get_by_value("+ Modulate").click();
        harness.run();

        assert!(
            harness.query_by_label(&lfo).is_some(),
            "an ordinary modulator is still on offer"
        );
        assert!(
            harness.query_by_label(&curve).is_none(),
            "{curve} must not be assignable to a macro"
        );
    }

    #[test]
    fn render_macro_detail_smoke_trigger_button() {
        let data = fixture_with_macros();
        let uuid = data.macros[2].uuid.clone();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_detail(ui, &uuid, &data, &mut actions);
        });
    }

    #[test]
    fn render_macro_detail_missing_uuid_is_graceful() {
        let data = fixture_with_macros();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_macro_detail(ui, "does-not-exist", &data, &mut actions);
        });
    }
}
