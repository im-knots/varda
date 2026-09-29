//! Cue bank: the arrangement's cue points as pads in Performance mode.
//!
//! Drawn from the cue list every frame, so renames, moves and deletes need no
//! reconciliation.

use super::super::{UIActions, UIData, widgets};
use crate::arrangement::Cue;
use crate::engine::EngineCommand;
use crate::engine::value::param::ParamAddress;
use crate::transport::TransportSource;

/// Same color as the ruler's cue marks.
const COLOR: egui::Color32 = super::arrangement::CUE_COLOR;

const COLUMNS: usize = 2;
const BUTTON_HEIGHT: f32 = 24.0;
const GAP: f32 = 4.0;

/// Two buttons per row, in ruler order.
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

    // While chasing timecode the master owns the position, so pads show disabled.
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

/// MIDI-learn highlight for this cue's control-surface address.
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
    let path = ParamAddress::cue_fire(&cue.uuid).to_string();
    if data.midi_learn_target.as_deref() == Some(path.as_str()) {
        widgets::draw_midi_learn_selected(ui, rect);
    } else {
        widgets::draw_midi_learn_glow(ui, rect);
    }
    let id = ui.id().with(("cue_midi_learn", cue.uuid.as_str()));
    if ui.interact(rect, id, egui::Sense::click()).clicked() {
        actions
            .commands
            .push(EngineCommand::MidiLearnSelect { path });
    }
}
