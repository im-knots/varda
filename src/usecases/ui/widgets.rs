use super::{
    AudioUIData, ModAssignmentUI, ModSourceUI, ModSourceUIEntry, ParamUIInfo, modulator_color,
};
use crate::engine::EngineCommand;
use crate::params::ParamValue;

/// Callback that builds a modulation-assignment command from (`param_path`, `source_name`).
type MakeModAssign<'a> = &'a dyn Fn(&str, &str) -> EngineCommand;

/// Callback that builds a command from a `param_path` alone.
type MakeParamCommand<'a> = &'a dyn Fn(&str) -> EngineCommand;

/// The commands the 〰 dropdown can emit. Bundled because only the caller knows
/// how to build a parameter's full target path, and they always travel together.
struct ModMenu<'a> {
    assign: MakeModAssign<'a>,
    /// Detach one source, leaving any others on the parameter alone. Without it
    /// the checklist could only ever be ticked, and un-ticking would have to go
    /// through "Clear all" and lose the other sources with it.
    unassign: MakeModAssign<'a>,
    remove: MakeParamCommand<'a>,
    automate: Option<MakeParamCommand<'a>>,
}

/// One line of the 〰 checklist.
struct ModRow<'a> {
    /// Index in the *unfiltered* source list, which picks the modulator's colour
    /// and number. Those have to match the modulation panel's cards.
    idx: usize,
    entry: &'a ModSourceUIEntry,
    assigned: bool,
}

/// One entry of a 〰 checklist, in the modulator's own colour.
///
/// Shared with the mod-on-mod menu in the modulation panel so the two cannot
/// disagree about what a tick looks like.
pub fn mod_tick_label(assigned: bool, label: &str, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(format!("{} {label}", if assigned { "☑" } else { "☐" })).color(color)
}

/// Which sources the checklist lists, and which of them are ticked.
///
/// An automation curve belongs to the one parameter it was drawn for, so an
/// unassigned one is not on offer: sharing a shape between parameters is copy
/// and paste between lanes, which leaves each parameter its own curve to edit.
/// See /spec/automation.md § One envelope per parameter. An *assigned* one is
/// still listed, because a parameter driven by a lane would otherwise show an
/// empty checklist under a coloured ghost, which is the confusion this list
/// exists to remove.
fn mod_rows<'a>(
    modulation_sources: &'a [ModSourceUIEntry],
    assignments: &[ModAssignmentUI],
) -> Vec<ModRow<'a>> {
    modulation_sources
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let assigned = assignments.iter().any(|a| a.source_id == entry.uuid);
            let offerable = !matches!(entry.source, ModSourceUI::Envelope { .. });
            (offerable || assigned).then_some(ModRow {
                idx,
                entry,
                assigned,
            })
        })
        .collect()
}

/// The 〰 dropdown on a modulatable parameter: a checklist of every source, with
/// the ones driving this parameter ticked, plus an automation lane and a clear.
///
/// A checklist rather than a list of assign actions because a parameter can have
/// several sources stacked on it, and the affordances that hint at that only
/// carry one: the ghost line and the coloured label both take the colour of the
/// *first* assignment, so two sources and one source look identical. Ticks are
/// the only place the whole set is visible.
///
/// Shared by the deck and effect param renderers so the two cannot drift.
fn modulation_dropdown(
    ui: &mut egui::Ui,
    id_salt: String,
    param_name: &str,
    modulation_sources: &[ModSourceUIEntry],
    assignments: &[ModAssignmentUI],
    menu: &ModMenu,
    commands: &mut Vec<EngineCommand>,
) {
    let ModMenu {
        assign: assign_fn,
        unassign: unassign_fn,
        remove: remove_fn,
        automate: automate_fn,
    } = *menu;
    let rows = mod_rows(modulation_sources, assignments);
    // Automation needs no existing source, so the menu is worth showing even
    // when nothing has been created yet.
    if rows.is_empty() && automate_fn.is_none() {
        return;
    }

    let active: Vec<String> = rows
        .iter()
        .filter(|r| r.assigned)
        .map(|r| r.entry.label(r.idx))
        .collect();

    let response = egui::ComboBox::from_id_salt(id_salt)
        .selected_text("〰")
        .width(30.0)
        // Ticking one source should not dismiss the list. Stacking two sources
        // is one decision, and closing after each would make it two visits.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            if !rows.is_empty() {
                ui.label(egui::RichText::new("Modulation").small().strong());
                for ModRow {
                    idx,
                    entry,
                    assigned,
                } in &rows
                {
                    let text = mod_tick_label(*assigned, &entry.label(*idx), modulator_color(*idx));
                    if ui.selectable_label(*assigned, text).clicked() {
                        commands.push(if *assigned {
                            unassign_fn(param_name, &entry.uuid)
                        } else {
                            assign_fn(param_name, &entry.uuid)
                        });
                    }
                }
                ui.separator();
            }
            if let Some(automate) = automate_fn
                && ui
                    .button("＋ Automation lane")
                    .on_hover_text(
                        "Draw this parameter as a curve against the show position.\n\
                         The curve sets the value outright, so it plays back the same every run.",
                    )
                    .clicked()
            {
                commands.push(automate(param_name));
            }
            // Only worth offering once there is something to clear, and it says
            // "all" because a checklist makes single removal the obvious gesture.
            if !active.is_empty() {
                ui.separator();
                if ui.button("Clear all").clicked() {
                    commands.push(remove_fn(param_name));
                }
            }
        })
        .response;

    // Readable without opening anything, which is the case that matters on a
    // dark stage. The colours cannot say how many; the names can.
    response.on_hover_text(if active.is_empty() {
        "No modulation assigned".to_string()
    } else {
        format!("Modulated by {}", active.join(", "))
    });
}

