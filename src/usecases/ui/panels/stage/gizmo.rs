//! Transform gizmo: selection bounds, scale/rotate handles, and drag initiation.

use super::super::super::SurfaceUI;
use super::state::StageEditorState;

/// Pixel margin between the content bounding box and the transform gizmo box.
/// Kept wide enough that the corner scale handles clear the surface's own
/// corner vertices, so vertex editing and gizmo scaling don't fight over the
/// same clicks.
pub(super) const GIZMO_MARGIN_PX: f32 = 20.0;
/// Pixel offset of the rotation knob above the gizmo box's top edge.
pub(super) const GIZMO_ROTATE_OFFSET_PX: f32 = 28.0;
/// Hit-test radius (pixels) for gizmo scale/rotate handles.
pub(super) const GIZMO_HANDLE_HIT_PX: f32 = 14.0;

/// Active scale-drag on the transform gizmo. `last_sx`/`last_sy` track the total
/// scale so far so each frame can emit only the incremental delta.
#[derive(Debug, Clone, Copy)]
pub(super) struct ScaleDrag {
    pub(super) pivot: [f32; 2],
    pub(super) start_handle: [f32; 2],
    pub(super) last_sx: f32,
    pub(super) last_sy: f32,
    pub(super) axis_x: bool,
    pub(super) axis_y: bool,
}

/// Active rotate-drag on the transform gizmo. `last_angle` tracks the previous
/// frame's pointer angle so each frame emits only the incremental delta.
#[derive(Debug, Clone, Copy)]
pub(super) struct RotateDrag {
    pub(super) center: [f32; 2],
    pub(super) last_angle: f32,
}

/// Union bounding box `(x, y, w, h)` in normalized coords of the selected
/// surfaces, or `None` when the selection is empty.
pub(super) fn selection_bounds(
    surfaces: &[SurfaceUI],
    selected: &std::collections::BTreeSet<String>,
) -> Option<(f32, f32, f32, f32)> {
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    let mut any = false;
    for s in surfaces.iter().filter(|s| selected.contains(&s.uuid)) {
        for v in s.vertices.iter().chain(s.extra_contours.iter().flatten()) {
            min_x = min_x.min(v[0]);
            min_y = min_y.min(v[1]);
            max_x = max_x.max(v[0]);
            max_y = max_y.max(v[1]);
            any = true;
        }
    }
    if any {
        Some((min_x, min_y, max_x - min_x, max_y - min_y))
    } else {
        None
    }
}

/// The gizmo's eight scale handles as `(handle, pivot, scales_x, scales_y)` in
/// normalized coords for the given box. The pivot is always the opposite handle.
// x/y/w/h and l/t/r/b are the idiomatic names for this box geometry.
#[allow(clippy::many_single_char_names)]
pub(super) fn gizmo_scale_handles(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> [([f32; 2], [f32; 2], bool, bool); 8] {
    let (l, t, r, b) = (x, y, x + w, y + h);
    let (mx, my) = (x + w * 0.5, y + h * 0.5);
    [
        ([l, t], [r, b], true, true),    // top-left ↔ bottom-right
        ([r, t], [l, b], true, true),    // top-right ↔ bottom-left
        ([r, b], [l, t], true, true),    // bottom-right ↔ top-left
        ([l, b], [r, t], true, true),    // bottom-left ↔ top-right
        ([mx, t], [mx, b], false, true), // top ↔ bottom
        ([mx, b], [mx, t], false, true), // bottom ↔ top
        ([r, my], [l, my], true, false), // right ↔ left
        ([l, my], [r, my], true, false), // left ↔ right
    ]
}

/// If the pointer began on a transform-gizmo handle for the current selection,
/// start the matching scale/rotate drag and return `true`, clearing any other
/// drag state. Returns `false` when no gizmo handle was hit.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_begin_gizmo_drag(
    state: &mut StageEditorState,
    surfaces: &[SurfaceUI],
    pos: egui::Pos2,
    nx: f32,
    ny: f32,
    canvas_rect: egui::Rect,
    canvas_width: f32,
    canvas_height: f32,
) -> bool {
    let Some((bx, by, bw, bh)) = selection_bounds(surfaces, &state.selected_surfaces) else {
        return false;
    };
    let mx = GIZMO_MARGIN_PX / canvas_width;
    let my = GIZMO_MARGIN_PX / canvas_height;
    let (gx, gy, gw, gh) = (bx - mx, by - my, bw + 2.0 * mx, bh + 2.0 * my);
    let to_px = |p: [f32; 2]| {
        egui::pos2(
            canvas_rect.left() + p[0] * canvas_width,
            canvas_rect.top() + p[1] * canvas_height,
        )
    };
    let center = [gx + gw * 0.5, gy + gh * 0.5];

    // Rotation knob first (it sits outside the box, so it can't clash).
    let top_mid = to_px([gx + gw * 0.5, gy]);
    let knob = egui::pos2(top_mid.x, top_mid.y - GIZMO_ROTATE_OFFSET_PX);
    if pos.distance(knob) < GIZMO_HANDLE_HIT_PX {
        let angle = (ny - center[1]).atan2(nx - center[0]);
        clear_all_drag(state);
        state.dragging_rotate = Some(RotateDrag {
            center,
            last_angle: angle,
        });
        return true;
    }

    for (handle, pivot, axis_x, axis_y) in gizmo_scale_handles(gx, gy, gw, gh) {
        if pos.distance(to_px(handle)) < GIZMO_HANDLE_HIT_PX {
            clear_all_drag(state);
            state.dragging_scale = Some(ScaleDrag {
                pivot,
                start_handle: handle,
                last_sx: 1.0,
                last_sy: 1.0,
                axis_x,
                axis_y,
            });
            return true;
        }
    }
    false
}

