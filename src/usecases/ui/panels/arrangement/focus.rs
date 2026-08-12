//! The focus area: the stretch of show being worked on.
//!
//! A bar on its own strip above the ruler, drawn, moved, and resized like a
//! region. Looping is one thing done to it rather than the thing it is, so the
//! range outlives the loop it sets and can be zoomed to instead. See
//! /spec/arrangement.md § The focus area.
//!
//! The strip is its own band rather than a modifier-drag on the ruler because
//! the ruler already scrubs on press, drops a cue on double-click, and carries
//! cue handles that claim the band around themselves.

use super::super::super::state::FocusRange;
use super::super::super::{UIActions, UIData};
use super::regions::press_origin;
use super::{min_span, snap_seconds, TimeAxis};
use crate::engine::EngineCommand;
use crate::transport::LoopRegion;

/// Height of the strip. Tall enough to grab, short enough that it does not read
/// as a row of the arrangement.
pub(super) const STRIP_HEIGHT: f32 = 12.0;

/// Grab zone for either edge, in pixels, on both sides of it. Same reasoning as
/// a region's: aiming at a one-pixel line and landing just outside it is the
/// normal way to miss.
const EDGE_GRAB: f32 = 5.0;

const COLOR: egui::Color32 = egui::Color32::from_rgb(120, 180, 255);

/// Which part of the bar a drag has hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    Move,
    ResizeStart,
    ResizeEnd,
}

/// A held edit, carried across the frames of one gesture.
#[derive(Clone, Copy)]
struct Drag {
    grab: Grab,
    origin: FocusRange,
    /// Show position the pointer was over when the drag started, so the edit is
    /// an absolute offset rather than accumulated deltas.
    at: f64,
}

/// Draw the strip and handle every gesture on it.
pub(super) fn render(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    strip: egui::Rect,
    lanes: egui::Rect,
    axis: TimeAxis,
) {
    let painter = ui.painter_at(strip);
    painter.rect_filled(
        strip,
        0.0,
        ui.visuals().extreme_bg_color.gamma_multiply(0.5),
    );

    // Registered first, so the bar drawn on top of it takes the press: egui
    // gives it to the last widget registered.
    handle_create(ui, data, actions, strip, axis);

    if let Some(range) = data.arrangement_focus {
        draw(ui, data, range, strip, lanes, axis);
        handle_edit(ui, data, actions, range, strip, axis);
    }
}