/// The 〰 dropdown for a parameter that is a control of its own rather than one
/// row of a param list, addressed by its full modulation key.
///
/// The fader panels have no `ParamUIInfo` to hang a menu off, so they name the
/// key directly. See /spec/modulation.md § Parameter Addressing.
pub fn modulation_menu_for_key<S: std::hash::BuildHasher>(
    ui: &mut egui::Ui,
    id_salt: String,
    param_key: &str,
    modulation_sources: &[ModSourceUIEntry],
    mod_assignments: &std::collections::HashMap<String, Vec<ModAssignmentUI>, S>,
    commands: &mut Vec<EngineCommand>,
) {
    let assign = |target: &str, source_id: &str| EngineCommand::AssignModulation {
        target: target.to_string(),
        source_id: source_id.to_string(),
        amount: 1.0,
    };
    let unassign = |target: &str, source_id: &str| EngineCommand::ClearModulationSource {
        target: target.to_string(),
        source_id: source_id.to_string(),
    };
    let remove = |target: &str| EngineCommand::ClearModulation {
        target: target.to_string(),
    };
    let automate = |target: &str| EngineCommand::AddAutomationLane {
        target: target.to_string(),
        timebase: crate::timebase::Timebase::Transport,
    };
    modulation_dropdown(
        ui,
        id_salt,
        param_key,
        modulation_sources,
        mod_assignments.get(param_key).map_or(&[], Vec::as_slice),
        &ModMenu {
            assign: &assign,
            unassign: &unassign,
            remove: &remove,
            automate: Some(&automate),
        },
        commands,
    );
}

/// Build a set of prefixes whose params should be hidden.
/// Convention: a bool param named `<prefix>_mode` controls visibility of params
/// whose name starts with `<prefix>_`. When the bool is false, those params are hidden.
fn hidden_prefixes(params: &[ParamUIInfo]) -> Vec<String> {
    let mut prefixes = Vec::new();
    for p in params {
        if let Some(stem) = p.name.strip_suffix("_mode")
            && let ParamValue::Bool(false) = p.value
        {
            prefixes.push(format!("{stem}_"));
        }
    }
    prefixes
}

/// Check if a param name should be hidden based on `_mode` toggle conventions.
fn is_hidden(name: &str, hidden: &[String]) -> bool {
    // Don't hide the _mode toggle itself
    if name.ends_with("_mode") {
        return false;
    }
    hidden.iter().any(|prefix| name.starts_with(prefix))
}

/// The named groups of a parameter list, in the order their first member appears.
///
/// First appearance is the only ordering rule, so an author controls section order
/// by ordering `INPUTS` and there is no second mechanism to disagree with it.
fn named_group_order<'a>(params: &[&'a ParamUIInfo]) -> Vec<&'a str> {
    let mut groups: Vec<&str> = Vec::new();
    for name in params.iter().filter_map(|p| p.group.as_deref()) {
        if !groups.contains(&name) {
            groups.push(name);
        }
    }
    groups
}

/// The section headers a parameter list will render, for callers that need to name
/// a group rather than draw it — the exploration controls scope a randomize or a
/// mutate to one of these. See /spec/parameter-exploration.md.
pub fn param_groups(params: &[ParamUIInfo]) -> Vec<&str> {
    let hidden = hidden_prefixes(params);
    let visible: Vec<&ParamUIInfo> = params
        .iter()
        .filter(|p| !is_hidden(&p.name, &hidden))
        .collect();
    named_group_order(&visible)
}

/// Section a parameter list by the shader's `GROUP` keys, calling `row` for each
/// visible parameter.
///
/// Ungrouped parameters come first with no header and no collapse, so a shader can
/// keep its performance controls permanently in view. Named groups follow in
/// first-appearance order, the first open and the rest closed, which is what keeps
/// a fifty-parameter shader scannable. A shader declaring no groups renders as one
/// flat list, exactly as every shader did before groups existed.
/// See /spec/parameter-inspector.md.
fn render_grouped(
    ui: &mut egui::Ui,
    params: &[ParamUIInfo],
    id_prefix: &str,
    row: &mut dyn FnMut(&mut egui::Ui, &ParamUIInfo),
) {
    let hidden = hidden_prefixes(params);
    let visible: Vec<&ParamUIInfo> = params
        .iter()
        .filter(|p| !is_hidden(&p.name, &hidden))
        .collect();

    for param in visible.iter().filter(|p| p.group.is_none()) {
        row(ui, param);
    }

    let groups = named_group_order(&visible);
    for (idx, name) in groups.iter().enumerate() {
        egui::CollapsingHeader::new(egui::RichText::new(*name).small().strong())
            .id_salt(format!("paramgroup_{id_prefix}_{name}"))
            .default_open(idx == 0)
            .show(ui, |ui| {
                for param in visible.iter().filter(|p| p.group.as_deref() == Some(*name)) {
                    row(ui, param);
                }
            });
    }
}

