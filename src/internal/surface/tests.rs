//! Unit tests for the surface model, warp modes, and stage persistence.

use super::*;
use crate::renderer::context::OutputSource;

fn master_source() -> OutputSource {
    OutputSource::Master
}

// ── Surface creation tests ───────────────────────────────────────

#[test]
fn new_rect_creates_4_vertices() {
    let s = Surface::new_rect("Test".into(), 0.1, 0.2, 0.3, 0.4, master_source());
    assert_eq!(s.vertices.len(), 4);
    assert_eq!(s.name, "Test");
    assert!(!s.is_circle());
}

#[test]
fn new_rect_vertices_correct() {
    let s = Surface::new_rect("R".into(), 0.1, 0.2, 0.3, 0.4, master_source());
    // TL, TR, BR, BL
    assert!((s.vertices[0][0] - 0.1).abs() < 1e-5);
    assert!((s.vertices[0][1] - 0.2).abs() < 1e-5);
    assert!((s.vertices[1][0] - 0.4).abs() < 1e-5); // x + w
    assert!((s.vertices[2][1] - 0.6).abs() < 1e-5); // y + h
}

// ── Center tests ─────────────────────────────────────────────────

#[test]
fn center_of_rect() {
    let s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    let c = s.center();
    assert!((c[0] - 0.5).abs() < 1e-5);
    assert!((c[1] - 0.5).abs() < 1e-5);
}

#[test]
fn center_empty_vertices() {
    let s = Surface {
        uuid: generate_short_uuid(),
        name: "E".into(),
        vertices: vec![],
        extra_contours: vec![],
        source: master_source(),
        content_mapping: ContentMapping::default(),
        output_type: SurfaceOutputType::Projection,
        circle_hint: None,
        warp: None,
        warp_bound: false,
        path: None,
        holes: Vec::new(),
        hole_contours: Vec::new(),
    };
    assert_eq!(s.center(), [0.0, 0.0]);
}

// ── Contains (point-in-polygon) tests ────────────────────────────

#[test]
fn contains_point_inside_rect() {
    let s = Surface::new_rect("R".into(), 0.1, 0.1, 0.5, 0.5, master_source());
    assert!(s.contains(0.3, 0.3));
}

#[test]
fn contains_point_outside_rect() {
    let s = Surface::new_rect("R".into(), 0.1, 0.1, 0.5, 0.5, master_source());
    assert!(!s.contains(0.0, 0.0));
    assert!(!s.contains(0.9, 0.9));
}

#[test]
fn contains_fewer_than_3_vertices() {
    let s = Surface {
        uuid: generate_short_uuid(),
        name: "Line".into(),
        vertices: vec![[0.0, 0.0], [1.0, 1.0]],
        extra_contours: vec![],
        source: master_source(),
        content_mapping: ContentMapping::default(),
        output_type: SurfaceOutputType::Projection,
        circle_hint: None,
        warp: None,
        warp_bound: false,
        path: None,
        holes: Vec::new(),
        hole_contours: Vec::new(),
    };
    assert!(!s.contains(0.5, 0.5));
}

// ── Bounding box tests ───────────────────────────────────────────

#[test]
fn bounding_box_rect() {
    let s = Surface::new_rect("R".into(), 0.1, 0.2, 0.3, 0.4, master_source());
    let bb = s.bounding_box();
    assert!((bb.x - 0.1).abs() < 1e-5);
    assert!((bb.y - 0.2).abs() < 1e-5);
    assert!((bb.width - 0.3).abs() < 1e-5);
    assert!((bb.height - 0.4).abs() < 1e-5);
}

// ── Translate tests ──────────────────────────────────────────────

#[test]
fn translate_basic() {
    let mut s = Surface::new_rect("R".into(), 0.1, 0.1, 0.2, 0.2, master_source());
    s.translate(0.1, 0.1);
    let c = s.center();
    assert!((c[0] - 0.3).abs() < 1e-4);
    assert!((c[1] - 0.3).abs() < 1e-4);
}

#[test]
fn translate_clamps_to_canvas() {
    let mut s = Surface::new_rect("R".into(), 0.8, 0.8, 0.2, 0.2, master_source());
    s.translate(0.5, 0.5); // Would go past 1.0
    let bb = s.bounding_box();
    assert!(bb.x + bb.width <= 1.0 + 1e-5);
    assert!(bb.y + bb.height <= 1.0 + 1e-5);
}

#[test]
fn translate_clamps_negative() {
    let mut s = Surface::new_rect("R".into(), 0.1, 0.1, 0.2, 0.2, master_source());
    s.translate(-0.5, -0.5); // Would go below 0
    let bb = s.bounding_box();
    assert!(bb.x >= -1e-5);
    assert!(bb.y >= -1e-5);
}

// ── Rotate / scale tests ─────────────────────────────────────────

#[test]
fn rotate_90_maps_axis_around_origin() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.rotate(std::f32::consts::FRAC_PI_2, [0.0, 0.0]);
    // vertex (1,0) → (0,1) under clockwise (y-down) 90° rotation.
    assert!((s.vertices[1][0] - 0.0).abs() < 1e-4);
    assert!((s.vertices[1][1] - 1.0).abs() < 1e-4);
}

