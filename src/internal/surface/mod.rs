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
        use crate::renderer::warp::{BezierWarp, DEFAULT_BEZIER_TESS, WarpMode};
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
    ///
    /// # Panics
    ///
    /// Panics if the path is still absent after being populated above —
    /// unreachable.
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
        self.hole_contours = self
            .holes
            .iter()
            .map(super::super::engine::value::surface::SurfacePath::flatten)
            .collect();
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
    /// Contour 0 = primary vertices, 1+ = `extra_contours`[idx-1].
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
        copy.uuid.clone_from(&new_uuid);
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
    /// become `extra_contours`. Returns the UUID of the combined surface.
    ///
    /// # Panics
    ///
    /// Panics if the resolved index list is empty when its minimum is taken —
    /// unreachable, since an early return covers fewer than two indices.
    pub fn combine_surfaces(&mut self, uuids: &[String]) -> Option<String> {
        use geo::BooleanOps;

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
        let mut sorted_indices: Vec<usize> = indices.clone();
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
        .map(|v| geo::coord! { x: f64::from(v[0]), y: f64::from(v[1]) })
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
                .map(|v| geo::coord! { x: f64::from(v[0]), y: f64::from(v[1]) })
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
mod tests;