/// Drag across the strip to mark a range.
///
/// The range is published on every frame of the drag rather than at the end, so
/// the bar is drawn under the pointer while it is being drawn.
fn handle_create(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    strip: egui::Rect,
    axis: TimeAxis,
) {
    let id = ui.id().with("arrangement_focus_strip");
    let response = ui.interact(strip, id, egui::Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Focus area strip")
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    response
        .clone()
        .on_hover_text("Drag to mark the stretch you are working on, then right-click it to loop");

    let anchor_id = id.with("anchor");
    if response.drag_started() {
        if let Some(pos) = press_origin(&response) {
            let anchor = snap_seconds(data, axis.seconds(pos.x));
            ui.ctx()
                .memory_mut(|mem| mem.data.insert_temp(anchor_id, anchor));
        }
    }

    let anchor: Option<f64> = ui.ctx().memory(|mem| mem.data.get_temp(anchor_id));
    if let (Some(anchor), Some(pos)) = (anchor, response.interact_pointer_pos()) {
        let range = FocusRange::new(anchor, snap_seconds(data, axis.seconds(pos.x)));
        if response.dragged() && range.span() >= min_span(data) {
            actions.session.gesture_active = true;
            publish(actions, data, range);
        }
        if response.drag_stopped() {
            ui.ctx().memory_mut(|mem| mem.data.remove::<f64>(anchor_id));
        }
    }
}

/// Move the bar by its body, resize it by either edge.
fn handle_edit(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    range: FocusRange,
    strip: egui::Rect,
    axis: TimeAxis,
) {
    let bar = bar_rect(range, strip, axis);
    if bar.right() < strip.left() || bar.left() > strip.right() {
        return;
    }
    let id = ui.id().with("arrangement_focus");
    let response = ui.interact(
        bar.expand2(egui::vec2(EDGE_GRAB, 0.0)),
        id,
        egui::Sense::click_and_drag(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Focus area"));

    if response.hovered() {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            ui.ctx().set_cursor_icon(match grab_at(bar, pos.x) {
                Grab::Move => egui::CursorIcon::Grab,
                Grab::ResizeStart | Grab::ResizeEnd => egui::CursorIcon::ResizeHorizontal,
            });
        }
    }

    if response.drag_started() {
        if let Some(pos) = press_origin(&response) {
            let drag = Drag {
                grab: grab_at(bar, pos.x),
                origin: range,
                at: axis.seconds(pos.x),
            };
            ui.ctx().memory_mut(|mem| mem.data.insert_temp(id, drag));
        }
    }

    if response.dragged() {
        let drag: Option<Drag> = ui.ctx().memory(|mem| mem.data.get_temp(id));
        if let (Some(drag), Some(pos)) = (drag, response.interact_pointer_pos()) {
            actions.session.gesture_active = true;
            let edited = dragged(
                drag.grab,
                drag.origin,
                axis.seconds(pos.x) - drag.at,
                |s| snap_seconds(data, s),
                min_span(data),
            );
            if edited != range {
                publish(actions, data, edited);
            }
        }
    }

    if response.drag_stopped() {
        ui.ctx().memory_mut(|mem| mem.data.remove::<Drag>(id));
    }

    let looping = data.transport.loop_region.is_some();
    response
        .clone()
        .on_hover_text(hover_text(data, range, looping));
    response.context_menu(|ui| menu(ui, data, actions, range, strip.width(), looping));
}

fn menu(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    range: FocusRange,
    width: f32,
    looping: bool,
) {
    if looping {
        if ui
            .button("Stop looping")
            .on_hover_text("Playback runs on past the end of the range. The range stays marked.")
            .clicked()
        {
            actions
                .commands
                .push(EngineCommand::SetTransportLoop { region: None });
            ui.close();
        }
    } else if ui
        .button("Loop this range")
        .on_hover_text("Playback wraps back to the start of the range at its end.")
        .clicked()
    {
        actions.commands.push(EngineCommand::SetTransportLoop {
            region: LoopRegion::new(range.start, range.end).ok(),
        });
        ui.close();
    }

    if ui
        .button("Zoom to range")
        .on_hover_text("Fill the timeline with the range.")
        .clicked()
    {
        let (pps, scroll) = zoom_to(range, width);
        actions.session.set_arrangement_zoom = Some(pps);
        actions.session.set_arrangement_scroll = Some(scroll);
        ui.close();
    }

    if ui
        .button("Clear")
        .on_hover_text("Unmark the range, and stop looping it if it was.")
        .clicked()
    {
        actions.session.clear_arrangement_focus = true;
        if data.transport.loop_region.is_some() {
            actions
                .commands
                .push(EngineCommand::SetTransportLoop { region: None });
        }
        ui.close();
    }
}

/// Mark the range, and keep the loop on it while looping is on.
///
/// The loop following the range is what makes the drawn bar and the thing that
/// wraps playback the same object, which is the whole reason the bar is worth
/// looking at while a loop runs.
fn publish(actions: &mut UIActions, data: &UIData, range: FocusRange) {
    actions.session.set_arrangement_focus = Some(range);
    if data.transport.loop_region.is_some() {
        actions.commands.push(EngineCommand::SetTransportLoop {
            region: LoopRegion::new(range.start, range.end).ok(),
        });
    }
}

fn draw(
    ui: &egui::Ui,
    data: &UIData,
    range: FocusRange,
    strip: egui::Rect,
    lanes: egui::Rect,
    axis: TimeAxis,
) {
    let looping = data.transport.loop_region.is_some();
    let bar = bar_rect(range, strip, axis);
    let painter = ui.painter_at(strip);
    if looping {
        painter.rect_filled(bar, 1.0, COLOR.gamma_multiply(0.85));
    } else {
        painter.rect_filled(bar, 1.0, COLOR.gamma_multiply(0.25));
        painter.rect_stroke(
            bar,
            1.0,
            egui::Stroke::new(1.0_f32, COLOR),
            egui::StrokeKind::Inside,
        );
    }

    // Faint down the lanes, so the stretch reads against the arrangement it
    // covers without competing with the regions inside it.
    if looping {
        let over = egui::Rect::from_min_max(
            egui::pos2(bar.left().max(lanes.left()), lanes.top()),
            egui::pos2(bar.right().min(lanes.right()), lanes.bottom()),
        );
        if over.width() > 0.0 {
            ui.painter_at(lanes)
                .rect_filled(over, 0.0, COLOR.gamma_multiply(0.06));
        }
    }
}

fn hover_text(data: &UIData, range: FocusRange, looping: bool) -> String {
    let rate = data.transport.timecode_rate;
    format!(
        "Focus area {} to {}{}\nDrag to move, drag an edge to resize, right-click for loop and zoom",
        rate.format(range.start),
        rate.format(range.end),
        if looping { ", looping" } else { "" }
    )
}

fn bar_rect(range: FocusRange, strip: egui::Rect, axis: TimeAxis) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(axis.x(range.start), strip.top() + 1.0),
        egui::pos2(axis.x(range.end), strip.bottom() - 1.0),
    )
}

