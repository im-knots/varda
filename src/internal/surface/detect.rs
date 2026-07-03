//! Contour detection pipeline for surface auto-detection.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A single detected contour with computed geometry metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DetectedContour {
    /// Polygon vertices in normalized [0..1] coordinates.
    pub vertices: Vec<[f32; 2]>,
    /// Polygon area in normalized coordinates.
    pub area: f32,
    /// Whether the contour approximates a circle.
    pub is_circular: bool,
    /// If circular, the fitted (center, radius) in normalized coords.
    pub circle_fit: Option<([f32; 2], f32)>,
    /// Auto-generated name based on position (e.g. "top-left-1").
    pub suggested_name: String,
    /// Editable curve outline captured during SVG import (control points
    /// preserved). `None` for raster/DXF detection, which produce polylines only.
    #[serde(default)]
    pub path: Option<super::curve::SurfacePath>,
}

/// Method used to produce the binary image for contour detection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ToSchema, Default)]
pub enum DetectionMethod {
    /// Canny edge detector (good for line-art and SVG-like inputs).
    Canny,
    /// Simple threshold (industry standard for camera feeds with controlled lighting).
    #[default]
    Threshold,
}

/// Post-processing hull mode applied after simplification.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ToSchema, Default)]
pub enum HullMode {
    /// Keep the simplified polygon as-is.
    #[default]
    None,
    /// Replace with convex hull (removes concavities).
    ConvexHull,
}

/// Parameters controlling the contour detection pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct DetectionParams {
    /// Canny edge detector low threshold.
    pub canny_low: u8,
    /// Canny edge detector high threshold.
    pub canny_high: u8,
    /// Gaussian blur radius applied before edge detection.
    pub blur_radius: u32,
    /// Douglas-Peucker simplification tolerance (normalized).
    pub simplify_tolerance: f32,
    /// Minimum polygon area to keep (normalized).
    pub min_area: f32,
    /// Minimum vertex count after simplification.
    pub min_vertices: usize,
    /// Detection method: Canny or Threshold.
    pub detection_method: DetectionMethod,
    /// Threshold value for binary image creation (0-255).
    pub threshold: u8,
    /// Invert the threshold (foreground becomes background).
    pub invert: bool,
    /// Morphological close kernel radius (0 = disabled).
    pub morph_size: u32,
    /// Post-processing hull mode.
    pub hull_mode: HullMode,
}

impl Default for DetectionParams {
    fn default() -> Self {
        Self {
            canny_low: 50,
            canny_high: 150,
            blur_radius: 1,
            simplify_tolerance: 0.005,
            min_area: 0.001,
            min_vertices: 3,
            detection_method: DetectionMethod::default(),
            threshold: 127,
            invert: false,
            morph_size: 0,
            hull_mode: HullMode::default(),
        }
    }
}

/// Result of running contour detection on an image.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DetectionResult {
    /// Detected contours, sorted by area descending.
    pub contours: Vec<DetectedContour>,
    /// Width of the source image in pixels.
    pub source_width: u32,
    /// Height of the source image in pixels.
    pub source_height: u32,
}

