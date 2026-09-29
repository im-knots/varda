//! Surface geometry value types: paths, circle hints, content mapping, output
//! type, and reorder ops. Impls and bezier flattening live in
//! `internal::surface`, which re-exports these types.

// ── Curve authoring ──────────────────────

/// One segment of a [`SurfacePath`]. Each segment ends at `to`; its start is the
/// previous segment's endpoint (or the path's `start` for the first segment).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum PathSegment {
    /// Straight line to `to`.
    Line { to: [f32; 2] },
    /// Cubic bezier with control points `c1`, `c2`, ending at `to`.
    Cubic {
        c1: [f32; 2],
        c2: [f32; 2],
        to: [f32; 2],
    },
}

/// Which control point of a [`PathSegment::Cubic`] a handle refers to.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub enum CubicHandle {
    /// Control point leaving the segment's start anchor.
    C1,
    /// Control point entering the segment's end anchor.
    C2,
}

/// An editable curve outline for a surface, in normalized canvas coords [0..1].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct SurfacePath {
    /// Starting point of the path (first vertex of the flattened polygon).
    pub start: [f32; 2],
    /// Ordered segments; each continues from the previous endpoint.
    pub segments: Vec<PathSegment>,
    /// Whether the outline is closed. Surfaces render closed regardless; this
    /// records authoring intent for edit-time handles.
    #[serde(default = "default_true")]
    pub closed: bool,
}

fn default_true() -> bool {
    true
}

// ── Surface metadata ───────────────────

/// Marks a surface as a circle whose vertices are generated from radius and
/// sides. Converting to a polygon clears the hint and keeps the vertices.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CircleHint {
    pub center: [f32; 2],
    pub radius: f32,
    pub sides: u32,
    /// Canvas aspect ratio (width/height) the vertices were generated for.
    pub aspect_ratio: f32,
}

/// How content is mapped onto a surface.
///
/// - **Fill**: each surface shows the whole source.
/// - **Mapped**: the surface's canvas rect is its UV crop of the source, so a
///   surface at (0.2, 0.3, 0.1, 0.1) shows UVs (0.2, 0.3) to (0.3, 0.4).
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum ContentMapping {
    /// Whole source scaled to fill the surface.
    #[default]
    Fill,
    /// Canvas position is the UV crop into the source.
    Mapped,
}

/// How a surface connects to output hardware.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum SurfaceOutputType {
    /// Content is warped to the projector and surface shape.
    Projection,
    /// Pixel-accurate crop and scale, no perspective warp.
    LEDDirect,
}

/// A stacking-order move for a surface. `SurfaceManager.surfaces` order is the
/// stacking order (index 0 is drawn first).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub enum SurfaceReorderOp {
    /// Move to the top of the stack (drawn last, over everything).
    ToFront,
    /// Move to the bottom of the stack (drawn first, under everything).
    ToBack,
    /// Move one step toward the front (up in stacking order).
    Up,
    /// Move one step toward the back (down in stacking order).
    Down,
}
