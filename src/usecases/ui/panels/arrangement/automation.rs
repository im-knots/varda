//! Automation rows: the envelope editor.
//!
//! An envelope's editor is its lane rather than a card in the right panel,
//! because an arrangement produces hundreds of curves and a row of parameter
//! cards does not survive that. See /spec/arrangement.md § Envelopes are not
//! modulator cards.
//!
//! Every edit replaces the whole breakpoint list through
//! `SetEnvelopeBreakpoints`. The engine owns the sort invariant, so the panel
//! never has to maintain it.

use super::super::super::{ModSourceUI, UIActions, UIData};
use super::super::utils::channel_color;
use super::{snap_seconds, AutomationRow, Owner, RowGeometry};
use crate::engine::EngineCommand;
use crate::modulation::{Breakpoint, CurveKind};

/// Vertical breathing room so a breakpoint at 0.0 or 1.0 is still grabbable.
const PADDING: f32 = 5.0;
const POINT_RADIUS: f32 = 4.0;
/// How close a press must land to count as grabbing a breakpoint.
const GRAB_RADIUS: f32 = 7.0;
/// How close a press must land to the drawn curve to count as grabbing a
/// segment. Smaller than the breakpoint radius, which wins where they overlap.
const CURVE_GRAB: f32 = 6.0;
/// Vertical travel, in pixels, for one unit of tension.
const TENSION_PIXELS: f32 = 40.0;
/// Past about this the curve is indistinguishable from a hold, so the drag
/// stops rather than running away into numbers that all look the same.
const TENSION_LIMIT: f32 = 4.0;

/// What a drag on the curve track has hold of.
#[derive(Clone, Copy)]
enum CurveDrag {
    Point(usize),
    /// The segment leaving breakpoint `index`, bent by vertical travel from
    /// where the press landed.
    Tension {
        index: usize,
        origin: f32,
        grab_y: f32,
    },
    /// The flat run of breakpoints `first..=last`, raised or lowered bodily
    /// from the value `origin` it was pressed at.
    Level {
        first: usize,
        last: usize,
        origin: f32,
        grab_y: f32,
    },
}

/// What a curve's context menu was opened on: the breakpoint under the press if
/// there was one, and the time the press landed at, which is where a paste from
/// that menu lands.
#[derive(Clone, Copy)]
struct MenuSubject {
    point: Option<usize>,
    at: f64,
}

/// Which envelope the keyboard is addressing, and what it has on the clipboard.
fn selected_id() -> egui::Id {
    egui::Id::new("__arrangement_selected_envelope")
}

fn clipboard_id() -> egui::Id {
    egui::Id::new("__arrangement_breakpoint_clipboard")
}

pub(super) fn render_automation_row(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    row: &AutomationRow<'_>,
) {
    let selected: Option<String> = ui.ctx().memory(|mem| mem.data.get_temp(selected_id()));
    let is_selected = selected.as_deref() == Some(row.envelope_uuid);
    let color = channel_color(row.ch_idx);

    {
        let painter = ui.painter_at(geom.track);
        painter.rect_filled(
            geom.track,
            0.0,
            ui.visuals()
                .extreme_bg_color
                .gamma_multiply(if is_selected { 0.7 } else { 0.4 }),
        );
        draw_curve(&painter, geom, row.breakpoints, color);
    }
    ui.painter().text(
        geom.header.left_center() + egui::vec2(36.0, 0.0),
        egui::Align2::LEFT_CENTER,
        row.label(),
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );

    render_header(ui, data, actions, geom, row, is_selected);
    render_track(ui, data, actions, geom, row);
}

fn render_header(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    row: &AutomationRow<'_>,
    is_selected: bool,
) {
    let response = ui.interact(
        geom.header,
        ui.id().with(("arrangement_curve_header", geom.idx)),
        egui::Sense::click(),
    );
    let label = format!("{} automation", row.label());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            is_selected,
            label.clone(),
        )
    });
    if response.clicked() {
        select(ui.ctx(), row.envelope_uuid);
        // The bottom bar follows the selection, so clicking a curve puts the
        // parameters of whatever owns it within reach.
        match row.owner {
            Owner::Deck(ch_idx, deck_idx) => {
                actions.session.select_deck = Some((ch_idx, deck_idx));
            }
            Owner::Channel(ch_idx) => actions.session.select_channel = Some(ch_idx),
            Owner::Master => actions.session.select_master = true,
        }
    }
    response.context_menu(|ui| {
        // The header is beside the timeline rather than on it, so a paste from
        // here has no time of its own and falls back to the playhead.
        clipboard_items(ui, data, actions, row, data.transport.position);
        ui.separator();
        if ui.button("Remove automation lane").clicked() {
            actions
                .commands
                .push(EngineCommand::RemoveModulationSource {
                    uuid: row.envelope_uuid.to_string(),
                });
            ui.close();
        }
    });
}