/// Run the full contour detection pipeline on a grayscale image.
pub fn detect_contours(img: &image::GrayImage, params: &DetectionParams) -> DetectionResult {
    let (w, h) = img.dimensions();
    let wf = w as f32;
    let hf = h as f32;

    // Pad image by 2px on each side to work around imageproc Canny boundary bugs.
    let pad = 2u32;
    let padded_w = w + pad * 2;
    let padded_h = h + pad * 2;
    let mut padded = image::GrayImage::new(padded_w, padded_h);
    for y in 0..h {
        for x in 0..w {
            padded.put_pixel(x + pad, y + pad, *img.get_pixel(x, y));
        }
    }

    // 1. Gaussian blur
    let sigma = (params.blur_radius as f32).max(0.1);
    let blurred = if params.blur_radius == 0 {
        padded.clone()
    } else {
        imageproc::filter::gaussian_blur_f32(&padded, sigma)
    };

    // 2. Create binary image based on detection method
    let binary = match params.detection_method {
        DetectionMethod::Canny => {
            let canny_lo = f32::from(params.canny_low);
            let canny_hi = f32::from(params.canny_high).max(canny_lo);
            imageproc::edges::canny(&blurred, canny_lo, canny_hi)
        }
        DetectionMethod::Threshold => threshold_binary(&blurred, params.threshold, params.invert),
    };

    // 3. Optional morphological close
    let cleaned = if params.morph_size > 0 {
        morphological_close(&binary, params.morph_size)
    } else {
        binary
    };

    // 4. Border following (replaces old angle-from-centroid trace_contours)
    let raw_contours = follow_borders(&cleaned);

    // 5. Process each contour
    let pad_f = pad as f32;
    let mut contours: Vec<DetectedContour> = Vec::new();
    for (idx, raw) in raw_contours.iter().enumerate() {
        // Convert pixel coords to f32, subtracting pad offset
        let points: Vec<[f32; 2]> = raw
            .iter()
            .map(|&(x, y)| [(x as f32 - pad_f).max(0.0), (y as f32 - pad_f).max(0.0)])
            .collect();

        // Douglas-Peucker simplification
        let pixel_tolerance = params.simplify_tolerance * wf.max(hf);
        let simplified = douglas_peucker(&points, pixel_tolerance);

        // Optional convex hull
        let final_shape = match params.hull_mode {
            HullMode::None => simplified,
            HullMode::ConvexHull => convex_hull(&simplified),
        };

        if final_shape.len() < params.min_vertices {
            continue;
        }

        // Compute area in pixel space then normalize
        let pixel_area = shoelace_area(&final_shape);
        let norm_area = pixel_area / (wf * hf);

        if norm_area < params.min_area {
            continue;
        }

        // Normalize vertices to [0..1]
        let vertices: Vec<[f32; 2]> = final_shape
            .iter()
            .map(|p| [(p[0] / wf).clamp(0.0, 1.0), (p[1] / hf).clamp(0.0, 1.0)])
            .collect();

        // Circularity check
        let circle_fit = check_circularity(&vertices, norm_area);
        let is_circular = circle_fit.is_some();

        // Compute center for naming
        let cx: f32 = vertices.iter().map(|v| v[0]).sum::<f32>() / vertices.len() as f32;
        let cy: f32 = vertices.iter().map(|v| v[1]).sum::<f32>() / vertices.len() as f32;
        let suggested_name = suggest_name([cx, cy], idx);

        contours.push(DetectedContour {
            vertices,
            area: norm_area,
            is_circular,
            circle_fit,
            suggested_name,
            path: None,
        });
    }

    // Sort by area descending
    contours.sort_by(|a, b| {
        b.area
            .partial_cmp(&a.area)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    DetectionResult {
        contours,
        source_width: w,
        source_height: h,
    }
}

/// Compute the area of a polygon using the shoelace formula.
pub fn shoelace_area(vertices: &[[f32; 2]]) -> f32 {
    let n = vertices.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for i in 0..n {
        let j = (i + 1) % n;
        sum += vertices[i][0] * vertices[j][1];
        sum -= vertices[j][0] * vertices[i][1];
    }
    sum.abs() * 0.5
}

/// Douglas-Peucker polyline simplification.
fn douglas_peucker(points: &[[f32; 2]], tolerance: f32) -> Vec<[f32; 2]> {
    if points.len() <= 2 {
        return points.to_vec();
    }

    // Find the point with maximum distance from the line (first, last)
    let first = points[0];
    let last = points[points.len() - 1];
    let mut max_dist = 0.0f32;
    let mut max_idx = 0;

    for (i, p) in points.iter().enumerate().skip(1).take(points.len() - 2) {
        let d = perpendicular_distance(p, &first, &last);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    if max_dist > tolerance {
        let mut left = douglas_peucker(&points[..=max_idx], tolerance);
        let right = douglas_peucker(&points[max_idx..], tolerance);
        left.pop(); // Remove duplicate point at junction
        left.extend(right);
        left
    } else {
        vec![first, last]
    }
}

/// Perpendicular distance from point `p` to the line through `a` and `b`.
fn perpendicular_distance(p: &[f32; 2], a: &[f32; 2], b: &[f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len_sq = dx * dx + dy * dy;
    if len_sq < f32::EPSILON {
        let ex = p[0] - a[0];
        let ey = p[1] - a[1];
        return (ex * ex + ey * ey).sqrt();
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len_sq.sqrt()
}

/// Create a binary image by thresholding.
fn threshold_binary(img: &image::GrayImage, threshold: u8, invert: bool) -> image::GrayImage {
    let (w, h) = img.dimensions();
    let mut out = image::GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let px = img.get_pixel(x, y).0[0];
            let is_fg = if invert {
                px < threshold
            } else {
                px >= threshold
            };
            out.put_pixel(x, y, image::Luma([if is_fg { 255 } else { 0 }]));
        }
    }
    out
}

/// Morphological close (dilate then erode) on a binary image.
fn morphological_close(img: &image::GrayImage, kernel_size: u32) -> image::GrayImage {
    if kernel_size == 0 {
        return img.clone();
    }
    let radius = kernel_size as i32;
    let dilated = morph_dilate(img, radius);
    morph_erode(&dilated, radius)
}

/// Dilate: max-filter with square kernel of given radius.
fn morph_dilate(img: &image::GrayImage, radius: i32) -> image::GrayImage {
    let (w, h) = img.dimensions();
    let mut out = image::GrayImage::new(w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut max_val = 0u8;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx >= 0 && ny >= 0 && (nx as u32) < w && (ny as u32) < h {
                        max_val = max_val.max(img.get_pixel(nx as u32, ny as u32).0[0]);
                    }
                }
            }
            out.put_pixel(x as u32, y as u32, image::Luma([max_val]));
        }
    }
    out
}

