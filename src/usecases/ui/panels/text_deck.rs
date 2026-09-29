//! Text deck controls in the deck detail bar, laid out as Text, Style, Layout,
//! Motion and Transport columns.
//!
//! Used for sources whose controls carry `WidgetHint::TextDeck`. Controls keep
//! their learn target, modulation menu and inactive graying through the same
//! helpers as the generic column.

use super::super::{DeckUIInfo, UIActions, UIData};
use super::deck_detail::{
    font_family_control, inactive_scope, norm, set_source, source_affordances, source_param,
    text_control,
};
use super::utils::{column_open, render_collapsed_column, render_column_header};
use crate::engine::EngineCommand;
use crate::engine::value::provider::{
    ControlKind, ControlSpec, ControlValue, ProviderTypeSnapshot,
};

/// Inputs shared by every column.
struct Controls<'a> {
    deck: &'a DeckUIInfo,
    ty: &'a ProviderTypeSnapshot,
    data: &'a UIData,
}

impl<'a> Controls<'a> {
    fn spec(&self, name: &str) -> Option<&'a ControlSpec> {
        self.ty.params.iter().find(|s| s.name == name)
    }

    fn current(&self, name: &str) -> Option<&'a ControlValue> {
        self.deck.source.status.params.get(name)
    }

    fn note(&self, name: &str) -> Option<&'a String> {
        self.deck.source.status.display.get(name)
    }

    /// A control drawn by the generic helper.
    fn param(&self, ui: &mut egui::Ui, name: &str, actions: &mut UIActions) {
        if let Some(spec) = self.spec(name) {
            source_param(ui, self.deck, self.ty, spec, self.data, actions);
        }
    }

    /// A choice as a row of buttons, one per option, labeled `labels`.
    fn segmented(
        &self,
        ui: &mut egui::Ui,
        name: &str,
        labels: &[&str],
        hover: &[&str],
        actions: &mut UIActions,
    ) {
        let Some(spec) = self.spec(name) else {
            return;
        };
        let n = match &spec.kind {
            ControlKind::Choice { options } => options.len().max(1),
            _ => return,
        };
        let uuid = self.deck.uuid.as_str();
        let index = crate::source::choice_index(norm(self.deck, name), n);
        inactive_scope(ui, self.deck, name, self.data, |ui| {
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, label) in labels.iter().enumerate().take(n) {
                    let mut resp = ui.selectable_label(i == index, *label);
                    if let Some(text) = hover.get(i) {
                        resp = resp.on_hover_text(*text);
                    }
                    if resp.clicked() && i != index {
                        let value = crate::source::choice_value(i, n);
                        set_source(actions, uuid, name, ControlValue::Float(value));
                    }
                }
            });
            source_affordances(ui, row.response.rect, uuid, spec, self.data, actions);
        });
    }

    /// A toggle as a button that stays lit while on.
    fn toggle(
        &self,
        ui: &mut egui::Ui,
        name: &str,
        label: &str,
        hover: &str,
        actions: &mut UIActions,
    ) {
        let Some(spec) = self.spec(name) else {
            return;
        };
        let on = self
            .current(name)
            .and_then(ControlValue::as_f32)
            .is_some_and(|v| v > 0.5);
        let uuid = self.deck.uuid.as_str();
        inactive_scope(ui, self.deck, name, self.data, |ui| {
            let resp = ui.selectable_label(on, label).on_hover_text(hover);
            if resp.clicked() {
                set_source(actions, uuid, name, ControlValue::Bool(!on));
            }
            source_affordances(ui, resp.rect, uuid, spec, self.data, actions);
        });
    }

    /// An action as a small button.
    fn action(&self, ui: &mut egui::Ui, name: &str, label: &str, actions: &mut UIActions) {
        let Some(spec) = self.spec(name) else {
            return;
        };
        let resp = ui.button(label).on_hover_text(&spec.label);
        if resp.clicked() {
            actions.commands.push(EngineCommand::TriggerSourceAction {
                deck_uuid: self.deck.uuid.clone(),
                action: name.to_string(),
            });
        }
        source_affordances(ui, resp.rect, &self.deck.uuid, spec, self.data, actions);
    }
}

