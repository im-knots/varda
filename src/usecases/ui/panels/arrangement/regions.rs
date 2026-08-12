//! Region drawing and editing.
//!
//! A region is a span during which a deck is visible, and every edit here is an
//! `EngineCommand` against the lane that owns it. The panel never touches the
//! compiled opacity envelope: that is the engine's business, and it recompiles
//! from the regions on every write. See /spec/arrangement.md § Regions.
//!
//! Drag state lives in egui memory rather than in `UIData`, because a drag is
//! between two frames of the same gesture and has no business round-tripping
//! through the engine snapshot. What *is* pushed every frame is the resulting
//! region, so the drag is visible in the output while it is happening.

use super::super::super::{UIActions, UIData};
use super::super::utils::channel_color;
use super::{min_span, snap_seconds, LaneRow, RowGeometry, TimeAxis};
use crate::arrangement::RegionConfig;
use crate::engine::EngineCommand;

/// Grab zone for a region edge, in pixels. Applies on both sides of the edge:
/// aiming at a one-pixel line and landing just outside it is the normal way to
/// miss, and the thing just outside a region's edge is empty track, which would
/// otherwise start authoring a new region on top of the one being edited.
const EDGE_GRAB: f32 = 5.0;
/// Grab zone for a fade handle, in pixels.
const FADE_GRAB: f32 = 6.0;
/// Length of a region created by a double click rather than by a drag.
const DEFAULT_REGION_SECONDS: f64 = 4.0;

/// Which part of a region a drag has hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragKind {
    Move,
    ResizeStart,
    ResizeEnd,
    FadeIn,
    FadeOut,
}

/// A held region edit, carried across the frames of one gesture.
#[derive(Clone)]
struct RegionDrag {
    kind: DragKind,
    origin: RegionConfig,
    /// Show position the pointer was over when the drag started, so the edit is
    /// computed from an absolute offset rather than from accumulated deltas.
    grab: f64,
}

pub(super) fn render_lane_track(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    lane: &LaneRow<'_>,
) {
    // The background is registered first so a region drawn on top of it takes
    // the pointer: egui gives the press to the last widget registered.
    handle_create(ui, data, actions, geom, lane);

    let clip = ui.painter().with_clip_rect(geom.track);
    for (index, region) in lane.regions.iter().enumerate() {
        draw_region(&clip, ui.visuals(), region, geom, lane.ch_idx);
        handle_region_edit(ui, data, actions, geom, lane, index, region);
    }
}

/// Drag across empty track to author a span; click to drop a default one.
fn handle_create(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    lane: &LaneRow<'_>,
) {
    let id = ui.id().with(("arrangement_track", lane.uuid));
    let response = ui.interact(geom.track, id, egui::Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} timeline track", lane.name),
        )
    });

    let anchor_id = id.with("anchor");
    if response.drag_started() {
        if let Some(pos) = press_origin(&response) {
            let anchor = snap_seconds(data, geom.axis.seconds(pos.x));
            ui.ctx()
                .memory_mut(|mem| mem.data.insert_temp(anchor_id, anchor));
        }
    }

    let anchor: Option<f64> = ui.ctx().memory(|mem| mem.data.get_temp(anchor_id));
    if let (Some(anchor), Some(pos)) = (anchor, response.interact_pointer_pos()) {
        let cursor = snap_seconds(data, geom.axis.seconds(pos.x));
        let ghost = RegionConfig {
            start: anchor.min(cursor),
            end: anchor.max(cursor),
            fade_in: 0.0,
            fade_out: 0.0,
        };
        if response.dragged() {
            // One undo entry for the whole gesture, not one per frame.
            actions.session.gesture_active = true;
            draw_ghost(ui, geom, lane.ch_idx, &ghost);
        }
        if response.drag_stopped() {
            ui.ctx().memory_mut(|mem| mem.data.remove::<f64>(anchor_id));
            if ghost.span() >= min_span(data) {
                actions.commands.push(EngineCommand::AddRegion {
                    deck_uuid: lane.uuid.to_string(),
                    region: ghost,
                });
            }
        }
    }

    if response.clicked() {
        actions.session.select_deck = Some((lane.ch_idx, lane.deck_idx));
    }

    // Creating on a single click would put a region under every attempt to
    // select a lane, so the bare click selects and the double click authors.
    if response.double_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let start = snap_seconds(data, geom.axis.seconds(pos.x));
            actions.commands.push(EngineCommand::AddRegion {
                deck_uuid: lane.uuid.to_string(),
                region: RegionConfig::new(start, start + DEFAULT_REGION_SECONDS),
            });
        }
    }
}