/// A `long` (enum) parameter: a combo over its declared `LABELS`, or a stepper when
/// the shader declares no options. Without this the input is invisible in the
/// inspector even though its value reaches the GPU.
/// See /spec/parameter-inspector.md.
fn render_long_row(
    ui: &mut egui::Ui,
    param: &ParamUIInfo,
    label: egui::RichText,
    current: i32,
    id_prefix: &str,
    make_update: &dyn Fn(&str, ParamValue) -> EngineCommand,
    commands: &mut Vec<EngineCommand>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut selected = current;
        if param.choices.is_empty() {
            let mut drag = egui::DragValue::new(&mut selected).speed(1.0);
            if let (Some(lo), Some(hi)) = (param.min, param.max) {
                drag = drag.range(lo as i32..=hi as i32);
            }
            ui.add(drag);
        } else {
            let text = param
                .choices
                .iter()
                .find(|c| c.value == current)
                .map_or_else(|| current.to_string(), |c| c.label.clone());
            egui::ComboBox::from_id_salt(format!("long_{id_prefix}_{}", param.name))
                .selected_text(egui::RichText::new(text).small())
                .width(90.0)
                .show_ui(ui, |ui| {
                    for choice in &param.choices {
                        ui.selectable_value(
                            &mut selected,
                            choice.value,
                            egui::RichText::new(&choice.label).small(),
                        );
                    }
                });
        }
        if selected != current {
            commands.push(make_update(&param.name, ParamValue::Long(selected)));
        }
    });
}

/// A `point2D` parameter as paired numeric drags. ISF declares one `MIN` and `MAX`
/// for the input rather than one per axis, so both axes share an extent and this is
/// deliberately not an XY pad. See /spec/parameter-inspector.md.
fn render_point2d_row(
    ui: &mut egui::Ui,
    param: &ParamUIInfo,
    label: egui::RichText,
    current: [f32; 2],
    make_update: &dyn Fn(&str, ParamValue) -> EngineCommand,
    commands: &mut Vec<EngineCommand>,
) {
    let mut xy = current;
    let lo = param.min.unwrap_or(0.0);
    let hi = param.max.unwrap_or(1.0);
    let speed = f64::from(hi - lo) * 0.005;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut changed = false;
        for axis in &mut xy {
            changed |= ui
                .add(egui::DragValue::new(axis).speed(speed).range(lo..=hi))
                .changed();
        }
        if changed {
            commands.push(make_update(&param.name, ParamValue::Point2D(xy)));
        }
    });
}

/// Render parameter controls (sliders, checkboxes, color pickers) for a list of params.
/// Returns any param updates generated by user interaction.
// UI render fn taking many independent egui state/handle args; no shared invariant to bundle.
#[allow(clippy::too_many_arguments)]
pub fn render_params<S: std::hash::BuildHasher>(
    ui: &mut egui::Ui,
    params: &[ParamUIInfo],
    modulation_sources: &[ModSourceUIEntry],
    make_update: &dyn Fn(&str, ParamValue) -> EngineCommand,
    make_mod_assign: Option<MakeModAssign>,
    make_mod_unassign: Option<MakeModAssign>,
    make_mod_remove: Option<MakeParamCommand>,
    make_automation: Option<MakeParamCommand>,
    commands: &mut Vec<EngineCommand>,
    gesture_active: &mut bool,
    id_prefix: &str,
    midi_learn_path_prefix: Option<&str>,
    midi_learn_active: bool,
    midi_learn_select: &mut Option<String>,
    midi_learn_target: Option<&str>,
    mod_assignments: &std::collections::HashMap<String, Vec<ModAssignmentUI>, S>,
    mod_current_values: &std::collections::HashMap<String, f32, S>,
    mod_param_prefix: &str,
    keyboard_learn_active: bool,
    keyboard_learn_select: &mut Option<crate::keymap::KeyTarget>,
    keyboard_learn_target: Option<&str>,
) {
    render_grouped(ui, params, id_prefix, &mut |ui, param| {
        let label = param.label.as_ref().unwrap_or(&param.name);
        // Check if this param is modulated and get color info
        let mod_key = format!("{}:{}", mod_param_prefix, param.name);
        let assignments = mod_assignments.get(&mod_key);
        let is_modulated = assignments.is_some_and(|a| !a.is_empty());
        // Pick the primary modulator color (first assignment)
        let mod_label_color = assignments.and_then(|a| a.first()).map(|a| {
            let color_idx = modulation_sources
                .iter()
                .position(|e| e.uuid == a.source_id)
                .unwrap_or(0);
            modulator_color(color_idx)
        });
        match param.value {
            ParamValue::Float(mut v) => {
                let min = param.min.unwrap_or(0.0);
                let max = param.max.unwrap_or(1.0);
                ui.horizontal(|ui| {
                    // Color-code label if modulated
                    if let Some(color) = mod_label_color {
                        ui.label(egui::RichText::new(label).small().color(color));
                    } else {
                        ui.label(egui::RichText::new(label).small());
                    }
                    // Render slider — in learn mode, disable mouse interaction via a scope
                    let any_learn_active = midi_learn_active || keyboard_learn_active;
                    let slider_rect = if any_learn_active {
                        let inner = ui.scope(|ui| {
                            ui.disable();
                            ui.add(egui::Slider::new(&mut v, min..=max).show_value(false))
                        });
                        inner.inner.rect
                    } else {
                        let slider_response =
                            ui.add(egui::Slider::new(&mut v, min..=max).show_value(false));
                        // A held slider drag is a single undo gesture (collapsed
                        // by the runner's `gesture_active` edge).
                        if slider_response.dragged() {
                            *gesture_active = true;
                        }
                        if slider_response.changed() {
                            commands.push(make_update(&param.name, ParamValue::Float(v)));
                        }
                        slider_response.rect
                    };
                    // MIDI learn mode: glow + click overlay on the (now-enabled) outer ui
                    if midi_learn_active && let Some(prefix) = midi_learn_path_prefix {
                        let path = format!("{}/param/{}", prefix, param.name);
                        let is_target = midi_learn_target.is_some_and(|t| t == path);
                        if is_target {
                            draw_midi_learn_selected(ui, slider_rect);
                        } else {
                            draw_midi_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with(("midi_learn_param", &param.name));
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            *midi_learn_select = Some(path);
                        }
                    }
                    // Keyboard learn mode: orange glow + click overlay
                    if keyboard_learn_active && let Some(prefix) = midi_learn_path_prefix {
                        let path = format!("{}/param/{}", prefix, param.name);
                        let is_target = keyboard_learn_target.is_some_and(|t| t == path);
                        if is_target {
                            draw_keyboard_learn_selected(ui, slider_rect);
                        } else {
                            draw_keyboard_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with(("kb_learn_param", &param.name));
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            *keyboard_learn_select =
                                Some(crate::keymap::KeyTarget::ParamPath(path));
                        }
                    }
                    // Draw modulation ghost indicator on top of slider
                    if is_modulated && let Some(assigns) = assignments {
                        let mut total_offset = 0.0f32;
                        for a in assigns {
                            total_offset +=
                                mod_current_values.get(&a.source_id).copied().unwrap_or(0.0)
                                    * a.amount;
                        }
                        // Scale by param range to match GPU-side modulation
                        let range = max - min;
                        let modulated_val = (v + total_offset * range).clamp(min, max);
                        let frac = (modulated_val - min) / (max - min);
                        let x = slider_rect.left() + frac * slider_rect.width();
                        let color = mod_label_color.unwrap_or(egui::Color32::YELLOW);
                        let painter = ui.painter();
                        // Vertical line at modulated value position
                        painter.line_segment(
                            [
                                egui::pos2(x, slider_rect.top()),
                                egui::pos2(x, slider_rect.bottom()),
                            ],
                            egui::Stroke::new(2.0_f32, color),
                        );
                    }
                    if let (Some(assign_fn), Some(unassign_fn), Some(remove_fn)) =
                        (make_mod_assign, make_mod_unassign, make_mod_remove)
                    {
                        modulation_dropdown(
                            ui,
                            format!("mod_{}_{}", id_prefix, param.name),
                            &param.name,
                            modulation_sources,
                            assignments.map_or(&[], Vec::as_slice),
                            &ModMenu {
                                assign: assign_fn,
                                unassign: unassign_fn,
                                remove: remove_fn,
                                automate: make_automation,
                            },
                            commands,
                        );
                    }
                });
            }
            ParamValue::Bool(mut v) => {
                if ui
                    .checkbox(&mut v, egui::RichText::new(label).small())
                    .changed()
                {
                    commands.push(make_update(&param.name, ParamValue::Bool(v)));
                }
            }
            ParamValue::Color(c) => {
                let mut color = [c[0], c[1], c[2], c[3]];
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).small());
                    if ui.color_edit_button_rgba_unmultiplied(&mut color).changed() {
                        commands.push(make_update(&param.name, ParamValue::Color(color)));
                    }
                });
            }
            ParamValue::Long(v) => render_long_row(
                ui,
                param,
                egui::RichText::new(label).small(),
                v,
                id_prefix,
                make_update,
                commands,
            ),
            ParamValue::Point2D(p) => render_point2d_row(
                ui,
                param,
                egui::RichText::new(label).small(),
                p,
                make_update,
                commands,
            ),
        }
    });
}