#[test]
fn rotate_around_center_preserves_center() {
    let mut s = Surface::new_rect("R".into(), 0.2, 0.3, 0.4, 0.2, master_source());
    let c0 = s.center();
    s.rotate(0.7, c0);
    let c1 = s.center();
    assert!((c0[0] - c1[0]).abs() < 1e-4);
    assert!((c0[1] - c1[1]).abs() < 1e-4);
}

#[test]
fn scale_around_origin_scales_vertices() {
    let mut s = Surface::new_rect("R".into(), 0.1, 0.1, 0.2, 0.2, master_source());
    s.scale(2.0, 2.0, [0.0, 0.0]);
    assert!((s.vertices[0][0] - 0.2).abs() < 1e-4);
    assert!((s.vertices[2][0] - 0.6).abs() < 1e-4);
    assert!((s.vertices[2][1] - 0.6).abs() < 1e-4);
}

#[test]
fn scale_around_center_preserves_center() {
    let mut s = Surface::new_rect("R".into(), 0.2, 0.2, 0.4, 0.4, master_source());
    let c0 = s.center();
    s.scale(1.5, 0.5, c0);
    let c1 = s.center();
    assert!((c0[0] - c1[0]).abs() < 1e-4);
    assert!((c0[1] - c1[1]).abs() < 1e-4);
    let bb = s.bounding_box();
    assert!((bb.width - 0.6).abs() < 1e-4); // 0.4 * 1.5
    assert!((bb.height - 0.2).abs() < 1e-4); // 0.4 * 0.5
}

#[test]
fn scale_updates_circle_hint_radius_and_aspect() {
    let mut s = Surface::new_rect("C".into(), 0.0, 0.0, 0.4, 0.4, master_source());
    s.circle_hint = Some(CircleHint {
        center: [0.2, 0.2],
        radius: 0.2,
        sides: 8,
        aspect_ratio: 1.0,
    });
    s.scale(2.0, 3.0, [0.0, 0.0]);
    let h = s.circle_hint.unwrap();
    assert!((h.radius - 0.4).abs() < 1e-4); // 0.2 * sx
    assert!((h.aspect_ratio - 1.5).abs() < 1e-4); // 1.0 * sy/sx
    assert!((h.center[0] - 0.4).abs() < 1e-4);
    assert!((h.center[1] - 0.6).abs() < 1e-4);
}

#[test]
fn rotate_moves_circle_hint_center() {
    let mut s = Surface::new_rect("C".into(), 0.0, 0.0, 0.4, 0.4, master_source());
    s.circle_hint = Some(CircleHint {
        center: [1.0, 0.0],
        radius: 0.2,
        sides: 8,
        aspect_ratio: 1.0,
    });
    s.rotate(std::f32::consts::FRAC_PI_2, [0.0, 0.0]);
    let h = s.circle_hint.unwrap();
    assert!((h.center[0] - 0.0).abs() < 1e-4);
    assert!((h.center[1] - 1.0).abs() < 1e-4);
    assert!((h.radius - 0.2).abs() < 1e-4); // unchanged
}

#[test]
fn scale_transforms_path_control_points() {
    let mut s = Surface::new_rect("P".into(), 0.0, 0.0, 0.4, 0.4, master_source());
    s.path = Some(SurfacePath {
        start: [0.0, 0.0],
        segments: vec![PathSegment::Cubic {
            c1: [1.0, 0.0],
            c2: [2.0, 0.0],
            to: [3.0, 0.0],
        }],
        closed: true,
    });
    s.scale(2.0, 2.0, [0.0, 0.0]);
    let p = s.path.unwrap();
    assert_eq!(p.start, [0.0, 0.0]);
    match p.segments[0] {
        PathSegment::Cubic { c1, c2, to } => {
            assert!((c1[0] - 2.0).abs() < 1e-4);
            assert!((c2[0] - 4.0).abs() < 1e-4);
            assert!((to[0] - 6.0).abs() < 1e-4);
        }
        PathSegment::Line { .. } => panic!("expected cubic"),
    }
}

// ── Nearest vertex tests ─────────────────────────────────────────

#[test]
fn nearest_vertex_finds_closest() {
    let s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    // Point near top-left vertex (0,0)
    let idx = s.nearest_vertex(0.01, 0.01, 0.1);
    assert_eq!(idx, Some(0));
}

#[test]
fn nearest_vertex_none_when_far() {
    let s = Surface::new_rect("R".into(), 0.0, 0.0, 0.1, 0.1, master_source());
    let idx = s.nearest_vertex(0.9, 0.9, 0.01);
    assert_eq!(idx, None);
}

// ── CircleHint tests ─────────────────────────────────────────────

#[test]
fn circle_hint_generates_vertices() {
    let hint = CircleHint {
        center: [0.5, 0.5],
        radius: 0.2,
        sides: 8,
        aspect_ratio: 1.0,
    };
    let verts = hint.generate_vertices();
    assert_eq!(verts.len(), 8);
    // All vertices should be within canvas bounds
    for v in &verts {
        assert!(v[0] >= 0.0 && v[0] <= 1.0);
        assert!(v[1] >= 0.0 && v[1] <= 1.0);
    }
}

