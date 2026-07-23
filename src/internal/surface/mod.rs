//! Surface — Named regions in a 2D stage model that content is routed to.
//!
//! Surfaces are the middle layer of the three-layer output abstraction:
//!   Content (channels, master) → Surfaces → Outputs (displays, projectors)
//!
//! Surfaces are polygons — an ordered list of vertices in normalized canvas
//! coordinates [0..1]. Rectangles are just 4-vertex polygons. This supports
//! triangles, circles (N-gon approximations), and arbitrary shapes.

pub mod curve;
pub mod detect;
pub mod import;
pub mod mask;

pub use crate::engine::value::surface::{
    CircleHint, ContentMapping, CubicHandle, PathSegment, SurfaceOutputType, SurfacePath,
    SurfaceReorderOp,
};

use crate::deck::generate_short_uuid;
use crate::renderer::context::OutputSource;
use serde::{Deserialize, Serialize};

impl CircleHint {
    /// Generate polygon vertices from this circle hint.
    pub fn generate_vertices(&self) -> Vec<[f32; 2]> {
        let sides = self.sides.max(3);
        (0..sides)
            .map(|i| {
                let angle = 2.0 * std::f32::consts::PI * i as f32 / sides as f32;
                [
                    (self.center[0] + angle.cos() * self.radius).clamp(0.0, 1.0),
                    (self.center[1] + angle.sin() * self.radius * self.aspect_ratio)
                        .clamp(0.0, 1.0),
                ]
            })
            .collect()
    }
}

/// A polygon surface in the 2D stage layout.
///
/// Represents a physical screen, LED panel, or projection area in the venue.
/// Content sources are routed to surfaces, and surfaces are mapped to physical outputs.
///
/// Vertices are ordered polygon points in normalized canvas coordinates [0..1],
/// where (0,0) is top-left of the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surface {
    /// Stable UUID for this surface (8-char hex, persists across moves/saves)
    #[serde(default = "generate_short_uuid")]
    pub uuid: String,
    /// Unique name (e.g., "Main Screen", "Left LED", "DJ Booth")
    pub name: String,
    /// Ordered polygon vertices in normalized canvas coordinates [0..1] (primary contour)
    pub vertices: Vec<[f32; 2]>,
    /// Additional contours for combined non-overlapping surfaces.
    /// Each entry is a separate polygon that is part of this same surface.
    #[serde(default)]
    pub extra_contours: Vec<Vec<[f32; 2]>>,
    /// What content this surface displays
    pub source: OutputSource,
    /// How the content maps onto this surface
    pub content_mapping: ContentMapping,
    /// Output type determines how this surface connects to physical hardware
    pub output_type: SurfaceOutputType,
    /// If present, this surface was created as a circle and supports radius/sides editing.
    /// Vertices are regenerated from the hint when radius or sides change.
    #[serde(default)]
    pub circle_hint: Option<CircleHint>,
    /// Per-surface warp (corner-pin or mesh). `None` = no warp (render at the
    /// polygon's native position). Promoted from the former `default_warp`
    /// template; the serde `alias` keeps pre-8i.5 `.varda` files loading.
    #[serde(default, alias = "default_warp")]
    pub warp: Option<crate::renderer::warp::WarpMode>,
    /// When `true` (default for surfaces created in-app), the warp auto-conforms
    /// to this surface's outline — `effective_warp()` derives it and `warp` is
    /// ignored. When `false`, `warp` is authoritative and manually editable.
    /// Legacy `.varda` files (no field) load as `false`, preserving any
    /// hand-authored `warp` untouched.
    #[serde(default)]
    pub warp_bound: bool,
    /// Optional curve authoring layer. When present, `vertices` is regenerated
    /// from this path (flattened) whenever the path is edited — mirroring
    /// `circle_hint`. `None` = the polygon in `vertices` is authoritative.
    #[serde(default)]
    pub path: Option<curve::SurfacePath>,
    /// Subtractive cut-out holes (8i.7). Each hole is an editable closed
    /// [`SurfacePath`] in canvas coords, cut out of the surface fill via a baked
    /// coverage mask. Empty = no cut-outs.
    #[serde(default)]
    pub holes: Vec<curve::SurfacePath>,
    /// Flattened cache of `holes`, regenerated on edit (mirrors `path →
    /// vertices`). Canvas coords; the renderer bakes these into a uv-space mask.
    #[serde(default)]
    pub hole_contours: Vec<Vec<[f32; 2]>>,
}

impl std::fmt::Display for ContentMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentMapping::Fill => write!(f, "Fill"),
            ContentMapping::Mapped => write!(f, "Mapped"),
        }
    }
}

