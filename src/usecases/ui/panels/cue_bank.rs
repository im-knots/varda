//! The cue bank: the marks from the arrangement's ruler, as pads at the desk.
//!
//! A cue is marked against the timeline but wanted under the lights, and nobody
//! performs from Arrangement mode. The bank is drawn from the cue list every
//! frame rather than owning buttons of its own, so a rename, a move, or a delete
//! needs no reconciliation. See /spec/arrangement.md § The cue bank in
//! Performance mode.

use super::super::{widgets, UIActions, UIData};
use crate::arrangement::Cue;
use crate::engine::EngineCommand;
use crate::transport::TransportSource;

/// The ruler's cue colour, so a pad and its mark read as the same thing.
const COLOR: egui::Color32 = super::arrangement::CUE_COLOR;

const COLUMNS: usize = 2;
const BUTTON_HEIGHT: f32 = 24.0;
const GAP: f32 = 4.0;

/// Two buttons a row, in the order the ruler draws them.
pub(super) fn render_cue_bank(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let cues: &[Cue] = data
        .arrangement
        .as_ref()
        .map_or(&[], |a| a.config.cues.as_slice());
    if cues.is_empty() {
        return;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("◆ Cues")
                .strong()
                .size(11.0)
                .color(COLOR),
        );
        ui.label(
            egui::RichText::new(format!("· {}", cues.len()))
                .small()
                .weak(),
        );
    });

    // The position belongs to the timecode master while chasing, so the pads
    // are shown refusing rather than failing under the hand.
    let live = data.transport.source == TransportSource::Internal;
    let width = ((ui.available_width() - GAP * (COLUMNS as f32 - 1.0)) / COLUMNS as f32).max(24.0);

    for row in cues.chunks(COLUMNS) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for cue in row {
                render_pad(
                    ui,
                    data,
                    actions,
                    cue,
                    egui::vec2(width, BUTTON_HEIGHT),
                    live,
                );
            }
        });
        ui.add_space(GAP);
    }
}

fn render_pad(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    cue: &Cue,
    size: egui::Vec2,
    live: bool,
) {
    let at = data.transport.timecode_rate.format(cue.at);
    let response = ui.add_enabled(
        live,
        egui::Button::new(egui::RichText::new(&cue.name).size(11.0))
            .min_size(size)
            .fill(egui::Color32::from_rgb(30, 28, 18))
            .stroke(egui::Stroke::new(1.0_f32, COLOR.gamma_multiply(0.6))),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            live,
            format!("Cue {} at {at}", cue.name),
        )
    });
    let response = response
        .on_hover_text(format!("Go to {} at {at}", cue.name))
        .on_disabled_hover_text("Position is owned by the timecode master");

    if response.clicked() {
        actions.commands.push(EngineCommand::TriggerCue {
            uuid: cue.uuid.clone(),
        });
    }
    learn_overlay(ui, response.rect, cue, data, actions);
}

/// The same glow every other mappable control wears, over the path a control
/// surface reaches this cue by.
fn learn_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    cue: &Cue,
    data: &UIData,
    actions: &mut UIActions,
) {
    if !data.midi_learn_active {
        return;
    }
    let path = format!("cue/{}/fire", cue.uuid);
    if data.midi_learn_target.as_deref() == Some(path.as_str()) {
        widgets::draw_midi_learn_selected(ui, rect);
    } else {
        widgets::draw_midi_learn_glow(ui, rect);
    }
    let id = ui.id().with(("cue_midi_learn", cue.uuid.as_str()));
    if ui.interact(rect, id, egui::Sense::click()).clicked() {
        actions.session.midi_learn_select = Some(path);
    }
}