#[test]
fn circle_hint_min_3_sides() {
    let hint = CircleHint {
        center: [0.5, 0.5],
        radius: 0.1,
        sides: 1,
        aspect_ratio: 1.0,
    };
    let verts = hint.generate_vertices();
    assert_eq!(verts.len(), 3); // Clamped to min 3
}

#[test]
fn circle_hint_aspect_ratio() {
    let hint_square = CircleHint {
        center: [0.5, 0.5],
        radius: 0.2,
        sides: 4,
        aspect_ratio: 1.0,
    };
    let hint_wide = CircleHint {
        center: [0.5, 0.5],
        radius: 0.2,
        sides: 4,
        aspect_ratio: 2.0,
    };
    let verts_sq = hint_square.generate_vertices();
    let verts_wide = hint_wide.generate_vertices();
    // With wider aspect ratio, y spread should be larger
    let y_range_sq = verts_sq.iter().map(|v| v[1]).fold(f32::MIN, f32::max)
        - verts_sq.iter().map(|v| v[1]).fold(f32::MAX, f32::min);
    let y_range_wide = verts_wide.iter().map(|v| v[1]).fold(f32::MIN, f32::max)
        - verts_wide.iter().map(|v| v[1]).fold(f32::MAX, f32::min);
    assert!(y_range_wide > y_range_sq);
}

#[test]
fn surface_regenerate_circle_vertices() {
    let hint = CircleHint {
        center: [0.5, 0.5],
        radius: 0.2,
        sides: 6,
        aspect_ratio: 1.0,
    };
    let mut s = Surface {
        uuid: generate_short_uuid(),
        name: "C".into(),
        vertices: vec![[0.0, 0.0]], // dummy
        extra_contours: vec![],
        source: master_source(),
        content_mapping: ContentMapping::default(),
        output_type: SurfaceOutputType::Projection,
        circle_hint: Some(hint),
        warp: None,
        warp_bound: false,
        path: None,
        holes: Vec::new(),
        hole_contours: Vec::new(),
    };
    s.regenerate_circle_vertices();
    assert_eq!(s.vertices.len(), 6);
}

#[test]
fn surface_convert_to_polygon() {
    let hint = CircleHint {
        center: [0.5, 0.5],
        radius: 0.2,
        sides: 6,
        aspect_ratio: 1.0,
    };
    let mut s = Surface {
        uuid: generate_short_uuid(),
        name: "C".into(),
        vertices: hint.generate_vertices(),
        extra_contours: vec![],
        source: master_source(),
        content_mapping: ContentMapping::default(),
        output_type: SurfaceOutputType::Projection,
        circle_hint: Some(hint),
        warp: None,
        warp_bound: false,
        path: None,
        holes: Vec::new(),
        hole_contours: Vec::new(),
    };
    assert!(s.is_circle());
    s.convert_to_polygon();
    assert!(!s.is_circle());
    assert_eq!(s.vertices.len(), 6); // Vertices preserved
}

#[test]
fn surface_regenerate_from_path_flattens() {
    let path = SurfacePath {
        start: [0.0, 0.0],
        segments: vec![
            PathSegment::Line { to: [1.0, 0.0] },
            PathSegment::Line { to: [1.0, 1.0] },
        ],
        closed: true,
    };
    let mut s = Surface::new_rect("P".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.path = Some(path);
    assert!(s.has_path());
    s.regenerate_from_path();
    assert_eq!(s.vertices, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
}

// ── Subtractive holes (8i.7) ─────────────────────────────────────

fn square_hole(x0: f32, y0: f32, x1: f32, y1: f32) -> SurfacePath {
    SurfacePath::from_polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)
}

#[test]
fn add_and_remove_hole_regenerates_contours() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    assert!(!s.has_holes());
    s.add_hole(square_hole(0.2, 0.2, 0.4, 0.4));
    assert_eq!(s.holes.len(), 1);
    assert_eq!(s.hole_contours.len(), 1);
    assert!(!s.hole_contours[0].is_empty());
    assert!(s.has_holes());
    assert!(s.remove_hole(0));
    assert!(!s.has_holes());
    assert!(s.hole_contours.is_empty());
    assert!(!s.remove_hole(0));
}

#[test]
fn hole_uv_contours_normalizes_to_bounding_box() {
    // Surface bbox at (0.2,0.2) size 0.4; a hole centered inside it should
    // map to ~0.5,0.5 in uv space.
    let mut s = Surface::new_rect("R".into(), 0.2, 0.2, 0.4, 0.4, master_source());
    s.add_hole(square_hole(0.35, 0.35, 0.45, 0.45));
    let uv = s.hole_uv_contours();
    assert_eq!(uv.len(), 1);
    for p in &uv[0] {
        assert!((0.3..=0.7).contains(&p[0]), "u in range: {}", p[0]);
        assert!((0.3..=0.7).contains(&p[1]), "v in range: {}", p[1]);
    }
}

#[test]
fn hole_uv_contours_empty_without_holes() {
    let s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    assert!(s.hole_uv_contours().is_empty());
}

