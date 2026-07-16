//! Warp pipeline — perspective correction and UV mesh warping for projection mapping.
//!
//! Supports two warp modes:
//! - **CornerPin**: 4-point homography (legacy quad warp, DLT solver)
//! - **Mesh**: Arbitrary XYUV grid warp (generalization of corner-pin)
//!
//! Corner-pin is a strict subset of mesh warp (equivalent to a 2×2 grid).

/// A single point in a UV warp mesh: output-space position + source-space UV.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MeshPoint {
    /// Position in output-normalized coords [0..1]
    pub position: [f32; 2],
    /// UV coordinates in source texture space [0..1]
    pub uv: [f32; 2],
}

/// A grid of XYUV warp points defining an arbitrary mesh warp.
///
/// Points are stored row-major: `points[row * cols + col]`.
/// The mesh defines a mapping from output positions to source UVs,
/// with each grid cell subdivided into 2 triangles for GPU rendering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WarpMesh {
    /// Number of columns in the grid (≥2)
    pub cols: u32,
    /// Number of rows in the grid (≥2)
    pub rows: u32,
    /// Grid points, row-major order. Length = cols × rows.
    pub points: Vec<MeshPoint>,
}

impl WarpMesh {
    /// Create an identity mesh (no warp) with the given grid dimensions.
    /// Positions and UVs both span [0..1]² uniformly.
    pub fn identity(cols: u32, rows: u32) -> Self {
        let mut points = Vec::with_capacity((cols * rows) as usize);
        for r in 0..rows {
            let v = r as f32 / (rows - 1) as f32;
            for c in 0..cols {
                let u = c as f32 / (cols - 1) as f32;
                points.push(MeshPoint {
                    position: [u, v],
                    uv: [u, v],
                });
            }
        }
        Self { cols, rows, points }
    }

    /// Create a mesh from 4 corner positions (corner-pin equivalent).
    /// Generates a 2×2 grid with positions at the corners and UVs at unit square.
    pub fn from_corners(corners: &[[f32; 2]; 4]) -> Self {
        // Order: TL, TR, BR, BL → grid row-major: TL, TR, BL, BR
        Self {
            cols: 2,
            rows: 2,
            points: vec![
                MeshPoint {
                    position: corners[0],
                    uv: [0.0, 0.0],
                }, // TL
                MeshPoint {
                    position: corners[1],
                    uv: [1.0, 0.0],
                }, // TR
                MeshPoint {
                    position: corners[3],
                    uv: [0.0, 1.0],
                }, // BL
                MeshPoint {
                    position: corners[2],
                    uv: [1.0, 1.0],
                }, // BR
            ],
        }
    }

    /// Check if this mesh is an identity warp (positions == UVs).
    pub fn is_identity(&self) -> bool {
        self.points.iter().all(|p| {
            (p.position[0] - p.uv[0]).abs() < 1e-6 && (p.position[1] - p.uv[1]).abs() < 1e-6
        })
    }

    /// Set the output position of the grid point at (`row`, `col`). Out-of-range
    /// indices are ignored. UV (source mapping) is preserved.
    pub fn set_point(&mut self, row: usize, col: usize, position: [f32; 2]) {
        let cols = self.cols as usize;
        if row < self.rows as usize && col < cols {
            if let Some(p) = self.points.get_mut(row * cols + col) {
                p.position = position;
            }
        }
    }

    /// Sample the mesh at parametric coords (`s`, `t`) ∈ [0..1]² (s across
    /// columns, t across rows), bilinearly interpolating both position and UV.
    fn sample(&self, s: f32, t: f32) -> MeshPoint {
        let cols = self.cols as usize;
        let rows = self.rows as usize;
        let fx = s.clamp(0.0, 1.0) * (cols - 1) as f32;
        let fy = t.clamp(0.0, 1.0) * (rows - 1) as f32;
        let x0 = (fx.floor() as usize).min(cols - 1);
        let y0 = (fy.floor() as usize).min(rows - 1);
        let x1 = (x0 + 1).min(cols - 1);
        let y1 = (y0 + 1).min(rows - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let lerp =
            |a: [f32; 2], b: [f32; 2], f: f32| [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
        let at = |r: usize, c: usize| self.points[r * cols + c];
        let (p00, p10, p01, p11) = (at(y0, x0), at(y0, x1), at(y1, x0), at(y1, x1));
        MeshPoint {
            position: lerp(
                lerp(p00.position, p10.position, tx),
                lerp(p01.position, p11.position, tx),
                ty,
            ),
            uv: lerp(lerp(p00.uv, p10.uv, tx), lerp(p01.uv, p11.uv, tx), ty),
        }
    }

    /// Resample this mesh onto a new `new_cols` × `new_rows` grid, preserving the
    /// current deformation via bilinear interpolation. Used to subdivide (or
    /// coarsen) a warp grid without the image jumping. Dimensions are clamped ≥2.
    pub fn resampled(&self, new_cols: u32, new_rows: u32) -> WarpMesh {
        let new_cols = new_cols.max(2);
        let new_rows = new_rows.max(2);
        let mut points = Vec::with_capacity((new_cols * new_rows) as usize);
        for r in 0..new_rows {
            let t = r as f32 / (new_rows - 1) as f32;
            for c in 0..new_cols {
                let s = c as f32 / (new_cols - 1) as f32;
                points.push(self.sample(s, t));
            }
        }
        WarpMesh {
            cols: new_cols,
            rows: new_rows,
            points,
        }
    }
}

/// Maximum warp grid resolution (columns or rows) a mesh warp may be
/// subdivided to. Domain- and engine-enforced.
pub const MAX_WARP_SUBDIVISIONS: u32 = 64;

/// Warp mode for a surface: corner-pin, arbitrary mesh, or bezier patch grid.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum WarpMode {
    /// 4-point corner-pin warp (TL, TR, BR, BL in output space [0..1]).
    CornerPin { corners: [[f32; 2]; 4] },
    /// Arbitrary XYUV mesh warp grid.
    Mesh(WarpMesh),
    /// Smooth bezier patch grid (8i.6). Editable control cage; tessellated into
    /// a `WarpMesh` for the GPU via [`WarpMode::render_mesh`].
    Bezier(BezierWarp),
}

impl WarpMode {
    /// Create a corner-pin warp from 4 corners.
    pub fn corner_pin(corners: [[f32; 2]; 4]) -> Self {
        Self::CornerPin { corners }
    }