/// Move, resize, and fade one region.
fn handle_region_edit(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    lane: &LaneRow<'_>,
    index: usize,
    region: &RegionConfig,
) {
    let body = body_rect(region, geom);
    if body.right() < geom.track.left() || body.left() > geom.track.right() {
        return;
    }

    // Registered over the grab rect rather than the body, so a press aimed at an
    // edge and landing just outside it belongs to this region instead of to the
    // empty track behind it.
    let id = ui.id().with(("arrangement_region", lane.uuid, index));
    let response = ui.interact(
        grab_rect(body, index, lane.regions, geom),
        id,
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} region {}", lane.name, index + 1),
        )
    });

    if response.hovered() {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            ui.ctx()
                .set_cursor_icon(cursor_for(hit_kind(region, geom.axis, pos, body)));
        }
    }

    if response.drag_started() {
        if let Some(pos) = press_origin(&response) {
            let drag = RegionDrag {
                kind: hit_kind(region, geom.axis, pos, body),
                origin: *region,
                grab: geom.axis.seconds(pos.x),
            };
            ui.ctx().memory_mut(|mem| mem.data.insert_temp(id, drag));
        }
    }

    if response.dragged() {
        let drag: Option<RegionDrag> = ui.ctx().memory(|mem| mem.data.get_temp(id));
        if let (Some(drag), Some(pos)) = (drag, response.interact_pointer_pos()) {
            actions.session.gesture_active = true;
            let delta = geom.axis.seconds(pos.x) - drag.grab;
            let edited = apply_drag(
                drag.kind,
                &drag.origin,
                delta,
                |s| snap_seconds(data, s),
                min_span(data),
            );
            if edited != *region {
                actions.commands.push(EngineCommand::UpdateRegion {
                    deck_uuid: lane.uuid.to_string(),
                    index,
                    region: edited,
                });
            }
        }
    }

    if response.drag_stopped() {
        ui.ctx().memory_mut(|mem| mem.data.remove::<RegionDrag>(id));
    }

    if response.clicked() {
        actions.session.select_deck = Some((lane.ch_idx, lane.deck_idx));
    }

    response.context_menu(|ui| {
        if ui.button("Delete region").clicked() {
            actions.commands.push(EngineCommand::RemoveRegion {
                deck_uuid: lane.uuid.to_string(),
                index,
            });
            ui.close();
        }
        if ui.button("Clear fades").clicked() {
            actions.commands.push(EngineCommand::UpdateRegion {
                deck_uuid: lane.uuid.to_string(),
                index,
                region: RegionConfig {
                    fade_in: 0.0,
                    fade_out: 0.0,
                    ..*region
                },
            });
            ui.close();
        }
    });
}

/// Where the pointer went down, rather than where it has reached.
///
/// A drag is only recognised once the pointer has travelled a few pixels, so by
/// the time `drag_started` fires the pointer has already left the handle it was
/// aimed at. Grabbing by the press origin is what makes a five-pixel edge zone
/// hittable at all.
pub(super) fn press_origin(response: &egui::Response) -> Option<egui::Pos2> {
    response
        .ctx
        .input(|i| i.pointer.press_origin())
        .or_else(|| response.interact_pointer_pos())
}