#[test]
fn translate_moves_holes_in_step() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 0.5, 0.5, master_source());
    s.add_hole(square_hole(0.1, 0.1, 0.2, 0.2));
    let before = s.hole_contours[0][0];
    s.translate(0.1, 0.1);
    let after = s.hole_contours[0][0];
    assert!((after[0] - (before[0] + 0.1)).abs() < 1e-4);
    assert!((after[1] - (before[1] + 0.1)).abs() < 1e-4);
    // The hole path itself moved too.
    assert!((s.holes[0].start[0] - (before[0] + 0.1)).abs() < 1e-4);
}

#[test]
fn scale_maps_holes_via_map_geometry() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.add_hole(square_hole(0.2, 0.2, 0.4, 0.4));
    let before = s.hole_contours[0][0];
    s.scale(2.0, 2.0, [0.0, 0.0]);
    let after = s.hole_contours[0][0];
    assert!((after[0] - before[0] * 2.0).abs() < 1e-4);
    assert!((after[1] - before[1] * 2.0).abs() < 1e-4);
    assert!((s.holes[0].start[0] - before[0] * 2.0).abs() < 1e-4);
}

#[test]
fn verts_to_geo_with_holes_attaches_interiors() {
    let sq = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let hole = vec![[0.3, 0.3], [0.6, 0.3], [0.6, 0.6], [0.3, 0.6]];
    let poly = verts_to_geo_with_holes(&sq, &[hole]).unwrap();
    assert_eq!(poly.interiors().len(), 1);
    // Degenerate holes (< 3 points) are skipped.
    let poly2 = verts_to_geo_with_holes(&sq, &[vec![[0.1, 0.1], [0.2, 0.2]]]).unwrap();
    assert_eq!(poly2.interiors().len(), 0);
}

#[test]
fn surface_without_path_deserializes_from_legacy_json() {
    // Legacy stage.json surface (no `path` field) → path defaults to None.
    let json = r#"{
        "uuid":"abc12345","name":"Legacy",
        "vertices":[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]],
        "source":"Master","content_mapping":"Fill","output_type":"Projection"
    }"#;
    let s: Surface = serde_json::from_str(json).unwrap();
    assert!(s.path.is_none());
    assert!(!s.has_path());
    assert_eq!(s.vertices.len(), 4);
}

// ── Auto-warp binding (8i.5a) tests ──────────────────────────────

#[test]
fn new_surface_is_warp_bound_by_default() {
    let s = Surface::new_rect("R".into(), 0.1, 0.1, 0.4, 0.4, master_source());
    assert!(s.warp_bound);
}

#[test]
fn effective_warp_bound_rect_is_conforming_mesh() {
    use crate::renderer::warp::WarpMode;
    let s = Surface::new_rect("R".into(), 0.2, 0.3, 0.4, 0.2, master_source());
    // Bound → derived conforming warp (a 2×2 mesh at the four corners),
    // regardless of the (empty) stored `warp`.
    match s.effective_warp() {
        Some(WarpMode::Mesh(m)) => {
            assert_eq!((m.cols, m.rows), (2, 2));
            assert_eq!(m.points[0].position, [0.2, 0.3]);
        }
        other => panic!("expected conforming mesh, got {other:?}"),
    }
}

#[test]
fn effective_warp_unbound_returns_stored_warp() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.warp_bound = false;
    s.warp = None;
    assert!(s.effective_warp().is_none());
}

#[test]
fn unbind_materialises_conforming_warp() {
    let mut s = Surface::new_rect("R".into(), 0.1, 0.1, 0.5, 0.5, master_source());
    assert!(s.warp.is_none());
    s.set_warp_bound(false);
    assert!(!s.warp_bound);
    // The shape's conforming warp is now the editable stored warp.
    assert!(s.warp.is_some());
}

#[test]
fn rebind_clears_stored_warp() {
    let mut s = Surface::new_rect("R".into(), 0.1, 0.1, 0.5, 0.5, master_source());
    s.set_warp_bound(false);
    assert!(s.warp.is_some());
    s.set_warp_bound(true);
    assert!(s.warp_bound);
    assert!(s.warp.is_none());
}

#[test]
fn circle_conforming_warp_is_mesh() {
    use crate::renderer::warp::WarpMode;
    let hint = CircleHint {
        center: [0.5, 0.5],
        radius: 0.3,
        sides: 32,
        aspect_ratio: 1.0,
    };
    let uuid = generate_short_uuid();
    let s = Surface {
        uuid,
        name: "C".into(),
        vertices: hint.generate_vertices(),
        extra_contours: vec![],
        source: master_source(),
        content_mapping: ContentMapping::default(),
        output_type: SurfaceOutputType::Projection,
        circle_hint: Some(hint),
        warp: None,
        warp_bound: true,
        path: None,
        holes: Vec::new(),
        hole_contours: Vec::new(),
    };
    assert!(matches!(s.conforming_warp(), WarpMode::Mesh(_)));
}

#[test]
fn legacy_json_loads_unbound_preserving_warp() {
    // Pre-8i.5a file: no `warp_bound`, so it must default to false so any
    // stored warp stays authoritative.
    let json = r#"{
        "uuid":"abc12345","name":"Legacy",
        "vertices":[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]],
        "source":"Master","content_mapping":"Fill","output_type":"Projection"
    }"#;
    let s: Surface = serde_json::from_str(json).unwrap();
    assert!(!s.warp_bound);
}

// ── Contour tests ────────────────────────────────────────────────