fn render_track(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    row: &AutomationRow<'_>,
) {
    let id = ui.id().with(("arrangement_curve", geom.idx));
    let response = ui.interact(geom.track, id, egui::Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} automation curve", row.label()),
        )
    });

    if response.clicked() || response.drag_started() {
        select(ui.ctx(), row.envelope_uuid);
    }

    if response.hovered() {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            if point_at(row.breakpoints, geom, pos).is_none()
                && (bendable_segment_at(row.breakpoints, geom, pos).is_some()
                    || flat_run_at(row.breakpoints, geom, pos).is_some())
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
        }
    }

    let grabbed_id = id.with("grabbed");
    if response.drag_started() {
        let grabbed = super::regions::press_origin(&response).and_then(|pos| {
            // A breakpoint wins over the segment it sits on: moving a point is
            // the more common gesture and the more precise target.
            point_at(row.breakpoints, geom, pos)
                .map(CurveDrag::Point)
                .or_else(|| {
                    bendable_segment_at(row.breakpoints, geom, pos).map(|index| {
                        CurveDrag::Tension {
                            index,
                            origin: tension_of(row.breakpoints[index].curve),
                            grab_y: pos.y,
                        }
                    })
                })
                // Flat where a bend would not show, so the two never contend.
                .or_else(|| {
                    flat_run_at(row.breakpoints, geom, pos).map(|(first, last)| CurveDrag::Level {
                        first,
                        last,
                        origin: row.breakpoints[first].value,
                        grab_y: pos.y,
                    })
                })
        });
        ui.ctx()
            .memory_mut(|mem| mem.data.insert_temp(grabbed_id, grabbed));
    }

    if response.dragged() {
        let grabbed: Option<Option<CurveDrag>> =
            ui.ctx().memory(|mem| mem.data.get_temp(grabbed_id));
        if let (Some(Some(drag)), Some(pos)) = (grabbed, response.interact_pointer_pos()) {
            actions.session.gesture_active = true;
            let edited = match drag {
                CurveDrag::Point(index) => with_point_moved(
                    row.breakpoints,
                    index,
                    snap_seconds(data, geom.axis.seconds(pos.x)),
                    value_at(geom.track, pos.y),
                ),
                CurveDrag::Tension {
                    index,
                    origin,
                    grab_y,
                } => with_curve(
                    row.breakpoints,
                    index,
                    CurveKind::Linear {
                        tension: tension_from_drag(
                            origin,
                            pos.y - grab_y,
                            descends(row.breakpoints, index),
                        ),
                    },
                ),
                CurveDrag::Level {
                    first,
                    last,
                    origin,
                    grab_y,
                } => with_run_levelled(
                    row.breakpoints,
                    first,
                    last,
                    origin + value_travel(geom.track, grab_y - pos.y),
                ),
            };
            if edited != row.breakpoints {
                push_points(actions, row.envelope_uuid, edited);
            }
        }
    }

    if response.drag_stopped() {
        ui.ctx()
            .memory_mut(|mem| mem.data.remove::<Option<CurveDrag>>(grabbed_id));
    }

    if response.double_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let position = snap_seconds(data, geom.axis.seconds(pos.x));
            let points = match point_at(row.breakpoints, geom, pos) {
                // A double click on a point removes it, which is the gesture
                // every curve editor uses and saves a trip to the menu.
                Some(index) => without_point(row.breakpoints, index),
                None => with_point_added(row.breakpoints, position, value_at(geom.track, pos.y)),
            };
            push_points(actions, row.envelope_uuid, points);
        }
    }

    draw_points(ui, geom, row);
    point_context_menu(&response, ui, data, geom, row, actions);
}

/// Copying a shape between parameters, which is how one curve gets reused.
///
/// An envelope drives the one parameter it was drawn for, so the same shape on a
/// second parameter is a second curve rather than a shared source. See
/// /spec/automation.md § One envelope per parameter.
///
/// The clipboard is the one the keyboard shortcuts use, so a curve copied with
/// the menu pastes with `Cmd+V` and the other way around.
///
/// `anchor` is where the shape lands, in seconds: the point on the timeline the
/// menu was opened over, or the playhead where the press carries no time of its
/// own. See /spec/automation.md § One envelope per parameter.
fn clipboard_items(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    row: &AutomationRow<'_>,
    anchor: f64,
) {
    if ui
        .add_enabled(!row.breakpoints.is_empty(), egui::Button::new("Copy curve"))
        .on_disabled_hover_text("This lane has no points to copy")
        .clicked()
    {
        let copied = row.breakpoints.to_vec();
        ui.ctx()
            .memory_mut(|mem| mem.data.insert_temp(clipboard_id(), copied));
        ui.close();
    }

    let clipboard: Option<Vec<Breakpoint>> =
        ui.ctx().memory(|mem| mem.data.get_temp(clipboard_id()));
    let held = clipboard.as_ref().is_some_and(|points| !points.is_empty());
    if ui
        .add_enabled(held, egui::Button::new("Paste curve"))
        .on_hover_text(format!(
            "Lands at {}, replacing the span it covers",
            data.transport.timecode_rate.format(anchor)
        ))
        .on_disabled_hover_text("Copy a curve first")
        .clicked()
    {
        if let Some(clipboard) = clipboard {
            push_points(
                actions,
                row.envelope_uuid,
                pasted(row.breakpoints, &clipboard, anchor),
            );
        }
        ui.close();
    }
}