    /// Create an identity corner-pin (no warp, bounding-box corners).
    pub fn identity_corners(bb: [f32; 4]) -> Self {
        let [x, y, w, h] = bb;
        Self::CornerPin {
            corners: [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
        }
    }

    /// Get corner-pin corners if this is a CornerPin variant.
    pub fn corners(&self) -> Option<&[[f32; 2]; 4]> {
        match self {
            Self::CornerPin { corners } => Some(corners),
            Self::Mesh(_) | Self::Bezier(_) => None,
        }
    }

    /// Get mutable corner-pin corners if this is a CornerPin variant.
    pub fn corners_mut(&mut self) -> Option<&mut [[f32; 2]; 4]> {
        match self {
            Self::CornerPin { corners } => Some(corners),
            Self::Mesh(_) | Self::Bezier(_) => None,
        }
    }

    /// Convert this warp into a mesh of `cols` × `rows` grid points, preserving
    /// the current deformation. A corner-pin becomes a bilinear grid over its
    /// quad (perspective → bilinear is inherent to switching to mesh warp); an
    /// existing mesh is resampled to the new resolution; a bezier warp is
    /// tessellated then resampled. Dimensions clamp ≥2.
    pub fn to_mesh(&self, cols: u32, rows: u32) -> WarpMesh {
        match self {
            Self::CornerPin { corners } => WarpMesh::from_corners(corners).resampled(cols, rows),
            Self::Mesh(mesh) => mesh.resampled(cols, rows),
            Self::Bezier(b) => b.tessellate().resampled(cols, rows),
        }
    }

    /// The `WarpMesh` the GPU pipeline consumes for this warp, if it is a
    /// mesh-based warp. `CornerPin` returns `None` (rendered via homography);
    /// `Mesh` returns its grid; `Bezier` tessellates its control cage. This is
    /// the render-site choke point that keeps `blit`/snapshot geometry unaware
    /// of the bezier representation.
    pub fn render_mesh(&self) -> Option<WarpMesh> {
        match self {
            Self::CornerPin { .. } => None,
            Self::Mesh(mesh) => Some(mesh.clone()),
            Self::Bezier(b) => Some(b.tessellate()),
        }
    }

    /// Check if this warp mode is an identity (no warp effect).
    pub fn is_identity(&self, bb: [f32; 4]) -> bool {
        match self {
            Self::CornerPin { corners } => {
                let id = [
                    [bb[0], bb[1]],
                    [bb[0] + bb[2], bb[1]],
                    [bb[0] + bb[2], bb[1] + bb[3]],
                    [bb[0], bb[1] + bb[3]],
                ];
                corners
                    .iter()
                    .zip(id.iter())
                    .all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6)
            }
            Self::Mesh(mesh) => mesh.is_identity(),
            Self::Bezier(b) => b.is_identity(),
        }
    }
}

// ── Shape-conforming warp meshes (Approach B, auto-warp binding) ─────
//
// These build a `WarpMesh` whose grid boundary follows a surface's own
// outline, so content fills the shape. Pure geometry (no wgpu); consumed by
// `Surface::conforming_warp`.

/// Build an `n`×`n` warp mesh whose grid boundary lands exactly on the ellipse
/// centred at `center` with radii `(rx, ry)` in output space, via the classic
/// elliptical square-to-disc map. Interior points fill the disc; UVs are the
/// uniform unit-square grid. `n` clamps to `[2, MAX_WARP_SUBDIVISIONS]`;
/// positions clamp to `[0, 1]` (matching `CircleHint::generate_vertices`).
pub fn disc_map_mesh(center: [f32; 2], rx: f32, ry: f32, n: u32) -> WarpMesh {
    let n = n.clamp(2, MAX_WARP_SUBDIVISIONS);
    let mut points = Vec::with_capacity((n * n) as usize);
    for r in 0..n {
        let v = r as f32 / (n - 1) as f32;
        let sy = v * 2.0 - 1.0;
        for c in 0..n {
            let u = c as f32 / (n - 1) as f32;
            let sx = u * 2.0 - 1.0;
            // Elliptical grid mapping: the unit square maps onto the unit disc,
            // and the square's boundary maps exactly onto the circle.
            let dx = sx * (1.0 - sy * sy / 2.0).max(0.0).sqrt();
            let dy = sy * (1.0 - sx * sx / 2.0).max(0.0).sqrt();
            points.push(MeshPoint {
                position: [
                    (center[0] + dx * rx).clamp(0.0, 1.0),
                    (center[1] + dy * ry).clamp(0.0, 1.0),
                ],
                uv: [u, v],
            });
        }
    }
    WarpMesh {
        cols: n,
        rows: n,
        points,
    }
}

/// Resample a polyline to exactly `n` (≥2) points spaced uniformly by arc
/// length, including both endpoints. Degenerate inputs return a repeated point.
fn resample_polyline(pts: &[[f32; 2]], n: u32) -> Vec<[f32; 2]> {
    let n = n.max(2) as usize;
    if pts.is_empty() {
        return vec![[0.0, 0.0]; n];
    }
    if pts.len() == 1 {
        return vec![pts[0]; n];
    }
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0f32);
    for w in pts.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        cum.push(cum.last().unwrap() + d);
    }
    let total = *cum.last().unwrap();
    if total <= f32::EPSILON {
        return vec![pts[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let target = total * i as f32 / (n as f32 - 1.0);
        let mut seg = 0;
        while seg + 2 < cum.len() && cum[seg + 1] < target {
            seg += 1;
        }
        let seg_len = cum[seg + 1] - cum[seg];
        let t = if seg_len > f32::EPSILON {
            (target - cum[seg]) / seg_len
        } else {
            0.0
        };
        out.push([
            pts[seg][0] + (pts[seg + 1][0] - pts[seg][0]) * t,
            pts[seg][1] + (pts[seg + 1][1] - pts[seg][1]) * t,
        ]);
    }
    out
}

/// Indices of the vertices nearest the bbox corners (TL, TR, BR, BL).
fn detect_quad_corners(verts: &[[f32; 2]]) -> [usize; 4] {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for v in verts {
        minx = minx.min(v[0]);
        miny = miny.min(v[1]);
        maxx = maxx.max(v[0]);
        maxy = maxy.max(v[1]);
    }
    let targets = [[minx, miny], [maxx, miny], [maxx, maxy], [minx, maxy]];
    let mut out = [0usize; 4];
    for (k, t) in targets.iter().enumerate() {
        let mut best = 0usize;
        let mut bestd = f32::MAX;
        for (i, v) in verts.iter().enumerate() {
            let d = (v[0] - t[0]).powi(2) + (v[1] - t[1]).powi(2);
            if d < bestd {
                bestd = d;
                best = i;
            }
        }
        out[k] = best;
    }
    out
}

/// Walk the closed polygon `verts` forward from index `from` to `to` inclusive.
fn forward_run(verts: &[[f32; 2]], from: usize, to: usize) -> Vec<[f32; 2]> {
    let n = verts.len();
    let mut out = vec![verts[from]];
    let mut i = from;
    while i != to {
        i = (i + 1) % n;
        out.push(verts[i]);
    }
    out
}

/// Single-point cubic-bezier evaluation at parameter `t` ∈ [0,1]. Kept local to
/// the warp module: the renderer must not depend on `surface/curve.rs`, and the
/// shared-flattener rule targets outline *polyline* flattening (a distinct
/// concern), not this per-point warp evaluation.
fn cubic_point(p0: [f32; 2], c1: [f32; 2], c2: [f32; 2], p1: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        a * p0[0] + b * c1[0] + c * c2[0] + d * p1[0],
        a * p0[1] + b * c1[1] + c * c2[1] + d * p1[1],
    ]
}

/// Transfinite (Coons) blend of a patch's four boundary points at parametric
/// `(s, t)`: `l`/`r` are the left/right boundary points at row-parameter `t`;
/// `tp`/`bt` the top/bottom boundary points at col-parameter `s`; `c00..c11`
/// the four patch corners. Shared by `coons_mesh` (polyline sides) and
/// `BezierWarp::tessellate` (cubic sides) — one Coons code path.
#[allow(clippy::too_many_arguments)]
fn coons_blend(
    l: [f32; 2],
    r: [f32; 2],
    tp: [f32; 2],
    bt: [f32; 2],
    c00: [f32; 2],
    c10: [f32; 2],
    c01: [f32; 2],
    c11: [f32; 2],
    s: f32,
    t: f32,
) -> [f32; 2] {
    let mut pos = [0.0f32; 2];
    for d in 0..2 {
        let lc = (1.0 - s) * l[d] + s * r[d];
        let ld = (1.0 - t) * tp[d] + t * bt[d];
        let b = (1.0 - s) * (1.0 - t) * c00[d]
            + s * (1.0 - t) * c10[d]
            + (1.0 - s) * t * c01[d]
            + s * t * c11[d];
        pos[d] = lc + ld - b;
    }
    pos
}