#[test]
fn contour_count() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 0.5, 0.5, master_source());
    assert_eq!(s.contour_count(), 1);
    s.extra_contours
        .push(vec![[0.6, 0.6], [0.8, 0.6], [0.7, 0.8]]);
    assert_eq!(s.contour_count(), 2);
}

#[test]
fn contour_access() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 0.5, 0.5, master_source());
    s.extra_contours
        .push(vec![[0.6, 0.6], [0.8, 0.6], [0.7, 0.8]]);
    assert!(s.contour(0).is_some());
    assert!(s.contour(1).is_some());
    assert!(s.contour(2).is_none());
    assert!(s.contour_mut(0).is_some());
    assert!(s.contour_mut(1).is_some());
}

#[test]
fn contains_in_extra_contour() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 0.1, 0.1, master_source());
    s.extra_contours
        .push(vec![[0.5, 0.5], [0.9, 0.5], [0.9, 0.9], [0.5, 0.9]]);
    assert!(s.contains(0.7, 0.7)); // Inside extra contour
    assert!(!s.contains(0.3, 0.3)); // Between contours
}

// ── SurfaceManager tests ─────────────────────────────────────────

#[test]
fn new_rect_has_uuid() {
    let s = Surface::new_rect("Test".into(), 0.1, 0.2, 0.3, 0.4, master_source());
    assert_eq!(s.uuid.len(), 8);
}

// ── Bezier edge editing (8i.4) ───────────────────────────────────

