//! Subtractive hole mask baking (8i.7).
//!
//! Turns a surface's flattened hole contours (in the surface's **uv space**,
//! `[0..1]²`) into an `R8Unorm` coverage bitmap: `255` = content (keep), `0` =
//! hole (cut). This is a pure CPU scanline fill with no `wgpu` dependency, so it
//! lives in the domain layer and is unit-testable headless. The renderer uploads
//! the returned bytes to a per-surface mask texture sampled by `polygon.wgsl`.

/// Default square resolution of a baked hole mask.
pub const DEFAULT_MASK_RES: u32 = 512;

/// Supersample factor per axis for anti-aliased hole edges.
pub const MASK_SUPERSAMPLE: u32 = 2;

/// Bake `uv_contours` (closed polygons in `[0..1]` uv space) into a `res × res`
/// R8 coverage mask. A sample is a *hole* when it falls inside an odd number of
/// contours (even-odd rule, so nested holes carve back). Each output texel
/// averages `MASK_SUPERSAMPLE²` subsamples for a clean edge. With no contours
/// the whole mask is content (`255`), which the caller treats as a no-op.
pub fn bake_hole_mask(uv_contours: &[Vec<[f32; 2]>], res: u32) -> Vec<u8> {
    let res = res.max(1) as usize;
    if uv_contours.is_empty() {
        return vec![255u8; res * res];
    }
    let ss = MASK_SUPERSAMPLE.max(1) as usize;
    let hi = res * ss;
    let sub_per_texel = (ss * ss) as f32;

    // Count of hole subsamples per output texel.
    let mut hole_count = vec![0u16; res * res];
    // Reused per hi-res scanline crossing buffer.
    let mut xs: Vec<f32> = Vec::new();

    for sy in 0..hi {
        let y = (sy as f32 + 0.5) / hi as f32;
        xs.clear();
        for contour in uv_contours {
            let n = contour.len();
            if n < 3 {
                continue;
            }
            for i in 0..n {
                let a = contour[i];
                let b = contour[(i + 1) % n];
                let (ay, by) = (a[1], b[1]);
                // Half-open edge test avoids double-counting shared vertices.
                if (ay <= y && by > y) || (by <= y && ay > y) {
                    let t = (y - ay) / (by - ay);
                    xs.push(a[0] + t * (b[0] - a[0]));
                }
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let ty = sy / ss;
        // Toggle "inside" between successive crossing pairs.
        let mut k = 0;
        while k + 1 < xs.len() {
            let x0 = xs[k];
            let x1 = xs[k + 1];
            // Subsample columns whose center lies in (x0, x1] are holes.
            let sx_start = ((x0 * hi as f32).ceil().max(0.0)) as usize;
            let sx_end = ((x1 * hi as f32).ceil().max(0.0)) as usize;
            for sx in sx_start..sx_end.min(hi) {
                let tx = sx / ss;
                hole_count[ty * res + tx] += 1;
            }
            k += 2;
        }
    }

    hole_count
        .iter()
        .map(|&c| {
            let hole_frac = c as f32 / sub_per_texel;
            (255.0 * (1.0 - hole_frac)).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texel(mask: &[u8], res: u32, u: f32, v: f32) -> u8 {
        let x = ((u * res as f32) as u32).min(res - 1);
        let y = ((v * res as f32) as u32).min(res - 1);
        mask[(y * res + x) as usize]
    }

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn empty_contours_is_all_content() {
        let mask = bake_hole_mask(&[], 16);
        assert_eq!(mask.len(), 16 * 16);
        assert!(mask.iter().all(|&v| v == 255));
    }

    #[test]
    fn single_square_hole_cuts_center_keeps_corner() {
        let res = 64;
        let mask = bake_hole_mask(&[square(0.25, 0.25, 0.75, 0.75)], res);
        assert_eq!(texel(&mask, res, 0.5, 0.5), 0, "center is a hole");
        assert_eq!(texel(&mask, res, 0.05, 0.05), 255, "corner is content");
        assert_eq!(texel(&mask, res, 0.95, 0.95), 255, "corner is content");
    }

    #[test]
    fn two_holes_cut_both_regions() {
        let res = 64;
        let holes = vec![square(0.1, 0.1, 0.3, 0.3), square(0.6, 0.6, 0.9, 0.9)];
        let mask = bake_hole_mask(&holes, res);
        assert_eq!(texel(&mask, res, 0.2, 0.2), 0, "hole A");
        assert_eq!(texel(&mask, res, 0.75, 0.75), 0, "hole B");
        assert_eq!(texel(&mask, res, 0.5, 0.5), 255, "gap between holes");
    }

    #[test]
    fn degenerate_contour_ignored() {
        let mask = bake_hole_mask(&[vec![[0.1, 0.1], [0.2, 0.2]]], 16);
        assert!(mask.iter().all(|&v| v == 255));
    }

    #[test]
    fn nested_holes_carve_back_even_odd() {
        let res = 64;
        let holes = vec![square(0.2, 0.2, 0.8, 0.8), square(0.4, 0.4, 0.6, 0.6)];
        let mask = bake_hole_mask(&holes, res);
        assert_eq!(texel(&mask, res, 0.3, 0.3), 0, "outer ring is a hole");
        assert_eq!(texel(&mask, res, 0.5, 0.5), 255, "inner ring restored");
    }
}
