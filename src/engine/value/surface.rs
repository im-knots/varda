//! Surface geometry value types — path authoring, circle hints, content
//! mapping, output type, and reorder ops.
//!
//! Definitions moved from `internal::surface` / `internal::surface::curve`
//! (see /spec/engine-value-types.md). Inherent impls, `Display` impls, and
//! the bezier flattening algorithms stay in those modules, applied to these
//! re-exported types.

// ── Curve authoring (formerly `surface::curve`) ──────────────────────

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

// ── Surface metadata (formerly `internal::surface`) ───────────────────

/// Metadata that marks a surface as a "true circle" with editable radius/sides.
///
/// When present, the surface's vertices are generated from this hint.
/// Editing radius or sides regenerates vertices automatically.
/// Converting to polygon clears the hint, keeping vertices as-is.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CircleHint {
    pub center: [f32; 2],
    pub radius: f32,
    pub sides: u32,
    /// Canvas aspect ratio used when generating vertices (width/height).
    /// Stored so regeneration produces the same visual shape.
    pub aspect_ratio: f32,
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
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum ContentMapping {
    /// Entire source scaled to fill the surface (independent per surface)
    #[default]
    Fill,
    /// Surface position on canvas = UV crop into the source (spatial mapping)
    Mapped,
}

/// How this surface connects to physical output hardware
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum SurfaceOutputType {
    /// Projection — content is warped to match projector position/surface shape
    Projection,
    /// LED Direct — pixel-accurate crop/scale, no perspective warp
    LEDDirect,
}

/// A stacking-order move for a surface (8i.12). The `SurfaceManager.surfaces`
/// Vec order is authoritative (index 0 = bottom/drawn-first, last = top).
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