/// Erode: min-filter with square kernel of given radius.
fn morph_erode(img: &image::GrayImage, radius: i32) -> image::GrayImage {
    let (w, h) = img.dimensions();
    let mut out = image::GrayImage::new(w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut min_val = 255u8;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx >= 0 && ny >= 0 && (nx as u32) < w && (ny as u32) < h {
                        min_val = min_val.min(img.get_pixel(nx as u32, ny as u32).0[0]);
                    }
                }
            }
            out.put_pixel(x as u32, y as u32, image::Luma([min_val]));
        }
    }
    out
}

/// Check if a pixel is on the border (has at least one background neighbor or is on image edge).
fn is_border_pixel(img: &image::GrayImage, x: u32, y: u32, w: u32, h: u32) -> bool {
    if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
        return true;
    }
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = (x as i32 + dx) as u32;
            let ny = (y as i32 + dy) as u32;
            if img.get_pixel(nx, ny).0[0] == 0 {
                return true;
            }
        }
    }
    false
}

/// Follow borders using Moore neighbor tracing to extract ordered contour points.
///
/// Scans the binary image (0=bg, 255=fg) in raster order. For each unvisited
/// border pixel, traces the border clockwise using 8-connectivity, producing
/// an ordered sequence of boundary points.
fn follow_borders(binary: &image::GrayImage) -> Vec<Vec<(u32, u32)>> {
    // 8-direction deltas: E, SE, S, SW, W, NW, N, NE
    const DX: [i32; 8] = [1, 1, 0, -1, -1, -1, 0, 1];
    const DY: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];

    let (w, h) = binary.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut visited = vec![false; (w * h) as usize];
    let mut contours = Vec::new();
    let max_steps = (w * h) as usize;

    for y in 0..h {
        for x in 0..w {
            if binary.get_pixel(x, y).0[0] == 0 {
                continue;
            }
            if !is_border_pixel(binary, x, y, w, h) {
                continue;
            }
            let idx = (y * w + x) as usize;
            if visited[idx] {
                continue;
            }

            let contour =
                trace_single_border(binary, x, y, w, h, &DX, &DY, &mut visited, max_steps);
            if !contour.is_empty() {
                contours.push(contour);
            }
        }
    }
    contours
}