/// A titled column that scrolls within the bar and collapses to a strip when
/// its title is clicked, like the params and auto-transition columns.
fn column(
    ui: &mut egui::Ui,
    id: (&str, &str),
    title: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
) {
    let open_id = egui::Id::new(("text_col_open", id));
    if !column_open(ui, open_id) {
        // The strip spells the title without its icon.
        let name = title.split_once(' ').map_or(title, |(_, name)| name);
        render_collapsed_column(ui, name, open_id);
        return;
    }
    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                render_column_header(ui, title, open_id);
                let max_h = (ui.available_height() - 8.0).max(100.0);
                egui::ScrollArea::vertical()
                    .id_salt(id)
                    .max_height(max_h)
                    .show(ui, add);
            });
        });
}

fn weak_note(ui: &mut egui::Ui, note: Option<&String>) {
    if let Some(note) = note {
        ui.label(egui::RichText::new(note).small().weak());
    }
}

/// The text deck's columns, side by side.
pub(super) fn text_deck_columns(
    ui: &mut egui::Ui,
    deck: &DeckUIInfo,
    ty: &ProviderTypeSnapshot,
    data: &UIData,
    actions: &mut UIActions,
) {
    let d = Controls { deck, ty, data };
    let uuid = deck.uuid.as_str();

    column(
        ui,
        ("text_col", uuid),
        &format!("{} Text", ty.icon),
        300.0,
        |ui| {
            text_column(ui, &d, actions);
        },
    );
    column(ui, ("style_col", uuid), "Aa Style", 260.0, |ui| {
        style_column(ui, &d, actions);
    });
    column(ui, ("layout_col", uuid), "✥ Layout", 210.0, |ui| {
        d.param(ui, "position", actions);
        ui.horizontal(|ui| {
            ui.label("Align");
            d.segmented(ui, "align", &["Left", "Center", "Right"], &[], actions);
        });
        ui.horizontal(|ui| {
            ui.label("Vertical");
            d.segmented(ui, "valign", &["Top", "Middle", "Bottom"], &[], actions);
        });
    });
    column(ui, ("motion_col", uuid), "▶ Motion", 290.0, |ui| {
        motion_column(ui, &d, actions);
    });
    column(ui, ("transport_col", uuid), "⏱ Transport", 210.0, |ui| {
        d.segmented(
            ui,
            "chase",
            &["Auto", "Always", "Never"],
            &[
                "Follow the show transport while it runs",
                "Follow the transport even while it is stopped",
                "Never follow the transport",
            ],
            actions,
        );
        d.param(ui, "chase_offset", actions);
        d.param(ui, "chase_delay", actions);
    });
}

fn text_column(ui: &mut egui::Ui, d: &Controls, actions: &mut UIActions) {
    let uuid = d.deck.uuid.as_str();
    ui.horizontal(|ui| {
        d.segmented(
            ui,
            "format",
            &["Plain", "LRC", "VTT", "SRT"],
            &[
                "Plain text, one line per unit",
                "LRC lyrics with timestamps",
                "WebVTT captions",
                "SubRip captions",
            ],
            actions,
        );
        if let Some(spec) = d.spec("file")
            && let ControlKind::File { extensions } = &spec.kind
            && ui
                .button("📁 Load")
                .on_hover_text("Replace the text with a .txt, .lrc, .vtt or .srt file")
                .clicked()
        {
            actions.session.open_file_dialog = Some(crate::app::render::FileDialogRequest {
                target: crate::app::render::FileDialogTarget::SourceParam {
                    deck_uuid: uuid.to_string(),
                    name: spec.name.clone(),
                },
                label: spec.label.clone(),
                extensions: extensions.clone(),
            });
        }
    });
    if let Some(spec) = d.spec("text") {
        text_control(ui, uuid, spec, d.current("text"), true, actions);
        weak_note(ui, d.note("text"));
    }
    inactive_scope(ui, d.deck, "line", d.data, |ui| {
        append_line(ui, uuid, actions);
    });
}

