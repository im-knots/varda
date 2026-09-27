//! Library panel.
//!
//! Every deck source type draws the same way: its entries, its notices, and
//! how to create a deck no entry lists, all from the section it publishes. The
//! panel names no source type. See /spec/deck-source-providers.md.

use super::super::{LibraryDrag, UIActions, UIData};
use crate::engine::EngineCommand;
use crate::engine::value::provider::{
    ControlKind, LibraryCreate, LibraryEntry, LibraryNotice, ProviderTypeSnapshot,
};
use crate::engine::value::source::SourceConfig;

/// egui memory key carrying the dragged source from the library panel to the
/// deferred drop handler in `panels/dnd.rs`.
pub(crate) const SOURCE_DND_KEY: &str = "__lib_dnd_source";

/// One draggable library row.
///
/// The remove button is reserved on the right (via a right-to-left layout) and
/// the label truncates to the remaining width, so a long URL can never force
/// the library panel wider than its resized/default size. Double-clicking adds
/// the deck to the first channel.
fn entry_row(
    ui: &mut egui::Ui,
    source_type: &str,
    idx: usize,
    entry: &LibraryEntry,
    data: &UIData,
    actions: &mut UIActions,
) {
    let item_id = egui::Id::new(("lib_source", source_type, idx));
    let mut remove = false;
    let mut double_clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if entry.removable {
                remove = ui
                    .small_button("✕")
                    .on_hover_text("Remove from library")
                    .clicked();
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let response = ui
                    .dnd_drag_source(item_id, LibraryDrag::Source(entry.config.clone()), |ui| {
                        if let Some(connected) = entry.connected {
                            let color = if connected {
                                egui::Color32::from_rgb(100, 220, 100)
                            } else {
                                egui::Color32::from_rgb(160, 160, 160)
                            };
                            ui.label(egui::RichText::new("●").color(color));
                        }
                        let mut text = egui::RichText::new(format!("  {}", entry.label)).size(12.0);
                        if entry.highlight {
                            // Marking Varda's own windows is what turns an
                            // accidental feedback loop into a deliberate one.
                            text = text.color(egui::Color32::from_rgb(200, 170, 240));
                        }
                        ui.add(egui::Label::new(text).truncate())
                    })
                    .response;
                double_clicked = response.double_clicked();
                let hover = entry.hover.clone().unwrap_or_else(|| {
                    "Drag to a channel to create a deck, or double-click to add to the first channel"
                        .to_string()
                });
                response.on_hover_text(hover);
            });
        });
    });
    if let Some(detail) = &entry.detail {
        ui.label(egui::RichText::new(format!("  {detail}")).size(10.0).weak());
    }
    if ui.ctx().is_being_dragged(item_id) {
        ui.ctx().memory_mut(|mem| {
            mem.data
                .insert_temp(egui::Id::new(SOURCE_DND_KEY), entry.config.clone());
        });
    }
    if double_clicked && let Some(ch) = data.channels.first() {
        actions.commands.push(EngineCommand::AddDeck {
            channel_uuid: ch.uuid.clone(),
            source: entry.config.clone(),
        });
    }
    if remove {
        actions
            .commands
            .push(EngineCommand::RemoveSourceLibraryEntry {
                entry: entry.config.clone(),
            });
    }
}

/// A notice above a section's entries, with the action it offers.
fn notice(ui: &mut egui::Ui, source_type: &str, notice: &LibraryNotice, actions: &mut UIActions) {
    let color = match notice.level.as_str() {
        "error" => egui::Color32::from_rgb(220, 120, 120),
        "warning" => egui::Color32::from_rgb(220, 180, 120),
        _ => ui.visuals().weak_text_color(),
    };
    ui.label(egui::RichText::new(&notice.text).small().color(color));
    if let (Some(label), Some(action)) = (&notice.action_label, &notice.action)
        && ui.small_button(label).clicked()
    {
        actions.commands.push(EngineCommand::SourceLibraryAction {
            source_type: source_type.to_string(),
            action: action.clone(),
        });
    }
}

