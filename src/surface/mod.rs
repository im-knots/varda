//! Surface — Named regions in a 2D stage model that content is routed to.
//!
//! Surfaces are the middle layer of the three-layer output abstraction:
//!   Content (channels, master) → Surfaces → Outputs (displays, projectors)

use crate::renderer::context::OutputSource;
use serde::{Deserialize, Serialize};

/// A rectangular surface in the 2D stage layout.
///
/// Represents a physical screen, LED panel, or projection area in the venue.
/// Content sources are routed to surfaces, and surfaces are mapped to physical outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surface {
    /// Unique name (e.g., "Main Screen", "Left LED", "DJ Booth")
    pub name: String,
    /// Position and size on the 2D canvas, normalized [0..1] coordinates.
    /// (x, y, width, height) where (0,0) is top-left of the canvas.
    pub rect: SurfaceRect,
    /// What content this surface displays
    pub source: OutputSource,
    /// Output type determines how this surface connects to physical hardware
    pub output_type: SurfaceOutputType,
}

/// Normalized rectangle on the 2D canvas
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SurfaceRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SurfaceRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// Check if a point (in normalized canvas coords) is inside this rect
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width &&
        py >= self.y && py <= self.y + self.height
    }

    /// Return the corner closest to a point, or None if not near any corner.
    /// Returns (corner_index, distance) where corner_index is 0=TL, 1=TR, 2=BR, 3=BL.
    pub fn nearest_corner(&self, px: f32, py: f32, threshold: f32) -> Option<usize> {
        let corners = [
            (self.x, self.y),
            (self.x + self.width, self.y),
            (self.x + self.width, self.y + self.height),
            (self.x, self.y + self.height),
        ];
        corners.iter().enumerate()
            .map(|(i, (cx, cy))| {
                let dx = px - cx;
                let dy = py - cy;
                (i, (dx * dx + dy * dy).sqrt())
            })
            .filter(|(_, d)| *d < threshold)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(i, _)| i)
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

    /// Add a new surface with default positioning
    pub fn add_surface(&mut self, name: String, source: OutputSource) -> usize {
        // Place new surfaces in a grid-like pattern
        let count = self.surfaces.len();
        let col = count % 3;
        let row = count / 3;
        let x = 0.05 + col as f32 * 0.32;
        let y = 0.05 + row as f32 * 0.35;

        self.surfaces.push(Surface {
            name,
            rect: SurfaceRect::new(x, y, 0.28, 0.28),
            source,
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
            .find(|(_, s)| s.rect.contains(px, py))
            .map(|(i, _)| i)
    }
}