/// Curve shape and delete on the breakpoint the press landed on, and the
/// clipboard either way.
///
/// Which breakpoint the menu belongs to is decided **once, when it opens**, and
/// held in memory for as long as it is up. Deciding it from the live pointer
/// instead rewrites the menu the moment the hand moves toward the item it was
/// opened for, because reaching any item means leaving the point it belongs to:
/// the click then lands on whatever slid under the cursor.
fn point_context_menu(
    response: &egui::Response,
    ui: &egui::Ui,
    data: &UIData,
    geom: RowGeometry,
    row: &AutomationRow<'_>,
    actions: &mut UIActions,
) {
    let subject_id = ui.id().with(("curve_menu_subject", row.envelope_uuid));
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let subject = MenuSubject {
                point: point_at(row.breakpoints, geom, pos),
                at: snap_seconds(data, geom.axis.seconds(pos.x).max(0.0)),
            };
            ui.ctx()
                .memory_mut(|mem| mem.data.insert_temp(subject_id, subject));
        }
    }
    let Some(subject): Option<MenuSubject> = ui.ctx().memory(|mem| mem.data.get_temp(subject_id))
    else {
        return;
    };
    // The list can shrink under an open menu, so the frozen index is checked
    // rather than trusted.
    let point = subject.point.filter(|i| *i < row.breakpoints.len());

    response.context_menu(|ui| {
        if let Some(index) = point {
            for (label, curve) in [
                ("Linear", CurveKind::Linear { tension: 0.0 }),
                ("Smooth", CurveKind::Smooth),
                ("Hold", CurveKind::Step),
            ] {
                let active = std::mem::discriminant(&row.breakpoints[index].curve)
                    == std::mem::discriminant(&curve);
                if ui.selectable_label(active, label).clicked() {
                    push_points(
                        actions,
                        row.envelope_uuid,
                        with_curve(row.breakpoints, index, curve),
                    );
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Delete breakpoint").clicked() {
                push_points(
                    actions,
                    row.envelope_uuid,
                    without_point(row.breakpoints, index),
                );
                ui.close();
            }
            ui.separator();
        }
        // Offered wherever the press landed, because a point is the thing most
        // people will aim at when they mean "this curve".
        clipboard_items(ui, data, actions, row, subject.at);
    });
}

/// Copy and paste a curve's shape between lanes.
///
/// The clipboard holds breakpoints rather than a whole envelope, so a shape
/// authored on one parameter can be dropped onto another.
pub(super) fn handle_clipboard_shortcuts(ui: &egui::Ui, data: &UIData, actions: &mut UIActions) {
    // A keyboard shortcut must never fire while something is being typed into.
    if ui.ctx().memory(egui::Memory::focused).is_some() {
        return;
    }
    let Some(uuid): Option<String> = ui.ctx().memory(|mem| mem.data.get_temp(selected_id())) else {
        return;
    };
    let Some(points) = envelope_points(data, &uuid) else {
        return;
    };

    let (copy, paste) = ui.ctx().input(|i| {
        (
            i.modifiers.command && i.key_pressed(egui::Key::C),
            i.modifiers.command && i.key_pressed(egui::Key::V),
        )
    });

    if copy && !points.is_empty() {
        let copied = points.to_vec();
        ui.ctx()
            .memory_mut(|mem| mem.data.insert_temp(clipboard_id(), copied));
    }
    if paste {
        let clipboard: Option<Vec<Breakpoint>> =
            ui.ctx().memory(|mem| mem.data.get_temp(clipboard_id()));
        if let Some(clipboard) = clipboard {
            push_points(
                actions,
                &uuid,
                pasted(points, &clipboard, data.transport.position),
            );
        }
    }
}

/// Whether a curve owns the keyboard clipboard this frame.
///
/// `Cmd+C` over a selected lane means its breakpoints, not the deck the lane
/// belongs to, so the scene-object shortcuts stand down while one is selected.
pub(in crate::usecases::ui::panels) fn a_lane_is_selected(ctx: &egui::Context) -> bool {
    let selected: Option<String> = ctx.memory(|mem| mem.data.get_temp(selected_id()));
    selected.is_some()
}

fn select(ctx: &egui::Context, envelope_uuid: &str) {
    let uuid = envelope_uuid.to_string();
    ctx.memory_mut(|mem| mem.data.insert_temp(selected_id(), uuid));
}