/// Clear every drag-in-progress field on the stage editor state.
pub(super) fn clear_all_drag(state: &mut StageEditorState) {
    state.dragging_vertex = None;
    state.moving_surface = None;
    state.selection_rect_start = None;
    state.dragging_radius = None;
    state.dragging_edge = None;
    state.dragging_scale = None;
    state.dragging_rotate = None;
}

#[cfg(test)]
mod tests {
    use super::super::super::super::SurfaceUI;
    use super::super::state::DrawingTool;
    use super::*;

    fn selection(uuids: &[&str]) -> std::collections::BTreeSet<String> {
        uuids.iter().map(|u| (*u).to_string()).collect()
    }

    // ── selection_bounds ────────────────────────────────────────────

    #[test]
    fn selection_bounds_is_none_when_nothing_selected() {
        let surfaces = vec![SurfaceUI::test_quad("a", 0.1, 0.1, 0.2, 0.2)];
        assert!(selection_bounds(&surfaces, &selection(&[])).is_none());
    }

    #[test]
    fn selection_bounds_ignores_unselected_surfaces() {
        let surfaces = vec![
            SurfaceUI::test_quad("a", 0.1, 0.1, 0.2, 0.2),
            SurfaceUI::test_quad("b", 0.8, 0.8, 0.1, 0.1),
        ];
        let (x, y, w, h) = selection_bounds(&surfaces, &selection(&["a"])).expect("bounds");
        assert!((x - 0.1).abs() < 1e-6, "x = {x}");
        assert!((y - 0.1).abs() < 1e-6, "y = {y}");
        assert!((w - 0.2).abs() < 1e-6, "w = {w}");
        assert!((h - 0.2).abs() < 1e-6, "h = {h}");
    }

    #[test]
    fn selection_bounds_unions_multiple_surfaces() {
        let surfaces = vec![
            SurfaceUI::test_quad("a", 0.1, 0.1, 0.2, 0.2),
            SurfaceUI::test_quad("b", 0.6, 0.5, 0.2, 0.3),
        ];
        let (x, y, w, h) = selection_bounds(&surfaces, &selection(&["a", "b"])).expect("bounds");
        assert!((x - 0.1).abs() < 1e-6, "x = {x}");
        assert!((y - 0.1).abs() < 1e-6, "y = {y}");
        assert!((w - 0.7).abs() < 1e-6, "w = {w}"); // 0.1 → 0.8
        assert!((h - 0.7).abs() < 1e-6, "h = {h}"); // 0.1 → 0.8
    }

    /// Extra contours participate in the bounds, not just the primary outline.
    #[test]
    fn selection_bounds_includes_extra_contours() {
        let mut surface = SurfaceUI::test_quad("a", 0.4, 0.4, 0.1, 0.1);
        surface.extra_contours = vec![vec![[0.9, 0.9], [0.95, 0.95]]];
        let (x, y, w, h) = selection_bounds(&[surface], &selection(&["a"])).expect("bounds");
        assert!((x - 0.4).abs() < 1e-6, "x = {x}");
        assert!((y - 0.4).abs() < 1e-6, "y = {y}");
        assert!((w - 0.55).abs() < 1e-6, "w = {w}"); // 0.4 → 0.95
        assert!((h - 0.55).abs() < 1e-6, "h = {h}");
    }

    #[test]
    fn selection_bounds_is_none_for_surface_with_no_vertices() {
        let mut s = SurfaceUI::test_quad("a", 0.0, 0.0, 1.0, 1.0);
        s.vertices.clear();
        assert!(selection_bounds(&[s], &selection(&["a"])).is_none());
    }

