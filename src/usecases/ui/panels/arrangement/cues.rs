//! Cue points: the marks the transport's arrows walk between.
//!
//! Drawn on the ruler as a dot with its name, and down the lanes as a dashed
//! line, so a cue reads as an instant across every deck rather than as a mark on
//! the ruler alone. See /spec/arrangement.md § Cue points.

use super::{snap_seconds, TimeAxis};
use crate::engine::EngineCommand;
use crate::usecases::ui::{UIActions, UIData};

/// Not the transport's status colour: a cue is not the transport.
pub(super) const COLOR: egui::Color32 = egui::Color32::from_rgb(240, 200, 60);

const DOT_RADIUS: f32 = 4.0;
/// Wide enough to grab without covering the ruler either side of it.
const HANDLE_WIDTH: f32 = 11.0;

/// Draw and edit every cue in view.
///
/// Registered after the ruler's own interaction so a press on a handle belongs
/// to the cue rather than scrubbing the show out from under the drag.
pub(super) fn render(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    ruler: egui::Rect,
    lanes: egui::Rect,
    axis: TimeAxis,
) {
    let Some(arrangement) = data.arrangement.as_ref() else {
        return;
    };
    for cue in &arrangement.config.cues {
        let x = axis.x(cue.at);
        if x < ruler.left() || x > ruler.right() {
            continue;
        }
        draw(ui, cue, x, ruler, lanes);
        handle(ui, data, actions, cue, x, ruler, axis);
    }
}

fn draw(
    ui: &egui::Ui,
    cue: &crate::arrangement::Cue,
    x: f32,
    ruler: egui::Rect,
    lanes: egui::Rect,
) {
    let painter = ui.painter_at(lanes);
    painter.extend(egui::Shape::dashed_line(
        &[egui::pos2(x, lanes.top()), egui::pos2(x, lanes.bottom())],
        egui::Stroke::new(1.0_f32, COLOR.gamma_multiply(0.7)),
        2.0_f32,
        4.0_f32,
    ));

    let painter = ui.painter_at(ruler);
    let centre = egui::pos2(x, ruler.top() + DOT_RADIUS + 2.0);
    painter.circle_filled(centre, DOT_RADIUS, COLOR);
    painter.text(
        egui::pos2(x + DOT_RADIUS + 3.0, centre.y),
        egui::Align2::LEFT_CENTER,
        &cue.name,
        egui::FontId::proportional(10.0),
        COLOR,
    );
}

fn handle(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    cue: &crate::arrangement::Cue,
    x: f32,
    ruler: egui::Rect,
    axis: TimeAxis,
) {
    let id = ui.id().with(("arrangement_cue", &cue.uuid));
    let rect = egui::Rect::from_min_max(
        egui::pos2(x - HANDLE_WIDTH / 2.0, ruler.top()),
        egui::pos2(x + HANDLE_WIDTH / 2.0, ruler.bottom()),
    );
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Cue {}", cue.name))
    });

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            // One undo entry for the whole drag, not one per frame.
            actions.session.gesture_active = true;
            let at = snap_seconds(data, axis.seconds(pos.x).max(0.0));
            if (at - cue.at).abs() > f64::EPSILON {
                actions.commands.push(EngineCommand::UpdateCue {
                    uuid: cue.uuid.clone(),
                    at: Some(at),
                    name: None,
                });
            }
        }
    }

    let hover = format!(
        "{} at {}. Drag to move, right-click to rename or delete",
        cue.name,
        data.transport.timecode_rate.format(cue.at)
    );
    response.clone().on_hover_text(hover);
    response.context_menu(|ui| menu(ui, actions, cue));
}

/// Rename and delete. The name commits on Enter or when the field loses focus,
/// rather than per keystroke, so a rename is one undo entry.
fn menu(ui: &mut egui::Ui, actions: &mut UIActions, cue: &crate::arrangement::Cue) {
    let id = ui.id().with(("cue_name", &cue.uuid));
    let mut name: String = ui.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || cue.name.clone())
            .clone()
    });

    let edit = ui.add(
        egui::TextEdit::singleline(&mut name)
            .desired_width(120.0)
            .hint_text("Cue name"),
    );
    if edit.changed() {
        ui.data_mut(|d| d.insert_temp(id, name.clone()));
    }
    let committed = edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter));
    if committed && name != cue.name && !name.is_empty() {
        actions.commands.push(EngineCommand::UpdateCue {
            uuid: cue.uuid.clone(),
            at: None,
            name: Some(name),
        });
        ui.close();
    }

    if ui.button("Delete cue").clicked() {
        actions.commands.push(EngineCommand::RemoveCue {
            uuid: cue.uuid.clone(),
        });
        ui.data_mut(|d| d.remove::<String>(id));
        ui.close();
    }
}
