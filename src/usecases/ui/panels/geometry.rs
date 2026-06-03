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

    // Build mesh for the filled area
    let mut mesh = egui::Mesh::default();
    mesh.texture_id = egui::TextureId::default();
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
pub(super) fn triangulate_polygon(verts: &[egui::Pos2]) -> Vec<u32> {
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

pub(super) fn cross_2d(o: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

pub(super) fn is_ear(
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
    } else {
        if cross >= 0.0 {
            return false;
        }
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

pub(super) fn point_in_triangle(
    p: egui::Pos2,
    a: egui::Pos2,
    b: egui::Pos2,
    c: egui::Pos2,
) -> bool {
    let d0 = cross_2d(a, b, p);
    let d1 = cross_2d(b, c, p);
    let d2 = cross_2d(c, a, p);
    let has_neg = (d0 < 0.0) || (d1 < 0.0) || (d2 < 0.0);
    let has_pos = (d0 > 0.0) || (d1 > 0.0) || (d2 > 0.0);
    !(has_neg && has_pos)
}