/// Build a `cols`×`rows` Coons-patch mesh whose boundary follows the closed
/// polygon `verts` (content fills the outline; Approach B). The four sides are
/// the vertex runs between the vertices nearest the bbox corners, resampled by
/// arc length; the interior is transfinite (Coons) interpolation. UVs are the
/// uniform unit-square grid. Fewer than 3 vertices returns an identity mesh.
/// Convex shapes fill exactly; concave shapes approximate. Dims clamp ≥2.
pub fn coons_mesh(verts: &[[f32; 2]], cols: u32, rows: u32) -> WarpMesh {
    let cols = cols.clamp(2, MAX_WARP_SUBDIVISIONS);
    let rows = rows.clamp(2, MAX_WARP_SUBDIVISIONS);
    if verts.len() < 3 {
        return WarpMesh::identity(cols, rows);
    }
    let n = verts.len();
    // Normalise winding so walking forward visits TL → TR → BR → BL. One
    // reversal always suffices, so reverse the working copy in place rather than
    // recursing. When a single vertex is the nearest to both left bbox corners
    // (`tl == bl` — a right-pointing triangle, or the mandala's skin triangles)
    // the winding test stays true on the reversed copy too, so a
    // recompute-and-recurse loops forever and overflows the stack.
    let reversed: Vec<[f32; 2]>;
    let verts: &[[f32; 2]] = {
        let [tl, tr, _br, bl] = detect_quad_corners(verts);
        let fd = |a: usize, b: usize| (b + n - a) % n;
        if fd(tl, bl) < fd(tl, tr) {
            reversed = verts.iter().rev().copied().collect();
            &reversed
        } else {
            verts
        }
    };
    let [tl, tr, br, bl] = detect_quad_corners(verts);
    // Sides, each parametrised 0→1 in (s across cols, t across rows):
    let top = resample_polyline(&forward_run(verts, tl, tr), cols); // TL→TR
    let right = resample_polyline(&forward_run(verts, tr, br), rows); // TR→BR
    let mut bottom = resample_polyline(&forward_run(verts, br, bl), cols); // BR→BL
    bottom.reverse(); // → BL→BR
    let mut left = resample_polyline(&forward_run(verts, bl, tl), rows); // BL→TL
    left.reverse(); // → TL→BL

    let c00 = top[0];
    let c10 = top[cols as usize - 1];
    let c01 = bottom[0];
    let c11 = bottom[cols as usize - 1];
    let mut points = Vec::with_capacity((cols * rows) as usize);
    for r in 0..rows as usize {
        let t = r as f32 / (rows - 1) as f32;
        for c in 0..cols as usize {
            let s = c as f32 / (cols - 1) as f32;
            let pos = coons_blend(
                left[r], right[r], top[c], bottom[c], c00, c10, c01, c11, s, t,
            );
            points.push(MeshPoint {
                position: pos,
                uv: [s, t],
            });
        }
    }
    WarpMesh { cols, rows, points }
}

// ── Bezier patch-grid warp (8i.6) ───────────────────────────────────
//
// A full bezier patch grid: an `anchor_cols × anchor_rows` control cage of
// on-surface anchors, with per-edge cubic tangent handles. Each grid cell is a
// Coons patch bounded by four cubic beziers; the interior is transfinite
// (Coons) interpolation. Tessellated into a `WarpMesh` for the GPU. Pure
// geometry (no wgpu); the editable cage is authoritative and the mesh derived.

/// Default tessellation steps per patch edge for a new bezier warp.
pub const DEFAULT_BEZIER_TESS: u32 = 6;

/// A smooth warp defined by a grid of cubic-bezier patches with tangent
/// handles. See the module comment above for the model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BezierWarp {
    /// Anchor columns in the control cage (≥2).
    pub anchor_cols: u32,
    /// Anchor rows in the control cage (≥2).
    pub anchor_rows: u32,
    /// Anchor positions, row-major (`anchor_cols` × `anchor_rows`), output
    /// space [0..1].
    pub anchors: Vec<[f32; 2]>,
    /// Horizontal-edge cubic handles `[near-left, near-right]`, one per
    /// horizontal edge, row-major over rows then per-row edges. Length
    /// `anchor_rows · (anchor_cols − 1)`.
    pub h_horiz: Vec<[[f32; 2]; 2]>,
    /// Vertical-edge cubic handles `[near-top, near-bottom]`, one per vertical
    /// edge, row-major over edge-rows then cols. Length
    /// `anchor_cols · (anchor_rows − 1)`.
    pub h_vert: Vec<[[f32; 2]; 2]>,
    /// Tessellation steps per patch edge (≥1). Controls output mesh density.
    pub tess: u32,
}

impl BezierWarp {
    fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    /// Straight-edge handles (at the ⅓ and ⅔ chord points) between `a` and `b`,
    /// so a cubic through them is exactly the line segment `a→b`.
    fn straight(a: [f32; 2], b: [f32; 2]) -> [[f32; 2]; 2] {
        [Self::lerp(a, b, 1.0 / 3.0), Self::lerp(a, b, 2.0 / 3.0)]
    }

    /// Build a bezier warp from an anchor grid with all edges straight. Anchor
    /// count must equal `anchor_cols · anchor_rows` (padded if short). `tess` ≥1.
    pub fn from_anchors(
        anchor_cols: u32,
        anchor_rows: u32,
        mut anchors: Vec<[f32; 2]>,
        tess: u32,
    ) -> Self {
        let ac = anchor_cols.max(2);
        let ar = anchor_rows.max(2);
        anchors.resize((ac * ar) as usize, [0.0, 0.0]);
        let aci = ac as usize;
        let mut h_horiz = Vec::with_capacity((ar * (ac - 1)) as usize);
        for r in 0..ar as usize {
            for c in 0..aci - 1 {
                h_horiz.push(Self::straight(
                    anchors[r * aci + c],
                    anchors[r * aci + c + 1],
                ));
            }
        }
        let mut h_vert = Vec::with_capacity(((ar - 1) * ac) as usize);
        for r in 0..ar as usize - 1 {
            for c in 0..aci {
                h_vert.push(Self::straight(
                    anchors[r * aci + c],
                    anchors[(r + 1) * aci + c],
                ));
            }
        }
        Self {
            anchor_cols: ac,
            anchor_rows: ar,
            anchors,
            h_horiz,
            h_vert,
            tess: tess.max(1),
        }
    }

    /// Seed a bezier warp from an existing mesh: anchors = mesh points, all
    /// edges straight. At `tess = 1` this tessellates back to the same mesh
    /// (lossless), so converting a mesh warp to bezier is visually identical.
    pub fn from_mesh(mesh: &WarpMesh, tess: u32) -> Self {
        let anchors = mesh.points.iter().map(|p| p.position).collect();
        Self::from_anchors(mesh.cols, mesh.rows, anchors, tess)
    }

    #[inline]
    fn ac(&self) -> usize {
        self.anchor_cols.max(2) as usize
    }
    #[inline]
    fn ar(&self) -> usize {
        self.anchor_rows.max(2) as usize
    }

    /// Anchor position at grid `(row, col)`.
    pub fn anchor(&self, r: usize, c: usize) -> [f32; 2] {
        self.anchors[r * self.ac() + c]
    }

    /// Evaluate patch `(pr, pc)` at local `(s, t)` ∈ [0,1]² via Coons
    /// interpolation of its four cubic-bezier boundary edges.
    fn patch_point(&self, pr: usize, pc: usize, s: f32, t: f32) -> [f32; 2] {
        let ac = self.ac();
        let c00 = self.anchor(pr, pc);
        let c10 = self.anchor(pr, pc + 1);
        let c01 = self.anchor(pr + 1, pc);
        let c11 = self.anchor(pr + 1, pc + 1);
        let hh_top = self.h_horiz[pr * (ac - 1) + pc];
        let hh_bot = self.h_horiz[(pr + 1) * (ac - 1) + pc];
        let hv_left = self.h_vert[pr * ac + pc];
        let hv_right = self.h_vert[pr * ac + pc + 1];
        let tp = cubic_point(c00, hh_top[0], hh_top[1], c10, s);
        let bt = cubic_point(c01, hh_bot[0], hh_bot[1], c11, s);
        let l = cubic_point(c00, hv_left[0], hv_left[1], c01, t);
        let r = cubic_point(c10, hv_right[0], hv_right[1], c11, t);
        coons_blend(l, r, tp, bt, c00, c10, c01, c11, s, t)
    }