#[test]
fn convert_edge_lazily_builds_path_and_regenerates() {
    let mut s = Surface::new_rect("C".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    assert!(!s.has_path());
    s.convert_edge(0, true);
    assert!(s.has_path());
    assert!(s.path.as_ref().unwrap().is_edge_cubic(0));
    // Cubic edge 0 tessellates into more vertices than the original 4.
    assert!(s.vertices.len() > 4);
}

#[test]
fn ensure_path_clears_circle_hint() {
    let mut s = Surface::new_rect("C".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.circle_hint = Some(CircleHint {
        center: [0.5, 0.5],
        radius: 0.5,
        sides: 8,
        aspect_ratio: 1.0,
    });
    s.ensure_path();
    assert!(s.circle_hint.is_none());
}

#[test]
fn move_path_anchor_updates_vertices() {
    let mut s = Surface::new_rect("C".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.ensure_path();
    s.move_path_anchor(0, [0.2, 0.3]);
    assert!((s.vertices[0][0] - 0.2).abs() < 1e-5);
    assert!((s.vertices[0][1] - 0.3).abs() < 1e-5);
}

#[test]
fn move_path_handle_noop_without_path() {
    let mut s = Surface::new_rect("C".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.move_path_handle(0, CubicHandle::C1, [0.5, 0.5]);
    assert!(!s.has_path());
}

#[test]
fn manager_add_surface() {
    let mut mgr = SurfaceManager::new();
    let uuid = mgr.add_surface("Main".into(), master_source());
    assert_eq!(uuid.len(), 8);
    assert_eq!(mgr.surfaces.len(), 1);
    assert_eq!(mgr.surfaces[0].uuid, uuid);
}

#[test]
fn manager_add_path_surface_attaches_path_and_flattens() {
    let mut mgr = SurfaceManager::new();
    let mut path =
        curve::SurfacePath::from_polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], true);
    path.convert_edge_to_cubic(0);
    let uuid = mgr.add_path_surface("Curved".into(), path, master_source());
    let s = mgr.surfaces.iter().find(|s| s.uuid == uuid).unwrap();
    assert!(s.has_path());
    assert!(s.path.as_ref().unwrap().has_cubic());
    // A cubic edge flattens into more vertices than the raw 4 corners.
    assert!(s.vertices.len() > 4);
}

#[test]
fn manager_remove_surface() {
    let mut mgr = SurfaceManager::new();
    let uuid_a = mgr.add_surface("A".into(), master_source());
    mgr.add_surface("B".into(), master_source());
    assert!(mgr.remove_surface(&uuid_a));
    assert_eq!(mgr.surfaces.len(), 1);
    assert_eq!(mgr.surfaces[0].name, "B");
}

#[test]
fn manager_remove_not_found() {
    let mut mgr = SurfaceManager::new();
    assert!(!mgr.remove_surface("nonexist"));
}

#[test]
fn manager_surface_at() {
    let mut mgr = SurfaceManager::new();
    let uuid = mgr.add_surface("A".into(), master_source());
    // The first surface is placed at (0.05, 0.05) with size 0.28x0.28
    let found = mgr.surface_at(0.15, 0.15);
    assert_eq!(found, Some(uuid));
    let not_found = mgr.surface_at(0.99, 0.99);
    assert_eq!(not_found, None);
}

#[test]
fn manager_surface_at_returns_topmost() {
    let mut mgr = SurfaceManager::new();
    // Two overlapping surfaces
    mgr.surfaces.push(Surface::new_rect(
        "A".into(),
        0.0,
        0.0,
        0.5,
        0.5,
        master_source(),
    ));
    mgr.surfaces.push(Surface::new_rect(
        "B".into(),
        0.1,
        0.1,
        0.5,
        0.5,
        master_source(),
    ));
    let b_uuid = mgr.surfaces[1].uuid.clone();
    // At (0.2, 0.2) both contain, but B is topmost (last added)
    assert_eq!(mgr.surface_at(0.2, 0.2), Some(b_uuid));
}

#[test]
fn manager_add_polygon_surface() {
    let mut mgr = SurfaceManager::new();
    let verts = vec![[0.0, 0.0], [0.5, 0.0], [0.25, 0.5]];
    let uuid = mgr.add_polygon_surface("Triangle".into(), verts, master_source());
    assert_eq!(uuid.len(), 8);
    assert_eq!(mgr.surfaces[0].vertices.len(), 3);
}

#[test]
fn manager_add_circle_surface() {
    let mut mgr = SurfaceManager::new();
    let hint = CircleHint {
        center: [0.5, 0.5],
        radius: 0.2,
        sides: 16,
        aspect_ratio: 1.0,
    };
    let uuid = mgr.add_circle_surface("Circle".into(), hint, master_source());
    assert_eq!(uuid.len(), 8);
    assert!(mgr.surfaces[0].is_circle());
    assert_eq!(mgr.surfaces[0].vertices.len(), 16);
}

#[test]
fn manager_find_by_uuid() {
    let mut mgr = SurfaceManager::new();
    let uuid = mgr.add_surface("Test".into(), master_source());
    let (idx, surface) = mgr.find_by_uuid(&uuid).unwrap();
    assert_eq!(idx, 0);
    assert_eq!(surface.name, "Test");
    assert!(mgr.find_by_uuid("nonexist").is_none());
}

#[test]
fn manager_find_by_uuid_mut() {
    let mut mgr = SurfaceManager::new();
    let uuid = mgr.add_surface("Test".into(), master_source());
    let (idx, surface) = mgr.find_by_uuid_mut(&uuid).unwrap();
    assert_eq!(idx, 0);
    surface.name = "Changed".into();
    assert_eq!(mgr.surfaces[0].name, "Changed");
}

#[test]
fn manager_combine_surfaces() {
    let mut mgr = SurfaceManager::new();
    mgr.surfaces.push(Surface::new_rect(
        "A".into(),
        0.0,
        0.0,
        0.3,
        0.3,
        master_source(),
    ));
    mgr.surfaces.push(Surface::new_rect(
        "B".into(),
        0.2,
        0.2,
        0.3,
        0.3,
        master_source(),
    ));
    let uuid_a = mgr.surfaces[0].uuid.clone();
    let uuid_b = mgr.surfaces[1].uuid.clone();
    let result = mgr.combine_surfaces(&[uuid_a, uuid_b]);
    assert!(result.is_some());
    assert_eq!(mgr.surfaces.len(), 1);
    // Combined surfaces get a short sequential name, not the joined originals.
    assert_eq!(mgr.surfaces[0].name, "Combined 1");
}

#[test]
fn manager_combine_names_are_sequential() {
    let mut mgr = SurfaceManager::new();
    let rect = |n: &str, x: f32| Surface::new_rect(n.into(), x, 0.0, 0.2, 0.2, master_source());
    // Four disjoint surfaces → two independent combines.
    for (n, x) in [("a", 0.0), ("b", 0.3), ("c", 0.6), ("d", 0.9)] {
        mgr.surfaces.push(rect(n, x));
    }
    let (u0, u1) = (mgr.surfaces[0].uuid.clone(), mgr.surfaces[1].uuid.clone());
    mgr.combine_surfaces(&[u0, u1]);
    assert!(mgr.surfaces.iter().any(|s| s.name == "Combined 1"));

    let (u2, u3) = (
        mgr.surfaces
            .iter()
            .find(|s| s.name == "c")
            .unwrap()
            .uuid
            .clone(),
        mgr.surfaces
            .iter()
            .find(|s| s.name == "d")
            .unwrap()
            .uuid
            .clone(),
    );
    mgr.combine_surfaces(&[u2, u3]);
    // Second combine must not collide with the first.
    assert!(mgr.surfaces.iter().any(|s| s.name == "Combined 1"));
    assert!(mgr.surfaces.iter().any(|s| s.name == "Combined 2"));
}

// ── Stacking order (8i.12) ───────────────────────────────────────

fn mgr_abc() -> (SurfaceManager, String, String, String) {
    let mut mgr = SurfaceManager::new();
    let rect = |n: &str, x: f32| Surface::new_rect(n.into(), x, 0.0, 0.2, 0.2, master_source());
    for (n, x) in [("a", 0.0), ("b", 0.3), ("c", 0.6)] {
        mgr.surfaces.push(rect(n, x));
    }
    let (a, b, c) = (
        mgr.surfaces[0].uuid.clone(),
        mgr.surfaces[1].uuid.clone(),
        mgr.surfaces[2].uuid.clone(),
    );
    (mgr, a, b, c)
}

fn names(mgr: &SurfaceManager) -> Vec<String> {
    mgr.surfaces.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn reorder_to_front_moves_to_last() {
    let (mut mgr, a, _b, _c) = mgr_abc();
    assert!(mgr.reorder_surface(&a, SurfaceReorderOp::ToFront));
    assert_eq!(names(&mgr), vec!["b", "c", "a"]);
}

#[test]
fn reorder_to_back_moves_to_first() {
    let (mut mgr, _a, _b, c) = mgr_abc();
    assert!(mgr.reorder_surface(&c, SurfaceReorderOp::ToBack));
    assert_eq!(names(&mgr), vec!["c", "a", "b"]);
}

#[test]
fn reorder_up_moves_one_step_toward_front() {
    let (mut mgr, a, _b, _c) = mgr_abc();
    assert!(mgr.reorder_surface(&a, SurfaceReorderOp::Up));
    assert_eq!(names(&mgr), vec!["b", "a", "c"]);
}

#[test]
fn reorder_down_moves_one_step_toward_back() {
    let (mut mgr, _a, _b, c) = mgr_abc();
    assert!(mgr.reorder_surface(&c, SurfaceReorderOp::Down));
    assert_eq!(names(&mgr), vec!["a", "c", "b"]);
}

#[test]
fn reorder_up_at_top_is_noop_but_ok() {
    let (mut mgr, _a, _b, c) = mgr_abc();
    assert!(mgr.reorder_surface(&c, SurfaceReorderOp::Up));
    assert_eq!(names(&mgr), vec!["a", "b", "c"]);
}

#[test]
fn reorder_down_at_bottom_is_noop_but_ok() {
    let (mut mgr, a, _b, _c) = mgr_abc();
    assert!(mgr.reorder_surface(&a, SurfaceReorderOp::Down));
    assert_eq!(names(&mgr), vec!["a", "b", "c"]);
}

#[test]
fn reorder_unknown_uuid_returns_false() {
    let (mut mgr, _a, _b, _c) = mgr_abc();
    assert!(!mgr.reorder_surface("nope", SurfaceReorderOp::ToFront));
    assert_eq!(names(&mgr), vec!["a", "b", "c"]);
}

#[test]
fn manager_combine_fewer_than_2() {
    let mut mgr = SurfaceManager::new();
    mgr.surfaces.push(Surface::new_rect(
        "A".into(),
        0.0,
        0.0,
        0.3,
        0.3,
        master_source(),
    ));
    let uuid = mgr.surfaces[0].uuid.clone();
    assert_eq!(mgr.combine_surfaces(&[uuid]), None);
    assert_eq!(mgr.combine_surfaces(&[]), None);
}

// ── ContentMapping & SurfaceOutputType Display ────────────────────

#[test]
fn content_mapping_display() {
    assert_eq!(format!("{}", ContentMapping::Fill), "Fill");
    assert_eq!(format!("{}", ContentMapping::Mapped), "Mapped");
}

#[test]
fn surface_output_type_display() {
    assert_eq!(format!("{}", SurfaceOutputType::Projection), "Projection");
    assert_eq!(format!("{}", SurfaceOutputType::LEDDirect), "LED Direct");
}

#[test]
fn content_mapping_default() {
    assert_eq!(ContentMapping::default(), ContentMapping::Fill);
}

// ── Per-surface warp editing (8i.5) ───────────────────────────────

#[test]
fn warp_defaults_to_none() {
    let s = Surface::new_rect("R".into(), 0.1, 0.1, 0.4, 0.3, master_source());
    assert!(s.warp.is_none());
}

#[test]
fn set_warp_corner_seeds_identity_then_moves() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_corner(0, [0.2, 0.3]);
    match s.warp {
        Some(WarpMode::CornerPin { corners }) => assert_eq!(corners[0], [0.2, 0.3]),
        _ => panic!("expected corner-pin warp"),
    }
}

#[test]
fn set_warp_corner_ignored_out_of_range() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_corner(4, [0.2, 0.3]);
    assert!(s.warp.is_none());
}

#[test]
fn set_warp_subdivisions_makes_mesh_and_clamps() {
    use crate::renderer::warp::{MAX_WARP_SUBDIVISIONS, WarpMode};
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_subdivisions(1, 1000);
    match s.warp {
        Some(WarpMode::Mesh(mesh)) => {
            assert_eq!(mesh.cols, 2);
            assert_eq!(mesh.rows, MAX_WARP_SUBDIVISIONS);
        }
        _ => panic!("expected mesh warp"),
    }
}

#[test]
fn set_warp_corner_noop_on_mesh() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_subdivisions(3, 3);
    let before = format!("{:?}", s.warp);
    s.set_warp_corner(0, [0.9, 0.9]);
    assert_eq!(before, format!("{:?}", s.warp));
}