/// The rect this region takes presses on: its body, plus the outside half of
/// each edge's grab zone.
///
/// The expansion stops at the midpoint of the gap to a neighbour, so two regions
/// a few pixels apart split the space between them instead of one swallowing the
/// other's edge. Regions that touch or overlap therefore claim only their own
/// body, which keeps a shared boundary predictable.
fn grab_rect(
    body: egui::Rect,
    index: usize,
    siblings: &[RegionConfig],
    geom: RowGeometry,
) -> egui::Rect {
    // Half of whatever daylight there is, capped at the full grab zone.
    let share = |gap_px: f32| EDGE_GRAB.min((gap_px / 2.0).max(0.0));

    let before = index.checked_sub(1).and_then(|i| siblings.get(i));
    let left = before.map_or(EDGE_GRAB, |n| share(body.left() - geom.axis.x(n.end)));

    let after = siblings.get(index + 1);
    let right = after.map_or(EDGE_GRAB, |n| share(geom.axis.x(n.start) - body.right()));
    egui::Rect::from_min_max(
        egui::pos2(body.left() - left, body.top()),
        egui::pos2(body.right() + right, body.bottom()),
    )
}

/// Which handle a press at `pointer` has hold of.
///
/// Fade handles sit in the top half so that the far more common move and resize
/// gestures are not shadowed by them along the whole height of the region, and
/// only inside the body: a press past an edge is someone reaching for that edge,
/// and on a region with no fade the handle sits exactly on top of it.
fn hit_kind(
    region: &RegionConfig,
    axis: TimeAxis,
    pointer: egui::Pos2,
    body: egui::Rect,
) -> DragKind {
    // Inside a narrow region the two edge zones would meet and leave nowhere to
    // grab for a move, so they never take more than a third of the body each.
    let inner = EDGE_GRAB.min(body.width() / 3.0);

    if pointer.y < body.center().y && body.x_range().contains(pointer.x) {
        let (fade_in, fade_out) = region.clamped_fades();
        if (pointer.x - axis.x(region.start + fade_in)).abs() <= FADE_GRAB {
            return DragKind::FadeIn;
        }
        if (pointer.x - axis.x(region.end - fade_out)).abs() <= FADE_GRAB {
            return DragKind::FadeOut;
        }
    }
    if pointer.x <= body.left() + inner {
        return DragKind::ResizeStart;
    }
    if pointer.x >= body.right() - inner {
        return DragKind::ResizeEnd;
    }
    DragKind::Move
}

fn cursor_for(kind: DragKind) -> egui::CursorIcon {
    match kind {
        DragKind::Move => egui::CursorIcon::Grab,
        DragKind::ResizeStart | DragKind::ResizeEnd => egui::CursorIcon::ResizeHorizontal,
        DragKind::FadeIn | DragKind::FadeOut => egui::CursorIcon::ResizeColumn,
    }
}

/// The region a gesture has produced, given how far the pointer has travelled.
///
/// Pure so the arithmetic can be tested without a pointer: the drag itself is
/// only the source of `delta`.
fn apply_drag(
    kind: DragKind,
    origin: &RegionConfig,
    delta: f64,
    snap: impl Fn(f64) -> f64,
    min_span: f64,
) -> RegionConfig {
    let span = origin.span();
    match kind {
        DragKind::Move => {
            let start = snap(origin.start + delta);
            RegionConfig {
                start,
                end: start + span,
                ..*origin
            }
        }
        DragKind::ResizeStart => {
            let start = snap(origin.start + delta).min(origin.end - min_span);
            RegionConfig {
                start: start.max(0.0),
                ..*origin
            }
        }
        DragKind::ResizeEnd => RegionConfig {
            end: snap(origin.end + delta).max(origin.start + min_span),
            ..*origin
        },
        DragKind::FadeIn => RegionConfig {
            fade_in: snap(origin.fade_in + delta).clamp(0.0, span),
            ..*origin
        },
        DragKind::FadeOut => RegionConfig {
            fade_out: snap(origin.fade_out - delta).clamp(0.0, span),
            ..*origin
        },
    }
}

