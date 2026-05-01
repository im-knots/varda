//! Surface — Named regions in a 2D stage model that content is routed to.
//!
//! Surfaces are the middle layer of the three-layer output abstraction:
//!   Content (channels, master) → Surfaces → Outputs (displays, projectors)
//!
//! Surfaces are polygons — an ordered list of vertices in normalized canvas
//! coordinates [0..1]. Rectangles are just 4-vertex polygons. This supports
//! triangles, circles (N-gon approximations), and arbitrary shapes.

use crate::renderer::context::OutputSource;
use serde::{Deserialize, Serialize};

/// A polygon surface in the 2D stage layout.
///
/// Represents a physical screen, LED panel, or projection area in the venue.
/// Content sources are routed to surfaces, and surfaces are mapped to physical outputs.
///
/// Vertices are ordered polygon points in normalized canvas coordinates [0..1],
/// where (0,0) is top-left of the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surface {
    /// Unique name (e.g., "Main Screen", "Left LED", "DJ Booth")
    pub name: String,
    /// Ordered polygon vertices in normalized canvas coordinates [0..1]
    pub vertices: Vec<[f32; 2]>,
    /// What content this surface displays
    pub source: OutputSource,
    /// How the content maps onto this surface
    pub content_mapping: ContentMapping,
    /// Output type determines how this surface connects to physical hardware
    pub output_type: SurfaceOutputType,
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ContentMapping {
    /// Entire source scaled to fill the surface (independent per surface)
    Fill,
    /// Surface position on canvas = UV crop into the source (spatial mapping)
    Mapped,
}

impl Default for ContentMapping {
    fn default() -> Self {
        ContentMapping::Fill
    }
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
            name,
            vertices: vec![
                [x, y],
                [x + w, y],
                [x + w, y + h],
                [x, y + h],
            ],
            source,
            content_mapping: ContentMapping::default(),
            output_type: SurfaceOutputType::Projection,
        }
    }

    /// Axis-aligned bounding box of the polygon.
    pub fn bounding_box(&self) -> BoundingBox {
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for v in &self.vertices {
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

    /// Center of the polygon (average of all vertices).
    pub fn center(&self) -> [f32; 2] {
        if self.vertices.is_empty() {
            return [0.0, 0.0];
        }
        let n = self.vertices.len() as f32;
        let sum = self.vertices.iter().fold([0.0f32, 0.0f32], |acc, v| {
            [acc[0] + v[0], acc[1] + v[1]]
        });
        [sum[0] / n, sum[1] / n]
    }

    /// Check if a point is inside this polygon (ray-casting algorithm).
    pub fn contains(&self, px: f32, py: f32) -> bool {
        let n = self.vertices.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = (self.vertices[i][0], self.vertices[i][1]);
            let (xj, yj) = (self.vertices[j][0], self.vertices[j][1]);
            if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// Return the vertex index closest to a point, or None if not within threshold.
    pub fn nearest_vertex(&self, px: f32, py: f32, threshold: f32) -> Option<usize> {
        self.vertices.iter().enumerate()
            .map(|(i, v)| {
                let dx = px - v[0];
                let dy = py - v[1];
                (i, (dx * dx + dy * dy).sqrt())
            })
            .filter(|(_, d)| *d < threshold)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
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
    }
}

/// How this surface connects to physical output hardware
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self { surfaces: Vec::new() }
    }

    /// Add a new rectangular surface with default positioning
    pub fn add_surface(&mut self, name: String, source: OutputSource) -> usize {
        // Place new surfaces in a grid-like pattern
        let count = self.surfaces.len();
        let col = count % 3;
        let row = count / 3;
        let x = 0.05 + col as f32 * 0.32;
        let y = 0.05 + row as f32 * 0.35;

        self.surfaces.push(Surface::new_rect(name, x, y, 0.28, 0.28, source));
        self.surfaces.len() - 1
    }

    /// Add a surface with pre-defined vertices
    pub fn add_polygon_surface(&mut self, name: String, vertices: Vec<[f32; 2]>, source: OutputSource) -> usize {
        self.surfaces.push(Surface {
            name,
            vertices,
            source,
            content_mapping: ContentMapping::default(),
            output_type: SurfaceOutputType::Projection,
        });
        self.surfaces.len() - 1
    }

    /// Remove a surface by index
    pub fn remove_surface(&mut self, idx: usize) -> bool {
        if idx < self.surfaces.len() {
            self.surfaces.remove(idx);
            true
        } else {
            false
        }
    }

    /// Find a surface at a given canvas position (normalized coords)
    pub fn surface_at(&self, px: f32, py: f32) -> Option<usize> {
        // Search in reverse so topmost (last added) surfaces are found first
        self.surfaces.iter().enumerate().rev()
            .find(|(_, s)| s.contains(px, py))
            .map(|(i, _)| i)
    }
}