/// Axis-aligned bounding box of a polygon, in normalized canvas coords.
#[derive(Debug, Clone, Copy)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Surface {
    /// Create a rectangular surface (4 vertices: TL, TR, BR, BL).
    pub fn new_rect(name: String, x: f32, y: f32, w: f32, h: f32, source: OutputSource) -> Self {
        Self {
            uuid: generate_short_uuid(),
            name,
            vertices: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            extra_contours: Vec::new(),
            source,
            content_mapping: ContentMapping::default(),
            output_type: SurfaceOutputType::Projection,
            circle_hint: None,
            warp: None,
            warp_bound: true,
            path: None,
            holes: Vec::new(),
            hole_contours: Vec::new(),
        }
    }

    /// Whether this surface is a circle (has a `CircleHint`).
    pub fn is_circle(&self) -> bool {
        self.circle_hint.is_some()
    }

    /// Regenerate vertices from the circle hint. No-op if not a circle.
    pub fn regenerate_circle_vertices(&mut self) {
        if let Some(hint) = &self.circle_hint {
            self.vertices = hint.generate_vertices();
        }
    }

    /// Drop circle identity, keeping current vertices as a plain polygon.
    pub fn convert_to_polygon(&mut self) {
        self.circle_hint = None;
    }

    /// The surface's warp, or an identity corner-pin seeded from its bounding
    /// box when it has none. Used as the base for warp editing and rendering.
    pub fn warp_or_identity(&self) -> crate::renderer::warp::WarpMode {
        self.warp.clone().unwrap_or_else(|| {
            let bb = self.bounding_box();
            crate::renderer::warp::WarpMode::identity_corners([bb.x, bb.y, bb.width, bb.height])
        })
    }

    /// Move one corner-pin corner (0..4), seeding an identity corner-pin first
    /// when the surface has no warp. No-op if the warp is currently a mesh.
    pub fn set_warp_corner(&mut self, corner_idx: usize, position: [f32; 2]) {
        if corner_idx >= 4 || matches!(self.warp, Some(crate::renderer::warp::WarpMode::Mesh(_))) {
            return;
        }
        let mut warp = self.warp_or_identity();
        if let Some(corners) = warp.corners_mut() {
            corners[corner_idx] = position;
        }
        self.warp = Some(warp);
    }

    /// Clear any warp (back to no-warp / native polygon position).
    pub fn reset_warp(&mut self) {
        self.warp = None;
    }

    /// The warp actually applied when rendering/displaying this surface. While
    /// `warp_bound`, it is derived from the shape (`conforming_warp`); otherwise
    /// the stored `warp`. Single choke point for render, snapshot, and editor.
    pub fn effective_warp(&self) -> Option<crate::renderer::warp::WarpMode> {
        if self.warp_bound {
            Some(self.conforming_warp())
        } else {
            self.warp.clone()
        }
    }

    /// A warp whose grid boundary conforms to this surface's outline (Approach
    /// B, fill semantics): circles → elliptical disc-map mesh; quads → a 2×2
    /// mesh at the four vertices; other polygons → a Coons-patch mesh over the
    /// vertices nearest the bbox corners.
    pub fn conforming_warp(&self) -> crate::renderer::warp::WarpMode {
        use crate::renderer::warp::{self, WarpMesh, WarpMode};
        if let Some(hint) = &self.circle_hint {
            let n = (hint.sides / 4 + 2).clamp(3, warp::MAX_WARP_SUBDIVISIONS);
            return WarpMode::Mesh(warp::disc_map_mesh(
                hint.center,
                hint.radius,
                hint.radius * hint.aspect_ratio,
                n,
            ));
        }
        let v = &self.vertices;
        if v.len() == 4 {
            return WarpMode::Mesh(WarpMesh::from_corners(&[v[0], v[1], v[2], v[3]]));
        }
        let n = (v.len() as u32).clamp(3, 16);
        WarpMode::Mesh(warp::coons_mesh(v, n, n))
    }

    /// Bind or unbind the warp from the surface shape. Unbinding materialises
    /// the conforming warp into `warp` so fine-tuning starts from the shape;
    /// binding clears `warp` (it is re-derived from the shape while bound).
    pub fn set_warp_bound(&mut self, bound: bool) {
        if bound {
            self.warp_bound = true;
            self.warp = None;
        } else {
            self.warp = Some(self.conforming_warp());
            self.warp_bound = false;
        }
    }

    /// Convert the warp to a `cols` × `rows` mesh, preserving the current
    /// deformation. Dimensions clamp to `[2, MAX_WARP_SUBDIVISIONS]`.
    pub fn set_warp_subdivisions(&mut self, cols: u32, rows: u32) {
        let cols = cols.clamp(2, crate::renderer::warp::MAX_WARP_SUBDIVISIONS);
        let rows = rows.clamp(2, crate::renderer::warp::MAX_WARP_SUBDIVISIONS);
        let base = self.warp_or_identity();
        self.warp = Some(crate::renderer::warp::WarpMode::Mesh(
            base.to_mesh(cols, rows),
        ));
    }

    /// Move a single mesh grid point (row-major). No-op if the warp is not a mesh.
    pub fn set_warp_mesh_point(&mut self, row: usize, col: usize, position: [f32; 2]) {
        if let Some(crate::renderer::warp::WarpMode::Mesh(mesh)) = &mut self.warp {
            mesh.set_point(row, col, position);
        }
    }

    /// Convert the current warp into a smooth bezier patch grid (8i.6), seeding
    /// the control cage from the current warp's mesh (or an identity 2×2 over the
    /// bbox), so the shape is preserved. No-op if the warp is already bezier.
    /// Meaningful only while unbound (manual editing); the caller ensures that.
    pub fn convert_warp_to_bezier(&mut self) {
        use crate::renderer::warp::{BezierWarp, WarpMode, DEFAULT_BEZIER_TESS};
        let base = self.warp_or_identity();
        if matches!(base, WarpMode::Bezier(_)) {
            return;
        }
        let (cols, rows) = match &base {
            WarpMode::Mesh(m) => (m.cols, m.rows),
            _ => (2, 2),
        };
        let mesh = base.to_mesh(cols, rows);
        self.warp = Some(WarpMode::Bezier(BezierWarp::from_mesh(
            &mesh,
            DEFAULT_BEZIER_TESS,
        )));
    }

    /// Move a bezier-warp anchor `(row, col)`. No-op if the warp is not bezier.
    pub fn set_warp_bezier_anchor(&mut self, row: usize, col: usize, position: [f32; 2]) {
        if let Some(crate::renderer::warp::WarpMode::Bezier(b)) = &mut self.warp {
            b.move_anchor(row, col, position);
        }
    }

    /// Move a bezier-warp tangent handle. `horizontal` picks a horizontal edge
    /// (`(r,c)→(r,c+1)`) vs a vertical edge (`(r,c)→(r+1,c)`); `which` is 0/1.
    /// No-op if the warp is not bezier.
    pub fn set_warp_bezier_handle(
        &mut self,
        horizontal: bool,
        row: usize,
        col: usize,
        which: usize,
        position: [f32; 2],
    ) {
        if let Some(crate::renderer::warp::WarpMode::Bezier(b)) = &mut self.warp {
            b.move_handle(horizontal, row, col, which, position);
        }
    }

    /// Set the bezier-warp control-cage resolution (anchor `cols` × `rows`),
    /// resampling onto the current surface. No-op if the warp is not bezier.
    pub fn set_bezier_cage_subdivisions(&mut self, cols: u32, rows: u32) {
        if let Some(crate::renderer::warp::WarpMode::Bezier(b)) = &mut self.warp {
            b.set_cage_subdivisions(cols, rows);
        }
    }

    /// Whether this surface has a curve authoring path.
    pub fn has_path(&self) -> bool {
        self.path.is_some()
    }

    /// Regenerate vertices from the curve path. No-op if there's no path.
    pub fn regenerate_from_path(&mut self) {
        if let Some(path) = &self.path {
            self.vertices = path.flatten();
        }
    }

    /// Ensure a curve authoring path exists, lazily building one from the current
    /// polygon vertices. Curve editing supersedes circle regeneration, so any
    /// `circle_hint` is dropped.
    pub fn ensure_path(&mut self) -> &mut curve::SurfacePath {
        if self.path.is_none() {
            self.path = Some(curve::SurfacePath::from_polygon(&self.vertices, true));
            self.circle_hint = None;
        }
        self.path.as_mut().unwrap()
    }

    /// Regenerate the flattened `hole_contours` cache from `holes` (mirrors
    /// `regenerate_from_path` for the outline). Call after any hole edit.
    pub fn regenerate_holes(&mut self) {
        self.hole_contours = self.holes.iter().map(|h| h.flatten()).collect();
    }

    /// Add a subtractive cut-out hole (8i.7) from a closed [`SurfacePath`] in
    /// canvas coords, refreshing the flattened contour cache.
    pub fn add_hole(&mut self, hole: curve::SurfacePath) {
        self.holes.push(hole);
        self.regenerate_holes();
    }

    /// Remove the hole at `index`. Returns true if a hole was removed.
    pub fn remove_hole(&mut self, index: usize) -> bool {
        if index < self.holes.len() {
            self.holes.remove(index);
            self.regenerate_holes();
            true
        } else {
            false
        }
    }

    /// Whether this surface has any subtractive holes.
    pub fn has_holes(&self) -> bool {
        !self.holes.is_empty()
    }

    /// Convert this surface's outer outline into a closed [`SurfacePath`] in
    /// canvas coords, suitable for use as a subtractive hole in another surface
    /// (8i.7 "Make Hole"). Clones the authoring `path` when present so
    /// bezier/curved outlines stay curved; otherwise builds straight-line
    /// segments from `vertices`.
    pub fn outline_as_path(&self) -> curve::SurfacePath {
        match &self.path {
            Some(p) => p.clone(),
            None => curve::SurfacePath::from_polygon(&self.vertices, true),
        }
    }

    /// Project the flattened `hole_contours` (canvas coords) into surface uv
    /// space (`[0..1]²`, bounding-box normalized) for mask baking. Returns empty
    /// when there are no holes or the bounding box is degenerate.
    pub fn hole_uv_contours(&self) -> Vec<Vec<[f32; 2]>> {
        if self.hole_contours.is_empty() {
            return Vec::new();
        }
        let bb = self.bounding_box();
        if bb.width <= 0.0 || bb.height <= 0.0 {
            return Vec::new();
        }
        self.hole_contours
            .iter()
            .map(|c| {
                c.iter()
                    .map(|p| [(p[0] - bb.x) / bb.width, (p[1] - bb.y) / bb.height])
                    .collect()
            })
            .collect()
    }

    /// Convert edge `edge_idx` of the curve path to a cubic bezier (`to_cubic`)
    /// or back to a straight line, regenerating vertices. Lazily creates a path.
    pub fn convert_edge(&mut self, edge_idx: usize, to_cubic: bool) {
        self.ensure_path();
        if let Some(path) = &mut self.path {
            if to_cubic {
                path.convert_edge_to_cubic(edge_idx);
            } else {
                path.convert_edge_to_line(edge_idx);
            }
        }
        self.regenerate_from_path();
    }

    /// Move curve anchor `anchor_idx` to `pos`, regenerating vertices. No-op if
    /// the surface has no curve path.
    pub fn move_path_anchor(&mut self, anchor_idx: usize, pos: [f32; 2]) {
        if let Some(path) = &mut self.path {
            path.move_anchor(anchor_idx, pos);
            self.regenerate_from_path();
        }
    }

    /// Move cubic control `handle` of segment `segment_idx` to `pos`,
    /// regenerating vertices. No-op if the surface has no curve path.
    pub fn move_path_handle(&mut self, segment_idx: usize, handle: CubicHandle, pos: [f32; 2]) {
        if let Some(path) = &mut self.path {
            path.move_handle(segment_idx, handle, pos);
            self.regenerate_from_path();
        }
    }

    /// Axis-aligned bounding box of the polygon (including extra contours).
    pub fn bounding_box(&self) -> BoundingBox {
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for v in self.all_vertices() {
            min_x = min_x.min(v[0]);
            min_y = min_y.min(v[1]);
            max_x = max_x.max(v[0]);
            max_y = max_y.max(v[1]);
        }
        BoundingBox {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    /// Iterate over all vertices across all contours.
    pub fn all_vertices(&self) -> impl Iterator<Item = &[f32; 2]> {
        self.vertices
            .iter()
            .chain(self.extra_contours.iter().flat_map(|c| c.iter()))
    }

    /// Center of the polygon (average of all vertices).
    pub fn center(&self) -> [f32; 2] {
        if self.vertices.is_empty() {
            return [0.0, 0.0];
        }
        let n = self.vertices.len() as f32;
        let sum = self
            .vertices
            .iter()
            .fold([0.0f32, 0.0f32], |acc, v| [acc[0] + v[0], acc[1] + v[1]]);
        [sum[0] / n, sum[1] / n]
    }

    /// Check if a point is inside this surface (any contour, ray-casting algorithm).
    pub fn contains(&self, px: f32, py: f32) -> bool {
        Self::point_in_polygon(&self.vertices, px, py)
            || self
                .extra_contours
                .iter()
                .any(|c| Self::point_in_polygon(c, px, py))
    }

    /// Ray-casting point-in-polygon test for a single contour.
    fn point_in_polygon(verts: &[[f32; 2]], px: f32, py: f32) -> bool {
        let n = verts.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = (verts[i][0], verts[i][1]);
            let (xj, yj) = (verts[j][0], verts[j][1]);
            if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// Iterator over all contours (primary + extra).
    pub fn all_contours(&self) -> impl Iterator<Item = &Vec<[f32; 2]>> {
        std::iter::once(&self.vertices).chain(self.extra_contours.iter())
    }

    /// Return the vertex index closest to a point, or None if not within threshold.
    pub fn nearest_vertex(&self, px: f32, py: f32, threshold: f32) -> Option<usize> {
        self.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dx = px - v[0];
                let dy = py - v[1];
                (i, (dx * dx + dy * dy).sqrt())
            })
            .filter(|(_, d)| *d < threshold)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
    }

    /// Translate all vertices by (dx, dy), clamping to [0..1].
    pub fn translate(&mut self, dx: f32, dy: f32) {
        let bb = self.bounding_box();
        // Clamp translation so bbox stays in [0..1]
        let dx = dx.max(-bb.x).min(1.0 - (bb.x + bb.width));
        let dy = dy.max(-bb.y).min(1.0 - (bb.y + bb.height));
        for v in &mut self.vertices {
            v[0] += dx;
            v[1] += dy;
        }
        for contour in &mut self.extra_contours {
            for v in contour.iter_mut() {
                v[0] += dx;
                v[1] += dy;
            }
        }
        // Keep the curve authoring path in sync so path-backed surfaces move too.
        if let Some(path) = &mut self.path {
            path.start[0] += dx;
            path.start[1] += dy;
            for seg in &mut path.segments {
                match seg {
                    PathSegment::Line { to } => {
                        to[0] += dx;
                        to[1] += dy;
                    }
                    PathSegment::Cubic { c1, c2, to } => {
                        for p in [c1, c2, to] {
                            p[0] += dx;
                            p[1] += dy;
                        }
                    }
                }
            }
        }
        // Move subtractive holes in step with the outline.
        let tf = |p: [f32; 2]| [p[0] + dx, p[1] + dy];
        for hole in &mut self.holes {
            hole.apply_map(tf);
        }
        for contour in &mut self.hole_contours {
            for v in contour.iter_mut() {
                *v = tf(*v);
            }
        }
    }

    /// Rotate all geometry by `angle` radians (clockwise in canvas space, y-down)
    /// around `pivot`. The curve `path` and `circle_hint` are rotated in step so
    /// they stay consistent with `vertices`. A circle hint's center is rotated and
    /// its radius/aspect are left unchanged — exact for a true circle; an oriented
    /// ellipse is approximated (axis-aligned on the next radius/side regeneration).
    ///
    /// Unlike [`Surface::translate`], this does not clamp to `[0..1]`: clamping
    /// per-vertex would distort the shape, and partially off-canvas surfaces are
    /// valid. Callers constrain interactively.
    pub fn rotate(&mut self, angle: f32, pivot: [f32; 2]) {
        let (s, c) = angle.sin_cos();
        let rot = |p: [f32; 2]| -> [f32; 2] {
            let dx = p[0] - pivot[0];
            let dy = p[1] - pivot[1];
            [pivot[0] + dx * c - dy * s, pivot[1] + dx * s + dy * c]
        };
        self.map_geometry(rot);
    }

    /// Scale all geometry by `(sx, sy)` around `pivot`. The curve `path` and
    /// `circle_hint` are scaled in step: the hint's center scales around `pivot`,
    /// its `radius` follows the x-scale and its `aspect_ratio` absorbs the x/y
    /// difference. Like [`Surface::rotate`], this does not clamp to `[0..1]`.
    pub fn scale(&mut self, sx: f32, sy: f32, pivot: [f32; 2]) {
        let scl = |p: [f32; 2]| -> [f32; 2] {
            [
                pivot[0] + (p[0] - pivot[0]) * sx,
                pivot[1] + (p[1] - pivot[1]) * sy,
            ]
        };
        self.map_geometry(scl);
        if let Some(hint) = &mut self.circle_hint {
            hint.radius *= sx;
            if sx != 0.0 {
                hint.aspect_ratio *= sy / sx;
            }
        }
    }

    /// Apply a point transform to every geometry representation (vertices, extra
    /// contours, curve path, circle-hint center). Shared by `rotate`/`scale`.
    fn map_geometry(&mut self, f: impl Fn([f32; 2]) -> [f32; 2]) {
        for v in &mut self.vertices {
            *v = f(*v);
        }
        for contour in &mut self.extra_contours {
            for v in contour.iter_mut() {
                *v = f(*v);
            }
        }
        if let Some(path) = &mut self.path {
            path.apply_map(&f);
        }
        for hole in &mut self.holes {
            hole.apply_map(&f);
        }
        for contour in &mut self.hole_contours {
            for v in contour.iter_mut() {
                *v = f(*v);
            }
        }
        if let Some(hint) = &mut self.circle_hint {
            hint.center = f(hint.center);
        }
    }

    /// Get a mutable reference to a specific contour's vertices.
    /// Contour 0 = primary vertices, 1+ = extra_contours[idx-1].
    pub fn contour_mut(&mut self, contour_idx: usize) -> Option<&mut Vec<[f32; 2]>> {
        if contour_idx == 0 {
            Some(&mut self.vertices)
        } else {
            self.extra_contours.get_mut(contour_idx - 1)
        }
    }

    /// Get a reference to a specific contour's vertices.
    pub fn contour(&self, contour_idx: usize) -> Option<&Vec<[f32; 2]>> {
        if contour_idx == 0 {
            Some(&self.vertices)
        } else {
            self.extra_contours.get(contour_idx - 1)
        }
    }

    /// Total number of contours (1 primary + extra).
    pub fn contour_count(&self) -> usize {
        1 + self.extra_contours.len()
    }
}