/// A field that appends its line to the text on Enter and clears itself, for
/// feeding lines one at a time like a caption feed.
fn append_line(ui: &mut egui::Ui, deck_uuid: &str, actions: &mut UIActions) {
    let id = ui.id().with(("text_append", deck_uuid));
    let mut line: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let resp = ui.add(
        egui::TextEdit::singleline(&mut line)
            .hint_text("Add a line, press Enter")
            .desired_width(f32::INFINITY),
    );
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !line.is_empty() {
        set_source(
            actions,
            deck_uuid,
            "line",
            ControlValue::Text(std::mem::take(&mut line)),
        );
        resp.request_focus();
    }
    ui.data_mut(|d| d.insert_temp(id, line));
}

fn style_column(ui: &mut egui::Ui, d: &Controls, actions: &mut UIActions) {
    let uuid = d.deck.uuid.as_str();
    if let Some(spec) = d.spec("font") {
        font_family_control(ui, uuid, d.ty, spec, d.current("font"), actions);
        weak_note(ui, d.note("font"));
    }
    ui.horizontal(|ui| {
        d.param(ui, "weight", actions);
        d.toggle(ui, "italic", "I", "Italic", actions);
    });
    d.param(ui, "size", actions);
    d.param(ui, "line_spacing", actions);
    d.param(ui, "color", actions);
    d.param(ui, "background", actions);
    egui::CollapsingHeader::new("Speaker colors")
        .id_salt(("speaker_colors", uuid))
        .default_open(false)
        .show(ui, |ui| {
            for i in 1..=4 {
                d.param(ui, &format!("voice_{i}_color"), actions);
            }
        });
}

fn motion_column(ui: &mut egui::Ui, d: &Controls, actions: &mut UIActions) {
    let info = &d.deck.source.status.info;
    let unit = info.get("unit").and_then(serde_json::Value::as_u64);
    let units = info
        .get("units")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let chasing = info
        .get("chasing")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut state = match unit {
        Some(u) => format!("{} / {units}", u + 1),
        None => format!("- / {units}"),
    };
    if chasing {
        state.push_str("  · chasing");
    }
    ui.label(egui::RichText::new(state).small().weak());

    d.segmented(
        ui,
        "mode",
        &["Static", "Crawl", "Ticker", "Step"],
        &[
            "All lines at once",
            "All lines, moving up",
            "All lines in one row, moving left",
            "One unit at a time",
        ],
        actions,
    );
    ui.horizontal(|ui| {
        d.segmented(ui, "unit", &["Line", "Word"], &[], actions);
        ui.separator();
        d.segmented(
            ui,
            "clock",
            &["Rate", "Beat"],
            &["Speed in units per second", "Speed in units per beat"],
            actions,
        );
        ui.separator();
        d.toggle(
            ui,
            "loop",
            "⟲",
            "Loop back to the start after the end",
            actions,
        );
    });
    d.param(ui, "speed", actions);
    d.param(ui, "scroll", actions);
    ui.horizontal(|ui| {
        d.action(ui, "restart", "⏮", actions);
        d.action(ui, "previous", "◀", actions);
        d.action(ui, "next", "▶", actions);
    });

    ui.add_space(4.0);
    ui.label(egui::RichText::new("Transition").strong());
    d.segmented(
        ui,
        "transition",
        &["Cut", "Fade", "Roll-up", "Paint-on"],
        &[
            "Replace at once",
            "Cross-fade",
            "A window of lines moving up as each arrives",
            "Reveal in reading order",
        ],
        actions,
    );
    d.param(ui, "transition_time", actions);
    ui.horizontal(|ui| {
        ui.label("Lines");
        d.segmented(ui, "rollup_lines", &["1", "2", "3", "4"], &[], actions);
    });
}