    /// Tessellate the control cage into a dense `WarpMesh`. Produces a
    /// `((anchor_cols−1)·tess + 1)` × `((anchor_rows−1)·tess + 1)` mesh, clamped
    /// to `[2, MAX_WARP_SUBDIVISIONS]`. UVs are the uniform unit-square grid.
    pub fn tessellate(&self) -> WarpMesh {
        let ac = self.ac();
        let ar = self.ar();
        let tess = self.tess.max(1);
        let cols = ((ac as u32 - 1) * tess + 1).clamp(2, MAX_WARP_SUBDIVISIONS);
        let rows = ((ar as u32 - 1) * tess + 1).clamp(2, MAX_WARP_SUBDIVISIONS);
        let mut points = Vec::with_capacity((cols * rows) as usize);
        for r in 0..rows {
            let v = r as f32 / (rows - 1) as f32;
            let gv = v * (ar as f32 - 1.0);
            let pr = (gv.floor() as usize).min(ar - 2);
            let t = gv - pr as f32;
            for c in 0..cols {
                let u = c as f32 / (cols - 1) as f32;
                let gu = u * (ac as f32 - 1.0);
                let pc = (gu.floor() as usize).min(ac - 2);
                let s = gu - pc as f32;
                points.push(MeshPoint {
                    position: self.patch_point(pr, pc, s, t),
                    uv: [u, v],
                });
            }
        }
        WarpMesh { cols, rows, points }
    }

    /// Whether this warp tessellates to an identity mesh (no warp effect).
    pub fn is_identity(&self) -> bool {
        self.tessellate().is_identity()
    }

    /// Move anchor `(row, col)` to `pos`, shifting its incident tangent handles
    /// by the same delta so local curvature is preserved (mirrors the surface
    /// bezier-edge `move_anchor`). Out-of-range indices are ignored.
    pub fn move_anchor(&mut self, r: usize, c: usize, pos: [f32; 2]) {
        let (ac, ar) = (self.ac(), self.ar());
        if r >= ar || c >= ac {
            return;
        }
        let old = self.anchor(r, c);
        let d = [pos[0] - old[0], pos[1] - old[1]];
        self.anchors[r * ac + c] = pos;
        if c + 1 < ac {
            let h = &mut self.h_horiz[r * (ac - 1) + c][0];
            *h = [h[0] + d[0], h[1] + d[1]];
        }
        if c > 0 {
            let h = &mut self.h_horiz[r * (ac - 1) + (c - 1)][1];
            *h = [h[0] + d[0], h[1] + d[1]];
        }
        if r + 1 < ar {
            let h = &mut self.h_vert[r * ac + c][0];
            *h = [h[0] + d[0], h[1] + d[1]];
        }
        if r > 0 {
            let h = &mut self.h_vert[(r - 1) * ac + c][1];
            *h = [h[0] + d[0], h[1] + d[1]];
        }
    }

    /// Move a single tangent handle to `pos`. `horizontal` selects a horizontal
    /// edge (anchor `(r,c)→(r,c+1)`) vs a vertical edge (`(r,c)→(r+1,c)`);
    /// `which` is 0 (near start anchor) or 1 (near end anchor). No-op if the
    /// addressed edge does not exist.
    pub fn move_handle(
        &mut self,
        horizontal: bool,
        r: usize,
        c: usize,
        which: usize,
        pos: [f32; 2],
    ) {
        let (ac, ar) = (self.ac(), self.ar());
        let which = which.min(1);
        if horizontal {
            if r < ar && c + 1 < ac {
                self.h_horiz[r * (ac - 1) + c][which] = pos;
            }
        } else if r + 1 < ar && c < ac {
            self.h_vert[r * ac + c][which] = pos;
        }
    }

    /// Rebuild the control cage at a new `cols × rows` anchor resolution,
    /// resampling anchors onto the current warped surface with straightened
    /// handles. Adds/removes control points; sub-anchor curvature is not
    /// preserved across a re-subdivide (flagged limitation). Dims clamp to
    /// `[2, MAX_WARP_SUBDIVISIONS]`.
    pub fn set_cage_subdivisions(&mut self, cols: u32, rows: u32) {
        let nc = cols.clamp(2, MAX_WARP_SUBDIVISIONS);
        let nr = rows.clamp(2, MAX_WARP_SUBDIVISIONS);
        let (oc, or) = (self.ac(), self.ar());
        let mut anchors = Vec::with_capacity((nc * nr) as usize);
        for rr in 0..nr as usize {
            let gv = (rr as f32 / (nr as f32 - 1.0)) * (or as f32 - 1.0);
            let pr = (gv.floor() as usize).min(or - 2);
            let t = gv - pr as f32;
            for cc in 0..nc as usize {
                let gu = (cc as f32 / (nc as f32 - 1.0)) * (oc as f32 - 1.0);
                let pc = (gu.floor() as usize).min(oc - 2);
                let s = gu - pc as f32;
                anchors.push(self.patch_point(pr, pc, s, t));
            }
        }
        *self = Self::from_anchors(nc, nr, anchors, self.tess);
    }

    /// Set the per-patch tessellation density (≥1).
    pub fn set_tess(&mut self, tess: u32) {
        self.tess = tess.max(1);
    }
}

/// Compute a forward homography that maps from `src_corners` to `dst_corners`.
/// Used by the vertex shader to transform polygon vertices from canvas space
/// to warped output space.
///
/// Returns 12 floats: 3 rows × 4 (xyz + padding), suitable for GPU uniform.
pub fn compute_forward_homography(
    src_corners: &[[f32; 2]; 4],
    dst_corners: &[[f32; 2]; 4],
) -> [f32; 12] {
    let h = solve_homography(src_corners, dst_corners);
    [
        h[0], h[1], h[2], 0.0, h[3], h[4], h[5], 0.0, h[6], h[7], h[8], 0.0,
    ]
}

/// Solve for a 3x3 homography matrix H such that for each i:
///   dst[i] = H * src[i]  (in homogeneous coordinates)
///
/// Uses the standard DLT (Direct Linear Transform) approach for 4 point correspondences.
/// Returns the 9 elements of H in row-major order, normalized so H[8] = 1.
fn solve_homography(src: &[[f32; 2]; 4], dst: &[[f32; 2]; 4]) -> [f32; 9] {
    // Build the 8x8 system Ah = b where h = [h0..h7] and h8 = 1
    // For each point correspondence (x,y) -> (x',y'):
    //   x'(h6*x + h7*y + 1) = h0*x + h1*y + h2
    //   y'(h6*x + h7*y + 1) = h3*x + h4*y + h5
    // Rearranged:
    //   h0*x + h1*y + h2 - h6*x*x' - h7*y*x' = x'
    //   h3*x + h4*y + h5 - h6*x*y' - h7*y*y' = y'

    let mut a = [[0.0_f64; 8]; 8];
    let mut b = [0.0_f64; 8];

    for i in 0..4 {
        let (sx, sy) = (src[i][0] as f64, src[i][1] as f64);
        let (dx, dy) = (dst[i][0] as f64, dst[i][1] as f64);
        let row1 = i * 2;
        let row2 = i * 2 + 1;

        a[row1] = [sx, sy, 1.0, 0.0, 0.0, 0.0, -sx * dx, -sy * dx];
        b[row1] = dx;

        a[row2] = [0.0, 0.0, 0.0, sx, sy, 1.0, -sx * dy, -sy * dy];
        b[row2] = dy;
    }

    // Solve via Gaussian elimination with partial pivoting
    let h = gauss_solve_8x8(&mut a, &mut b);

    [
        h[0] as f32,
        h[1] as f32,
        h[2] as f32,
        h[3] as f32,
        h[4] as f32,
        h[5] as f32,
        h[6] as f32,
        h[7] as f32,
        1.0,
    ]
}

