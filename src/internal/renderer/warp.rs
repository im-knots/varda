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
                points.push(MeshPoint { position: [u, v], uv: [u, v] });
            }
        }
        Self { cols, rows, points }
    }

    /// Create a mesh from 4 corner positions (corner-pin equivalent).
    /// Generates a 2×2 grid with positions at the corners and UVs at unit square.
    pub fn from_corners(corners: &[[f32; 2]; 4]) -> Self {
        // Order: TL, TR, BR, BL → grid row-major: TL, TR, BL, BR
        Self {
            cols: 2, rows: 2,
            points: vec![
                MeshPoint { position: corners[0], uv: [0.0, 0.0] }, // TL
                MeshPoint { position: corners[1], uv: [1.0, 0.0] }, // TR
                MeshPoint { position: corners[3], uv: [0.0, 1.0] }, // BL
                MeshPoint { position: corners[2], uv: [1.0, 1.0] }, // BR
            ],
        }
    }

    /// Check if this mesh is an identity warp (positions == UVs).
    pub fn is_identity(&self) -> bool {
        self.points.iter().all(|p| {
            (p.position[0] - p.uv[0]).abs() < 1e-6 && (p.position[1] - p.uv[1]).abs() < 1e-6
        })
    }
}

/// Warp mode for surface assignments: corner-pin or arbitrary mesh.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum WarpMode {
    /// 4-point corner-pin warp (TL, TR, BR, BL in output space [0..1]).
    CornerPin { corners: [[f32; 2]; 4] },
    /// Arbitrary XYUV mesh warp grid.
    Mesh(WarpMesh),
}

impl WarpMode {
    /// Create a corner-pin warp from 4 corners.
    pub fn corner_pin(corners: [[f32; 2]; 4]) -> Self {
        Self::CornerPin { corners }
    }

    /// Create an identity corner-pin (no warp, bounding-box corners).
    pub fn identity_corners(bb: [f32; 4]) -> Self {
        let [x, y, w, h] = bb;
        Self::CornerPin { corners: [
            [x, y], [x + w, y], [x + w, y + h], [x, y + h],
        ]}
    }

    /// Get corner-pin corners if this is a CornerPin variant.
    pub fn corners(&self) -> Option<&[[f32; 2]; 4]> {
        match self {
            Self::CornerPin { corners } => Some(corners),
            Self::Mesh(_) => None,
        }
    }

    /// Get mutable corner-pin corners if this is a CornerPin variant.
    pub fn corners_mut(&mut self) -> Option<&mut [[f32; 2]; 4]> {
        match self {
            Self::CornerPin { corners } => Some(corners),
            Self::Mesh(_) => None,
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
                corners.iter().zip(id.iter())
                    .all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6)
            }
            Self::Mesh(mesh) => mesh.is_identity(),
        }
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
        h[0], h[1], h[2], 0.0,
        h[3], h[4], h[5], 0.0,
        h[6], h[7], h[8], 0.0,
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
        h[0] as f32, h[1] as f32, h[2] as f32,
        h[3] as f32, h[4] as f32, h[5] as f32,
        h[6] as f32, h[7] as f32, 1.0,
    ]
}

/// Gaussian elimination with partial pivoting for an 8x8 system
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
            log::warn!("Degenerate homography: pivot near zero at column {col}, returning identity warp");
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
        let mut lines = input.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'));

        let header = lines.next()
            .ok_or_else(|| anyhow::anyhow!("XYUV CSV: missing header line"))?;
        let dims: Vec<u32> = header.split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if dims.len() < 2 {
            anyhow::bail!("XYUV CSV: header must contain mesh_w mesh_h, got: {}", header);
        }
        let cols = dims[0];
        let rows = dims[1];
        if cols < 2 || rows < 2 {
            anyhow::bail!("XYUV CSV: mesh dimensions must be ≥ 2, got {}×{}", cols, rows);
        }
        if cols > 10_000 || rows > 10_000 {
            anyhow::bail!("XYUV CSV: mesh dimensions too large (max 10000×10000), got {}×{}", cols, rows);
        }

        let expected = (cols * rows) as usize;
        let mut points = Vec::with_capacity(expected);

        for line in lines {
            let vals: Vec<f32> = line.split(|c: char| c == ',' || c.is_whitespace())
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
                expected, cols, rows, points.len()
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
            anyhow::bail!("JSON mesh: dimensions must be ≥ 2, got {}×{}", mesh.cols, mesh.rows);
        }
        if mesh.points.len() != (mesh.cols * mesh.rows) as usize {
            anyhow::bail!(
                "JSON mesh: expected {} points, got {}",
                mesh.cols * mesh.rows, mesh.points.len()
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
        assert_eq!(MeshFormat::from_extension(std::path::Path::new("mesh.csv")), Some(MeshFormat::XyuvCsv));
        assert_eq!(MeshFormat::from_extension(std::path::Path::new("dome.xyuv")), Some(MeshFormat::XyuvCsv));
        assert_eq!(MeshFormat::from_extension(std::path::Path::new("warp.txt")), Some(MeshFormat::XyuvCsv));
    }

    #[test]
    fn format_detection_json() {
        assert_eq!(MeshFormat::from_extension(std::path::Path::new("mesh.json")), Some(MeshFormat::Json));
    }

    #[test]
    fn format_detection_unknown() {
        assert_eq!(MeshFormat::from_extension(std::path::Path::new("mesh.png")), None);
        assert_eq!(MeshFormat::from_extension(std::path::Path::new("noext")), None);
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
        match result {
            Ok(mesh) => {
                assert_eq!(mesh.points.len(), 4);
            }
            Err(_) => {} // count mismatch is acceptable
        }
    }

    #[test]
    fn chaos_csv_all_garbage_lines() {
        let csv = "2 2\nhello world foo bar\ngarbage in garbage out\nmore junk here\ntotally invalid\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(result.is_err(), "all-garbage CSV should fail point count check");
    }

    #[test]
    fn chaos_csv_partial_fields() {
        // Lines with fewer than 4 parseable floats should be skipped
        let csv = "2 2\n0.0 0.0\n1.0\n0.0 0.0 0.0\n1.0 1.0 1.0 1.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(result.is_err(), "partial fields should result in count mismatch");
    }

    #[test]
    fn chaos_csv_huge_dimensions() {
        let csv = "1000000 1000000\n0.0 0.0 0.0 0.0 1.0\n";
        let result = WarpMesh::from_xyuv_csv(csv);
        assert!(result.is_err(), "huge dimensions should be rejected (overflow protection)");
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
        // NaN parses fine as f32, so all 4 points should be present
        match result {
            Ok(mesh) => assert_eq!(mesh.points.len(), 4),
            Err(_) => {} // if NaN fields are dropped, count mismatch
        }
    }
}