/// Which part of the bar a press at `x` has hold of.
///
/// A bar narrower than the two grab zones is all edges and no body, which would
/// leave no way to move a short range; the body wins the middle in that case.
fn grab_at(bar: egui::Rect, x: f32) -> Grab {
    if bar.width() <= EDGE_GRAB * 2.0 {
        return Grab::Move;
    }
    if x <= bar.left() + EDGE_GRAB {
        Grab::ResizeStart
    } else if x >= bar.right() - EDGE_GRAB {
        Grab::ResizeEnd
    } else {
        Grab::Move
    }
}

/// The range an in-flight drag has produced.
///
/// Resizing stops at `min_span` rather than letting an edge cross the other:
/// a range inverted mid-drag would re-enter as a different range with the
/// grabbed edge now on the far side, which reads as the bar jumping.
fn dragged(
    grab: Grab,
    origin: FocusRange,
    delta: f64,
    snap: impl Fn(f64) -> f64,
    min_span: f64,
) -> FocusRange {
    match grab {
        Grab::Move => {
            let start = snap((origin.start + delta).max(0.0));
            FocusRange {
                start,
                end: start + origin.span(),
            }
        }
        Grab::ResizeStart => FocusRange {
            start: snap(origin.start + delta)
                .min(origin.end - min_span)
                .max(0.0),
            end: origin.end,
        },
        Grab::ResizeEnd => FocusRange {
            start: origin.start,
            end: snap(origin.end + delta).max(origin.start + min_span),
        },
    }
}