/// How a user creates a deck that no entry lists: a file picker per channel,
/// or a form whose values become a library entry to drag.
fn create(
    ui: &mut egui::Ui,
    source_type: &str,
    create: &LibraryCreate,
    data: &UIData,
    actions: &mut UIActions,
) {
    match create {
        LibraryCreate::File {
            field,
            extensions,
            label,
        } => {
            ui.label(egui::RichText::new(label).small().weak());
            for ch in &data.channels {
                if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                    actions.session.open_file_dialog =
                        Some(crate::app::render::FileDialogRequest {
                            source_type: source_type.to_string(),
                            field: field.clone(),
                            label: label.clone(),
                            extensions: extensions.clone(),
                            channel_uuid: ch.uuid.clone(),
                        });
                }
            }
        }
        LibraryCreate::Entry {
            fields,
            defaults,
            label,
            hint,
        } => {
            let adding_id = ui.id().with(("lib_adding", source_type));
            let values_id = ui.id().with(("lib_form", source_type));
            let adding: bool = ui.data(|d| d.get_temp(adding_id)).unwrap_or(false);
            if !adding {
                if ui.small_button(label).clicked() {
                    ui.data_mut(|d| d.insert_temp(adding_id, true));
                }
                return;
            }
            let mut values: Vec<String> = ui.data(|d| d.get_temp(values_id)).unwrap_or_else(|| {
                fields
                    .iter()
                    .map(|f| {
                        defaults
                            .get(&f.name)
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    })
                    .collect()
            });
            for (field, value) in fields.iter().zip(values.iter_mut()) {
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", field.label));
                    match &field.kind {
                        ControlKind::Choice { options } => {
                            egui::ComboBox::from_id_salt((
                                "lib_form_choice",
                                source_type,
                                &field.name,
                            ))
                            .selected_text(
                                options
                                    .iter()
                                    .find(|o| o.eq_ignore_ascii_case(value))
                                    .cloned()
                                    .unwrap_or_else(|| value.clone()),
                            )
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for option in options {
                                    if ui
                                        .selectable_label(
                                            option.eq_ignore_ascii_case(value),
                                            option,
                                        )
                                        .clicked()
                                    {
                                        value.clone_from(option);
                                    }
                                }
                            });
                        }
                        _ => {
                            ui.add(egui::TextEdit::singleline(value).desired_width(200.0));
                        }
                    }
                });
            }
            if let Some(hint) = hint {
                ui.label(egui::RichText::new(hint).weak().small());
            }
            ui.horizontal(|ui| {
                if ui.small_button("✓ Add").clicked() {
                    let mut entry = SourceConfig::new(source_type);
                    for (field, value) in fields.iter().zip(&values) {
                        entry.set(&field.name, value);
                    }
                    actions
                        .commands
                        .push(EngineCommand::AddSourceLibraryEntry { entry });
                    ui.data_mut(|d| {
                        d.insert_temp(adding_id, false);
                        d.remove::<Vec<String>>(values_id);
                    });
                    return;
                }
                if ui.small_button("✕ Cancel").clicked() {
                    ui.data_mut(|d| {
                        d.insert_temp(adding_id, false);
                        d.remove::<Vec<String>>(values_id);
                    });
                }
            });
            ui.data_mut(|d| d.insert_temp(values_id, values));
        }
    }
}

/// One source type's collapsible section.
fn source_section(
    ui: &mut egui::Ui,
    ty: &ProviderTypeSnapshot,
    data: &UIData,
    actions: &mut UIActions,
) {
    let section = &ty.library;
    let header = if section.entries.is_empty() && section.create.is_some() {
        format!("{} {}", ty.icon, ty.label)
    } else {
        format!("{} {} ({})", ty.icon, ty.label, section.entries.len())
    };
    egui::CollapsingHeader::new(egui::RichText::new(header).strong())
        .id_salt(("lib_source_section", &ty.type_id))
        .default_open(false)
        .show(ui, |ui| {
            if !ty.available {
                let reason = ty.unavailable_reason.as_deref().unwrap_or("Unavailable");
                ui.label(egui::RichText::new(reason).small().weak());
                return;
            }
            if section.rescan || section.note.is_some() {
                ui.horizontal(|ui| {
                    if section.rescan && ui.small_button("🔄 Rescan").clicked() {
                        actions.commands.push(EngineCommand::SourceLibraryAction {
                            source_type: ty.type_id.clone(),
                            action: "rescan".into(),
                        });
                    }
                    if let Some(note) = &section.note {
                        ui.label(egui::RichText::new(note).small().weak());
                    }
                });
            }
            for n in &section.notices {
                notice(ui, &ty.type_id, n, actions);
            }
            if let Some(how) = &section.create {
                create(ui, &ty.type_id, how, data, actions);
            }
            if section.entries.is_empty() && section.create.is_none() {
                let hint = if section.rescan {
                    "Nothing found — press Rescan"
                } else {
                    "Nothing to show"
                };
                ui.label(egui::RichText::new(hint).small().weak());
            }
            // Ungrouped entries first, then each group under its heading, in
            // the order the provider listed them.
            let mut groups: Vec<Option<&str>> = Vec::new();
            for entry in &section.entries {
                let group = entry.group.as_deref();
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            groups.sort_by_key(Option::is_some);
            for group in groups {
                if let Some(heading) = group {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(heading).small().weak());
                }
                for (idx, entry) in section.entries.iter().enumerate() {
                    if entry.group.as_deref() == group {
                        entry_row(ui, &ty.type_id, idx, entry, data, actions);
                    }
                }
            }
        });
    ui.add_space(4.0);
}