#[test]
fn set_warp_mesh_point_moves_point() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_subdivisions(3, 3);
    s.set_warp_mesh_point(1, 1, [0.55, 0.55]);
    match s.warp {
        Some(WarpMode::Mesh(mesh)) => {
            let p = mesh.points[mesh.cols as usize + 1].position;
            assert!((p[0] - 0.55).abs() < 1e-6 && (p[1] - 0.55).abs() < 1e-6);
        }
        _ => panic!("expected mesh warp"),
    }
}

#[test]
fn reset_warp_clears() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_subdivisions(3, 3);
    s.reset_warp();
    assert!(s.warp.is_none());
}

// ── Bezier warp (8i.6) ───────────────────────────────────────────

#[test]
fn convert_warp_to_bezier_seeds_cage_from_shape() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.1, 0.2, 0.5, 0.4, master_source());
    s.set_warp_bound(false); // manual editing
    s.convert_warp_to_bezier();
    match &s.warp {
        Some(WarpMode::Bezier(b)) => {
            // Seeded from the identity 2×2 corner-pin over the bbox.
            assert_eq!((b.anchor_cols, b.anchor_rows), (2, 2));
            assert_eq!(b.anchor(0, 0), [0.1, 0.2]);
            assert_eq!(b.anchor(1, 1), [0.6, 0.6]);
        }
        other => panic!("expected bezier warp, got {other:?}"),
    }
}