/// Trace a single border starting at (sx, sy) using Moore neighbor tracing.
// Tight tracing loop; args are scalar cursor/state values passed by value for speed.
#[allow(clippy::too_many_arguments)]
fn trace_single_border(
    binary: &image::GrayImage,
    sx: u32,
    sy: u32,
    w: u32,
    h: u32,
    dx: &[i32; 8],
    dy: &[i32; 8],
    visited: &mut [bool],
    max_steps: usize,
) -> Vec<(u32, u32)> {
    let mut contour = Vec::new();
    visited[(sy * w + sx) as usize] = true;
    contour.push((sx, sy));

    // Find the first background neighbor to establish entry direction
    // Default entry from west (direction 4) if no background neighbor found
    let mut entry_dir: usize = 4;
    for d in 0..8 {
        let nx = sx as i32 + dx[d];
        let ny = sy as i32 + dy[d];
        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
            // Out of bounds = background, enter from opposite direction
            entry_dir = (d + 4) % 8;
            break;
        }
        if binary.get_pixel(nx as u32, ny as u32).0[0] == 0 {
            entry_dir = (d + 4) % 8;
            break;
        }
    }

    let mut cx = sx;
    let mut cy = sy;

    for _ in 0..max_steps {
        // Search clockwise from (entry_dir + 1) % 8
        let start_search = (entry_dir + 1) % 8;
        let mut found = false;

        for i in 0..8 {
            let d = (start_search + i) % 8;
            let nx = cx as i32 + dx[d];
            let ny = cy as i32 + dy[d];

            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }

            let nxu = nx as u32;
            let nyu = ny as u32;

            if binary.get_pixel(nxu, nyu).0[0] == 0 {
                continue;
            }

            // Must be a border pixel to be part of the contour
            if !is_border_pixel(binary, nxu, nyu, w, h) {
                continue;
            }

            // Found next border pixel
            if nxu == sx && nyu == sy {
                // Back to start — contour is complete
                return contour;
            }

            let nidx = (nyu * w + nxu) as usize;
            if !visited[nidx] {
                visited[nidx] = true;
                contour.push((nxu, nyu));
            }

            // Update entry direction: we came from the opposite of d
            entry_dir = (d + 4) % 8;
            cx = nxu;
            cy = nyu;
            found = true;
            break;
        }

        if !found {
            // Isolated pixel or stuck — return what we have
            break;
        }
    }

    contour
}

