//! Polygon triangulation utilities for the stage editor.

// ── Polygon triangulation (ear-clipping) ────────────────────────────

/// Build an `egui::Shape` for an arbitrary (possibly concave) polygon using
/// ear-clipping triangulation. Falls back to `convex_polygon` for ≤4 vertices
/// where convexity is likely.
pub(super) fn polygon_shape(
    verts: &[egui::Pos2],
    fill: egui::Color32,
    stroke: egui::Stroke,
) -> egui::Shape {
    if verts.len() < 3 {
        return egui::Shape::Noop;
    }

    // Triangulate
    let indices = triangulate_polygon(verts);
    if indices.is_empty() {
        // Fallback if triangulation fails
        return egui::Shape::convex_polygon(verts.to_vec(), fill, stroke);
    }

    // Build mesh for the filled area (default texture_id targets the font atlas)
    let mut mesh = egui::Mesh::default();
    for &p in verts {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: p,
            uv: egui::pos2(0.0, 0.0),
            color: fill,
        });
    }
    mesh.indices = indices;

    let mut shapes = vec![egui::Shape::mesh(mesh)];

    // Draw outline on top
    if stroke.width > 0.0 {
        let mut outline = verts.to_vec();
        outline.push(verts[0]); // close the loop
        shapes.push(egui::Shape::line(outline, stroke));
    }

    egui::Shape::Vec(shapes)
}

/// Ear-clipping triangulation for a simple polygon.
/// Returns triangle indices into the vertex array.
fn triangulate_polygon(verts: &[egui::Pos2]) -> Vec<u32> {
    let n = verts.len();
    if n < 3 {
        return Vec::new();
    }

    // Work with a mutable index list
    let mut idx: Vec<usize> = (0..n).collect();
    let mut result = Vec::with_capacity((n - 2) * 3);

    // Determine winding: positive = CCW
    let signed_area: f32 = idx
        .windows(2)
        .map(|w| {
            let a = verts[w[0]];
            let b = verts[w[1]];
            (b.x - a.x) * (b.y + a.y)
        })
        .sum::<f32>()
        + {
            let a = verts[*idx.last().expect("polygon must have >= 3 vertices")];
            let b = verts[idx[0]];
            (b.x - a.x) * (b.y + a.y)
        };
    let ccw = signed_area < 0.0; // screen coords: y-down, so negative area = CCW

    let mut remaining = idx.len();
    let mut fail_count = 0;
    let mut i = 0;

    while remaining > 2 && fail_count < remaining {
        let prev = idx[(i + remaining - 1) % remaining];
        let curr = idx[i % remaining];
        let next = idx[(i + 1) % remaining];

        if is_ear(verts, &idx, prev, curr, next, ccw) {
            result.push(prev as u32);
            result.push(curr as u32);
            result.push(next as u32);
            idx.remove(i % remaining);
            remaining -= 1;
            fail_count = 0;
            if i >= remaining && remaining > 0 {
                i = 0;
            }
        } else {
            i = (i + 1) % remaining;
            fail_count += 1;
        }
    }

    result
}