    // ── gizmo_scale_handles ─────────────────────────────────────────

    /// The eight handles are the four corners then the four edge midpoints, and
    /// every pivot is the diagonally/axially opposite point.
    #[test]
    fn gizmo_scale_handles_layout_and_pivots() {
        let handles = gizmo_scale_handles(0.0, 0.0, 1.0, 1.0);
        let expected: [([f32; 2], [f32; 2], bool, bool); 8] = [
            ([0.0, 0.0], [1.0, 1.0], true, true),
            ([1.0, 0.0], [0.0, 1.0], true, true),
            ([1.0, 1.0], [0.0, 0.0], true, true),
            ([0.0, 1.0], [1.0, 0.0], true, true),
            ([0.5, 0.0], [0.5, 1.0], false, true),
            ([0.5, 1.0], [0.5, 0.0], false, true),
            ([1.0, 0.5], [0.0, 0.5], true, false),
            ([0.0, 0.5], [1.0, 0.5], true, false),
        ];
        for (i, (got, want)) in handles.iter().zip(expected.iter()).enumerate() {
            assert_eq!(got.0, want.0, "handle {i} position");
            assert_eq!(got.1, want.1, "handle {i} pivot");
            assert_eq!(got.2, want.2, "handle {i} scales_x");
            assert_eq!(got.3, want.3, "handle {i} scales_y");
        }
    }

    /// Corner handles scale both axes; edge handles scale exactly one.
    #[test]
    fn gizmo_scale_handles_axis_flags() {
        let handles = gizmo_scale_handles(0.2, 0.3, 0.4, 0.5);
        let corners = &handles[0..4];
        assert!(
            corners.iter().all(|h| h.2 && h.3),
            "all four corners scale both axes"
        );
        for h in &handles[4..8] {
            assert!(h.2 ^ h.3, "edge handle scales exactly one axis: {h:?}");
        }
    }

    #[test]
    fn gizmo_scale_handles_offset_box() {
        let handles = gizmo_scale_handles(0.2, 0.3, 0.4, 0.5);
        assert_eq!(handles[0].0, [0.2, 0.3], "top-left");
        assert_eq!(handles[2].0, [0.6, 0.8], "bottom-right");
        assert_eq!(handles[4].0, [0.4, 0.3], "top-middle");
        assert_eq!(handles[6].0, [0.6, 0.55], "right-middle");
    }

    // ── clear_all_drag ──────────────────────────────────────────────

    #[test]
    fn clear_all_drag_clears_every_drag_field() {
        let mut state = StageEditorState {
            dragging_vertex: Some(("a".into(), 0, 0)),
            moving_surface: Some(("a".into(), 0.0, 0.0)),
            selection_rect_start: Some([0.0, 0.0]),
            dragging_radius: Some("a".into()),
            dragging_edge: Some(("a".into(), 0, 0, [0.0, 0.0], [0.0, 0.0], [0.0, 0.0])),
            dragging_scale: Some(ScaleDrag {
                pivot: [0.0, 0.0],
                start_handle: [1.0, 1.0],
                last_sx: 1.0,
                last_sy: 1.0,
                axis_x: true,
                axis_y: true,
            }),
            dragging_rotate: Some(RotateDrag {
                center: [0.5, 0.5],
                last_angle: 0.0,
            }),
            ..StageEditorState::default()
        };
        clear_all_drag(&mut state);
        assert!(state.dragging_vertex.is_none());
        assert!(state.moving_surface.is_none());
        assert!(state.selection_rect_start.is_none());
        assert!(state.dragging_radius.is_none());
        assert!(state.dragging_edge.is_none());
        assert!(state.dragging_scale.is_none());
        assert!(state.dragging_rotate.is_none());
    }

    /// `clear_all_drag` must not disturb selection or tool choice.
    #[test]
    fn clear_all_drag_preserves_selection_and_tool() {
        let mut state = StageEditorState {
            tool: DrawingTool::Circle,
            selected_surfaces: selection(&["a", "b"]),
            polygon_verts: vec![[0.1, 0.1]],
            ..StageEditorState::default()
        };
        clear_all_drag(&mut state);
        assert_eq!(state.tool, DrawingTool::Circle);
        assert_eq!(state.selected_surfaces, selection(&["a", "b"]));
        assert_eq!(state.polygon_verts, vec![[0.1, 0.1]]);
    }
    // ── try_begin_gizmo_drag ────────────────────────────────────────
    //
    // Geometry for these tests: a 1000x500 canvas at the origin and one selected
    // surface spanning (0.25, 0.25)-(0.75, 0.75). The gizmo box is inflated by
    // GIZMO_MARGIN_PX on each side — 20/1000 in x and 20/500 in y — giving
    // (0.23, 0.21, 0.54, 0.58), centred on (0.5, 0.5).