fn body_rect(region: &RegionConfig, geom: RowGeometry) -> egui::Rect {
    let left = geom.axis.x(region.start);
    let right = geom.axis.x(region.end).max(left + 1.0);
    egui::Rect::from_min_max(
        egui::pos2(left, geom.track.top() + 3.0),
        egui::pos2(right, geom.track.bottom() - 3.0),
    )
}

/// A region drawn as its opacity envelope: the fades are the shape, not a
/// decoration on it.
fn draw_region(
    painter: &egui::Painter,
    visuals: &egui::Visuals,
    region: &RegionConfig,
    geom: RowGeometry,
    ch_idx: usize,
) {
    if !region.is_valid() {
        return;
    }
    let body = body_rect(region, geom);
    if body.right() < geom.track.left() || body.left() > geom.track.right() {
        return;
    }

    let color = channel_color(ch_idx);
    painter.rect_filled(body, 2.0, color.gamma_multiply(0.6));
    painter.rect_stroke(
        body,
        2.0,
        egui::Stroke::new(1.0_f32, color),
        egui::StrokeKind::Inside,
    );

    let (fade_in, fade_out) = region.clamped_fades();
    let shade = visuals.extreme_bg_color.gamma_multiply(0.6);
    if fade_in > 0.0 {
        painter.add(egui::Shape::convex_polygon(
            vec![
                body.left_top(),
                egui::pos2(geom.axis.x(region.start + fade_in), body.top()),
                body.left_bottom(),
            ],
            shade,
            egui::Stroke::NONE,
        ));
    }
    if fade_out > 0.0 {
        painter.add(egui::Shape::convex_polygon(
            vec![
                body.right_top(),
                egui::pos2(geom.axis.x(region.end - fade_out), body.top()),
                body.right_bottom(),
            ],
            shade,
            egui::Stroke::NONE,
        ));
    }

    // Fade handles, drawn last so they read as grabbable rather than as part of
    // the shading.
    for x in [
        geom.axis.x(region.start + fade_in),
        geom.axis.x(region.end - fade_out),
    ] {
        painter.circle_filled(egui::pos2(x, body.top() + 3.0), 3.0, color);
    }
}

