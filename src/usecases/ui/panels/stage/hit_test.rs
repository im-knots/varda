//! Pure canvas geometry and hit-testing for the stage editor.
//!
//! No `egui` painting and no state mutation — just the screen ↔ normalized
//! coordinate mapping and point-in-surface queries the interaction handlers need.
//! Kept free of side effects so it can be unit-tested directly.

use super::super::super::SurfaceUI;

/// The stage canvas's placement and snapping configuration for one frame.
///
/// Normalized coordinates are `[0, 1]` across the canvas, which is what every
/// surface vertex and warp point is stored in.
#[derive(Debug, Clone, Copy)]
pub(super) struct CanvasGeometry {
    pub(super) rect: egui::Rect,
    pub(super) width: f32,
    pub(super) height: f32,
    /// Grid spacing in normalized units. `<= 0.001` disables snapping.
    pub(super) grid_size: f32,
    pub(super) snap_enabled: bool,
}

impl CanvasGeometry {
    pub(super) fn new(rect: egui::Rect, grid_size: f32, snap_enabled: bool) -> Self {
        Self {
            rect,
            width: rect.width(),
            height: rect.height(),
            grid_size,
            snap_enabled,
        }
    }

    /// Quantize one normalized axis value to the grid, when snapping is on.
    pub(super) fn snap(self, v: f32) -> f32 {
        if self.snap_enabled && self.grid_size > 0.001 {
            (v / self.grid_size).round() * self.grid_size
        } else {
            v
        }
    }

    /// Screen position → snapped normalized position, clamped to the canvas.
    pub(super) fn to_norm(self, pos: egui::Pos2) -> [f32; 2] {
        let [nx, ny] = self.to_norm_raw(pos);
        [self.snap(nx), self.snap(ny)]
    }

    /// Screen position → un-snapped normalized position, clamped to the canvas.
    ///
    /// Bezier anchor/handle editing needs sub-grid precision: hit-testing against
    /// off-grid control points and dragging them must not snap-jump to the grid.
    pub(super) fn to_norm_raw(self, pos: egui::Pos2) -> [f32; 2] {
        [
            ((pos.x - self.rect.left()) / self.width).clamp(0.0, 1.0),
            ((pos.y - self.rect.top()) / self.height).clamp(0.0, 1.0),
        ]
    }

    /// Normalized position → screen position.
    pub(super) fn to_screen(self, p: [f32; 2]) -> egui::Pos2 {
        egui::pos2(
            self.rect.left() + p[0] * self.width,
            self.rect.top() + p[1] * self.height,
        )
    }
}