/// Render effect parameter controls with optional modulation assignment
// UI render fn taking many independent egui state/handle args; no shared invariant to bundle.
#[allow(clippy::too_many_arguments)]
pub fn render_effect_params<S: std::hash::BuildHasher>(
    ui: &mut egui::Ui,
    params: &[ParamUIInfo],
    modulation_sources: &[ModSourceUIEntry],
    make_update: &dyn Fn(&str, ParamValue) -> EngineCommand,
    make_mod_assign: Option<MakeModAssign>,
    make_mod_unassign: Option<MakeModAssign>,
    make_mod_remove: Option<MakeParamCommand>,
    make_automation: Option<MakeParamCommand>,
    commands: &mut Vec<EngineCommand>,
    gesture_active: &mut bool,
    id_prefix: &str,
    midi_learn_path_prefix: Option<&str>,
    midi_learn_active: bool,
    midi_learn_select: &mut Option<String>,
    midi_learn_target: Option<&str>,
    mod_assignments: &std::collections::HashMap<String, Vec<ModAssignmentUI>, S>,
    mod_current_values: &std::collections::HashMap<String, f32, S>,
    mod_param_prefix: &str,
    keyboard_learn_active: bool,
    keyboard_learn_select: &mut Option<crate::keymap::KeyTarget>,
    keyboard_learn_target: Option<&str>,
) {
    render_grouped(ui, params, id_prefix, &mut |ui, param| {
        let label = param.label.as_ref().unwrap_or(&param.name);
        let mod_key = format!("{}:{}", mod_param_prefix, param.name);
        let assignments = mod_assignments.get(&mod_key);
        let is_modulated = assignments.is_some_and(|a| !a.is_empty());
        let mod_label_color = assignments.and_then(|a| a.first()).map(|a| {
            let color_idx = modulation_sources
                .iter()
                .position(|e| e.uuid == a.source_id)
                .unwrap_or(0);
            modulator_color(color_idx)
        });
        match param.value {
            ParamValue::Float(mut v) => {
                let min = param.min.unwrap_or(0.0);
                let max = param.max.unwrap_or(1.0);
                ui.horizontal(|ui| {
                    if let Some(color) = mod_label_color {
                        ui.label(egui::RichText::new(label).small().weak().color(color));
                    } else {
                        ui.label(egui::RichText::new(label).small().weak());
                    }
                    // Render slider — in learn mode, disable mouse interaction via a scope
                    let any_learn_active = midi_learn_active || keyboard_learn_active;
                    let slider_rect = if any_learn_active {
                        let inner = ui.scope(|ui| {
                            ui.disable();
                            ui.add(egui::Slider::new(&mut v, min..=max).show_value(false))
                        });
                        inner.inner.rect
                    } else {
                        let slider_resp =
                            ui.add(egui::Slider::new(&mut v, min..=max).show_value(false));
                        // A held slider drag is a single undo gesture (collapsed
                        // by the runner's `gesture_active` edge).
                        if slider_resp.dragged() {
                            *gesture_active = true;
                        }
                        if slider_resp.changed() {
                            commands.push(make_update(&param.name, ParamValue::Float(v)));
                        }
                        slider_resp.rect
                    };
                    // MIDI learn mode: glow + click overlay
                    if midi_learn_active && let Some(prefix) = midi_learn_path_prefix {
                        let path = format!("{}/param/{}", prefix, param.name);
                        let is_target = midi_learn_target.is_some_and(|t| t == path);
                        if is_target {
                            draw_midi_learn_selected(ui, slider_rect);
                        } else {
                            draw_midi_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with(("midi_learn_fx_param", &param.name));
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            *midi_learn_select = Some(path);
                        }
                    }
                    // Keyboard learn mode: orange glow + click overlay
                    if keyboard_learn_active && let Some(prefix) = midi_learn_path_prefix {
                        let path = format!("{}/param/{}", prefix, param.name);
                        let is_target = keyboard_learn_target.is_some_and(|t| t == path);
                        if is_target {
                            draw_keyboard_learn_selected(ui, slider_rect);
                        } else {
                            draw_keyboard_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with(("kb_learn_fx_param", &param.name));
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            *keyboard_learn_select =
                                Some(crate::keymap::KeyTarget::ParamPath(path));
                        }
                    }
                    // Draw modulation ghost indicator
                    if is_modulated && let Some(assigns) = assignments {
                        let mut total_offset = 0.0f32;
                        for a in assigns {
                            total_offset +=
                                mod_current_values.get(&a.source_id).copied().unwrap_or(0.0)
                                    * a.amount;
                        }
                        // Scale by param range to match GPU-side modulation
                        let range = max - min;
                        let modulated_val = (v + total_offset * range).clamp(min, max);
                        let frac = (modulated_val - min) / (max - min);
                        let x = slider_rect.left() + frac * slider_rect.width();
                        let color = mod_label_color.unwrap_or(egui::Color32::YELLOW);
                        let painter = ui.painter();
                        painter.line_segment(
                            [
                                egui::pos2(x, slider_rect.top()),
                                egui::pos2(x, slider_rect.bottom()),
                            ],
                            egui::Stroke::new(2.0_f32, color),
                        );
                    }
                    if let (Some(assign_fn), Some(unassign_fn), Some(remove_fn)) =
                        (make_mod_assign, make_mod_unassign, make_mod_remove)
                    {
                        modulation_dropdown(
                            ui,
                            format!("mod_{}_{}", id_prefix, param.name),
                            &param.name,
                            modulation_sources,
                            assignments.map_or(&[], Vec::as_slice),
                            &ModMenu {
                                assign: assign_fn,
                                unassign: unassign_fn,
                                remove: remove_fn,
                                automate: make_automation,
                            },
                            commands,
                        );
                    }
                });
            }
            ParamValue::Bool(mut v) => {
                if ui
                    .checkbox(&mut v, egui::RichText::new(label).small().weak())
                    .changed()
                {
                    commands.push(make_update(&param.name, ParamValue::Bool(v)));
                }
            }
            ParamValue::Color(c) => {
                let mut color = [c[0], c[1], c[2], c[3]];
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).small().weak());
                    if ui.color_edit_button_rgba_unmultiplied(&mut color).changed() {
                        commands.push(make_update(&param.name, ParamValue::Color(color)));
                    }
                });
            }
            ParamValue::Long(v) => render_long_row(
                ui,
                param,
                egui::RichText::new(label).small().weak(),
                v,
                id_prefix,
                make_update,
                commands,
            ),
            ParamValue::Point2D(p) => render_point2d_row(
                ui,
                param,
                egui::RichText::new(label).small().weak(),
                p,
                make_update,
                commands,
            ),
        }
    });
}