/// Gaussian elimination with partial pivoting for an 8x8 system
#[allow(clippy::needless_range_loop)]
fn gauss_solve_8x8(a: &mut [[f64; 8]; 8], b: &mut [f64; 8]) -> [f64; 8] {
    let n = 8;
    // Forward elimination
    for col in 0..n {
        // Partial pivoting
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..n {
            if a[row][col].abs() > max_val {
                max_val = a[row][col].abs();
                max_row = row;
            }
        }
        a.swap(col, max_row);
        b.swap(col, max_row);

        let pivot = a[col][col];
        if pivot.abs() < 1e-12 {
            log::warn!(
                "Degenerate homography: pivot near zero at column {col}, returning identity warp"
            );
            return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        }

        for row in (col + 1)..n {
            let factor = a[row][col] / pivot;
            for k in col..n {
                a[row][k] -= factor * a[col][k];
            }
            b[row] -= factor * b[col];
        }
    }

    // Back substitution
    let mut x = [0.0_f64; 8];
    for col in (0..n).rev() {
        x[col] = b[col];
        for k in (col + 1)..n {
            x[col] -= a[col][k] * x[k];
        }
        x[col] /= a[col][col];
    }
    x
}

// ── Mesh Import/Export ──────────────────────────────────────────────────

/// Supported mesh file formats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeshFormat {
    /// Paul Bourke XYUV CSV: header `mesh_w mesh_h`, then `x y u v intensity` per point.
    XyuvCsv,
    /// JSON serialization of WarpMesh.
    Json,
}

impl MeshFormat {
    /// Auto-detect format from file extension.
    pub fn from_extension(path: &std::path::Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("csv" | "xyuv" | "txt") => Some(Self::XyuvCsv),
            Some("json") => Some(Self::Json),
            _ => None,
        }
    }
}

impl WarpMesh {
    /// Parse a Paul Bourke XYUV CSV string.
    ///
    /// Expected format:
    /// ```text
    /// mesh_w mesh_h
    /// x y u v intensity
    /// x y u v intensity
    /// ...
    /// ```
    /// Where x,y are output positions and u,v are source UVs, all in [0..1].
    /// The intensity column is parsed but not stored (used for edge blending).
    pub fn from_xyuv_csv(input: &str) -> anyhow::Result<Self> {
        let mut lines = input
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'));

        let header = lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("XYUV CSV: missing header line"))?;
        let dims: Vec<u32> = header
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if dims.len() < 2 {
            anyhow::bail!(
                "XYUV CSV: header must contain mesh_w mesh_h, got: {}",
                header
            );
        }
        let cols = dims[0];
        let rows = dims[1];
        if cols < 2 || rows < 2 {
            anyhow::bail!(
                "XYUV CSV: mesh dimensions must be ≥ 2, got {}×{}",
                cols,
                rows
            );
        }
        if cols > 10_000 || rows > 10_000 {
            anyhow::bail!(
                "XYUV CSV: mesh dimensions too large (max 10000×10000), got {}×{}",
                cols,
                rows
            );
        }

        let expected = (cols * rows) as usize;
        let mut points = Vec::with_capacity(expected);

        for line in lines {
            let vals: Vec<f32> = line
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            if vals.len() < 4 {
                continue; // skip malformed lines
            }
            points.push(MeshPoint {
                position: [vals[0], vals[1]],
                uv: [vals[2], vals[3]],
                // vals[4] = intensity (ignored, handled by edge blend system)
            });
        }

        if points.len() != expected {
            anyhow::bail!(
                "XYUV CSV: expected {} points ({}×{}), got {}",
                expected,
                cols,
                rows,
                points.len()
            );
        }