pub(super) fn render_library_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.horizontal(|ui| {
        ui.heading("📚 Library");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("◀")
                .on_hover_text("Close library (L)")
                .clicked()
            {
                actions.session.toggle_library_panel = true;
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            drag: egui::scroll_area::DragScroll::Never,
            mouse_wheel: true,
        })
        .show(ui, |ui| {
            // === DECK SOURCES ===
            for ty in data.sources.iter().filter(|t| t.listed) {
                source_section(ui, ty, data, actions);
            }

            // === EFFECTS ===
            let fx_header =
                egui::RichText::new(format!("🔮 Effects ({})", data.filters.len())).strong();
            egui::CollapsingHeader::new(fx_header)
                .id_salt("lib_effects")
                .default_open(false)
                .show(ui, |ui| {
                    for (name, filter_idx) in &data.filters {
                        let item_id = egui::Id::new(("lib_fx", *filter_idx));
                        ui.dnd_drag_source(item_id, LibraryDrag::Effect(*filter_idx), |ui| {
                            ui.label(egui::RichText::new(format!("  ◇ {name}")).size(12.0));
                        });
                        // Store effect filter index in temp memory for deferred drop handler
                        if ui.ctx().is_being_dragged(item_id) {
                            ui.ctx().memory_mut(|mem| {
                                mem.data
                                    .insert_temp(egui::Id::new("__lib_dnd_fx_idx"), *filter_idx);
                            });
                        }
                    }
                });
            ui.add_space(4.0);

            // === DECK PRESETS ===
            if !data.deck_presets.is_empty() {
                let deck_preset_header =
                    egui::RichText::new(format!("💾 Deck Presets ({})", data.deck_presets.len()))
                        .strong();
                egui::CollapsingHeader::new(deck_preset_header)
                    .id_salt("lib_deck_presets")
                    .default_open(false)
                    .show(ui, |ui| {
                        for (idx, name) in data.deck_presets.iter().enumerate() {
                            let item_id = egui::Id::new(("lib_deck_preset", idx));
                            let resp = ui
                                .dnd_drag_source(item_id, LibraryDrag::DeckPreset(idx), |ui| {
                                    ui.label(egui::RichText::new(format!("  ◈ {name}")).size(12.0));
                                })
                                .response;
                            if ui.ctx().is_being_dragged(item_id) {
                                ui.ctx().memory_mut(|mem| {
                                    mem.data.insert_temp(
                                        egui::Id::new("__lib_dnd_deck_preset_idx"),
                                        idx,
                                    );
                                });
                            }
                            if resp.double_clicked()
                                && let Some(ch) = data.channels.first()
                            {
                                actions.commands.push(EngineCommand::LoadDeckPreset {
                                    channel_uuid: ch.uuid.clone(),
                                    preset_name: name.clone(),
                                });
                            }
                            resp.on_hover_text("Drag to a channel to load this deck preset");
                        }
                    });

                ui.add_space(4.0);
            }

            // === CHANNEL PRESETS ===
            if !data.channel_presets.is_empty() {
                let ch_preset_header = egui::RichText::new(format!(
                    "💾 Channel Presets ({})",
                    data.channel_presets.len()
                ))
                .strong();
                egui::CollapsingHeader::new(ch_preset_header)
                    .id_salt("lib_ch_presets")
                    .default_open(false)
                    .show(ui, |ui| {
                        for (idx, name) in data.channel_presets.iter().enumerate() {
                            let item_id = egui::Id::new(("lib_ch_preset", idx));
                            let resp = ui
                                .dnd_drag_source(item_id, LibraryDrag::ChannelPreset(idx), |ui| {
                                    ui.label(egui::RichText::new(format!("  ◈ {name}")).size(12.0));
                                })
                                .response;
                            if ui.ctx().is_being_dragged(item_id) {
                                ui.ctx().memory_mut(|mem| {
                                    mem.data
                                        .insert_temp(egui::Id::new("__lib_dnd_ch_preset_idx"), idx);
                                });
                            }
                            if resp.double_clicked() {
                                actions.commands.push(EngineCommand::LoadChannelPreset {
                                    target_channel_uuid: None,
                                    preset_name: name.clone(),
                                });
                            }
                            resp.on_hover_text("Double-click to add this channel to the mixer");
                        }
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_library_panel_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_library_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_library_panel_smoke_empty() {
        let mut data = UIData::test_fixture();
        data.sources = std::sync::Arc::default();
        data.filters.clear();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_library_panel(ui, &data, &mut actions);
        });
    }
}