fn envelope_points<'a>(data: &'a UIData, uuid: &str) -> Option<&'a [Breakpoint]> {
    data.modulation_sources
        .iter()
        .find(|e| e.uuid == uuid)
        .and_then(|e| match &e.source {
            ModSourceUI::Envelope { breakpoints } => Some(breakpoints.as_slice()),
            _ => None,
        })
}

fn push_points(actions: &mut UIActions, uuid: &str, breakpoints: Vec<Breakpoint>) {
    actions
        .commands
        .push(EngineCommand::SetEnvelopeBreakpoints {
            uuid: uuid.to_string(),
            breakpoints,
        });
}

fn point_y(track: egui::Rect, value: f32) -> f32 {
    let usable = (track.height() - 2.0 * PADDING).max(1.0);
    track.bottom() - PADDING - value.clamp(0.0, 1.0) * usable
}

fn value_at(track: egui::Rect, y: f32) -> f32 {
    let usable = (track.height() - 2.0 * PADDING).max(1.0);
    ((track.bottom() - PADDING - y) / usable).clamp(0.0, 1.0)
}

/// How much value a vertical travel of `dy` pixels is worth, upward positive.
///
/// Unclamped, unlike [`value_at`]: a drag that leaves the lane and comes back
/// has to return to where it was rather than to the edge it was pinned at.
fn value_travel(track: egui::Rect, dy: f32) -> f32 {
    dy / (track.height() - 2.0 * PADDING).max(1.0)
}

