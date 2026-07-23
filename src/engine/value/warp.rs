//! Warp value types — corner-pin, mesh, and bezier patch-grid warp data.
//!
//! Definitions moved from `internal::renderer::warp` (see
//! /spec/engine-value-types.md). All mesh/tessellation *algorithms* stay in
//! `renderer::warp` as inherent impls on these re-exported types — only the
//! plain data shapes live here.

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

/// A smooth warp defined by a grid of cubic-bezier patches with tangent
/// handles. See `renderer::warp` module docs for the model.
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
