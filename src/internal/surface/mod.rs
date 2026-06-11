//! Surface — Named regions in a 2D stage model that content is routed to.
//!
//! Surfaces are the middle layer of the three-layer output abstraction:
//!   Content (channels, master) → Surfaces → Outputs (displays, projectors)
//!
//! Surfaces are polygons — an ordered list of vertices in normalized canvas
//! coordinates [0..1]. Rectangles are just 4-vertex polygons. This supports
//! triangles, circles (N-gon approximations), and arbitrary shapes.

pub mod detect;
pub mod import;

use crate::deck::generate_short_uuid;
use crate::renderer::context::OutputSource;
use serde::{Deserialize, Serialize};

/// Metadata that marks a surface as a "true circle" with editable radius/sides.
///
/// When present, the surface's vertices are generated from this hint.
/// Editing radius or sides regenerates vertices automatically.
/// Converting to polygon clears the hint, keeping vertices as-is.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CircleHint {
    pub center: [f32; 2],
    pub radius: f32,
    pub sides: u32,
    /// Canvas aspect ratio used when generating vertices (width/height).
    /// Stored so regeneration produces the same visual shape.
    pub aspect_ratio: f32,
}

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
    /// Pre-computed warp mesh for dome-generated surfaces.
    /// When assigned to an output, this is used instead of identity corners.
    #[serde(default)]
    pub default_warp: Option<crate::renderer::warp::WarpMode>,
}

/// How content is mapped onto a surface.
///
/// - **Fill**: The entire source texture is scaled to fill this surface. Each surface
///   with the same source gets an independent full copy.
/// - **Mapped**: The surface's canvas position determines which region of the source
///   it displays. The canvas IS the content space — a surface at (0.2, 0.3, 0.1, 0.1)
///   shows source UVs from (0.2, 0.3) to (0.3, 0.4). Multiple surfaces with the same
///   source in Mapped mode implicitly form a group, each showing its slice of one
///   continuous image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum ContentMapping {
    /// Entire source scaled to fill the surface (independent per surface)
    #[default]
    Fill,
    /// Surface position on canvas = UV crop into the source (spatial mapping)
    Mapped,
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
            default_warp: None,
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

/// How this surface connects to physical output hardware
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum SurfaceOutputType {
    /// Projection — content is warped to match projector position/surface shape
    Projection,
    /// LED Direct — pixel-accurate crop/scale, no perspective warp
    LEDDirect,
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
            default_warp: None,
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
            default_warp: None,
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

    /// Find a surface at a given canvas position (normalized coords). Returns UUID.
    pub fn surface_at(&self, px: f32, py: f32) -> Option<String> {
        // Search in reverse so topmost (last added) surfaces are found first
        self.surfaces
            .iter()
            .rev()
            .find(|s| s.contains(px, py))
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

        // Build combined surface name and properties from first selected
        let name = {
            let names: Vec<&str> = indices
                .iter()
                .filter_map(|&i| self.surfaces.get(i).map(|s| s.name.as_str()))
                .collect();
            names.join(" + ")
        };
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
            default_warp: None,
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
            default_warp: None,
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
            default_warp: None,
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
            default_warp: None,
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
            default_warp: None,
        };
        assert!(s.is_circle());
        s.convert_to_polygon();
        assert!(!s.is_circle());
        assert_eq!(s.vertices.len(), 6); // Vertices preserved
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

    #[test]
    fn manager_add_surface() {
        let mut mgr = SurfaceManager::new();
        let uuid = mgr.add_surface("Main".into(), master_source());
        assert_eq!(uuid.len(), 8);
        assert_eq!(mgr.surfaces.len(), 1);
        assert_eq!(mgr.surfaces[0].uuid, uuid);
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
        assert!(mgr.surfaces[0].name.contains("A"));
        assert!(mgr.surfaces[0].name.contains("B"));
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
}