impl std::fmt::Display for SurfaceOutputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceOutputType::Projection => write!(f, "Projection"),
            SurfaceOutputType::LEDDirect => write!(f, "LED Direct"),
        }
    }
}

/// Manages all surfaces in the stage layout
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SurfaceManager {
    pub surfaces: Vec<Surface>,
    /// Active dome setup (if dome slices have been generated)
    #[serde(default)]
    pub dome_setup: Option<crate::renderer::slicer::DomeSetup>,
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self {
            surfaces: Vec::new(),
            dome_setup: None,
        }
    }

    /// Add a new rectangular surface with default positioning. Returns the new surface's UUID.
    pub fn add_surface(&mut self, name: String, source: OutputSource) -> String {
        // Place new surfaces in a grid-like pattern
        let count = self.surfaces.len();
        let col = count % 3;
        let row = count / 3;
        let x = 0.05 + col as f32 * 0.32;
        let y = 0.05 + row as f32 * 0.35;

        let surface = Surface::new_rect(name, x, y, 0.28, 0.28, source);
        let uuid = surface.uuid.clone();
        self.surfaces.push(surface);
        uuid
    }

    /// Add a surface with pre-defined vertices. Returns the new surface's UUID.
    pub fn add_polygon_surface(
        &mut self,
        name: String,
        vertices: Vec<[f32; 2]>,
        source: OutputSource,
    ) -> String {
        let uuid = generate_short_uuid();
        self.surfaces.push(Surface {
            uuid: uuid.clone(),
            name,
            vertices,
            extra_contours: Vec::new(),
            source,
            content_mapping: ContentMapping::default(),
            output_type: SurfaceOutputType::Projection,
            circle_hint: None,
            warp: None,
            warp_bound: true,
            path: None,
            holes: Vec::new(),
            hole_contours: Vec::new(),
        });
        uuid
    }

    /// Add a surface authored as an editable curve [`SurfacePath`]. Vertices are
    /// generated by flattening the path (which stays the authoritative source for
    /// downstream routing/warp). Returns the new surface's UUID.
    pub fn add_path_surface(
        &mut self,
        name: String,
        path: curve::SurfacePath,
        source: OutputSource,
    ) -> String {
        let uuid = generate_short_uuid();
        let vertices = path.flatten();
        self.surfaces.push(Surface {
            uuid: uuid.clone(),
            name,
            vertices,
            extra_contours: Vec::new(),
            source,
            content_mapping: ContentMapping::default(),
            output_type: SurfaceOutputType::Projection,
            circle_hint: None,
            warp: None,
            warp_bound: true,
            path: Some(path),
            holes: Vec::new(),
            hole_contours: Vec::new(),
        });
        uuid
    }

    /// Add a circle surface with a `CircleHint`. Vertices are generated from the hint. Returns the new surface's UUID.
    pub fn add_circle_surface(
        &mut self,
        name: String,
        hint: CircleHint,
        source: OutputSource,
    ) -> String {
        let uuid = generate_short_uuid();
        let vertices = hint.generate_vertices();
        self.surfaces.push(Surface {
            uuid: uuid.clone(),
            name,
            vertices,
            extra_contours: Vec::new(),
            source,
            content_mapping: ContentMapping::default(),
            output_type: SurfaceOutputType::Projection,
            circle_hint: Some(hint),
            warp: None,
            warp_bound: true,
            path: None,
            holes: Vec::new(),
            hole_contours: Vec::new(),
        });
        uuid
    }

    /// Remove a surface by UUID. Returns true if found and removed.
    pub fn remove_surface(&mut self, uuid: &str) -> bool {
        if let Some(pos) = self.surfaces.iter().position(|s| s.uuid == uuid) {
            self.surfaces.remove(pos);
            true
        } else {
            false
        }
    }

    /// Change the stacking order of a surface (8i.12) by moving it within the
    /// authoritative `surfaces` Vec (index 0 = bottom, last = top). Returns
    /// `true` if the surface exists (a move at a boundary is a successful no-op),
    /// `false` if `uuid` is unknown.
    pub fn reorder_surface(&mut self, uuid: &str, op: SurfaceReorderOp) -> bool {
        let Some(pos) = self.surfaces.iter().position(|s| s.uuid == uuid) else {
            return false;
        };
        let last = self.surfaces.len() - 1;
        let new_pos = match op {
            SurfaceReorderOp::ToFront => last,
            SurfaceReorderOp::ToBack => 0,
            SurfaceReorderOp::Up => (pos + 1).min(last),
            SurfaceReorderOp::Down => pos.saturating_sub(1),
        };
        if new_pos != pos {
            let s = self.surfaces.remove(pos);
            self.surfaces.insert(new_pos, s);
        }
        true
    }

    /// Find a surface at a given canvas position (normalized coords). Returns UUID.
    pub fn surface_at(&self, px: f32, py: f32) -> Option<String> {
        // Search in reverse so topmost (last added) surfaces are found first
        self.surfaces
            .iter()
            .rev()
            .find(|s| s.contains(px, py))
            .map(|s| s.uuid.clone())
    }

    /// Resolve the target surface for a "Make Hole" punch (8i.7): the topmost
    /// *other* surface whose polygon contains `source`'s centroid. Reverse
    /// iteration matches draw order (last = top). Returns the target UUID, or
    /// `None` if `source` is unknown or sits over no other surface.
    pub fn resolve_hole_target(&self, source_uuid: &str) -> Option<String> {
        let (_, source) = self.find_by_uuid(source_uuid)?;
        let [cx, cy] = source.center();
        self.surfaces
            .iter()
            .rev()
            .find(|s| s.uuid != source_uuid && s.contains(cx, cy))
            .map(|s| s.uuid.clone())
    }

    /// Find a surface by UUID, returning its index and a reference.
    pub fn find_by_uuid(&self, uuid: &str) -> Option<(usize, &Surface)> {
        self.surfaces
            .iter()
            .enumerate()
            .find(|(_, s)| s.uuid == uuid)
    }

    /// Find a surface by UUID, returning its index and a mutable reference.
    pub fn find_by_uuid_mut(&mut self, uuid: &str) -> Option<(usize, &mut Surface)> {
        self.surfaces
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.uuid == uuid)
    }

    /// Duplicate a surface by UUID. Returns the new surface's UUID if found.
    pub fn duplicate_surface(&mut self, uuid: &str) -> Option<String> {
        let original = self.surfaces.iter().find(|s| s.uuid == uuid)?.clone();
        let new_uuid = generate_short_uuid();
        let mut copy = original;
        copy.uuid = new_uuid.clone();
        copy.name = format!("{} (copy)", copy.name);
        // Offset slightly so it's visible
        for v in &mut copy.vertices {
            v[0] += 0.02;
            v[1] += 0.02;
        }
        self.surfaces.push(copy);
        Some(new_uuid)
    }

    /// Next sequential name for a combined surface: "Combined 1", "Combined 2",
    /// … — the lowest integer not already used by an existing "Combined N"
    /// surface. Keeps combined names short so they don't overflow the stage list.
    fn next_combined_name(&self) -> String {
        let max = self
            .surfaces
            .iter()
            .filter_map(|s| s.name.strip_prefix("Combined "))
            .filter_map(|n| n.trim().parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("Combined {}", max + 1)
    }

    /// Combine multiple surfaces into one using polygon boolean union.
    /// Overlapping regions merge into a single outline. Disjoint regions
    /// become extra_contours. Returns the UUID of the combined surface.
    pub fn combine_surfaces(&mut self, uuids: &[String]) -> Option<String> {
        if uuids.len() < 2 {
            return None;
        }

        // Resolve UUIDs to indices
        let indices: Vec<usize> = uuids
            .iter()
            .filter_map(|uuid| self.surfaces.iter().position(|s| s.uuid == *uuid))
            .collect();
        if indices.len() < 2 {
            return None;
        }

        let first_idx = *indices.iter().min().unwrap();

        // Collect all contours as geo polygons
        let mut geo_polys: Vec<geo::Polygon<f64>> = Vec::new();
        for &idx in &indices {
            if idx >= self.surfaces.len() {
                return None;
            }
            let surface = &self.surfaces[idx];
            if let Some(p) = verts_to_geo(&surface.vertices) {
                geo_polys.push(p);
            }
            for ec in &surface.extra_contours {
                if let Some(p) = verts_to_geo(ec) {
                    geo_polys.push(p);
                }
            }
        }

        if geo_polys.is_empty() {
            return None;
        }

        // Iteratively union all polygons
        use geo::BooleanOps;
        let mut result = geo::MultiPolygon::new(vec![geo_polys[0].clone()]);
        for poly in &geo_polys[1..] {
            let other = geo::MultiPolygon::new(vec![poly.clone()]);
            result = result.union(&other);
        }

        // Convert back to vertex arrays
        let mut all_contours: Vec<Vec<[f32; 2]>> = result
            .0
            .iter()
            .map(|p| geo_to_verts(p.exterior()))
            .collect();

        if all_contours.is_empty() {
            return None;
        }

        // Inherit content properties from the first selected surface.
        let source = self.surfaces[first_idx].source.clone();
        let content_mapping = self.surfaces[first_idx].content_mapping;
        let output_type = self.surfaces[first_idx].output_type;

        // Remove selected surfaces in reverse order to preserve indices
        let mut sorted_indices: Vec<usize> = indices.to_vec();
        sorted_indices.sort_unstable();
        sorted_indices.dedup();
        for &idx in sorted_indices.iter().rev() {
            if idx < self.surfaces.len() {
                self.surfaces.remove(idx);
            }
        }

        // Short sequential name, computed against the surfaces that remain.
        let name = self.next_combined_name();

        let new_uuid = generate_short_uuid();
        let primary = all_contours.remove(0);
        let combined = Surface {
            uuid: new_uuid.clone(),
            name,
            vertices: primary,
            extra_contours: all_contours,
            source,
            content_mapping,
            output_type,
            circle_hint: None,
            warp: None,
            warp_bound: true,
            path: None,
            holes: Vec::new(),
            hole_contours: Vec::new(),
        };

        let insert_at = first_idx.min(self.surfaces.len());
        self.surfaces.insert(insert_at, combined);
        Some(new_uuid)
    }
}

