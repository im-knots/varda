//! Tier 3 render-math invariants — see /spec/render-testing.md.
//!
//! Property-based tests for the pure warp math that feeds the GPU vertex
//! shader. These run everywhere (no GPU needed). The input space is the unit
//! square perturbed by bounded per-corner offsets — the real-world shape of a
//! corner-pin warp — which keeps the DLT solve non-degenerate.

use proptest::prelude::*;
use varda::renderer::warp::compute_forward_homography;

const UNIT_SQUARE: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// Apply a packed forward homography (12 floats, row-major with per-row padding)
/// to a 2D point in homogeneous space.
fn apply(h: &[f32; 12], p: [f32; 2]) -> [f32; 2] {
    let (x, y) = (p[0], p[1]);
    let w = h[8] * x + h[9] * y + h[10];
    [
        (h[0] * x + h[1] * y + h[2]) / w,
        (h[4] * x + h[5] * y + h[6]) / w,
    ]
}

/// Build a perturbed quad from 8 bounded offsets (one per corner coordinate).
fn perturbed_quad(offs: [f32; 8]) -> [[f32; 2]; 4] {
    [
        [UNIT_SQUARE[0][0] + offs[0], UNIT_SQUARE[0][1] + offs[1]],
        [UNIT_SQUARE[1][0] + offs[2], UNIT_SQUARE[1][1] + offs[3]],
        [UNIT_SQUARE[2][0] + offs[4], UNIT_SQUARE[2][1] + offs[5]],
        [UNIT_SQUARE[3][0] + offs[6], UNIT_SQUARE[3][1] + offs[7]],
    ]
}

proptest! {
    /// The forward homography must map the source corners exactly onto the
    /// destination corners (this is the defining property of the DLT solve).
    #[test]
    fn homography_maps_src_corners_to_dst(
        offs in proptest::array::uniform8(-0.25f32..0.25f32)
    ) {
        let dst = perturbed_quad(offs);
        let h = compute_forward_homography(&UNIT_SQUARE, &dst);
        for i in 0..4 {
            let mapped = apply(&h, UNIT_SQUARE[i]);
            prop_assert!((mapped[0] - dst[i][0]).abs() < 2e-3,
                "corner {i} x: {} vs {}", mapped[0], dst[i][0]);
            prop_assert!((mapped[1] - dst[i][1]).abs() < 2e-3,
                "corner {i} y: {} vs {}", mapped[1], dst[i][1]);
        }
    }

    /// forward(src→dst) followed by forward(dst→src) returns the corners to
    /// their originals — the warp is invertible.
    #[test]
    fn homography_forward_then_inverse_is_identity(
        offs in proptest::array::uniform8(-0.25f32..0.25f32)
    ) {
        let dst = perturbed_quad(offs);
        let fwd = compute_forward_homography(&UNIT_SQUARE, &dst);
        let inv = compute_forward_homography(&dst, &UNIT_SQUARE);
        for corner in UNIT_SQUARE {
            let there = apply(&fwd, corner);
            let back = apply(&inv, there);
            prop_assert!((back[0] - corner[0]).abs() < 5e-3,
                "roundtrip x: {} vs {}", back[0], corner[0]);
            prop_assert!((back[1] - corner[1]).abs() < 5e-3,
                "roundtrip y: {} vs {}", back[1], corner[1]);
        }
    }

    /// A no-op warp (src == dst) is the identity on any interior point.
    #[test]
    fn identity_homography_preserves_interior_points(
        x in 0.0f32..1.0f32,
        y in 0.0f32..1.0f32,
    ) {
        let h = compute_forward_homography(&UNIT_SQUARE, &UNIT_SQUARE);
        let mapped = apply(&h, [x, y]);
        prop_assert!((mapped[0] - x).abs() < 1e-4, "id x: {} vs {}", mapped[0], x);
        prop_assert!((mapped[1] - y).abs() < 1e-4, "id y: {} vs {}", mapped[1], y);
    }
}