fn cross_2d(o: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn is_ear(
    verts: &[egui::Pos2],
    idx: &[usize],
    prev: usize,
    curr: usize,
    next: usize,
    ccw: bool,
) -> bool {
    let cross = cross_2d(verts[prev], verts[curr], verts[next]);
    // For CCW winding, an ear has positive cross product
    if ccw {
        if cross <= 0.0 {
            return false;
        }
    } else if cross >= 0.0 {
        return false;
    }

    // Check no other vertex is inside this triangle
    for &vi in idx {
        if vi == prev || vi == curr || vi == next {
            continue;
        }
        if point_in_triangle(verts[vi], verts[prev], verts[curr], verts[next]) {
            return false;
        }
    }
    true
}

fn point_in_triangle(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> bool {
    let d0 = cross_2d(a, b, p);
    let d1 = cross_2d(b, c, p);
    let d2 = cross_2d(c, a, p);
    let has_neg = (d0 < 0.0) || (d1 < 0.0) || (d2 < 0.0);
    let has_pos = (d0 > 0.0) || (d1 > 0.0) || (d2 > 0.0);
    !(has_neg && has_pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(x, y)
    }

    /// Every index a triangulation emits must reference a real vertex, and the
    /// output must be whole triangles.
    fn assert_valid_indices(indices: &[u32], n: usize) {
        assert_eq!(indices.len() % 3, 0, "indices must form whole triangles");
        for &i in indices {
            assert!((i as usize) < n, "index {i} out of range for {n} verts");
        }
    }

    #[test]
    fn triangulate_degenerate_is_empty() {
        assert!(triangulate_polygon(&[]).is_empty());
        assert!(triangulate_polygon(&[p(0.0, 0.0)]).is_empty());
        assert!(triangulate_polygon(&[p(0.0, 0.0), p(1.0, 0.0)]).is_empty());
    }

    #[test]
    fn triangulate_convex_quad_yields_two_triangles() {
        // CW in screen (y-down) coords.
        let quad = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        let indices = triangulate_polygon(&quad);
        assert_eq!(indices.len(), 6, "a quad is two triangles");
        assert_valid_indices(&indices, quad.len());
    }

    #[test]
    fn triangulate_simple_polygon_emits_n_minus_two_triangles() {
        // Convex pentagon: a simple polygon of n verts triangulates to n-2 tris.
        let penta = [
            p(0.0, 0.0),
            p(2.0, 0.0),
            p(3.0, 1.5),
            p(1.0, 3.0),
            p(-1.0, 1.5),
        ];
        let indices = triangulate_polygon(&penta);
        assert_eq!(indices.len(), (penta.len() - 2) * 3);
        assert_valid_indices(&indices, penta.len());
    }

    #[test]
    fn triangulate_concave_polygon_is_fully_triangulated() {
        // Concave L-shape (one reflex vertex). Ear-clipping must still fully
        // triangulate it into n-2 triangles with valid indices.
        let l_shape = [
            p(0.0, 0.0),
            p(2.0, 0.0),
            p(2.0, 1.0),
            p(1.0, 1.0),
            p(1.0, 2.0),
            p(0.0, 2.0),
        ];
        let indices = triangulate_polygon(&l_shape);
        assert_eq!(indices.len(), (l_shape.len() - 2) * 3);
        assert_valid_indices(&indices, l_shape.len());
    }

    #[test]
    fn cross_2d_sign_tracks_turn_direction() {
        let o = p(0.0, 0.0);
        let a = p(1.0, 0.0);
        // Right turn vs left turn have opposite signs; collinear is zero.
        assert!(cross_2d(o, a, p(1.0, 1.0)) > 0.0);
        assert!(cross_2d(o, a, p(1.0, -1.0)) < 0.0);
        assert_eq!(cross_2d(o, a, p(2.0, 0.0)), 0.0);
    }

    #[test]
    fn point_in_triangle_detects_inside_outside_and_boundary() {
        let a = p(0.0, 0.0);
        let b = p(4.0, 0.0);
        let c = p(0.0, 4.0);
        assert!(point_in_triangle(p(1.0, 1.0), a, b, c), "interior point");
        assert!(!point_in_triangle(p(3.0, 3.0), a, b, c), "exterior point");
        // A point on an edge is treated as inside (no strictly-mixed signs).
        assert!(point_in_triangle(p(2.0, 0.0), a, b, c), "edge point");
    }

    #[test]
    fn is_ear_accepts_convex_corner_and_rejects_interior_containing_corner() {
        // This square winds CCW under triangulate_polygon's shoelace sign
        // convention (screen coords, y-down), so pass ccw = true.
        let verts = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        let idx = [0usize, 1, 2, 3];
        // Corner 0->1->2 is a convex ear and vertex 3 lies outside triangle(0,1,2).
        assert!(is_ear(&verts, &idx, 0, 1, 2, true));

        // A concave quad where the tested corner's triangle swallows the reflex
        // vertex must be rejected as an ear. Diamond with vertex 2 pulled inward.
        let concave = [p(0.0, 0.0), p(2.0, 1.0), p(1.0, 1.0), p(0.0, 2.0)];
        let cidx = [0usize, 1, 2, 3];
        // Triangle(3,0,1) contains the reflex vertex 2, so corner 0 is not an ear.
        assert!(!is_ear(&concave, &cidx, 3, 0, 1, true));
    }
}