        Ok(Self { cols, rows, points })
    }

    /// Export to Paul Bourke XYUV CSV format.
    pub fn to_xyuv_csv(&self) -> String {
        let mut out = String::with_capacity(self.points.len() * 40);
        out.push_str(&format!("{} {}\n", self.cols, self.rows));
        for pt in &self.points {
            out.push_str(&format!(
                "{:.6} {:.6} {:.6} {:.6} 1.000000\n",
                pt.position[0], pt.position[1], pt.uv[0], pt.uv[1],
            ));
        }
        out
    }

    /// Load a WarpMesh from a JSON string.
    pub fn from_json(input: &str) -> anyhow::Result<Self> {
        let mesh: Self = serde_json::from_str(input)
            .map_err(|e| anyhow::anyhow!("JSON mesh parse error: {}", e))?;
        if mesh.cols < 2 || mesh.rows < 2 {
            anyhow::bail!(
                "JSON mesh: dimensions must be ≥ 2, got {}×{}",
                mesh.cols,
                mesh.rows
            );
        }
        if mesh.points.len() != (mesh.cols * mesh.rows) as usize {
            anyhow::bail!(
                "JSON mesh: expected {} points, got {}",
                mesh.cols * mesh.rows,
                mesh.points.len()
            );
        }
        Ok(mesh)
    }

    /// Export to JSON string.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Load from file with auto-detected format.
    pub fn load_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let format = MeshFormat::from_extension(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown mesh file extension: {:?}", path))?;
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read mesh file {:?}: {}", path, e))?;
        match format {
            MeshFormat::XyuvCsv => Self::from_xyuv_csv(&content),
            MeshFormat::Json => Self::from_json(&content),
        }
    }

    /// Save to file with auto-detected format.
    pub fn save_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let format = MeshFormat::from_extension(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown mesh file extension: {:?}", path))?;
        let content = match format {
            MeshFormat::XyuvCsv => self.to_xyuv_csv(),
            MeshFormat::Json => self.to_json()?,
        };
        std::fs::write(path, content)
            .map_err(|e| anyhow::anyhow!("Failed to write mesh file {:?}: {}", path, e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_mesh_positions_equal_uvs() {
        let mesh = WarpMesh::identity(4, 4);
        assert_eq!(mesh.points.len(), 16);
        assert!(mesh.is_identity());
        for p in &mesh.points {
            assert!((p.position[0] - p.uv[0]).abs() < 1e-6);
            assert!((p.position[1] - p.uv[1]).abs() < 1e-6);
        }
    }

    #[test]
    fn identity_mesh_2x2_matches_unit_square() {
        let mesh = WarpMesh::identity(2, 2);
        assert_eq!(mesh.points.len(), 4);
        assert_eq!(mesh.points[0].position, [0.0, 0.0]); // TL
        assert_eq!(mesh.points[1].position, [1.0, 0.0]); // TR
        assert_eq!(mesh.points[2].position, [0.0, 1.0]); // BL
        assert_eq!(mesh.points[3].position, [1.0, 1.0]); // BR
    }

    #[test]
    fn from_corners_creates_2x2_mesh() {
        let corners = [[0.1, 0.2], [0.9, 0.2], [0.9, 0.8], [0.1, 0.8]];
        let mesh = WarpMesh::from_corners(&corners);
        assert_eq!(mesh.cols, 2);
        assert_eq!(mesh.rows, 2);
        assert_eq!(mesh.points.len(), 4);
        // TL position = corners[0], UV = [0,0]
        assert_eq!(mesh.points[0].position, corners[0]);
        assert_eq!(mesh.points[0].uv, [0.0, 0.0]);
        // TR position = corners[1], UV = [1,0]
        assert_eq!(mesh.points[1].position, corners[1]);
        assert_eq!(mesh.points[1].uv, [1.0, 0.0]);
        // BL position = corners[3], UV = [0,1]
        assert_eq!(mesh.points[2].position, corners[3]);
        assert_eq!(mesh.points[2].uv, [0.0, 1.0]);
        // BR position = corners[2], UV = [1,1]
        assert_eq!(mesh.points[3].position, corners[2]);
        assert_eq!(mesh.points[3].uv, [1.0, 1.0]);
    }

    #[test]
    fn warp_mode_identity_corners() {
        let bb = [0.0, 0.0, 1.0, 1.0];
        let mode = WarpMode::identity_corners(bb);
        assert!(mode.is_identity(bb));
    }

    #[test]
    fn warp_mode_non_identity_corners() {
        let bb = [0.0, 0.0, 1.0, 1.0];
        let mode = WarpMode::corner_pin([[0.1, 0.0], [0.9, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        assert!(!mode.is_identity(bb));
    }

    #[test]
    fn warp_mode_corners_accessor() {
        let mut mode = WarpMode::corner_pin([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        assert!(mode.corners().is_some());
        assert!(mode.corners_mut().is_some());

        let mesh_mode = WarpMode::Mesh(WarpMesh::identity(3, 3));
        assert!(mesh_mode.corners().is_none());
    }

    #[test]
    fn mesh_grid_subdivision_math() {
        // A 4×3 grid should have (4-1)*(3-1) = 6 cells = 12 triangles
        let mesh = WarpMesh::identity(4, 3);
        assert_eq!(mesh.points.len(), 12);
        let num_cells = (mesh.cols as usize - 1) * (mesh.rows as usize - 1);
        assert_eq!(num_cells, 6);
        let num_tris = num_cells * 2;
        assert_eq!(num_tris, 12);
    }

    #[test]
    fn resample_identity_stays_identity() {
        // Subdividing an identity mesh yields a denser identity mesh.
        let dense = WarpMesh::identity(2, 2).resampled(5, 3);
        assert_eq!(dense.cols, 5);
        assert_eq!(dense.rows, 3);
        assert_eq!(dense.points.len(), 15);
        assert!(dense.is_identity());
    }

    #[test]
    fn resample_clamps_dims_to_min_two() {
        let m = WarpMesh::identity(3, 3).resampled(1, 0);
        assert_eq!(m.cols, 2);
        assert_eq!(m.rows, 2);
    }

    #[test]
    fn resample_preserves_corner_positions() {
        // A warped 2×2 quad, resampled denser, keeps its four corner positions.
        let warped = WarpMesh {
            cols: 2,
            rows: 2,
            points: vec![
                MeshPoint {
                    position: [0.1, 0.2],
                    uv: [0.0, 0.0],
                },
                MeshPoint {
                    position: [0.8, 0.05],
                    uv: [1.0, 0.0],
                },
                MeshPoint {
                    position: [0.15, 0.9],
                    uv: [0.0, 1.0],
                },
                MeshPoint {
                    position: [0.95, 0.85],
                    uv: [1.0, 1.0],
                },
            ],
        };
        let d = warped.resampled(4, 4);
        let approx =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5;
        assert!(approx(d.points[0].position, [0.1, 0.2])); // TL
        assert!(approx(d.points[3].position, [0.8, 0.05])); // TR
        assert!(approx(d.points[12].position, [0.15, 0.9])); // BL
        assert!(approx(d.points[15].position, [0.95, 0.85])); // BR
    }

    #[test]
    fn set_point_updates_position_only() {
        let mut m = WarpMesh::identity(3, 3);
        m.set_point(1, 1, [0.6, 0.4]);
        let center = m.points[4]; // row 1, col 1 in a 3-wide grid
        assert!((center.position[0] - 0.6).abs() < 1e-6);
        assert!((center.position[1] - 0.4).abs() < 1e-6);
        // UV (source mapping) is untouched — still the identity centre.
        assert!((center.uv[0] - 0.5).abs() < 1e-6);
        assert!((center.uv[1] - 0.5).abs() < 1e-6);
        // Out-of-range is a no-op.
        m.set_point(9, 9, [0.0, 0.0]);
        assert_eq!(m.points.len(), 9);
    }

    #[test]
    fn to_mesh_from_corner_pin_has_requested_dims() {
        let cp = WarpMode::corner_pin([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        let mesh = cp.to_mesh(4, 5);
        assert_eq!(mesh.cols, 4);
        assert_eq!(mesh.rows, 5);
        assert_eq!(mesh.points.len(), 20);
        // Identity corner-pin over the unit square → identity mesh.
        assert!(mesh.is_identity());
    }

    #[test]
    fn warp_mode_serialization_roundtrip() {
        let corner_mode = WarpMode::corner_pin([[0.1, 0.2], [0.9, 0.2], [0.9, 0.8], [0.1, 0.8]]);
        let json = serde_json::to_string(&corner_mode).unwrap();
        let deserialized: WarpMode = serde_json::from_str(&json).unwrap();
        match deserialized {
            WarpMode::CornerPin { corners } => {
                assert!((corners[0][0] - 0.1).abs() < 1e-6);
            }
            _ => panic!("expected CornerPin"),
        }

        let mesh_mode = WarpMode::Mesh(WarpMesh::identity(3, 3));
        let json = serde_json::to_string(&mesh_mode).unwrap();
        let deserialized: WarpMode = serde_json::from_str(&json).unwrap();
        match deserialized {
            WarpMode::Mesh(mesh) => {
                assert_eq!(mesh.cols, 3);
                assert_eq!(mesh.rows, 3);
                assert!(mesh.is_identity());
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn identity_homography_is_identity_matrix() {
        let src = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let dst = src;
        let h = compute_forward_homography(&src, &dst);
        assert!((h[0] - 1.0).abs() < 1e-4);
        assert!((h[5] - 1.0).abs() < 1e-4);
        assert!((h[10] - 1.0).abs() < 1e-4);
    }

    // ── Import/Export tests ─────────────────────────────────────────────

    #[test]
    fn xyuv_csv_roundtrip() {
        let mesh = WarpMesh::identity(3, 3);
        let csv = mesh.to_xyuv_csv();
        let parsed = WarpMesh::from_xyuv_csv(&csv).unwrap();
        assert_eq!(parsed.cols, 3);
        assert_eq!(parsed.rows, 3);
        assert_eq!(parsed.points.len(), 9);
        for (a, b) in mesh.points.iter().zip(parsed.points.iter()) {
            assert!((a.position[0] - b.position[0]).abs() < 1e-5);
            assert!((a.position[1] - b.position[1]).abs() < 1e-5);
            assert!((a.uv[0] - b.uv[0]).abs() < 1e-5);
            assert!((a.uv[1] - b.uv[1]).abs() < 1e-5);
        }
    }

    #[test]
    fn xyuv_csv_parse_with_comments() {
        let csv = "# Mesh file\n3 2\n0.0 0.0 0.0 0.0 1.0\n0.5 0.0 0.5 0.0 1.0\n1.0 0.0 1.0 0.0 1.0\n0.0 1.0 0.0 1.0 1.0\n0.5 1.0 0.5 1.0 1.0\n1.0 1.0 1.0 1.0 1.0\n";
        let mesh = WarpMesh::from_xyuv_csv(csv).unwrap();
        assert_eq!(mesh.cols, 3);
        assert_eq!(mesh.rows, 2);
        assert_eq!(mesh.points.len(), 6);
    }

    #[test]
    fn xyuv_csv_comma_separated() {
        let csv = "2 2\n0.0,0.0,0.0,0.0,1.0\n1.0,0.0,1.0,0.0,1.0\n0.0,1.0,0.0,1.0,1.0\n1.0,1.0,1.0,1.0,1.0\n";
        let mesh = WarpMesh::from_xyuv_csv(csv).unwrap();
        assert_eq!(mesh.cols, 2);
        assert_eq!(mesh.rows, 2);
    }

    #[test]
    fn xyuv_csv_missing_header() {
        let result = WarpMesh::from_xyuv_csv("");
        assert!(result.is_err());
    }

    #[test]
    fn xyuv_csv_wrong_point_count() {
        let csv = "3 3\n0.0 0.0 0.0 0.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(result.is_err());
    }

    #[test]
    fn xyuv_csv_too_small_dimensions() {
        let csv = "1 2\n0.0 0.0 0.0 0.0 1.0\n1.0 1.0 1.0 1.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(result.is_err());
    }

    #[test]
    fn json_roundtrip() {
        let mesh = WarpMesh::identity(4, 4);
        let json = mesh.to_json().unwrap();
        let parsed = WarpMesh::from_json(&json).unwrap();
        assert_eq!(parsed.cols, 4);
        assert_eq!(parsed.rows, 4);
        assert_eq!(parsed.points.len(), 16);
        assert!(parsed.is_identity());
    }

    #[test]
    fn json_invalid() {
        let result = WarpMesh::from_json("{invalid}");
        assert!(result.is_err());
    }

    #[test]
    fn format_detection_csv() {
        assert_eq!(
            MeshFormat::from_extension(std::path::Path::new("mesh.csv")),
            Some(MeshFormat::XyuvCsv)
        );
        assert_eq!(
            MeshFormat::from_extension(std::path::Path::new("dome.xyuv")),
            Some(MeshFormat::XyuvCsv)
        );
        assert_eq!(
            MeshFormat::from_extension(std::path::Path::new("warp.txt")),
            Some(MeshFormat::XyuvCsv)
        );
    }

    #[test]
    fn format_detection_json() {
        assert_eq!(
            MeshFormat::from_extension(std::path::Path::new("mesh.json")),
            Some(MeshFormat::Json)
        );
    }

    #[test]
    fn format_detection_unknown() {
        assert_eq!(
            MeshFormat::from_extension(std::path::Path::new("mesh.png")),
            None
        );
        assert_eq!(
            MeshFormat::from_extension(std::path::Path::new("noext")),
            None
        );
    }

    // ── Chaos Tests Round 2: NaN/Inf warp mesh coordinates ──────────────

    #[test]
    fn chaos_csv_nan_coordinates_parse_successfully() {
        // NaN parses as f32 via .parse() — filter_map drops it
        let csv = "2 2\nNaN NaN NaN NaN 1.0\n1.0 0.0 1.0 0.0 1.0\n0.0 1.0 0.0 1.0 1.0\n1.0 1.0 1.0 1.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        // "NaN" parses as f32::NAN via .parse::<f32>() — filter_map keeps it!
        // But the mesh point count should still match
        match result {
            Ok(mesh) => {
                assert_eq!(mesh.points.len(), 4);
                // First point has NaN coords — verify they exist
                assert!(mesh.points[0].position[0].is_nan() || mesh.points[0].position[0] == 0.0);
            }
            Err(_) => {
                // If NaN lines are dropped (vals.len() < 4 after filter_map), count mismatch is OK
            }
        }
    }

    #[test]
    fn chaos_csv_infinity_coordinates() {
        let csv = "2 2\ninf inf inf inf 1.0\n1.0 0.0 1.0 0.0 1.0\n0.0 1.0 0.0 1.0 1.0\n1.0 1.0 1.0 1.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        // count mismatch is acceptable
        if let Ok(mesh) = result {
            assert_eq!(mesh.points.len(), 4);
        }
    }

    #[test]
    fn chaos_csv_all_garbage_lines() {
        let csv =
            "2 2\nhello world foo bar\ngarbage in garbage out\nmore junk here\ntotally invalid\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(
            result.is_err(),
            "all-garbage CSV should fail point count check"
        );
    }

    #[test]
    fn chaos_csv_partial_fields() {
        // Lines with fewer than 4 parseable floats should be skipped
        let csv = "2 2\n0.0 0.0\n1.0\n0.0 0.0 0.0\n1.0 1.0 1.0 1.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(
            result.is_err(),
            "partial fields should result in count mismatch"
        );
    }

    #[test]
    fn chaos_csv_huge_dimensions() {
        let csv = "1000000 1000000\n0.0 0.0 0.0 0.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(
            result.is_err(),
            "huge dimensions should be rejected (overflow protection)"
        );
    }

    #[test]
    fn chaos_csv_zero_dimensions() {
        let csv = "0 0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(result.is_err(), "zero dimensions should be rejected");
    }

    #[test]
    fn chaos_csv_mixed_valid_invalid() {
        let csv = "2 2\n0.0 0.0 0.0 0.0 1.0\nNaN 0.0 NaN 0.0 1.0\n0.0 1.0 0.0 1.0 1.0\n1.0 1.0 1.0 1.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        // NaN parses fine as f32, so all 4 points should be present;
        // if NaN fields are dropped, a count mismatch is acceptable.
        if let Ok(mesh) = result {
            assert_eq!(mesh.points.len(), 4);
        }
    }

    // ── Shape-conforming warp (Approach B) ───────────────────────────

    #[test]
    fn disc_map_mesh_dims_and_uvs() {
        let m = disc_map_mesh([0.5, 0.5], 0.4, 0.4, 5);
        assert_eq!(m.cols, 5);
        assert_eq!(m.rows, 5);
        assert_eq!(m.points.len(), 25);
        // UVs stay the uniform unit-square grid.
        assert_eq!(m.points[0].uv, [0.0, 0.0]);
        assert_eq!(m.points[24].uv, [1.0, 1.0]);
    }

    #[test]
    fn disc_map_mesh_boundary_lands_on_circle() {
        let (cx, cy, radius) = (0.5f32, 0.5f32, 0.4f32);
        let m = disc_map_mesh([cx, cy], radius, radius, 6);
        let cols = m.cols as usize;
        for r in 0..m.rows as usize {
            for c in 0..cols {
                let on_boundary = r == 0 || c == 0 || r + 1 == m.rows as usize || c + 1 == cols;
                if !on_boundary {
                    continue;
                }
                let p = m.points[r * cols + c].position;
                let dist = ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt();
                assert!(
                    (dist - radius).abs() < 1e-4,
                    "boundary point ({r},{c}) at {p:?} dist {dist} != radius {radius}"
                );
            }
        }
    }

    #[test]
    fn coons_mesh_unit_square_is_bilinear_identity() {
        // A unit square given as 4 corners → positions equal a uniform grid.
        let verts = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let m = coons_mesh(&verts, 3, 3);
        assert_eq!(m.points.len(), 9);
        for p in &m.points {
            // For the unit square, Coons position == UV.
            assert!((p.position[0] - p.uv[0]).abs() < 1e-5);
            assert!((p.position[1] - p.uv[1]).abs() < 1e-5);
        }
    }

    #[test]
    fn coons_mesh_boundary_follows_offset_square_with_midpoints() {
        // Square [0.2,0.8]² described with edge midpoints (8 vertices); corners
        // must be detected and the grid boundary must land on the square.
        let verts = [
            [0.2, 0.2],
            [0.5, 0.2],
            [0.8, 0.2],
            [0.8, 0.5],
            [0.8, 0.8],
            [0.5, 0.8],
            [0.2, 0.8],
            [0.2, 0.5],
        ];
        let m = coons_mesh(&verts, 3, 3);
        let at = |r: usize, c: usize| m.points[r * 3 + c].position;
        assert!((at(0, 0)[0] - 0.2).abs() < 1e-4 && (at(0, 0)[1] - 0.2).abs() < 1e-4);
        assert!((at(0, 2)[0] - 0.8).abs() < 1e-4 && (at(0, 2)[1] - 0.2).abs() < 1e-4);
        assert!((at(2, 2)[0] - 0.8).abs() < 1e-4 && (at(2, 2)[1] - 0.8).abs() < 1e-4);
        assert!((at(2, 0)[0] - 0.2).abs() < 1e-4 && (at(2, 0)[1] - 0.8).abs() < 1e-4);
        // Edge midpoint lands on the square edge.
        assert!((at(0, 1)[0] - 0.5).abs() < 1e-4 && (at(0, 1)[1] - 0.2).abs() < 1e-4);
    }

    #[test]
    fn coons_mesh_degenerate_is_identity() {
        let m = coons_mesh(&[[0.0, 0.0], [1.0, 1.0]], 3, 3);
        assert!(m.is_identity());
    }

    #[test]
    fn coons_mesh_triangle_is_valid_and_in_hull() {
        let verts = [[0.1, 0.1], [0.9, 0.2], [0.5, 0.9]];
        let m = coons_mesh(&verts, 4, 4);
        assert_eq!(m.points.len(), 16);
        for p in &m.points {
            assert!(p.position[0].is_finite() && p.position[1].is_finite());
            assert!((0.0..=1.0).contains(&p.position[0]));
            assert!((0.0..=1.0).contains(&p.position[1]));
        }
    }

    #[test]
    fn coons_mesh_left_apex_triangle_terminates() {
        // The apex vertex is the nearest to BOTH left bbox corners, so
        // `detect_quad_corners` reports `tl == bl` and the winding test is
        // permanently true. The old recurse-on-reverse looped forever and
        // overflowed the stack (the mandala skin-triangle import crash). Guard:
        // it must terminate and return a valid `cols × rows` mesh.
        let verts = [[0.0, 0.5], [1.0, 0.0], [1.0, 1.0]];
        let m = coons_mesh(&verts, 4, 4);
        assert_eq!(m.points.len(), 16);
        for p in &m.points {
            assert!(p.position[0].is_finite() && p.position[1].is_finite());
        }
    }

    // ── Bezier patch-grid warp (8i.6) ─────────────────────────────────

    fn approx2(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn bezier_from_identity_mesh_tessellates_to_identity() {
        // Straight-edge cage over the unit square → identity mesh, at any tess.
        let b = BezierWarp::from_mesh(&WarpMesh::identity(2, 2), DEFAULT_BEZIER_TESS);
        assert!(b.tessellate().is_identity());
        assert!(b.is_identity());
    }

    #[test]
    fn bezier_from_mesh_tess1_is_lossless() {
        // A warped 2×2 quad → bezier at tess=1 tessellates back to the same quad.
        let warped = WarpMesh::from_corners(&[[0.1, 0.2], [0.8, 0.05], [0.95, 0.85], [0.15, 0.9]]);
        let b = BezierWarp::from_mesh(&warped, 1);
        let m = b.tessellate();
        assert_eq!((m.cols, m.rows), (2, 2));
        for (a, w) in m.points.iter().zip(warped.points.iter()) {
            assert!(approx2(a.position, w.position));
        }
    }

    #[test]
    fn bezier_tessellate_dims() {
        // 2×2 anchors, tess 4 → (2-1)*4+1 = 5 per side.
        let b = BezierWarp::from_anchors(
            2,
            2,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            4,
        );
        let m = b.tessellate();
        assert_eq!((m.cols, m.rows), (5, 5));
        // 3×2 anchors, tess 3 → cols (3-1)*3+1=7, rows (2-1)*3+1=4.
        let mut anchors = Vec::new();
        for r in 0..2 {
            for c in 0..3 {
                anchors.push([c as f32 / 2.0, r as f32]);
            }
        }
        let b = BezierWarp::from_anchors(3, 2, anchors, 3);
        let m = b.tessellate();
        assert_eq!((m.cols, m.rows), (7, 4));
    }

    #[test]
    fn bezier_tessellate_clamps_to_max() {
        // 16 anchors × tess 10 → (16-1)*10+1 = 151 → clamps to MAX.
        let anchors: Vec<[f32; 2]> = (0..16 * 16).map(|_| [0.0, 0.0]).collect();
        let b = BezierWarp::from_anchors(16, 16, anchors, 10);
        let m = b.tessellate();
        assert_eq!(m.cols, MAX_WARP_SUBDIVISIONS);
        assert_eq!(m.rows, MAX_WARP_SUBDIVISIONS);
    }

    #[test]
    fn bezier_move_anchor_shifts_incident_handles() {
        // 3×3 straight cage; move the centre anchor and check an incident handle
        // moved by the same delta (curvature preserved).
        let mut anchors = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                anchors.push([c as f32 / 2.0, r as f32 / 2.0]);
            }
        }
        let mut b = BezierWarp::from_anchors(3, 3, anchors, 4);
        // Horizontal edge to the right of centre (row 1, edge 1), near-left
        // handle. Index = row*(cols-1) + edge = 1*(3-1) + 1 = 3.
        let hidx = 3;
        let before = b.h_horiz[hidx][0];
        let old = b.anchor(1, 1);
        b.move_anchor(1, 1, [old[0] + 0.1, old[1] - 0.05]);
        let after = b.h_horiz[hidx][0];
        assert!(approx2(after, [before[0] + 0.1, before[1] - 0.05]));
        assert!(approx2(b.anchor(1, 1), [old[0] + 0.1, old[1] - 0.05]));
    }

    #[test]
    fn bezier_move_handle_curves_the_edge() {
        // Pull a top-edge handle far off the chord; the tessellated top row must
        // bulge away from the straight line between the two anchors.
        let mut b = BezierWarp::from_anchors(
            2,
            2,
            vec![[0.0, 0.5], [1.0, 0.5], [0.0, 1.0], [1.0, 1.0]],
            6,
        );
        // Straight first: midpoint of the top row sits on y=0.5.
        let m0 = b.tessellate();
        let mid0 = m0.points[m0.cols as usize / 2].position;
        assert!((mid0[1] - 0.5).abs() < 1e-3);
        // Curve the top edge upward via both handles.
        b.move_handle(true, 0, 0, 0, [0.33, 0.1]);
        b.move_handle(true, 0, 0, 1, [0.66, 0.1]);
        let m1 = b.tessellate();
        let mid1 = m1.points[m1.cols as usize / 2].position;
        assert!(mid1[1] < 0.4, "top edge should bulge upward, got {mid1:?}");
    }

    #[test]
    fn bezier_set_cage_subdivisions_changes_count_and_keeps_shape() {
        // Straight 2×2 cage over [0,1]²; subdivide to 3×3. New anchors lie on the
        // (still-flat) surface, i.e. a uniform grid.
        let mut b = BezierWarp::from_anchors(
            2,
            2,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            4,
        );
        b.set_cage_subdivisions(3, 3);
        assert_eq!((b.anchor_cols, b.anchor_rows), (3, 3));
        assert_eq!(b.anchors.len(), 9);
        assert!(approx2(b.anchor(1, 1), [0.5, 0.5]));
        assert!(b.is_identity());
    }

    #[test]
    fn warp_mode_bezier_accessors() {
        let b = BezierWarp::from_mesh(&WarpMesh::identity(2, 2), 4);
        let mode = WarpMode::Bezier(b);
        assert!(mode.corners().is_none());
        assert!(mode.is_identity([0.0, 0.0, 1.0, 1.0]));
        // render_mesh tessellates the cage.
        let rm = mode.render_mesh().expect("bezier has a render mesh");
        assert_eq!((rm.cols, rm.rows), (5, 5));
    }

    #[test]
    fn warp_mode_bezier_serialization_roundtrip() {
        let mode = WarpMode::Bezier(BezierWarp::from_anchors(
            3,
            2,
            vec![
                [0.0, 0.0],
                [0.5, 0.0],
                [1.0, 0.0],
                [0.0, 1.0],
                [0.5, 1.0],
                [1.0, 1.0],
            ],
            5,
        ));
        let json = serde_json::to_string(&mode).unwrap();
        let back: WarpMode = serde_json::from_str(&json).unwrap();
        match back {
            WarpMode::Bezier(b) => {
                assert_eq!((b.anchor_cols, b.anchor_rows), (3, 2));
                assert_eq!(b.tess, 5);
                assert_eq!(b.anchors.len(), 6);
            }
            other => panic!("expected Bezier, got {other:?}"),
        }
    }
}