/// The zoom and scroll that put the range across the timeline.
fn zoom_to(range: FocusRange, width: f32) -> (f32, f64) {
    let span = range.span().max(f64::from(f32::EPSILON));
    let pps = f64::from(width) / span;
    (pps as f32, range.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    const STRIP: egui::Rect = egui::Rect {
        min: egui::pos2(100.0, 0.0),
        max: egui::pos2(700.0, STRIP_HEIGHT),
    };

    fn axis() -> TimeAxis {
        TimeAxis {
            left: 100.0,
            scroll: 0.0,
            pps: 40.0,
        }
    }

    fn range() -> FocusRange {
        FocusRange {
            start: 2.0,
            end: 6.0,
        }
    }

    /// A range drawn right to left is the same stretch of show as one drawn left
    /// to right. Anything else would make half of all drags produce nothing.
    #[test]
    fn a_range_is_the_same_whichever_way_it_was_drawn() {
        assert_eq!(FocusRange::new(6.0, 2.0), FocusRange::new(2.0, 6.0));
        assert_eq!(
            FocusRange::new(-4.0, 2.0).start,
            0.0,
            "the show starts at 0"
        );
    }

    #[test]
    fn the_edges_are_grabbable_and_the_middle_moves() {
        let bar = bar_rect(range(), STRIP, axis());
        assert_eq!(grab_at(bar, bar.left() + 1.0), Grab::ResizeStart);
        assert_eq!(grab_at(bar, bar.right() - 1.0), Grab::ResizeEnd);
        assert_eq!(grab_at(bar, bar.center().x), Grab::Move);
    }

    /// Zoomed far out, a range can be a few pixels wide, and both grab zones
    /// would cover all of it. Moving it is then the only edit that still makes
    /// sense, and resizing is a zoom away.
    #[test]
    fn a_bar_too_narrow_to_have_edges_can_still_be_moved() {
        let narrow = egui::Rect::from_min_max(egui::pos2(200.0, 0.0), egui::pos2(206.0, 10.0));
        assert_eq!(grab_at(narrow, narrow.left()), Grab::Move);
        assert_eq!(grab_at(narrow, narrow.right()), Grab::Move);
    }

    #[test]
    fn moving_the_bar_keeps_its_length() {
        let moved = dragged(Grab::Move, range(), 3.0, |s| s, 0.1);
        assert!((moved.span() - range().span()).abs() < f64::EPSILON);
        assert!((moved.start - 5.0).abs() < f64::EPSILON);
    }

    /// Dragged hard left, the bar stops at the start of the show rather than
    /// running into negative positions the transport cannot reach.
    #[test]
    fn the_bar_stops_at_the_start_of_the_show() {
        let moved = dragged(Grab::Move, range(), -100.0, |s| s, 0.1);
        assert!((moved.start - 0.0).abs() < f64::EPSILON);
        assert!((moved.span() - range().span()).abs() < f64::EPSILON);
    }

    /// An edge dragged past the other one stops rather than inverting, so the
    /// grabbed edge stays the edge under the pointer.
    #[test]
    fn an_edge_cannot_cross_the_other_one() {
        let min = 0.5;
        let squashed = dragged(Grab::ResizeStart, range(), 100.0, |s| s, min);
        assert!((squashed.end - range().end).abs() < f64::EPSILON);
        assert!((squashed.start - (range().end - min)).abs() < f64::EPSILON);

        let squashed = dragged(Grab::ResizeEnd, range(), -100.0, |s| s, min);
        assert!((squashed.start - range().start).abs() < f64::EPSILON);
        assert!((squashed.end - (range().start + min)).abs() < f64::EPSILON);
    }

    /// A scene saved with a loop opens with the strip already showing it, so the
    /// thing wrapping playback is visible before anyone drags anything.
    #[test]
    fn a_marked_range_can_be_sent_to_the_transport_as_a_loop() {
        let mut data = super::super::tests::fixture_with_arrangement();
        data.arrangement_focus = Some(range());
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            super::super::render_arrangement(ui, &data, &mut actions);
        });

        harness.get_by_label("Focus area").click_secondary();
        harness.run();
        harness.get_by_label("Loop this range").click();
        harness.run();
        drop(harness);

        let looped = actions.commands.iter().find_map(|c| match c {
            EngineCommand::SetTransportLoop { region } => *region,
            _ => None,
        });
        let looped = looped.expect("the menu must set the loop");
        assert!((looped.start - range().start).abs() < f64::EPSILON);
        assert!((looped.end - range().end).abs() < f64::EPSILON);
    }

    /// Clearing is not just an unmark while a loop is running on the range:
    /// leaving playback wrapping inside a stretch nothing is drawn around is how
    /// a show appears to hang.
    #[test]
    fn clearing_a_looping_range_stops_the_loop_with_it() {
        let mut data = super::super::tests::fixture_with_arrangement();
        data.arrangement_focus = Some(range());
        data.transport.loop_region = LoopRegion::new(range().start, range().end).ok();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            super::super::render_arrangement(ui, &data, &mut actions);
        });

        harness.get_by_label("Focus area").click_secondary();
        harness.run();
        harness.get_by_label("Clear").click();
        harness.run();
        drop(harness);

        assert!(actions.session.clear_arrangement_focus);
        assert!(actions
            .commands
            .iter()
            .any(|c| matches!(c, EngineCommand::SetTransportLoop { region: None })));
    }

    /// While a loop is on, the bar and the loop are meant to be one object, so
    /// an edit to the bar has to carry the loop with it.
    #[test]
    fn moving_the_bar_while_looping_moves_the_loop() {
        let data = {
            let mut data = super::super::tests::fixture_with_arrangement();
            data.arrangement_focus = Some(range());
            data.transport.loop_region = LoopRegion::new(range().start, range().end).ok();
            data
        };
        let mut actions = UIActions::new();
        publish(&mut actions, &data, FocusRange::new(10.0, 14.0));

        assert_eq!(
            actions.session.set_arrangement_focus,
            Some(FocusRange::new(10.0, 14.0))
        );
        let moved = actions
            .commands
            .iter()
            .find_map(|c| match c {
                EngineCommand::SetTransportLoop { region } => *region,
                _ => None,
            })
            .expect("the loop follows the range");
        assert!((moved.start - 10.0).abs() < f64::EPSILON);
    }

    /// Without a loop running there is nothing to keep in step, and pushing a
    /// loop command per frame of a drag would start one nobody asked for.
    #[test]
    fn moving_the_bar_without_a_loop_leaves_the_transport_alone() {
        let data = super::super::tests::fixture_with_arrangement();
        let mut actions = UIActions::new();
        publish(&mut actions, &data, FocusRange::new(10.0, 14.0));

        assert!(actions.session.set_arrangement_focus.is_some());
        assert!(actions.commands.is_empty(), "{:?}", actions.commands);
    }

    #[test]
    fn zooming_to_a_range_puts_its_start_at_the_left_edge() {
        let (pps, scroll) = zoom_to(range(), 600.0);
        assert!((scroll - range().start).abs() < f64::EPSILON);
        // Four seconds across six hundred pixels.
        assert!((pps - 150.0).abs() < f32::EPSILON, "{pps}");
    }
}