#[test]
fn convert_warp_to_bezier_preserves_mesh_dims() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_bound(false);
    s.set_warp_subdivisions(4, 3);
    s.convert_warp_to_bezier();
    match &s.warp {
        Some(WarpMode::Bezier(b)) => assert_eq!((b.anchor_cols, b.anchor_rows), (4, 3)),
        other => panic!("expected bezier warp, got {other:?}"),
    }
}

#[test]
fn convert_warp_to_bezier_noop_when_already_bezier() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_bound(false);
    s.convert_warp_to_bezier();
    s.set_bezier_cage_subdivisions(3, 3);
    s.convert_warp_to_bezier(); // must not reset the 3×3 cage back to 2×2
    match &s.warp {
        Some(WarpMode::Bezier(b)) => assert_eq!((b.anchor_cols, b.anchor_rows), (3, 3)),
        other => panic!("expected bezier warp, got {other:?}"),
    }
}

#[test]
fn set_warp_bezier_anchor_moves_it() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_bound(false);
    s.convert_warp_to_bezier();
    s.set_warp_bezier_anchor(0, 0, [0.2, 0.3]);
    match &s.warp {
        Some(WarpMode::Bezier(b)) => assert_eq!(b.anchor(0, 0), [0.2, 0.3]),
        other => panic!("expected bezier warp, got {other:?}"),
    }
}

#[test]
fn set_warp_bezier_handle_and_anchor_noop_on_mesh() {
    // On a non-bezier warp these are no-ops (don't panic / don't change type).
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_bound(false);
    s.set_warp_subdivisions(3, 3);
    let before = format!("{:?}", s.warp);
    s.set_warp_bezier_anchor(0, 0, [0.9, 0.9]);
    s.set_warp_bezier_handle(true, 0, 0, 0, [0.5, 0.5]);
    s.set_bezier_cage_subdivisions(5, 5);
    assert_eq!(before, format!("{:?}", s.warp));
}

#[test]
fn effective_warp_bezier_is_returned_when_unbound() {
    use crate::renderer::warp::WarpMode;
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    s.set_warp_bound(false);
    s.convert_warp_to_bezier();
    // Effective warp hands the bezier cage through (for the editor); render
    // sites tessellate it via WarpMode::render_mesh.
    assert!(matches!(s.effective_warp(), Some(WarpMode::Bezier(_))));
    assert!(s.effective_warp().unwrap().render_mesh().is_some());
}

// ── Make-Hole (punch) domain tests (8i.7) ────────────────────────

#[test]
fn outline_as_path_falls_back_to_vertices_when_no_path() {
    let s = Surface::new_rect("R".into(), 0.1, 0.2, 0.3, 0.4, master_source());
    assert!(s.path.is_none());
    // Flattened outline path matches the polygon vertices.
    assert_eq!(s.outline_as_path().flatten(), s.vertices);
}

#[test]
fn outline_as_path_clones_existing_path() {
    let mut s = Surface::new_rect("R".into(), 0.0, 0.0, 0.5, 0.5, master_source());
    let p = s.ensure_path().clone();
    assert_eq!(s.outline_as_path(), p);
}

fn sm(surfaces: Vec<Surface>) -> SurfaceManager {
    SurfaceManager {
        surfaces,
        dome_setup: None,
    }
}

#[test]
fn resolve_hole_target_picks_surface_under_source_centroid() {
    let big = Surface::new_rect("Big".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    let small = Surface::new_rect("Small".into(), 0.4, 0.4, 0.2, 0.2, master_source());
    let (big_id, small_id) = (big.uuid.clone(), small.uuid.clone());
    let mgr = sm(vec![big, small]);
    assert_eq!(mgr.resolve_hole_target(&small_id), Some(big_id));
}

#[test]
fn resolve_hole_target_none_when_nothing_behind() {
    let small = Surface::new_rect("Small".into(), 0.4, 0.4, 0.2, 0.2, master_source());
    let small_id = small.uuid.clone();
    let mgr = sm(vec![small]);
    assert_eq!(mgr.resolve_hole_target(&small_id), None);
}

#[test]
fn resolve_hole_target_picks_topmost_other() {
    // Two full-canvas rects both contain the small rect's centroid; the punch
    // must target the topmost (last in draw order), never the source itself.
    let a = Surface::new_rect("A".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    let b = Surface::new_rect("B".into(), 0.0, 0.0, 1.0, 1.0, master_source());
    let small = Surface::new_rect("Small".into(), 0.4, 0.4, 0.2, 0.2, master_source());
    let (b_id, small_id) = (b.uuid.clone(), small.uuid.clone());
    let mgr = sm(vec![a, b, small]);
    assert_eq!(mgr.resolve_hole_target(&small_id), Some(b_id));
}