/// Index of the breakpoint under `pointer`, if any.
fn point_at(points: &[Breakpoint], geom: RowGeometry, pointer: egui::Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, bp)| {
            let at = egui::pos2(geom.axis.x(bp.position), point_y(geom.track, bp.value));
            (i, at.distance(pointer))
        })
        .filter(|(_, d)| *d <= GRAB_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The segment under `pointer`, if bending it would show.
///
/// A flat segment is refused: `shape()` interpolates between two equal values,
/// so every tension produces the same straight line and the drag would be dead
/// in the hand. Outside the drawn range there is no segment at all, because the
/// envelope holds its end values there.
fn bendable_segment_at(
    points: &[Breakpoint],
    geom: RowGeometry,
    pointer: egui::Pos2,
) -> Option<usize> {
    let position = geom.axis.seconds(pointer.x);
    let index = points
        .windows(2)
        .position(|pair| pair[0].position <= position && position < pair[1].position)?;
    if (points[index].value - points[index + 1].value).abs() < f32::EPSILON {
        return None;
    }

    let mut cursor = 0;
    let value = crate::modulation::evaluate_envelope(points, position, &mut cursor);
    ((pointer.y - point_y(geom.track, value)).abs() <= CURVE_GRAB).then_some(index)
}

/// The flat stretch of curve under `pointer`, as the inclusive range of
/// breakpoints holding it there.
///
/// This is the half of the track a bend cannot claim: a segment between two
/// equal values draws the same straight line at every tension, and outside the
/// drawn range the envelope holds its end value. Both look like a line worth
/// grabbing, and dragging one bodily up or down is what a performer means by it.
///
/// The run is widened across every neighbour at the same value, because what
/// reads as one flat line has to move as one rather than breaking into a ramp at
/// a breakpoint that was never visible.
fn flat_run_at(
    points: &[Breakpoint],
    geom: RowGeometry,
    pointer: egui::Pos2,
) -> Option<(usize, usize)> {
    let position = geom.axis.seconds(pointer.x);
    let last_index = points.len().checked_sub(1)?;
    // Strictly before the first point: a press exactly on it belongs to the
    // segment leaving it, or the two gestures would both claim that one column
    // of pixels and which one ran would be an accident of ordering.
    let seed = if position < points[0].position {
        0
    } else if position >= points[last_index].position {
        last_index
    } else {
        let index = points
            .windows(2)
            .position(|pair| pair[0].position <= position && position < pair[1].position)?;
        // A segment between two different values is a bend's, whatever shape it
        // draws on the way.
        if (points[index].value - points[index + 1].value).abs() > f32::EPSILON {
            return None;
        }
        index
    };

    let value = points[seed].value;
    if (pointer.y - point_y(geom.track, value)).abs() > CURVE_GRAB {
        return None;
    }
    let level = |i: &usize| (points[*i].value - value).abs() < f32::EPSILON;
    let first = (0..seed).rev().take_while(level).last().unwrap_or(seed);
    let last = (seed + 1..points.len())
        .take_while(level)
        .last()
        .unwrap_or(seed);
    Some((first, last))
}

fn tension_of(curve: CurveKind) -> f32 {
    match curve {
        CurveKind::Linear { tension } => tension,
        // A held or smoothed segment starts from straight, since bending it is
        // a request for the eased shape rather than for more of what it was.
        CurveKind::Step | CurveKind::Smooth => 0.0,
    }
}

fn descends(points: &[Breakpoint], index: usize) -> bool {
    points
        .get(index + 1)
        .is_some_and(|next| next.value < points[index].value)
}

/// Tension from vertical drag travel.
///
/// Dragging up bulges the curve up, which for a falling segment means *less*
/// tension: the shaping function runs along the segment rather than along the
/// screen, so the sign has to follow the segment's direction or half the curves
/// in a lane bend the wrong way.
fn tension_from_drag(origin: f32, dy: f32, descends: bool) -> f32 {
    let up = -dy / TENSION_PIXELS;
    let signed = if descends { -up } else { up };
    (origin + signed).clamp(-TENSION_LIMIT, TENSION_LIMIT)
}

/// Move one breakpoint, penned in by its neighbours.
///
/// Clamping rather than reordering keeps the list sorted without renumbering
/// mid-drag, which would otherwise hand the gesture a different point halfway
/// through.
fn with_point_moved(
    points: &[Breakpoint],
    index: usize,
    position: f64,
    value: f32,
) -> Vec<Breakpoint> {
    const GAP: f64 = 1e-4;
    let mut out = points.to_vec();
    let Some(point) = out.get_mut(index) else {
        return out;
    };
    let low = index
        .checked_sub(1)
        .and_then(|i| points.get(i))
        .map_or(0.0, |p| p.position + GAP);
    let high = points
        .get(index + 1)
        .map_or(f64::INFINITY, |p| p.position - GAP);
    point.position = position.clamp(low, high.max(low));
    point.value = value.clamp(0.0, 1.0);
    out
}

/// Set every breakpoint in `first..=last` to `value`, which is what raising a
/// flat line does: the run keeps its length and its shape and changes height.
fn with_run_levelled(
    points: &[Breakpoint],
    first: usize,
    last: usize,
    value: f32,
) -> Vec<Breakpoint> {
    let mut out = points.to_vec();
    for point in out.iter_mut().take(last + 1).skip(first) {
        point.value = value.clamp(0.0, 1.0);
    }
    out
}

/// Insert a breakpoint, inheriting the shape of the segment it lands in.
fn with_point_added(points: &[Breakpoint], position: f64, value: f32) -> Vec<Breakpoint> {
    let index = points
        .iter()
        .position(|p| p.position > position)
        .unwrap_or(points.len());
    let curve = index
        .checked_sub(1)
        .and_then(|i| points.get(i))
        .map_or_else(CurveKind::default, |p| p.curve);
    let mut out = points.to_vec();
    out.insert(
        index,
        Breakpoint {
            position,
            value: value.clamp(0.0, 1.0),
            curve,
        },
    );
    out
}

fn without_point(points: &[Breakpoint], index: usize) -> Vec<Breakpoint> {
    let mut out = points.to_vec();
    if index < out.len() {
        out.remove(index);
    }
    out
}

fn with_curve(points: &[Breakpoint], index: usize, curve: CurveKind) -> Vec<Breakpoint> {
    let mut out = points.to_vec();
    if let Some(point) = out.get_mut(index) {
        point.curve = curve;
    }
    out
}

/// Drop a copied shape in at `at`, clearing whatever it lands on.
///
/// Merging instead would leave the pasted curve fighting the points it was
/// pasted over, which is never what was meant.
fn pasted(points: &[Breakpoint], clipboard: &[Breakpoint], at: f64) -> Vec<Breakpoint> {
    let Some(first) = clipboard.first() else {
        return points.to_vec();
    };
    let offset = at - first.position;
    let last = clipboard.last().map_or(at, |p| p.position + offset);

    let mut out: Vec<Breakpoint> = points
        .iter()
        .filter(|p| p.position < at || p.position > last)
        .copied()
        .collect();
    out.extend(clipboard.iter().map(|p| Breakpoint {
        position: p.position + offset,
        ..*p
    }));
    out.sort_by(|a, b| a.position.total_cmp(&b.position));
    out
}

/// Sampled rather than joined point to point, so eased and held segments draw as
/// the shapes they actually are.
const SAMPLE_PX: f32 = 3.0;

fn draw_curve(
    painter: &egui::Painter,
    geom: RowGeometry,
    points: &[Breakpoint],
    color: egui::Color32,
) {
    if points.is_empty() {
        return;
    }
    let mut cursor = 0;
    let mut path = Vec::new();
    let mut x = geom.track.left();
    while x <= geom.track.right() {
        let value = crate::modulation::evaluate_envelope(points, geom.axis.seconds(x), &mut cursor);
        path.push(egui::pos2(x, point_y(geom.track, value)));
        x += SAMPLE_PX;
    }
    painter.add(egui::Shape::line(
        path,
        egui::Stroke::new(1.5_f32, color.gamma_multiply(0.9)),
    ));
}

fn draw_points(ui: &egui::Ui, geom: RowGeometry, row: &AutomationRow<'_>) {
    let painter = ui.painter_at(geom.track);
    let color = channel_color(row.ch_idx);
    for bp in row.breakpoints {
        let at = egui::pos2(geom.axis.x(bp.position), point_y(geom.track, bp.value));
        if !geom.track.expand(POINT_RADIUS).contains(at) {
            continue;
        }
        painter.circle(
            at,
            POINT_RADIUS,
            color,
            egui::Stroke::new(1.0_f32, ui.visuals().extreme_bg_color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(position: f64, value: f32) -> Breakpoint {
        Breakpoint {
            position,
            value,
            curve: CurveKind::default(),
        }
    }

    fn curve() -> Vec<Breakpoint> {
        vec![linear(0.0, 0.0), linear(4.0, 1.0), linear(8.0, 0.25)]
    }

    fn geom() -> RowGeometry {
        RowGeometry {
            header: egui::Rect::from_min_size(egui::pos2(-100.0, 0.0), egui::vec2(100.0, 40.0)),
            track: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 40.0)),
            axis: super::super::TimeAxis {
                left: 0.0,
                scroll: 0.0,
                pps: 10.0,
            },
            idx: 0,
        }
    }

    /// Dragging up bends the curve up on the way up and on the way down alike,
    /// which means the stored tension has to change sign with the segment.
    #[test]
    fn bending_follows_the_pointer_on_rising_and_falling_segments() {
        let up = tension_from_drag(0.0, -TENSION_PIXELS, false);
        assert!((up - 1.0).abs() < 1e-6);

        let down = tension_from_drag(0.0, -TENSION_PIXELS, true);
        assert!(
            (down + 1.0).abs() < 1e-6,
            "a falling segment bends up with negative tension"
        );
    }

    #[test]
    fn bending_resumes_from_the_shape_it_started_with() {
        let further = tension_from_drag(1.5, -TENSION_PIXELS, false);
        assert!((further - 2.5).abs() < 1e-6, "the drag is relative");

        let pinned = tension_from_drag(0.0, -100.0 * TENSION_PIXELS, false);
        assert!((pinned - TENSION_LIMIT).abs() < 1e-6);
    }

    /// A bend is a request for an eased shape, so a held or smoothed segment
    /// becomes linear at the tension the drag asks for rather than compounding
    /// whatever it was.
    #[test]
    fn a_held_or_smoothed_segment_starts_from_straight() {
        assert!(tension_of(CurveKind::Step).abs() < f32::EPSILON);
        assert!(tension_of(CurveKind::Smooth).abs() < f32::EPSILON);
        assert!((tension_of(CurveKind::Linear { tension: 2.0 }) - 2.0).abs() < f32::EPSILON);

        let bent = with_curve(&curve(), 0, CurveKind::Linear { tension: 1.0 });
        assert!(matches!(bent[0].curve, CurveKind::Linear { tension } if tension == 1.0));
    }

    /// The drawn curve is what the pointer aims at, so the hit test has to
    /// follow the eased shape rather than a straight line between the points.
    #[test]
    fn a_segment_is_grabbed_along_the_curve_it_draws() {
        let mut points = curve();
        points[0].curve = CurveKind::Linear { tension: 3.0 };
        let geom = geom();
        let at_2s = geom.axis.x(2.0);

        let mut cursor = 0;
        let drawn = crate::modulation::evaluate_envelope(&points, 2.0, &mut cursor);
        let straight = 0.5;
        assert!(
            (drawn - straight).abs() > 0.2,
            "the fixture needs a segment that bulges well off the straight line"
        );

        assert_eq!(
            bendable_segment_at(&points, geom, egui::pos2(at_2s, point_y(geom.track, drawn))),
            Some(0)
        );
        assert_eq!(
            bendable_segment_at(
                &points,
                geom,
                egui::pos2(at_2s, point_y(geom.track, straight))
            ),
            None,
            "the straight line between the points is not where the curve is"
        );
    }

    /// Bending a flat segment cannot show, so the gesture declines rather than
    /// starting a drag that does nothing.
    #[test]
    fn a_flat_segment_refuses_to_bend() {
        let flat = vec![linear(0.0, 0.5), linear(4.0, 0.5)];
        let geom = geom();
        let on_the_line = egui::pos2(geom.axis.x(2.0), point_y(geom.track, 0.5));

        assert_eq!(bendable_segment_at(&flat, geom, on_the_line), None);
    }

    /// Outside the drawn range the envelope holds its end values, so there is
    /// no segment to bend even though there is a line to point at.
    #[test]
    fn the_held_tails_outside_the_curve_are_not_segments() {
        let points = curve();
        let geom = geom();

        let before = egui::pos2(geom.axis.x(-2.0), point_y(geom.track, 0.0));
        let after = egui::pos2(geom.axis.x(12.0), point_y(geom.track, 0.25));
        assert_eq!(bendable_segment_at(&points, geom, before), None);
        assert_eq!(bendable_segment_at(&points, geom, after), None);
    }

    /// The other half of the bend gesture: where bending is dead, the line is
    /// dragged bodily instead, and both breakpoints holding it move together.
    #[test]
    fn a_flat_segment_is_grabbed_as_a_whole_line() {
        let flat = vec![linear(0.0, 0.5), linear(4.0, 0.5), linear(8.0, 1.0)];
        let geom = geom();
        let on_the_line = egui::pos2(geom.axis.x(2.0), point_y(geom.track, 0.5));

        assert_eq!(flat_run_at(&flat, geom, on_the_line), Some((0, 1)));
    }

    /// What reads as one flat line has to move as one, however many breakpoints
    /// happen to sit along it: breaking it into a ramp at a point that was never
    /// visible is not what the hand asked for.
    #[test]
    fn a_flat_run_widens_across_every_point_at_the_same_value() {
        let points = vec![
            linear(0.0, 0.2),
            linear(4.0, 0.6),
            linear(8.0, 0.6),
            linear(12.0, 0.6),
            linear(16.0, 0.9),
        ];
        let geom = geom();

        for seconds in [6.0, 10.0] {
            let on_the_line = egui::pos2(geom.axis.x(seconds), point_y(geom.track, 0.6));
            assert_eq!(
                flat_run_at(&points, geom, on_the_line),
                Some((1, 3)),
                "at {seconds}s"
            );
        }
    }

    /// Outside the drawn range the envelope holds its end value, which is a flat
    /// line like any other and the only way to raise a one-point curve.
    #[test]
    fn the_held_tails_are_flat_lines_that_can_be_dragged() {
        let points = curve();
        let geom = geom();

        let before = egui::pos2(geom.axis.x(-2.0), point_y(geom.track, 0.0));
        let after = egui::pos2(geom.axis.x(12.0), point_y(geom.track, 0.25));
        assert_eq!(flat_run_at(&points, geom, before), Some((0, 0)));
        assert_eq!(flat_run_at(&points, geom, after), Some((2, 2)));

        let lone = vec![linear(4.0, 0.75)];
        let anywhere = egui::pos2(geom.axis.x(30.0), point_y(geom.track, 0.75));
        assert_eq!(flat_run_at(&lone, geom, anywhere), Some((0, 0)));
        assert_eq!(flat_run_at(&[], geom, anywhere), None);
    }

    /// The two line gestures divide the track between them: a press that bends
    /// must never also level, or the drag that starts depends on which branch
    /// was written first.
    #[test]
    fn bending_and_levelling_never_claim_the_same_press() {
        let mut points = curve();
        points.push(linear(12.0, 0.25));
        let geom = geom();

        let mut cursor = 0;
        for step in 0..120 {
            let seconds = -2.0 + f64::from(step) * 0.125;
            let value = crate::modulation::evaluate_envelope(&points, seconds, &mut cursor);
            let on_the_line = egui::pos2(geom.axis.x(seconds), point_y(geom.track, value));

            let bend = bendable_segment_at(&points, geom, on_the_line).is_some();
            let level = flat_run_at(&points, geom, on_the_line).is_some();
            assert!(!(bend && level), "both claimed the press at {seconds}s");
            assert!(bend || level, "neither claimed the press at {seconds}s");
        }
    }

    /// A line is only grabbed where it is drawn, so a press in the empty part of
    /// the lane still falls through to whatever else wants it.
    #[test]
    fn a_press_away_from_the_line_grabs_nothing() {
        let flat = vec![linear(0.0, 0.5), linear(4.0, 0.5)];
        let geom = geom();
        let above = egui::pos2(
            geom.axis.x(2.0),
            point_y(geom.track, 0.5) - 3.0 * CURVE_GRAB,
        );

        assert_eq!(flat_run_at(&flat, geom, above), None);
    }

    #[test]
    fn levelling_moves_the_whole_run_and_nothing_else() {
        let points = vec![linear(0.0, 0.2), linear(4.0, 0.6), linear(8.0, 0.6)];
        let raised = with_run_levelled(&points, 1, 2, 0.8);

        assert!((raised[0].value - 0.2).abs() < f32::EPSILON, "untouched");
        assert!((raised[1].value - 0.8).abs() < f32::EPSILON);
        assert!((raised[2].value - 0.8).abs() < f32::EPSILON);
        assert!(
            raised
                .iter()
                .zip(&points)
                .all(|(a, b)| (a.position - b.position).abs() < 1e-9),
            "levelling is vertical only"
        );

        let over = with_run_levelled(&points, 0, 2, 4.0);
        assert!(over.iter().all(|p| (p.value - 1.0).abs() < f32::EPSILON));
        let under = with_run_levelled(&points, 0, 2, -4.0);
        assert!(under.iter().all(|p| p.value.abs() < f32::EPSILON));
    }

    /// The drag is relative to where it was pressed and unclamped on the way, so
    /// dragging out of the lane and back returns the line to where it was rather
    /// than leaving it pinned at the edge it hit.
    #[test]
    fn a_level_drag_out_of_the_lane_and_back_returns_to_its_value() {
        let track = geom().track;
        let usable = track.height() - 2.0 * PADDING;

        let up = value_travel(track, usable / 2.0);
        assert!((up - 0.5).abs() < 1e-6, "half the lane is half the range");
        assert!(
            (value_travel(track, -usable) + 1.0).abs() < 1e-6,
            "down is negative and unclamped"
        );

        let origin = 0.5_f32;
        let miles_up = (origin + value_travel(track, 10.0 * usable)).clamp(0.0, 1.0);
        assert!((miles_up - 1.0).abs() < f32::EPSILON);
        let back = origin + value_travel(track, 0.0);
        assert!((back - origin).abs() < f32::EPSILON);
    }

    #[test]
    fn a_point_moves_in_both_axes() {
        let moved = with_point_moved(&curve(), 1, 5.0, 0.5);
        assert!((moved[1].position - 5.0).abs() < 1e-9);
        assert!((moved[1].value - 0.5).abs() < f32::EPSILON);
    }

    /// A dragged point stops at its neighbours rather than swapping past them,
    /// so the list stays sorted and the gesture keeps hold of the same point.
    #[test]
    fn a_point_cannot_be_dragged_past_its_neighbours() {
        let past_right = with_point_moved(&curve(), 1, 100.0, 0.5);
        assert!(past_right[1].position < past_right[2].position);

        let past_left = with_point_moved(&curve(), 1, -100.0, 0.5);
        assert!(past_left[1].position > past_left[0].position);
        assert!(
            past_left.windows(2).all(|w| w[0].position < w[1].position),
            "the list must stay sorted"
        );
    }

    #[test]
    fn a_value_is_clamped_to_the_lane() {
        let high = with_point_moved(&curve(), 0, 0.0, 4.0);
        assert!((high[0].value - 1.0).abs() < f32::EPSILON);
        let low = with_point_moved(&curve(), 0, 0.0, -4.0);
        assert!(low[0].value.abs() < f32::EPSILON);
    }

    #[test]
    fn a_new_point_lands_in_order_and_inherits_the_segment_shape() {
        let mut shaped = curve();
        shaped[0].curve = CurveKind::Step;

        let added = with_point_added(&shaped, 2.0, 0.75);
        assert_eq!(added.len(), 4);
        assert!((added[1].position - 2.0).abs() < 1e-9);
        assert!(matches!(added[1].curve, CurveKind::Step));
        assert!(added.windows(2).all(|w| w[0].position <= w[1].position));
    }

    #[test]
    fn a_point_added_before_everything_still_sorts_first() {
        let added = with_point_added(&curve(), -1.0, 0.5);
        assert!((added[0].position + 1.0).abs() < 1e-9);
        assert!(added.windows(2).all(|w| w[0].position <= w[1].position));
    }

    #[test]
    fn deleting_a_point_leaves_the_rest_alone() {
        let cut = without_point(&curve(), 1);
        assert_eq!(cut.len(), 2);
        assert!((cut[1].position - 8.0).abs() < 1e-9);
        // An index past the end is a stale click, not a panic.
        assert_eq!(without_point(&curve(), 9).len(), 3);
    }

    #[test]
    fn a_segment_can_be_reshaped_without_moving_it() {
        let held = with_curve(&curve(), 0, CurveKind::Step);
        assert!(matches!(held[0].curve, CurveKind::Step));
        assert!((held[0].position - curve()[0].position).abs() < 1e-9);
    }

    /// Paste lands at the playhead and clears what it covers, rather than
    /// leaving two curves fighting over the same span.
    #[test]
    fn pasting_replaces_the_span_it_covers() {
        let target = vec![linear(0.0, 0.0), linear(11.0, 0.5), linear(30.0, 1.0)];
        let out = pasted(&target, &curve(), 10.0);

        assert!(out.windows(2).all(|w| w[0].position <= w[1].position));
        assert!(
            !out.iter().any(|p| (p.position - 11.0).abs() < 1e-9),
            "a point inside the pasted span must not survive"
        );
        assert!(out.iter().any(|p| (p.position - 10.0).abs() < 1e-9));
        assert!(out.iter().any(|p| (p.position - 18.0).abs() < 1e-9));
        assert!(
            out.iter().any(|p| (p.position - 30.0).abs() < 1e-9),
            "points outside the pasted span are untouched"
        );
    }

    #[test]
    fn pasting_nothing_changes_nothing() {
        assert_eq!(pasted(&curve(), &[], 5.0), curve());
    }

    /// Screen y and normalized value must round-trip, or a point jumps the
    /// moment it is grabbed.
    #[test]
    fn value_and_pixel_round_trip() {
        let track = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(600.0, 46.0));
        for value in [0.0_f32, 0.25, 0.5, 1.0] {
            let back = value_at(track, point_y(track, value));
            assert!((back - value).abs() < 0.01, "{value} came back as {back}");
        }
    }
}