/// Compute convex hull of a set of 2D points using Andrew's monotone chain algorithm.
fn convex_hull(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() <= 3 {
        return points.to_vec();
    }

    let mut sorted: Vec<[f32; 2]> = points.to_vec();
    sorted.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Build lower hull
    let mut lower = Vec::new();
    for &p in &sorted {
        while lower.len() >= 2 && cross(&lower[lower.len() - 2], &lower[lower.len() - 1], &p) <= 0.0
        {
            lower.pop();
        }
        lower.push(p);
    }

    // Build upper hull
    let mut upper = Vec::new();
    for &p in sorted.iter().rev() {
        while upper.len() >= 2 && cross(&upper[upper.len() - 2], &upper[upper.len() - 1], &p) <= 0.0
        {
            upper.pop();
        }
        upper.push(p);
    }

    // Remove last point of each half because it's repeated
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// 2D cross product of vectors OA and OB where O is origin point.
fn cross(o: &[f32; 2], a: &[f32; 2], b: &[f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Check if a polygon is approximately circular.
///
/// Computes the bounding circle (centroid + max vertex distance) and compares
/// the polygon area to the circle area. If the ratio exceeds 0.85, returns
/// the fitted circle as `(center, radius)`.
pub fn check_circularity(vertices: &[[f32; 2]], area: f32) -> Option<([f32; 2], f32)> {
    if vertices.len() < 6 {
        return None;
    }

    let n = vertices.len() as f32;
    let cx = vertices.iter().map(|v| v[0]).sum::<f32>() / n;
    let cy = vertices.iter().map(|v| v[1]).sum::<f32>() / n;

    let max_r = vertices
        .iter()
        .map(|v| {
            let dx = v[0] - cx;
            let dy = v[1] - cy;
            (dx * dx + dy * dy).sqrt()
        })
        .fold(0.0f32, f32::max);

    if max_r < f32::EPSILON {
        return None;
    }

    let circle_area = std::f32::consts::PI * max_r * max_r;
    let ratio = area / circle_area;

    if ratio > 0.85 {
        Some(([cx, cy], max_r))
    } else {
        None
    }
}

/// Suggest a display name for a contour based on its center position.
pub fn suggest_name(center: [f32; 2], index: usize) -> String {
    let vertical = if center[1] < 0.33 {
        "top"
    } else if center[1] > 0.66 {
        "bottom"
    } else {
        "center"
    };

    let horizontal = if center[0] < 0.33 {
        "left"
    } else if center[0] > 0.66 {
        "right"
    } else {
        "center"
    };

    let position = if vertical == "center" && horizontal == "center" {
        "center".to_string()
    } else if vertical == "center" {
        horizontal.to_string()
    } else if horizontal == "center" {
        vertical.to_string()
    } else {
        format!("{vertical}-{horizontal}")
    };

    format!("{position}-{}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shoelace_area_unit_square() {
        let verts = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let area = shoelace_area(&verts);
        assert!((area - 1.0).abs() < 1e-6, "Expected ~1.0, got {area}");
    }

    #[test]
    fn shoelace_area_triangle() {
        let verts = [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]];
        let area = shoelace_area(&verts);
        assert!((area - 0.5).abs() < 1e-6, "Expected ~0.5, got {area}");
    }

    #[test]
    fn shoelace_area_degenerate() {
        assert_eq!(shoelace_area(&[]), 0.0);
        assert_eq!(shoelace_area(&[[0.0, 0.0]]), 0.0);
        assert_eq!(shoelace_area(&[[0.0, 0.0], [1.0, 1.0]]), 0.0);
    }

    #[test]
    fn douglas_peucker_preserves_simple_line() {
        let pts = vec![[0.0, 0.0], [1.0, 0.0]];
        let result = douglas_peucker(&pts, 0.1);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn douglas_peucker_simplifies_collinear() {
        let pts = vec![[0.0, 0.0], [0.5, 0.0], [1.0, 0.0]];
        let result = douglas_peucker(&pts, 0.01);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn douglas_peucker_keeps_non_collinear() {
        let pts = vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]];
        let result = douglas_peucker(&pts, 0.01);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn check_circularity_detects_circle() {
        // Generate a near-perfect circle
        let n = 32;
        let r = 0.3;
        let cx = 0.5;
        let cy = 0.5;
        let verts: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let angle = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
                [cx + r * angle.cos(), cy + r * angle.sin()]
            })
            .collect();
        let area = shoelace_area(&verts);
        let result = check_circularity(&verts, area);
        assert!(result.is_some(), "Expected circular detection");
        let (center, radius) = result.unwrap();
        assert!((center[0] - cx).abs() < 0.01);
        assert!((center[1] - cy).abs() < 0.01);
        assert!((radius - r).abs() < 0.02);
    }

    #[test]
    fn check_circularity_rejects_rectangle() {
        let verts = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.1], [0.0, 0.1]];
        let area = shoelace_area(&verts);
        assert!(check_circularity(&verts, area).is_none());
    }

    #[test]
    fn suggest_name_quadrants() {
        assert!(suggest_name([0.1, 0.1], 0).contains("top"));
        assert!(suggest_name([0.9, 0.9], 0).contains("bottom"));
        assert!(suggest_name([0.5, 0.5], 0).contains("center"));
        assert!(suggest_name([0.1, 0.9], 0).contains("left"));
    }

    #[test]
    fn detection_params_default() {
        let p = DetectionParams::default();
        assert_eq!(p.canny_low, 50);
        assert_eq!(p.canny_high, 150);
        assert_eq!(p.blur_radius, 1);
        assert!((p.simplify_tolerance - 0.005).abs() < 1e-6);
        assert!((p.min_area - 0.001).abs() < 1e-6);
        assert_eq!(p.min_vertices, 3);
        assert_eq!(p.threshold, 127);
        assert!(!p.invert);
        assert_eq!(p.morph_size, 0);
        assert!(matches!(p.detection_method, DetectionMethod::Threshold));
        assert!(matches!(p.hull_mode, HullMode::None));
    }

    #[test]
    fn detect_contours_empty_image() {
        let img = image::GrayImage::new(100, 100); // all black
        let result = detect_contours(&img, &DetectionParams::default());
        assert!(result.contours.is_empty());
        assert_eq!(result.source_width, 100);
        assert_eq!(result.source_height, 100);
    }

    #[test]
    fn detect_contours_white_rect_on_black() {
        let mut img = image::GrayImage::new(200, 200);
        // Draw a white rectangle
        for y in 50..150 {
            for x in 50..150 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let params = DetectionParams {
            min_area: 0.01,
            min_vertices: 3,
            ..DetectionParams::default()
        };
        let result = detect_contours(&img, &params);
        // Should detect at least one contour from the rectangle edges
        assert!(
            !result.contours.is_empty(),
            "Expected at least one contour from a white rectangle on black background"
        );
    }

    #[test]
    fn threshold_binary_basic() {
        let mut img = image::GrayImage::new(10, 10);
        for x in 0..10 {
            for y in 0..10 {
                img.put_pixel(x, y, image::Luma([if x >= 5 { 200 } else { 50 }]));
            }
        }
        let result = threshold_binary(&img, 127, false);
        assert_eq!(result.get_pixel(0, 0).0[0], 0);
        assert_eq!(result.get_pixel(5, 0).0[0], 255);
        // Inverted
        let inv = threshold_binary(&img, 127, true);
        assert_eq!(inv.get_pixel(0, 0).0[0], 255);
        assert_eq!(inv.get_pixel(5, 0).0[0], 0);
    }

    #[test]
    fn follow_borders_empty_image() {
        let img = image::GrayImage::new(50, 50);
        let contours = follow_borders(&img);
        assert!(contours.is_empty());
    }

    #[test]
    fn follow_borders_single_rect() {
        let mut img = image::GrayImage::new(100, 100);
        for y in 30..70 {
            for x in 30..70 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let contours = follow_borders(&img);
        assert_eq!(
            contours.len(),
            1,
            "Expected exactly 1 contour, got {}",
            contours.len()
        );
        // All contour points should be border pixels
        for &(x, y) in &contours[0] {
            assert!(is_border_pixel(&img, x, y, 100, 100));
        }
        // Contour should have many points (perimeter of 40x40 rect)
        assert!(
            contours[0].len() > 10,
            "Contour too short: {}",
            contours[0].len()
        );
    }

    #[test]
    fn follow_borders_two_separate_rects() {
        let mut img = image::GrayImage::new(200, 100);
        // Left rect
        for y in 20..80 {
            for x in 10..50 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        // Right rect (well separated)
        for y in 20..80 {
            for x in 100..140 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let contours = follow_borders(&img);
        assert_eq!(
            contours.len(),
            2,
            "Expected 2 contours, got {}",
            contours.len()
        );
    }

    #[test]
    fn follow_borders_single_pixel() {
        let mut img = image::GrayImage::new(50, 50);
        img.put_pixel(25, 25, image::Luma([255u8]));
        let contours = follow_borders(&img);
        assert_eq!(contours.len(), 1);
        assert_eq!(contours[0].len(), 1);
    }

    #[test]
    fn follow_borders_concave_l_shape() {
        // L-shaped region: a single connected concave shape.
        // Moore neighbor tracing may produce 1 or 2 contours at the inner corner;
        // both are valid — the key is that border pixels are found and ordered.
        let mut img = image::GrayImage::new(100, 100);
        // Vertical bar (x: 10..30, y: 10..90)
        for y in 10..90 {
            for x in 10..30 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        // Horizontal bar at bottom overlapping with vertical bar (x: 10..70, y: 70..90)
        for y in 70..90 {
            for x in 10..70 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let contours = follow_borders(&img);
        assert!(
            !contours.is_empty(),
            "Expected at least 1 contour from L-shape"
        );
        assert!(
            contours.len() <= 2,
            "Expected at most 2 contours from L-shape, got {}",
            contours.len()
        );
        let total_points: usize = contours.iter().map(|c| c.len()).sum();
        assert!(
            total_points > 20,
            "L-shape border too short: {total_points} total points"
        );
    }

    #[test]
    fn morphological_close_fills_gap() {
        let mut img = image::GrayImage::new(20, 10);
        // Two blocks with a 1-pixel gap
        for y in 2..8 {
            for x in 2..8 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
            // gap at x=8
            for x in 9..15 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let closed = morphological_close(&img, 1);
        // The gap pixel (8, 5) should now be filled
        assert_eq!(
            closed.get_pixel(8, 5).0[0],
            255,
            "Gap should be filled by morph close"
        );
    }

    #[test]
    fn convex_hull_square() {
        let pts = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let hull = convex_hull(&pts);
        assert_eq!(hull.len(), 4);
    }

    #[test]
    fn convex_hull_concave() {
        // L-shape vertices — hull should remove the concavity
        let pts = vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.5],
            [0.5, 0.5],
            [0.5, 1.0],
            [0.0, 1.0],
        ];
        let hull = convex_hull(&pts);
        // Convex hull of an L should have 4-5 vertices (bounding box corners + one more)
        assert!(
            hull.len() <= 5,
            "Hull should have removed concavity, got {} verts",
            hull.len()
        );
        // Area of hull should be larger than area of L
        let l_area = shoelace_area(&pts);
        let hull_area = shoelace_area(&hull);
        assert!(
            hull_area > l_area,
            "Hull area {} should be > L area {}",
            hull_area,
            l_area
        );
    }

    #[test]
    fn detect_contours_threshold_mode() {
        let mut img = image::GrayImage::new(200, 200);
        for y in 50..150 {
            for x in 50..150 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let params = DetectionParams {
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            min_area: 0.01,
            min_vertices: 3,
            ..DetectionParams::default()
        };
        let result = detect_contours(&img, &params);
        assert!(
            !result.contours.is_empty(),
            "Expected at least one contour in threshold mode"
        );
    }

    #[test]
    fn detect_contours_canny_mode_still_works() {
        let mut img = image::GrayImage::new(200, 200);
        for y in 50..150 {
            for x in 50..150 {
                img.put_pixel(x, y, image::Luma([255u8]));
            }
        }
        let params = DetectionParams {
            detection_method: DetectionMethod::Canny,
            canny_low: 50,
            canny_high: 150,
            min_area: 0.01,
            min_vertices: 3,
            ..DetectionParams::default()
        };
        let result = detect_contours(&img, &params);
        assert!(
            !result.contours.is_empty(),
            "Canny mode should still detect contours from white rect"
        );
    }
}