// ── Geo conversion helpers ──────────────────────────────────────────

/// Convert `[f32; 2]` vertices to a `geo::Polygon<f64>`.
pub(crate) fn verts_to_geo(verts: &[[f32; 2]]) -> Option<geo::Polygon<f64>> {
    if verts.len() < 3 {
        return None;
    }
    let coords: Vec<geo::Coord<f64>> = verts
        .iter()
        .map(|v| geo::coord! { x: v[0] as f64, y: v[1] as f64 })
        .collect();
    let ring = geo::LineString::new(coords);
    Some(geo::Polygon::new(ring, vec![]))
}

/// Convert exterior `verts` plus subtractive `holes` (interior rings) to a
/// `geo::Polygon<f64>` so boolean ops exclude the cut-outs (8i.7). Holes with
/// fewer than 3 points are skipped.
pub(crate) fn verts_to_geo_with_holes(
    verts: &[[f32; 2]],
    holes: &[Vec<[f32; 2]>],
) -> Option<geo::Polygon<f64>> {
    if verts.len() < 3 {
        return None;
    }
    let to_ring = |vs: &[[f32; 2]]| -> geo::LineString<f64> {
        geo::LineString::new(
            vs.iter()
                .map(|v| geo::coord! { x: v[0] as f64, y: v[1] as f64 })
                .collect(),
        )
    };
    let interiors: Vec<geo::LineString<f64>> = holes
        .iter()
        .filter(|h| h.len() >= 3)
        .map(|h| to_ring(h))
        .collect();
    Some(geo::Polygon::new(to_ring(verts), interiors))
}

/// Convert a `geo::LineString` exterior ring back to `Vec<[f32; 2]>`.
fn geo_to_verts(ring: &geo::LineString<f64>) -> Vec<[f32; 2]> {
    // geo rings are closed (last == first), drop the duplicate
    let pts: Vec<[f32; 2]> = ring.coords().map(|c| [c.x as f32, c.y as f32]).collect();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts[..pts.len() - 1].to_vec()
    } else {
        pts
    }
}

#[cfg(test)]
mod tests {
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
            _ => panic!("expected cubic"),
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
        let mut path = curve::SurfacePath::from_polygon(
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            true,
        );
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
        use crate::renderer::warp::{WarpMode, MAX_WARP_SUBDIVISIONS};
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
}