/// Render audio level bars
pub fn render_audio_levels(ui: &mut egui::Ui, audio: &AudioUIData) {
    if audio.enabled {
        ui.horizontal(|ui| {
            ui.label("Vol:");
            ui.add(egui::ProgressBar::new(audio.level).desired_width(100.0));
        });
        ui.horizontal(|ui| {
            ui.label("Bass:");
            ui.add(
                egui::ProgressBar::new(audio.bass)
                    .desired_width(100.0)
                    .fill(egui::Color32::from_rgb(220, 60, 60)),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Mid:");
            ui.add(
                egui::ProgressBar::new(audio.mid)
                    .desired_width(100.0)
                    .fill(egui::Color32::from_rgb(60, 220, 60)),
            );
        });
        ui.horizontal(|ui| {
            ui.label("High:");
            ui.add(
                egui::ProgressBar::new(audio.treble)
                    .desired_width(100.0)
                    .fill(egui::Color32::from_rgb(60, 60, 220)),
            );
        });
        if let Some(bpm) = audio.bpm {
            ui.horizontal(|ui| {
                ui.label(format!("BPM: {bpm:.0}"));
                ui.add(
                    egui::ProgressBar::new(audio.beat_phase)
                        .desired_width(50.0)
                        .fill(egui::Color32::from_rgb(255, 165, 0)),
                );
            });
        }
    } else {
        ui.label("⚠ No audio input");
    }
}

/// Draw a pulsing purple glow around a rect to indicate it's a MIDI-learnable target.
pub fn draw_midi_learn_glow(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let glow_color = egui::Color32::from_rgba_unmultiplied(180, 80, 220, 80);
    let expanded = rect.expand(2.0);
    painter.rect_stroke(
        expanded,
        3.0,
        egui::Stroke::new(2.0_f32, glow_color),
        egui::StrokeKind::Outside,
    );
}

/// Draw a brighter glow for the currently selected MIDI learn target.
pub fn draw_midi_learn_selected(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let glow_color = egui::Color32::from_rgba_unmultiplied(255, 100, 50, 120);
    let expanded = rect.expand(3.0);
    painter.rect_stroke(
        expanded,
        3.0,
        egui::Stroke::new(3.0_f32, glow_color),
        egui::StrokeKind::Outside,
    );
}

/// Draw an orange glow around a rect for keyboard-learnable target.
pub fn draw_keyboard_learn_glow(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let glow_color = egui::Color32::from_rgba_unmultiplied(255, 165, 0, 80);
    let expanded = rect.expand(2.0);
    painter.rect_stroke(
        expanded,
        3.0,
        egui::Stroke::new(2.0_f32, glow_color),
        egui::StrokeKind::Outside,
    );
}

/// Draw a brighter orange glow for the currently selected keyboard learn target.
pub fn draw_keyboard_learn_selected(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let glow_color = egui::Color32::from_rgba_unmultiplied(255, 120, 0, 120);
    let expanded = rect.expand(3.0);
    painter.rect_stroke(
        expanded,
        3.0,
        egui::Stroke::new(3.0_f32, glow_color),
        egui::StrokeKind::Outside,
    );
}

/// A rotary knob for a normalized `0.0..=1.0` value. Vertical drag adjusts it
/// (drag **up** = increase, down = decrease); the arc and pointer fill in the
/// `accent` color. Returns the response with `.changed()` set on movement, so
/// callers can emit a live value action and attach a MIDI-learn overlay.
pub fn render_knob(
    ui: &mut egui::Ui,
    value: &mut f32,
    diameter: f32,
    accent: egui::Color32,
    ghost: Option<(f32, egui::Color32)>,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(diameter, diameter),
        egui::Sense::click_and_drag(),
    );

    if response.dragged() {
        let dy = response.drag_delta().y;
        if dy != 0.0 {
            // 200px of vertical travel spans the full range — precise but reachable.
            *value = (*value - dy / 200.0).clamp(0.0, 1.0);
            response.mark_changed();
        }
    }

    if ui.is_rect_visible(rect) {
        // The knob sweeps 270° clockwise from lower-left (min) to lower-right (max),
        // passing through the top. Screen space is y-down, so angles grow clockwise.
        const START_DEG: f32 = 135.0;
        const SWEEP_DEG: f32 = 270.0;

        let painter = ui.painter();
        let center = rect.center();
        let radius = diameter * 0.5 - 2.0;

        // Body
        painter.circle_filled(center, radius, egui::Color32::from_rgb(28, 28, 38));
        painter.circle_stroke(
            center,
            radius,
            egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(70, 70, 90)),
        );

        let v = value.clamp(0.0, 1.0);
        let start = START_DEG.to_radians();
        let end = (START_DEG + SWEEP_DEG * v).to_radians();
        let on_arc = |a: f32, r: f32| center + egui::vec2(a.cos(), a.sin()) * r;

        // Full track (faint) then the filled value arc on top.
        let arc_r = radius - 2.0;
        let track_end = (START_DEG + SWEEP_DEG).to_radians();
        let track: Vec<egui::Pos2> = (0..=48)
            .map(|i| {
                let a = start + (track_end - start) * (i as f32 / 48.0);
                on_arc(a, arc_r)
            })
            .collect();
        painter.add(egui::Shape::line(
            track,
            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(55, 55, 70)),
        ));
        let fill: Vec<egui::Pos2> = (0..=48)
            .map(|i| {
                let a = start + (end - start) * (i as f32 / 48.0);
                on_arc(a, arc_r)
            })
            .collect();
        painter.add(egui::Shape::line(fill, egui::Stroke::new(2.5_f32, accent)));

        // Modulation ghost: a marker at the effective (base + offset) value in
        // the modulator's color, so a modulated knob visibly tracks the source
        // (mirrors the ghost line on modulated param sliders).
        if let Some((gv, gcolor)) = ghost {
            let gend = (START_DEG + SWEEP_DEG * gv.clamp(0.0, 1.0)).to_radians();
            painter.line_segment(
                [center, on_arc(gend, radius - 3.0)],
                egui::Stroke::new(1.5_f32, gcolor),
            );
            painter.circle_filled(on_arc(gend, arc_r), 2.5_f32, gcolor);
        }

        // Pointer from center to the current angle.
        painter.line_segment(
            [center, on_arc(end, radius - 3.0)],
            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(220, 220, 230)),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecases::ui::ModSourceUI;
    use egui_kittest::kittest::Queryable;

    fn lfo_entry(uuid: &str) -> ModSourceUIEntry {
        ModSourceUIEntry {
            uuid: uuid.to_string(),
            source: ModSourceUI::LFO {
                waveform: crate::modulation::LFOWaveform::Sine,
                frequency: 1.0,
                phase: 0.0,
                amplitude: 1.0,
                bipolar: false,
            },
            timebase: crate::timebase::Timebase::FreeRun,
        }
    }

    fn envelope_entry(uuid: &str) -> ModSourceUIEntry {
        ModSourceUIEntry {
            uuid: uuid.to_string(),
            source: ModSourceUI::Envelope {
                breakpoints: Vec::new(),
            },
            timebase: crate::timebase::Timebase::Transport,
        }
    }

    fn assigned_to(source_id: &str) -> ModAssignmentUI {
        ModAssignmentUI {
            source_id: source_id.to_string(),
            amount: 1.0,
        }
    }

    fn param_in(name: &str, group: Option<&str>) -> ParamUIInfo {
        ParamUIInfo {
            name: name.to_string(),
            label: None,
            value: ParamValue::Float(0.0),
            min: Some(0.0),
            max: Some(1.0),
            group: group.map(str::to_string),
            choices: Vec::new(),
        }
    }

    #[test]
    fn group_order_follows_first_appearance() {
        let params = [
            param_in("a", Some("Look")),
            param_in("b", Some("Formula")),
            param_in("c", Some("Look")),
            param_in("d", Some("Camera")),
        ];
        let refs: Vec<&ParamUIInfo> = params.iter().collect();
        assert_eq!(
            named_group_order(&refs),
            vec!["Look", "Formula", "Camera"],
            "a group sits where its first member appears, so INPUTS order is the \
             only thing an author has to reason about"
        );
    }

    #[test]
    fn ungrouped_params_produce_no_named_group() {
        let params = [param_in("a", None), param_in("b", None)];
        let refs: Vec<&ParamUIInfo> = params.iter().collect();
        assert!(
            named_group_order(&refs).is_empty(),
            "a shader that declares no groups must render as one flat list"
        );
    }

    #[test]
    fn group_order_ignores_hidden_params() {
        // Grouping filters by the `_mode` convention before ordering, so a group
        // whose only members are hidden must not leave an empty header behind —
        // nor a scope in the exploration controls that addresses nothing.
        let mut toggle = param_in("detail_mode", None);
        toggle.value = ParamValue::Bool(false);
        let params = [
            param_in("visible", Some("Look")),
            toggle,
            param_in("detail_shape", Some("Detail")),
        ];
        assert_eq!(param_groups(&params), vec!["Look"]);
    }

    /// Render `params` through `render_grouped`, each row a bare label so a query
    /// for a parameter's name answers "is this control in view".
    fn grouped_harness(params: Vec<ParamUIInfo>) -> egui_kittest::Harness<'static> {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 600.0))
            .build_ui(move |ui| {
                render_grouped(ui, &params, "test", &mut |ui, param| {
                    ui.label(&param.name);
                });
            });
        harness.run();
        harness
    }

    #[test]
    fn grouped_params_open_the_first_group_and_close_the_rest() {
        let harness = grouped_harness(vec![
            param_in("brightness", None),
            param_in("fold_scale", Some("Formula")),
            param_in("saturation", Some("Grade")),
        ]);

        assert!(
            harness.query_by_label("brightness").is_some(),
            "an ungrouped parameter stays in view with no header to open"
        );
        assert!(
            harness.query_by_label("Formula").is_some()
                && harness.query_by_label("Grade").is_some(),
            "every named group contributes a header, open or not"
        );
        assert!(
            harness.query_by_label("fold_scale").is_some(),
            "the first named group opens, so a shader does not present as headers alone"
        );
        assert!(
            harness.query_by_label("saturation").is_none(),
            "later groups start closed, which is what keeps a long shader scannable"
        );
    }

    #[test]
    fn clicking_a_closed_group_header_reveals_its_params() {
        let mut harness = grouped_harness(vec![
            param_in("fold_scale", Some("Formula")),
            param_in("saturation", Some("Grade")),
        ]);

        harness.get_by_label("Grade").click();
        // Collapsing headers animate open, so settle before querying.
        harness.run();
        harness.run();

        assert!(
            harness.query_by_label("saturation").is_some(),
            "a closed group must be one click from its contents"
        );
    }

    /// Open the dropdown and hand it to `act`, which drives the harness.
    fn in_dropdown(
        sources: &[ModSourceUIEntry],
        assignments: &[ModAssignmentUI],
        automate: bool,
        act: &dyn Fn(&mut egui_kittest::Harness<'_>),
    ) -> Vec<EngineCommand> {
        let mut commands = Vec::new();
        {
            let assign = |name: &str, uuid: &str| EngineCommand::AssignModulation {
                target: name.to_string(),
                source_id: uuid.to_string(),
                amount: 1.0,
            };
            let unassign = |name: &str, uuid: &str| EngineCommand::ClearModulationSource {
                target: name.to_string(),
                source_id: uuid.to_string(),
            };
            let remove = |name: &str| EngineCommand::ClearModulation {
                target: name.to_string(),
            };
            let add_lane = |name: &str| EngineCommand::AddAutomationLane {
                target: name.to_string(),
                timebase: crate::timebase::Timebase::Transport,
            };
            let mut harness = egui_kittest::Harness::new_ui(|ui| {
                modulation_dropdown(
                    ui,
                    "test".to_string(),
                    "opacity",
                    sources,
                    assignments,
                    &ModMenu {
                        assign: &assign,
                        unassign: &unassign,
                        remove: &remove,
                        automate: if automate {
                            Some(&add_lane as MakeParamCommand)
                        } else {
                            None
                        },
                    },
                    &mut commands,
                );
            });
            // A ComboBox exposes its selected text as AccessKit `value`, not `label`.
            harness.get_by_value("〰").click();
            harness.run();
            act(&mut harness);
        }
        commands
    }

    /// Drive the dropdown open and click `label`, returning what it emitted.
    fn click_in_dropdown(
        sources: &[ModSourceUIEntry],
        assignments: &[ModAssignmentUI],
        automate: bool,
        label: &str,
    ) -> Vec<EngineCommand> {
        in_dropdown(sources, assignments, automate, &|harness| {
            harness.get_by_label(label).click();
            harness.run();
        })
    }

    #[test]
    fn the_automation_entry_creates_a_lane_for_this_parameter() {
        let commands = click_in_dropdown(&[lfo_entry("lfo1")], &[], true, "＋ Automation lane");
        assert!(
            commands.iter().any(|c| matches!(
                c,
                EngineCommand::AddAutomationLane { target, timebase }
                    if target == "opacity" && *timebase == crate::timebase::Timebase::Transport
            )),
            "expected a transport-locked lane on this parameter, got {commands:?}"
        );
    }

    /// Automation needs no existing source, so the menu has to be reachable on a
    /// scene that has never created a modulator.
    #[test]
    fn the_menu_opens_with_no_modulation_sources_at_all() {
        let commands = click_in_dropdown(&[], &[], true, "＋ Automation lane");
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, EngineCommand::AddAutomationLane { .. }))
        );
    }

    #[test]
    fn ticking_a_source_assigns_it() {
        let commands = click_in_dropdown(&[lfo_entry("lfo1")], &[], true, "☐ LFO 1");
        assert!(commands.iter().any(|c| matches!(
            c,
            EngineCommand::AssignModulation { target, source_id, .. }
                if target == "opacity" && source_id == "lfo1"
        )));
    }

    /// The point of the checklist: a source already on the parameter reads as
    /// ticked, so the set driving it is legible without opening anything else.
    #[test]
    fn an_assigned_source_reads_as_ticked() {
        in_dropdown(
            &[lfo_entry("lfo1"), lfo_entry("lfo2")],
            &[assigned_to("lfo2")],
            true,
            &|harness| {
                assert!(
                    harness.query_by_label("☐ LFO 1").is_some(),
                    "an unassigned source is offered unticked"
                );
                assert!(
                    harness.query_by_label("☑ LFO 2").is_some(),
                    "an assigned source is ticked"
                );
            },
        );
    }

    /// Un-ticking detaches only that source. Two stacked modulators, and
    /// removing one has to leave the other driving the parameter.
    #[test]
    fn unticking_a_source_detaches_only_that_one() {
        let commands = click_in_dropdown(
            &[lfo_entry("lfo1"), lfo_entry("lfo2")],
            &[assigned_to("lfo1"), assigned_to("lfo2")],
            true,
            "☑ LFO 1",
        );
        assert!(
            commands.iter().any(|c| matches!(
                c,
                EngineCommand::ClearModulationSource { target, source_id }
                    if target == "opacity" && source_id == "lfo1"
            )),
            "expected only lfo1 detached, got {commands:?}"
        );
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, EngineCommand::ClearModulation { .. })),
            "un-ticking one source must not clear the whole parameter"
        );
    }

    #[test]
    fn clearing_removes_every_assignment() {
        let commands = click_in_dropdown(
            &[lfo_entry("lfo1")],
            &[assigned_to("lfo1")],
            true,
            "Clear all",
        );
        assert!(commands.iter().any(|c| matches!(
            c,
            EngineCommand::ClearModulation { target } if target == "opacity"
        )));
    }

    /// Nothing to clear, so the entry stays out of the way rather than sitting
    /// there as a no-op under a list of empty boxes.
    #[test]
    fn an_unmodulated_parameter_offers_no_clear() {
        in_dropdown(&[lfo_entry("lfo1")], &[], true, &|harness| {
            assert!(harness.query_by_label("Clear all").is_none());
        });
    }

    /// Envelopes are still named for the lanes and cards that list them, even
    /// though this menu does not offer them.
    #[test]
    fn an_envelope_is_labelled_as_automation() {
        assert_eq!(envelope_entry("e1").label(2), "Automation 3");
    }

    /// A curve drives the one parameter it was drawn for. Offering it here would
    /// let two parameters share a source, and then editing either lane would
    /// silently rewrite the other.
    #[test]
    fn an_automation_curve_is_not_offered_as_a_source() {
        in_dropdown(
            &[lfo_entry("lfo1"), envelope_entry("env1")],
            &[],
            false,
            &|harness| {
                assert!(
                    harness.query_by_label("☐ LFO 1").is_some(),
                    "an ordinary modulator is still on offer"
                );
                assert!(
                    harness.query_by_label("☐ Automation 2").is_none(),
                    "a curve must not be assignable to a second parameter"
                );
            },
        );
    }

    /// A lane already drives this parameter, so it is listed even though an
    /// unassigned one would not be. Leaving it out would show an empty checklist
    /// under the coloured ghost the lane is drawing.
    #[test]
    fn an_assigned_curve_is_listed_so_the_ghost_has_an_owner() {
        in_dropdown(
            &[lfo_entry("lfo1"), envelope_entry("env1")],
            &[assigned_to("env1")],
            false,
            &|harness| {
                assert!(harness.query_by_label("☑ Automation 2").is_some());
            },
        );
    }

    /// The menu still opens for a parameter in a scene whose only sources are
    /// curves, because drawing a new one has to stay reachable.
    #[test]
    fn a_scene_of_only_curves_still_offers_a_new_lane() {
        let commands =
            click_in_dropdown(&[envelope_entry("env1")], &[], true, "＋ Automation lane");
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, EngineCommand::AddAutomationLane { .. }))
        );
    }
}
