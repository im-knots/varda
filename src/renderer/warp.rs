//! Quad warp — perspective correction via homography for projection mapping.
//!
//! The warp pipeline takes a content texture and applies a perspective transform
//! defined by 4 corner correspondences (corner-pin calibration).



/// The 4 corner positions of the warped quad in normalized output space [0..1].
/// Default is the identity (full-screen rectangle).
/// The user drags these corners during calibration to align with physical surfaces.
///
/// Order: top-left, top-right, bottom-right, bottom-left
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct WarpCorners {
    pub corners: [[f32; 2]; 4],
}

impl Default for WarpCorners {
    fn default() -> Self {
        Self {
            corners: [
                [0.0, 0.0], // top-left
                [1.0, 0.0], // top-right
                [1.0, 1.0], // bottom-right
                [0.0, 1.0], // bottom-left
            ],
        }
    }
}

impl WarpCorners {
    /// Check if corners are at identity (no warp needed)
    pub fn is_identity(&self) -> bool {
        let id = Self::default();
        self.corners.iter().zip(id.corners.iter())
            .all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6)
    }

    /// Compute a 3x3 homography matrix that maps from unit square UVs to the
    /// warped quad defined by these corners.
    ///
    /// Given source corners (unit square):
    ///   (0,0) (1,0) (1,1) (0,1)
    /// And destination corners (the user-dragged positions):
    ///   corners[0..4]
    ///
    /// We need the *inverse* mapping: for each pixel in the output, where does
    /// it sample from in the source texture? So we compute the homography from
    /// the destination corners back to the unit square.
    pub fn compute_homography_matrix(&self) -> [f32; 12] {
        // We need: given output UV → source UV
        // That means mapping from the warped quad back to [0,1]x[0,1]
        // Use the standard 4-point homography solver.
        let dst = [
            [0.0_f32, 0.0], // maps to source TL
            [1.0, 0.0],     // maps to source TR
            [1.0, 1.0],     // maps to source BR
            [0.0, 1.0],     // maps to source BL
        ];
        let src = self.corners;

        let h = solve_homography(&src, &dst);

        // Pack into a padded layout for GPU (3 x vec4, using only xyz of each)
        [
            h[0], h[1], h[2], 0.0,
            h[3], h[4], h[5], 0.0,
            h[6], h[7], h[8], 0.0,
        ]
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
            // Degenerate — return identity
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