    const CANVAS_W: f32 = 1000.0;
    const CANVAS_H: f32 = 500.0;

    fn gizmo_fixture() -> (Vec<SurfaceUI>, StageEditorState, egui::Rect) {
        let surfaces = vec![SurfaceUI::test_quad("a", 0.25, 0.25, 0.5, 0.5)];
        let state = StageEditorState {
            selected_surfaces: selection(&["a"]),
            ..StageEditorState::default()
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_W, CANVAS_H));
        (surfaces, state, rect)
    }

    fn begin_at(
        state: &mut StageEditorState,
        surfaces: &[SurfaceUI],
        rect: egui::Rect,
        pos: egui::Pos2,
    ) -> bool {
        try_begin_gizmo_drag(
            state,
            surfaces,
            pos,
            pos.x / CANVAS_W,
            pos.y / CANVAS_H,
            rect,
            CANVAS_W,
            CANVAS_H,
        )
    }

    #[test]
    fn gizmo_drag_declined_when_selection_empty() {
        let (surfaces, _, rect) = gizmo_fixture();
        let mut state = StageEditorState::default();
        assert!(!begin_at(
            &mut state,
            &surfaces,
            rect,
            egui::pos2(230.0, 105.0)
        ));
        assert!(state.dragging_scale.is_none());
        assert!(state.dragging_rotate.is_none());
    }

    #[test]
    fn gizmo_drag_declined_away_from_any_handle() {
        let (surfaces, mut state, rect) = gizmo_fixture();
        // Dead centre of the gizmo box — no handle lives there.
        assert!(!begin_at(
            &mut state,
            &surfaces,
            rect,
            egui::pos2(500.0, 250.0)
        ));
        assert!(state.dragging_scale.is_none());
        assert!(state.dragging_rotate.is_none());
    }

    #[test]
    fn gizmo_rotate_knob_starts_rotate_drag() {
        let (surfaces, mut state, rect) = gizmo_fixture();
        // Knob sits GIZMO_ROTATE_OFFSET_PX above the box's top edge midpoint.
        assert!(begin_at(
            &mut state,
            &surfaces,
            rect,
            egui::pos2(500.0, 77.0)
        ));
        let rotate = state.dragging_rotate.expect("rotate drag started");
        assert!((rotate.center[0] - 0.5).abs() < 1e-6, "{:?}", rotate.center);
        assert!((rotate.center[1] - 0.5).abs() < 1e-6, "{:?}", rotate.center);
        assert!(state.dragging_scale.is_none(), "rotate excludes scale");
    }

    #[test]
    fn gizmo_corner_handle_starts_scale_drag_with_opposite_pivot() {
        let (surfaces, mut state, rect) = gizmo_fixture();
        // Top-left handle of the inflated box.
        assert!(begin_at(
            &mut state,
            &surfaces,
            rect,
            egui::pos2(230.0, 105.0)
        ));
        let scale = state.dragging_scale.expect("scale drag started");
        assert!((scale.start_handle[0] - 0.23).abs() < 1e-6, "{scale:?}");
        assert!((scale.start_handle[1] - 0.21).abs() < 1e-6, "{scale:?}");
        // Pivot is the opposite (bottom-right) corner.
        assert!((scale.pivot[0] - 0.77).abs() < 1e-6, "{scale:?}");
        assert!((scale.pivot[1] - 0.79).abs() < 1e-6, "{scale:?}");
        assert!(scale.axis_x && scale.axis_y, "corner scales both axes");
        assert!((scale.last_sx - 1.0).abs() < 1e-6);
        assert!((scale.last_sy - 1.0).abs() < 1e-6);
        assert!(state.dragging_rotate.is_none(), "scale excludes rotate");
    }

    /// Beginning a gizmo drag must cancel any other in-progress drag, so the two
    /// gestures can never run simultaneously.
    #[test]
    fn gizmo_drag_clears_conflicting_drag_state() {
        let (surfaces, mut state, rect) = gizmo_fixture();
        state.dragging_vertex = Some(("a".into(), 0, 2));
        state.moving_surface = Some(("a".into(), 0.4, 0.4));
        state.selection_rect_start = Some([0.1, 0.1]);
        assert!(begin_at(
            &mut state,
            &surfaces,
            rect,
            egui::pos2(230.0, 105.0)
        ));
        assert!(state.dragging_vertex.is_none());
        assert!(state.moving_surface.is_none());
        assert!(state.selection_rect_start.is_none());
        assert!(state.dragging_scale.is_some());
    }
}