/// The span a create-drag would produce, shown while the pointer is down.
fn draw_ghost(ui: &egui::Ui, geom: RowGeometry, ch_idx: usize, ghost: &RegionConfig) {
    let color = channel_color(ch_idx);
    ui.painter_at(geom.track).rect_stroke(
        body_rect(ghost, geom),
        2.0,
        egui::Stroke::new(1.0_f32, color),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis() -> TimeAxis {
        TimeAxis {
            left: 0.0,
            scroll: 0.0,
            pps: 10.0,
        }
    }

    fn geom() -> RowGeometry {
        RowGeometry {
            header: egui::Rect::from_min_size(egui::pos2(-100.0, 0.0), egui::vec2(100.0, 32.0)),
            track: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 32.0)),
            axis: axis(),
            idx: 0,
        }
    }

    fn region() -> RegionConfig {
        RegionConfig {
            start: 10.0,
            end: 20.0,
            fade_in: 1.0,
            fade_out: 2.0,
        }
    }

    fn no_snap(s: f64) -> f64 {
        s.max(0.0)
    }

    #[test]
    fn dragging_the_body_moves_without_resizing() {
        let moved = apply_drag(DragKind::Move, &region(), 5.0, no_snap, 0.04);
        assert!((moved.start - 15.0).abs() < 1e-9);
        assert!((moved.span() - region().span()).abs() < 1e-9);
        assert!((moved.fade_in - 1.0).abs() < 1e-9, "fades ride along");
    }

    #[test]
    fn dragging_an_edge_resizes_only_that_edge() {
        let start = apply_drag(DragKind::ResizeStart, &region(), -4.0, no_snap, 0.04);
        assert!((start.start - 6.0).abs() < 1e-9);
        assert!((start.end - 20.0).abs() < 1e-9);

        let end = apply_drag(DragKind::ResizeEnd, &region(), 3.0, no_snap, 0.04);
        assert!((end.start - 10.0).abs() < 1e-9);
        assert!((end.end - 23.0).abs() < 1e-9);
    }

    /// A resize that overshoots must not invert the region, which would make it
    /// invalid and be rejected by the engine mid-gesture.
    #[test]
    fn a_resize_cannot_turn_a_region_inside_out() {
        let collapsed = apply_drag(DragKind::ResizeStart, &region(), 100.0, no_snap, 0.04);
        assert!(collapsed.is_valid());
        assert!((collapsed.span() - 0.04).abs() < 1e-9);

        let backwards = apply_drag(DragKind::ResizeEnd, &region(), -100.0, no_snap, 0.04);
        assert!(backwards.is_valid());
        assert!((backwards.span() - 0.04).abs() < 1e-9);
    }

    /// A region cannot start before the show does.
    #[test]
    fn a_region_cannot_be_dragged_before_zero() {
        let moved = apply_drag(DragKind::Move, &region(), -100.0, no_snap, 0.04);
        assert!(moved.start >= 0.0);
        let resized = apply_drag(DragKind::ResizeStart, &region(), -100.0, no_snap, 0.04);
        assert!(resized.start >= 0.0);
    }

    #[test]
    fn fade_handles_grow_inward_and_stop_at_the_far_edge() {
        let faded = apply_drag(DragKind::FadeIn, &region(), 2.0, no_snap, 0.04);
        assert!((faded.fade_in - 3.0).abs() < 1e-9);
        assert!((faded.start - 10.0).abs() < 1e-9, "the span does not move");

        let clamped = apply_drag(DragKind::FadeIn, &region(), 100.0, no_snap, 0.04);
        assert!((clamped.fade_in - region().span()).abs() < 1e-9);

        // The out handle is grabbed from the right, so a rightward drag shortens
        // the fade rather than lengthening it.
        let out = apply_drag(DragKind::FadeOut, &region(), 1.0, no_snap, 0.04);
        assert!((out.fade_out - 1.0).abs() < 1e-9);
    }

    #[test]
    fn snapping_applies_to_the_edit_not_to_the_stored_span() {
        let snap = |s: f64| (s * 25.0).round() / 25.0;
        let moved = apply_drag(DragKind::Move, &region(), 0.031, snap, 0.04);
        assert!((moved.start - 10.04).abs() < 1e-9);
        assert!(
            (moved.span() - region().span()).abs() < 1e-9,
            "snapping the start must not stretch the region"
        );
    }

    /// The failure this exists to prevent: aiming at an edge, landing a pixel
    /// past it, and authoring a second region on top of the one being resized.
    #[test]
    fn an_edge_claims_the_track_just_outside_it() {
        let region = region();
        let body = body_rect(&region, geom());
        let alone = grab_rect(body, 0, std::slice::from_ref(&region), geom());

        assert!((alone.left() - (body.left() - EDGE_GRAB)).abs() < 1e-6);
        assert!((alone.right() - (body.right() + EDGE_GRAB)).abs() < 1e-6);
        assert!(
            (alone.top() - body.top()).abs() < 1e-6
                && (alone.bottom() - body.bottom()).abs() < 1e-6,
            "the claim is horizontal only: vertically a region is already generous"
        );

        assert_eq!(
            hit_kind(&region, axis(), at(9.7, body.center().y + 1.0), body),
            DragKind::ResizeStart,
            "a press just left of the start edge is reaching for that edge"
        );
        assert_eq!(
            hit_kind(&region, axis(), at(20.3, body.center().y + 1.0), body),
            DragKind::ResizeEnd
        );
    }

    /// Two regions a few pixels apart split the gap rather than one swallowing
    /// the other's edge, so which one a press near the boundary edits is
    /// predictable from where it landed.
    #[test]
    fn neighbours_split_the_gap_between_them() {
        let siblings = vec![
            RegionConfig::new(10.0, 20.0),
            // 0.4 s later, which is 4 px at this zoom: narrower than two grab
            // zones, so both have to give way.
            RegionConfig::new(20.4, 30.0),
        ];
        let first = grab_rect(body_rect(&siblings[0], geom()), 0, &siblings, geom());
        let second = grab_rect(body_rect(&siblings[1], geom()), 1, &siblings, geom());

        assert!(
            first.right() <= second.left() + 1e-6,
            "{first:?} and {second:?} must not overlap"
        );
        assert!(
            (first.right() - axis().x(20.2)).abs() < 1e-6,
            "each takes half the gap"
        );
    }

    /// Touching regions claim nothing outside themselves, so the shared boundary
    /// belongs to whichever body the pointer is actually inside.
    #[test]
    fn touching_regions_do_not_reach_past_each_other() {
        let siblings = vec![RegionConfig::new(10.0, 20.0), RegionConfig::new(20.0, 30.0)];
        let first = grab_rect(body_rect(&siblings[0], geom()), 0, &siblings, geom());
        let second = grab_rect(body_rect(&siblings[1], geom()), 1, &siblings, geom());

        assert!((first.right() - axis().x(20.0)).abs() < 1e-6);
        assert!((second.left() - axis().x(20.0)).abs() < 1e-6);
    }

    /// Zoomed out, a region can be narrower than two grab zones. Both edges must
    /// stay reachable and there must still be somewhere to grab for a move.
    #[test]
    fn a_narrow_region_keeps_all_three_gestures() {
        // 0.9 s at 10 px/s: 9 px wide, against a 5 px edge zone.
        let narrow = RegionConfig::new(10.0, 10.9);
        let body = body_rect(&narrow, geom());
        let y = body.center().y + 1.0;

        let kinds: Vec<DragKind> = [0.1, 0.45, 0.8]
            .iter()
            .map(|t| hit_kind(&narrow, axis(), at(10.0 + t, y), body))
            .collect();
        assert_eq!(
            kinds,
            vec![DragKind::ResizeStart, DragKind::Move, DragKind::ResizeEnd]
        );
    }

    fn at(seconds: f64, y: f32) -> egui::Pos2 {
        egui::pos2(axis().x(seconds), y)
    }

    #[test]
    fn edges_and_fade_handles_claim_their_own_grab_zones() {
        let region = region();
        let body = body_rect(&region, geom());

        let top = body.top() + 1.0;
        let middle = body.center().y + 1.0;

        assert_eq!(
            hit_kind(&region, axis(), at(10.0, middle), body),
            DragKind::ResizeStart
        );
        assert_eq!(
            hit_kind(&region, axis(), at(20.0, middle), body),
            DragKind::ResizeEnd
        );
        assert_eq!(
            hit_kind(&region, axis(), at(15.0, middle), body),
            DragKind::Move
        );
        assert_eq!(
            hit_kind(&region, axis(), at(11.0, top), body),
            DragKind::FadeIn
        );
        assert_eq!(
            hit_kind(&region, axis(), at(18.0, top), body),
            DragKind::FadeOut
        );
        // Below the midline the fade handle gives way to the move gesture, which
        // is the one a performer reaches for far more often.
        assert_eq!(
            hit_kind(&region, axis(), at(11.0, middle), body),
            DragKind::Move
        );
    }

    /// A fade handle sits exactly on the edge when its fade is zero, so letting
    /// it claim the track outside the region would make every near miss a drag
    /// that clamps at zero and appears to do nothing.
    #[test]
    fn a_fade_handle_does_not_claim_track_outside_the_region() {
        let region = RegionConfig::new(10.0, 20.0);
        let body = body_rect(&region, geom());
        let top = body.top() + 1.0;

        assert_eq!(
            hit_kind(&region, axis(), at(9.7, top), body),
            DragKind::ResizeStart
        );
        assert_eq!(
            hit_kind(&region, axis(), at(20.3, top), body),
            DragKind::ResizeEnd
        );
        // Inside, the handle still wins the top half, which is how a fade is
        // pulled out of an edge that has none yet.
        assert_eq!(
            hit_kind(&region, axis(), at(10.2, top), body),
            DragKind::FadeIn
        );
    }
}