/// UUID of the topmost surface containing the normalized point, if any.
///
/// Surfaces are tested back-to-front (last drawn wins) with an even-odd crossing
/// test, matching the stacking order the canvas paints in.
pub(super) fn point_in_any_surface(surfaces: &[SurfaceUI], nx: f32, ny: f32) -> Option<String> {
    for surface in surfaces.iter().rev() {
        let verts = &surface.vertices;
        let n = verts.len();
        if n >= 3 {
            let mut inside = false;
            let mut j = n - 1;
            for k in 0..n {
                let (xi, yi) = (verts[k][0], verts[k][1]);
                let (xj, yj) = (verts[j][0], verts[j][1]);
                if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
                    inside = !inside;
                }
                j = k;
            }
            if inside {
                return Some(surface.uuid.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1000x500 canvas at the origin, snapping off unless a test enables it.
    fn geom(grid: f32, snap: bool) -> CanvasGeometry {
        CanvasGeometry::new(
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 500.0)),
            grid,
            snap,
        )
    }

    #[test]
    fn to_norm_raw_maps_corners_to_unit_square() {
        let g = geom(0.05, false);
        assert_eq!(g.to_norm_raw(egui::pos2(0.0, 0.0)), [0.0, 0.0]);
        assert_eq!(g.to_norm_raw(egui::pos2(1000.0, 500.0)), [1.0, 1.0]);
        assert_eq!(g.to_norm_raw(egui::pos2(500.0, 250.0)), [0.5, 0.5]);
    }

    #[test]
    fn to_norm_raw_clamps_outside_the_canvas() {
        let g = geom(0.05, false);
        assert_eq!(g.to_norm_raw(egui::pos2(-200.0, -50.0)), [0.0, 0.0]);
        assert_eq!(g.to_norm_raw(egui::pos2(5000.0, 5000.0)), [1.0, 1.0]);
    }

    #[test]
    fn to_norm_raw_respects_canvas_offset() {
        let g = CanvasGeometry::new(
            egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(200.0, 100.0)),
            0.05,
            false,
        );
        assert_eq!(g.to_norm_raw(egui::pos2(100.0, 40.0)), [0.0, 0.0]);
        assert_eq!(g.to_norm_raw(egui::pos2(300.0, 140.0)), [1.0, 1.0]);
        assert_eq!(g.to_norm_raw(egui::pos2(200.0, 90.0)), [0.5, 0.5]);
    }

    #[test]
    fn snap_is_identity_when_disabled() {
        let g = geom(0.25, false);
        assert!((g.snap(0.31) - 0.31).abs() < 1e-6);
    }

    /// A grid at or below 0.001 disables snapping, guarding a divide-by-tiny.
    #[test]
    fn snap_is_identity_for_degenerate_grid() {
        let g = geom(0.0, true);
        assert!((g.snap(0.31) - 0.31).abs() < 1e-6);
        let g = geom(0.001, true);
        assert!((g.snap(0.31) - 0.31).abs() < 1e-6);
    }

    #[test]
    fn snap_rounds_to_nearest_grid_step() {
        let g = geom(0.25, true);
        assert!((g.snap(0.31) - 0.25).abs() < 1e-6, "{}", g.snap(0.31));
        assert!((g.snap(0.4) - 0.5).abs() < 1e-6, "{}", g.snap(0.4));
        assert!((g.snap(0.0) - 0.0).abs() < 1e-6);
        assert!((g.snap(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn to_norm_applies_snapping_but_to_norm_raw_does_not() {
        let g = geom(0.25, true);
        let pos = egui::pos2(310.0, 155.0); // raw (0.31, 0.31)
        assert_eq!(g.to_norm(pos), [0.25, 0.25]);
        let raw = g.to_norm_raw(pos);
        assert!((raw[0] - 0.31).abs() < 1e-6, "{raw:?}");
    }

    #[test]
    fn to_screen_round_trips_with_to_norm_raw() {
        let g = geom(0.05, false);
        let p = [0.37, 0.62];
        let back = g.to_norm_raw(g.to_screen(p));
        assert!((back[0] - p[0]).abs() < 1e-5, "{back:?}");
        assert!((back[1] - p[1]).abs() < 1e-5, "{back:?}");
    }

    // ── point_in_any_surface ────────────────────────────────────────

    #[test]
    fn point_inside_a_quad_is_detected() {
        let surfaces = vec![SurfaceUI::test_quad("a", 0.2, 0.2, 0.6, 0.6)];
        assert_eq!(
            point_in_any_surface(&surfaces, 0.5, 0.5),
            Some("a".to_string())
        );
    }

    #[test]
    fn point_outside_every_surface_is_none() {
        let surfaces = vec![SurfaceUI::test_quad("a", 0.2, 0.2, 0.1, 0.1)];
        assert_eq!(point_in_any_surface(&surfaces, 0.9, 0.9), None);
    }

    /// Overlapping surfaces resolve to the last one in the list, matching the
    /// canvas's back-to-front paint order.
    #[test]
    fn topmost_surface_wins_when_overlapping() {
        let surfaces = vec![
            SurfaceUI::test_quad("under", 0.0, 0.0, 1.0, 1.0),
            SurfaceUI::test_quad("over", 0.4, 0.4, 0.2, 0.2),
        ];
        assert_eq!(
            point_in_any_surface(&surfaces, 0.5, 0.5),
            Some("over".to_string())
        );
        // Outside the top surface, the one beneath still answers.
        assert_eq!(
            point_in_any_surface(&surfaces, 0.05, 0.05),
            Some("under".to_string())
        );
    }

    #[test]
    fn degenerate_surfaces_are_skipped() {
        let mut two_verts = SurfaceUI::test_quad("a", 0.0, 0.0, 1.0, 1.0);
        two_verts.vertices.truncate(2);
        assert_eq!(point_in_any_surface(&[two_verts], 0.5, 0.5), None);

        let mut empty = SurfaceUI::test_quad("b", 0.0, 0.0, 1.0, 1.0);
        empty.vertices.clear();
        assert_eq!(point_in_any_surface(&[empty], 0.5, 0.5), None);
    }

    /// Concave outlines must use the crossing test, not a bounding box: the notch
    /// of an L-shape is inside the bbox but outside the polygon.
    #[test]
    fn concave_outline_excludes_its_notch() {
        let mut l_shape = SurfaceUI::test_quad("l", 0.0, 0.0, 1.0, 1.0);
        l_shape.vertices = vec![
            [0.0, 0.0],
            [0.4, 0.0],
            [0.4, 0.6],
            [1.0, 0.6],
            [1.0, 1.0],
            [0.0, 1.0],
        ];
        assert_eq!(
            point_in_any_surface(&[l_shape.clone()], 0.2, 0.8),
            Some("l".to_string()),
            "inside the solid part"
        );
        assert_eq!(
            point_in_any_surface(&[l_shape], 0.8, 0.2),
            None,
            "the notch is inside the bbox but outside the polygon"
        );
    }
}
