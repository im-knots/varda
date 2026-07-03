//! File import pipelines for surface auto-detection (raster, SVG, DXF).

use std::io::Cursor;

use usvg::tiny_skia_path;

use super::curve::{quad_to_cubic, PathSegment, SurfacePath};
use super::detect::{
    check_circularity, detect_contours, shoelace_area, suggest_name, DetectedContour,
    DetectionParams, DetectionResult,
};

// ── Error type ─────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("Failed to load image: {0}")]
    ImageLoad(String),
    #[error("Failed to parse SVG: {0}")]
    SvgParse(String),
    #[error("Failed to parse DXF: {0}")]
    DxfParse(String),
    #[error("Unsupported file format: {0}")]
    UnsupportedFormat(String),
    #[error("No contours detected")]
    NoContours,
    #[error("Detection panicked: {0}")]
    InternalPanic(String),
}

// ── Raster image import ────────────────────────────────────────────

/// Detect surfaces from a raster image (PNG/JPG bytes).
pub fn detect_from_image(
    image_data: &[u8],
    params: &DetectionParams,
) -> Result<DetectionResult, ImportError> {
    let img =
        image::load_from_memory(image_data).map_err(|e| ImportError::ImageLoad(e.to_string()))?;
    let gray = img.to_luma8();
    let result = detect_contours(&gray, params);
    if result.contours.is_empty() {
        return Err(ImportError::NoContours);
    }
    Ok(result)
}

/// Detect surfaces from raw RGBA pixel data (e.g. camera frame).
/// Converts RGBA to grayscale directly, avoiding a PNG encode/decode round-trip.
///
/// The inner detection pipeline (`imageproc` Canny, blur, etc.) is wrapped in
/// `catch_unwind` so that third-party panics never propagate to the caller.
/// This is critical for live-performance safety — the main render thread must
/// never crash due to edge-detection issues.
pub fn detect_from_rgba(
    rgba: &[u8],
    w: u32,
    h: u32,
    params: &DetectionParams,
) -> Result<DetectionResult, ImportError> {
    let expected = (w as usize) * (h as usize) * 4;
    if rgba.len() < expected {
        return Err(ImportError::ImageLoad(format!(
            "RGBA buffer too small: expected {} bytes, got {}",
            expected,
            rgba.len()
        )));
    }
    // Convert RGBA to grayscale using luminance formula
    let gray_pixels: Vec<u8> = rgba
        .chunks_exact(4)
        .map(|px| {
            let r = px[0] as f32;
            let g = px[1] as f32;
            let b = px[2] as f32;
            (0.299 * r + 0.587 * g + 0.114 * b) as u8
        })
        .collect();
    let gray = image::GrayImage::from_raw(w, h, gray_pixels).ok_or_else(|| {
        ImportError::ImageLoad("Failed to create grayscale image from RGBA".into())
    })?;

    // Wrap detection in catch_unwind to absorb any imageproc panics
    let params_clone = params.clone();
    let detect_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        detect_contours(&gray, &params_clone)
    }));

    match detect_result {
        Ok(result) => {
            if result.contours.is_empty() {
                Err(ImportError::NoContours)
            } else {
                Ok(result)
            }
        }
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic in detection pipeline".to_string()
            };
            log::debug!("Detection pipeline panicked (recovered): {}", msg);
            Err(ImportError::InternalPanic(msg))
        }
    }
}

// ── File path dispatch ─────────────────────────────────────────────

/// Detect surfaces from a file, dispatching by extension.
pub fn detect_from_file(
    path: &std::path::Path,
    params: &DetectionParams,
) -> Result<DetectionResult, ImportError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let data = std::fs::read(path).map_err(|e| ImportError::ImageLoad(e.to_string()))?;
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tiff" | "tif" | "webp" => {
            detect_from_image(&data, params)
        }
        "svg" => detect_from_svg(&data),
        "dxf" => detect_from_dxf(&data),
        other => Err(ImportError::UnsupportedFormat(other.to_string())),
    }
}

// ── SVG import ─────────────────────────────────────────────────────

/// Detect surfaces from SVG data (extracts geometric paths directly).
///
/// Cubic/quadratic bezier control points are preserved into a [`SurfacePath`] on
/// each detected contour, so curved outlines import as first-class editable
/// curves rather than pre-flattened polygons.
pub fn detect_from_svg(svg_data: &[u8]) -> Result<DetectionResult, ImportError> {
    let tree = usvg::Tree::from_data(svg_data, &usvg::Options::default())
        .map_err(|e| ImportError::SvgParse(e.to_string()))?;

    let mut paths: Vec<SurfacePath> = Vec::new();
    walk_svg_group(tree.root(), &mut paths);

    if paths.is_empty() {
        return Err(ImportError::NoContours);
    }

    // Flatten each path once for bounding-box, area, and circularity checks.
    let polylines: Vec<Vec<[f32; 2]>> = paths.iter().map(|p| p.flatten()).collect();

    // Compute bounding box of all points for normalization.
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for poly in &polylines {
        for pt in poly {
            min_x = min_x.min(pt[0]);
            min_y = min_y.min(pt[1]);
            max_x = max_x.max(pt[0]);
            max_y = max_y.max(pt[1]);
        }
    }
    let width = (max_x - min_x).max(1e-6);
    let height = (max_y - min_y).max(1e-6);
    let normalize = |pt: [f32; 2]| [(pt[0] - min_x) / width, (pt[1] - min_y) / height];

    let mut contours = Vec::new();
    for (i, (poly, raw_path)) in polylines.iter().zip(paths.iter()).enumerate() {
        let normalized: Vec<[f32; 2]> = poly.iter().map(|pt| normalize(*pt)).collect();
        if normalized.len() < 3 {
            continue;
        }
        let area = shoelace_area(&normalized).abs();
        if area < 0.001 {
            continue;
        }
        let circle_fit = check_circularity(&normalized, area);
        let is_circular = circle_fit.is_some();
        let cx: f32 = normalized.iter().map(|v| v[0]).sum::<f32>() / normalized.len() as f32;
        let cy: f32 = normalized.iter().map(|v| v[1]).sum::<f32>() / normalized.len() as f32;
        let suggested_name = suggest_name([cx, cy], i);
        // Normalize the path's control points with the same transform as the
        // flattened vertices so both spaces stay in sync.
        let mut path = raw_path.clone();
        path.apply_map(normalize);
        contours.push(DetectedContour {
            vertices: normalized,
            area,
            is_circular,
            circle_fit,
            suggested_name,
            path: Some(path),
        });
    }

    // Sort by area descending
    contours.sort_by(|a, b| {
        b.area
            .partial_cmp(&a.area)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if contours.is_empty() {
        return Err(ImportError::NoContours);
    }
    Ok(DetectionResult {
        contours,
        source_width: width as u32,
        source_height: height as u32,
    })
}

/// Recursively walk a usvg Group, extracting an editable [`SurfacePath`] from
/// each Path node.
fn walk_svg_group(group: &usvg::Group, out: &mut Vec<SurfacePath>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(ref g) => walk_svg_group(g, out),
            usvg::Node::Path(ref p) => {
                if let Some(path) = svg_path_to_surface_path(p.data()) {
                    // Require at least a triangle's worth of geometry.
                    if path.flatten().len() >= 3 {
                        out.push(path);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Convert a `tiny_skia_path::Path` into a closed [`SurfacePath`], preserving
/// bezier control points. Quadratic segments are converted losslessly to cubics
/// (the only curved segment kind). Returns `None` if the path has no drawable
/// segments. Subpaths after the first are joined with straight segments, matching
/// the historical single-outline behavior.
fn svg_path_to_surface_path(path: &tiny_skia_path::Path) -> Option<SurfacePath> {
    let mut start: Option<[f32; 2]> = None;
    let mut segments: Vec<PathSegment> = Vec::new();
    let mut last = [0.0f32; 2];

    for seg in path.segments() {
        match seg {
            tiny_skia_path::PathSegment::MoveTo(pt) => {
                let p = [pt.x, pt.y];
                if start.is_none() {
                    start = Some(p);
                } else {
                    segments.push(PathSegment::Line { to: p });
                }
                last = p;
            }
            tiny_skia_path::PathSegment::LineTo(pt) => {
                let p = [pt.x, pt.y];
                segments.push(PathSegment::Line { to: p });
                last = p;
            }
            tiny_skia_path::PathSegment::QuadTo(ctrl, pt) => {
                let p1 = [pt.x, pt.y];
                let (c1, c2) = quad_to_cubic(last, [ctrl.x, ctrl.y], p1);
                segments.push(PathSegment::Cubic { c1, c2, to: p1 });
                last = p1;
            }
            tiny_skia_path::PathSegment::CubicTo(c1, c2, pt) => {
                let p1 = [pt.x, pt.y];
                segments.push(PathSegment::Cubic {
                    c1: [c1.x, c1.y],
                    c2: [c2.x, c2.y],
                    to: p1,
                });
                last = p1;
            }
            tiny_skia_path::PathSegment::Close => {}
        }
    }

    let start = start?;
    if segments.is_empty() {
        return None;
    }
    // Detected outlines are filled regions: close with an explicit segment back
    // to the start unless the last point already coincides with it.
    if (last[0] - start[0]).abs() > 1e-4 || (last[1] - start[1]).abs() > 1e-4 {
        segments.push(PathSegment::Line { to: start });
    }
    Some(SurfacePath {
        start,
        segments,
        closed: true,
    })
}

// ── DXF import ─────────────────────────────────────────────────────

const DXF_MIN_AREA: f32 = 0.001;
const ARC_SEGMENTS: usize = 32;
const CLOSE_TOLERANCE: f64 = 1e-4;

/// Detect surfaces from DXF data.
pub fn detect_from_dxf(dxf_data: &[u8]) -> Result<DetectionResult, ImportError> {
    let mut cursor = Cursor::new(dxf_data);
    let drawing =
        dxf::Drawing::load(&mut cursor).map_err(|e| ImportError::DxfParse(e.to_string()))?;

    let mut polylines: Vec<(Vec<[f64; 2]>, bool)> = Vec::new(); // (points, is_circular)

    for entity in drawing.entities() {
        match &entity.specific {
            dxf::entities::EntityType::Line(line) => {
                polylines.push((vec![[line.p1.x, line.p1.y], [line.p2.x, line.p2.y]], false));
            }
            dxf::entities::EntityType::LwPolyline(poly) => {
                let pts: Vec<[f64; 2]> = poly.vertices.iter().map(|v| [v.x, v.y]).collect();
                if pts.len() >= 2 {
                    let mut pts = pts;
                    // Close if flagged or if start ≈ end
                    if poly.is_closed() || close_enough(&pts) {
                        close_polyline(&mut pts);
                    }
                    polylines.push((pts, false));
                }
            }
            dxf::entities::EntityType::Circle(circle) => {
                let pts = approximate_circle(circle.center.x, circle.center.y, circle.radius);
                polylines.push((pts, true));
            }
            dxf::entities::EntityType::Arc(arc) => {
                let pts = approximate_arc(
                    arc.center.x,
                    arc.center.y,
                    arc.radius,
                    arc.start_angle,
                    arc.end_angle,
                );
                polylines.push((pts, false));
            }
            dxf::entities::EntityType::Ellipse(ellipse) => {
                let pts = approximate_ellipse(
                    ellipse.center.x,
                    ellipse.center.y,
                    ellipse.major_axis.x,
                    ellipse.major_axis.y,
                    ellipse.minor_axis_ratio,
                );
                polylines.push((pts, true));
            }
            _ => {}
        }
    }

    if polylines.is_empty() {
        return Err(ImportError::NoContours);
    }

    // Compute bounding box for normalization
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for (pts, _) in &polylines {
        for pt in pts {
            min_x = min_x.min(pt[0]);
            min_y = min_y.min(pt[1]);
            max_x = max_x.max(pt[0]);
            max_y = max_y.max(pt[1]);
        }
    }
    let width = (max_x - min_x).max(1e-10);
    let height = (max_y - min_y).max(1e-10);

    let mut contours = Vec::new();
    for (i, (pts, is_circular_hint)) in polylines.iter().enumerate() {
        let normalized: Vec<[f32; 2]> = pts
            .iter()
            .map(|pt| {
                [
                    ((pt[0] - min_x) / width) as f32,
                    ((pt[1] - min_y) / height) as f32,
                ]
            })
            .collect();
        if normalized.len() < 3 {
            continue;
        }
        let area = shoelace_area(&normalized).abs();
        if area < DXF_MIN_AREA {
            continue;
        }
        let circle_fit = if *is_circular_hint {
            check_circularity(&normalized, area)
        } else {
            None
        };
        let is_circular = circle_fit.is_some() || *is_circular_hint;
        let cx: f32 = normalized.iter().map(|v| v[0]).sum::<f32>() / normalized.len() as f32;
        let cy: f32 = normalized.iter().map(|v| v[1]).sum::<f32>() / normalized.len() as f32;
        let suggested_name = suggest_name([cx, cy], i);
        contours.push(DetectedContour {
            vertices: normalized,
            area,
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

    if contours.is_empty() {
        return Err(ImportError::NoContours);
    }
    Ok(DetectionResult {
        contours,
        source_width: width as u32,
        source_height: height as u32,
    })
}

// ── DXF geometry helpers ───────────────────────────────────────────

fn close_enough(pts: &[[f64; 2]]) -> bool {
    if pts.len() < 2 {
        return false;
    }
    let first = pts[0];
    let last = pts[pts.len() - 1];
    (first[0] - last[0]).abs() < CLOSE_TOLERANCE && (first[1] - last[1]).abs() < CLOSE_TOLERANCE
}

fn close_polyline(pts: &mut Vec<[f64; 2]>) {
    if pts.len() >= 2 && !close_enough(pts) {
        pts.push(pts[0]);
    }
}

fn approximate_circle(cx: f64, cy: f64, r: f64) -> Vec<[f64; 2]> {
    let tau = std::f64::consts::TAU;
    (0..ARC_SEGMENTS)
        .map(|i| {
            let angle = tau * i as f64 / ARC_SEGMENTS as f64;
            [cx + r * angle.cos(), cy + r * angle.sin()]
        })
        .collect()
}

fn approximate_arc(cx: f64, cy: f64, r: f64, start_deg: f64, end_deg: f64) -> Vec<[f64; 2]> {
    let start = start_deg.to_radians();
    let mut end = end_deg.to_radians();
    if end <= start {
        end += std::f64::consts::TAU;
    }
    let steps = ARC_SEGMENTS;
    (0..=steps)
        .map(|i| {
            let angle = start + (end - start) * i as f64 / steps as f64;
            [cx + r * angle.cos(), cy + r * angle.sin()]
        })
        .collect()
}

fn approximate_ellipse(
    cx: f64,
    cy: f64,
    major_x: f64,
    major_y: f64,
    minor_ratio: f64,
) -> Vec<[f64; 2]> {
    let major_len = (major_x * major_x + major_y * major_y).sqrt();
    let minor_len = major_len * minor_ratio;
    let rotation = major_y.atan2(major_x);
    let tau = std::f64::consts::TAU;
    (0..ARC_SEGMENTS)
        .map(|i| {
            let angle = tau * i as f64 / ARC_SEGMENTS as f64;
            let ex = major_len * angle.cos();
            let ey = minor_len * angle.sin();
            // Rotate by the major axis angle
            let rx = ex * rotation.cos() - ey * rotation.sin();
            let ry = ex * rotation.sin() + ey * rotation.cos();
            [cx + rx, cy + ry]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::detect::{DetectionMethod, HullMode};
    use super::*;

    #[test]
    fn detect_from_image_rejects_empty() {
        let result = detect_from_image(&[], &DetectionParams::default());
        assert!(result.is_err());
    }

    #[test]
    fn detect_from_image_with_white_rect() {
        // Create a simple PNG with a white rectangle on black background
        let mut img = image::RgbaImage::new(200, 200);
        for y in 50..150 {
            for x in 50..150 {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(encoder, &img, 200, 200, image::ExtendedColorType::Rgba8)
            .unwrap();

        let params = DetectionParams {
            min_area: 0.01,
            min_vertices: 3,
            ..DetectionParams::default()
        };
        let result = detect_from_image(&buf, &params);
        assert!(
            result.is_ok(),
            "Expected detection to succeed: {:?}",
            result.err()
        );
        let det = result.unwrap();
        assert!(!det.contours.is_empty(), "Expected at least one contour");
    }

    #[test]
    fn detect_from_svg_simple_rect() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <rect x="10" y="10" width="80" height="80" fill="black" stroke="black"/>
        </svg>"#;
        let result = detect_from_svg(svg);
        assert!(
            result.is_ok(),
            "Expected SVG detection to succeed: {:?}",
            result.err()
        );
        let det = result.unwrap();
        assert!(
            !det.contours.is_empty(),
            "Expected at least one contour from SVG rect"
        );
    }

    #[test]
    fn detect_from_svg_circle() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="40" fill="black"/>
        </svg>"#;
        let result = detect_from_svg(svg);
        assert!(result.is_ok());
        let det = result.unwrap();
        assert!(!det.contours.is_empty());
    }

    #[test]
    fn detect_from_svg_preserves_cubic_control_points() {
        // A path whose top edge is a cubic bezier; the rest are straight lines.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <path d="M10,10 C 30,-10 70,-10 90,10 L 90,90 L 10,90 Z" fill="black"/>
        </svg>"#;
        let det = detect_from_svg(svg).expect("cubic SVG should detect");
        let contour = det.contours.first().expect("one contour");
        let path = contour.path.as_ref().expect("path captured for SVG import");
        assert!(
            path.has_cubic(),
            "expected at least one cubic segment from the C command"
        );
    }

    #[test]
    fn detect_from_svg_rect_path_has_no_cubic() {
        // A purely straight-line outline keeps a path but with no curvature, so
        // it is created as a plain editable polygon (path dropped at confirm).
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <rect x="10" y="10" width="80" height="80" fill="black"/>
        </svg>"#;
        let det = detect_from_svg(svg).expect("rect SVG should detect");
        let contour = det.contours.first().expect("one contour");
        let path = contour.path.as_ref().expect("path captured for SVG import");
        assert!(!path.has_cubic(), "a rect has no cubic segments");
    }

    #[test]
    fn detect_from_svg_empty_rejects() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"></svg>"#;
        let result = detect_from_svg(svg);
        assert!(result.is_err());
    }

    #[test]
    fn detect_from_svg_invalid_rejects() {
        let result = detect_from_svg(b"not valid svg");
        assert!(result.is_err());
    }

    #[test]
    fn detect_from_dxf_invalid_rejects() {
        let result = detect_from_dxf(b"not valid dxf");
        assert!(result.is_err());
    }

    #[test]
    fn detect_from_file_unsupported_extension() {
        // Create a temp file with an unsupported extension
        let dir = std::env::temp_dir();
        let path = dir.join("test_detect.xyz");
        std::fs::write(&path, b"dummy").unwrap();
        let result = detect_from_file(&path, &DetectionParams::default());
        let _ = std::fs::remove_file(&path);
        assert!(matches!(result, Err(ImportError::UnsupportedFormat(_))));
    }

    #[test]
    fn import_error_display() {
        let e = ImportError::ImageLoad("test".into());
        assert!(e.to_string().contains("test"));
        let e = ImportError::NoContours;
        assert!(e.to_string().contains("No contours"));
    }

    #[test]
    fn detect_from_rgba_white_rect_on_black() {
        let (w, h) = (200u32, 200u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // Paint a white rectangle in the center
        for y in 50..150 {
            for x in 50..150 {
                let idx = ((y * w + x) * 4) as usize;
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
        let params = DetectionParams {
            min_area: 0.01,
            min_vertices: 3,
            ..DetectionParams::default()
        };
        let result = detect_from_rgba(&rgba, w, h, &params);
        assert!(
            result.is_ok(),
            "Expected detection to succeed: {:?}",
            result.err()
        );
        let det = result.unwrap();
        assert!(!det.contours.is_empty(), "Expected at least one contour");
    }

    #[test]
    fn detect_from_rgba_all_black_no_contours() {
        let (w, h) = (100u32, 100u32);
        let rgba = vec![0u8; (w * h * 4) as usize];
        let params = DetectionParams::default();
        let result = detect_from_rgba(&rgba, w, h, &params);
        assert!(matches!(result, Err(ImportError::NoContours)));
    }

    #[test]
    fn detect_from_rgba_buffer_too_small() {
        let result = detect_from_rgba(&[0u8; 10], 100, 100, &DetectionParams::default());
        assert!(matches!(result, Err(ImportError::ImageLoad(_))));
    }

    fn encode_rgba_to_png(img: &image::RgbaImage) -> Vec<u8> {
        let (w, h) = img.dimensions();
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(encoder, img, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    #[test]
    fn multi_screen_stage() {
        let mut img = image::RgbaImage::new(600, 200);
        let white = image::Rgba([255, 255, 255, 255]);
        for y in 25..175 {
            for x in 25..175 {
                img.put_pixel(x, y, white);
            }
        }
        for y in 25..175 {
            for x in 225..375 {
                img.put_pixel(x, y, white);
            }
        }
        for y in 25..175 {
            for x in 425..575 {
                img.put_pixel(x, y, white);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("multi-screen detection failed");
        assert_eq!(
            det.contours.len(),
            3,
            "Expected 3 screens, got {}",
            det.contours.len()
        );
        let names: Vec<&str> = det
            .contours
            .iter()
            .map(|c| c.suggested_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("left")),
            "No left screen: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("center")),
            "No center screen: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("right")),
            "No right screen: {:?}",
            names
        );
        for c in &det.contours {
            assert!(c.area > 0.05, "Screen area too small: {}", c.area);
        }
    }

    #[test]
    fn quadrant_projection_setup() {
        let mut img = image::RgbaImage::new(400, 400);
        let white = image::Rgba([255, 255, 255, 255]);
        let rects = [(25u32, 25u32), (225, 25), (25, 225), (225, 225)];
        for &(rx, ry) in &rects {
            for y in ry..ry + 150 {
                for x in rx..rx + 150 {
                    img.put_pixel(x, y, white);
                }
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("quadrant detection failed");
        assert_eq!(
            det.contours.len(),
            4,
            "Expected 4 quadrants, got {}",
            det.contours.len()
        );
        for c in &det.contours {
            assert!(
                c.area > 0.10 && c.area < 0.20,
                "Quadrant area out of range: {} for {}",
                c.area,
                c.suggested_name
            );
        }
        let names: Vec<&str> = det
            .contours
            .iter()
            .map(|c| c.suggested_name.as_str())
            .collect();
        assert!(names
            .iter()
            .any(|n| n.contains("top") && n.contains("left")));
        assert!(names
            .iter()
            .any(|n| n.contains("top") && n.contains("right")));
        assert!(names
            .iter()
            .any(|n| n.contains("bottom") && n.contains("left")));
        assert!(names
            .iter()
            .any(|n| n.contains("bottom") && n.contains("right")));
    }

    #[test]
    fn mixed_shapes_rects_and_circle() {
        // Square image so circle normalization preserves aspect ratio
        let mut img = image::RgbaImage::new(400, 400);
        let white = image::Rgba([255, 255, 255, 255]);
        // Top-left rect: x=20..120, y=20..120
        for y in 20..120 {
            for x in 20..120 {
                img.put_pixel(x, y, white);
            }
        }
        // Bottom-right rect: x=280..380, y=280..380
        for y in 280..380 {
            for x in 280..380 {
                img.put_pixel(x, y, white);
            }
        }
        // Center circle: cx=200, cy=200, r=70
        for y in 0..400 {
            for x in 0..400 {
                let dx = x as f64 - 200.0;
                let dy = y as f64 - 200.0;
                if dx * dx + dy * dy <= 70.0 * 70.0 {
                    img.put_pixel(x, y, white);
                }
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("mixed shapes detection failed");
        assert!(
            det.contours.len() >= 3,
            "Expected >= 3 shapes, got {}",
            det.contours.len()
        );
        let circular_count = det.contours.iter().filter(|c| c.is_circular).count();
        assert!(
            circular_count >= 1,
            "Expected at least 1 circular contour, got {}",
            circular_count
        );
        let non_circular = det.contours.iter().filter(|c| !c.is_circular).count();
        assert!(
            non_circular >= 2,
            "Expected at least 2 non-circular contours, got {}",
            non_circular
        );
    }

    #[test]
    fn nested_rectangles() {
        let mut img = image::RgbaImage::new(400, 400);
        let white = image::Rgba([255, 255, 255, 255]);
        for y in 50..350 {
            for x in 50..350 {
                img.put_pixel(x, y, white);
            }
        }
        let black = image::Rgba([0, 0, 0, 255]);
        for y in 150..250 {
            for x in 150..250 {
                img.put_pixel(x, y, black);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("nested detection failed");
        assert!(
            det.contours.len() >= 2,
            "Expected >= 2 contours for nested rects, got {}",
            det.contours.len()
        );
        assert!(det.contours[0].area > det.contours.last().unwrap().area);
    }

    #[test]
    fn l_shaped_stage() {
        let mut img = image::RgbaImage::new(400, 400);
        let white = image::Rgba([255, 255, 255, 255]);
        for y in 50..350 {
            for x in 50..200 {
                img.put_pixel(x, y, white);
            }
        }
        for y in 200..350 {
            for x in 200..350 {
                img.put_pixel(x, y, white);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("L-shape detection failed");
        assert!(
            !det.contours.is_empty(),
            "Expected at least 1 contour for L-shape"
        );
        let total_area: f32 = det.contours.iter().map(|c| c.area).sum();
        assert!(
            total_area > 0.15,
            "L-shape total area too small: {}",
            total_area
        );
    }

    #[test]
    fn many_small_screens() {
        let mut img = image::RgbaImage::new(600, 400);
        let white = image::Rgba([255, 255, 255, 255]);
        let positions = [
            (50u32, 50u32),
            (250, 50),
            (450, 50),
            (50, 250),
            (250, 250),
            (450, 250),
        ];
        for &(rx, ry) in &positions {
            for y in ry..ry + 80 {
                for x in rx..rx + 80 {
                    img.put_pixel(x, y, white);
                }
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.001,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("many screens detection failed");
        assert_eq!(
            det.contours.len(),
            6,
            "Expected 6 screens, got {}",
            det.contours.len()
        );
        for c in &det.contours {
            assert!(c.area < 0.05, "Screen area too large: {}", c.area);
        }
        for w in det.contours.windows(2) {
            assert!(w[0].area >= w[1].area, "Not sorted descending");
        }
    }

    #[test]
    fn threshold_vs_canny_both_detect() {
        let mut img = image::RgbaImage::new(300, 300);
        let white = image::Rgba([255, 255, 255, 255]);
        for y in 50..250 {
            for x in 50..250 {
                img.put_pixel(x, y, white);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let thresh_params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            ..DetectionParams::default()
        };
        let canny_params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Canny,
            canny_low: 50,
            canny_high: 150,
            ..DetectionParams::default()
        };
        let t = detect_from_image(&buf, &thresh_params).expect("Threshold mode failed");
        let c = detect_from_image(&buf, &canny_params).expect("Canny mode failed");
        assert!(!t.contours.is_empty(), "Threshold found no contours");
        assert!(!c.contours.is_empty(), "Canny found no contours");
    }

    #[test]
    fn grayscale_noise_threshold_robustness() {
        let mut img = image::RgbaImage::new(400, 400);
        let gray_bg = image::Rgba([100, 100, 100, 255]);
        let bright = image::Rgba([240, 240, 240, 255]);
        let noise = image::Rgba([140, 140, 140, 255]);
        for y in 0..400 {
            for x in 0..400 {
                img.put_pixel(x, y, gray_bg);
            }
        }
        for y in 50..150 {
            for x in 50..150 {
                img.put_pixel(x, y, bright);
            }
        }
        for y in 250..350 {
            for x in 250..350 {
                img.put_pixel(x, y, bright);
            }
        }
        for y in 50..100 {
            for x in 250..300 {
                img.put_pixel(x, y, noise);
            }
        }
        for y in 300..350 {
            for x in 50..100 {
                img.put_pixel(x, y, noise);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let params = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 180,
            ..DetectionParams::default()
        };
        let det = detect_from_image(&buf, &params).expect("grayscale threshold detection failed");
        assert_eq!(
            det.contours.len(),
            2,
            "Expected 2 bright rects only (noise below threshold), got {}",
            det.contours.len()
        );
    }

    #[test]
    fn convex_hull_mode_fills_concavity() {
        let mut img = image::RgbaImage::new(400, 400);
        let white = image::Rgba([255, 255, 255, 255]);
        for y in 50..350 {
            for x in 50..200 {
                img.put_pixel(x, y, white);
            }
        }
        for y in 200..350 {
            for x in 200..350 {
                img.put_pixel(x, y, white);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let no_hull = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            hull_mode: HullMode::None,
            ..DetectionParams::default()
        };
        let with_hull = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            hull_mode: HullMode::ConvexHull,
            ..DetectionParams::default()
        };
        let det_no = detect_from_image(&buf, &no_hull).expect("no-hull detection failed");
        let det_yes = detect_from_image(&buf, &with_hull).expect("hull detection failed");
        let area_no: f32 = det_no.contours.iter().map(|c| c.area).sum();
        let area_yes: f32 = det_yes.contours.iter().map(|c| c.area).sum();
        assert!(
            area_yes >= area_no,
            "Convex hull area ({}) should be >= non-hull area ({})",
            area_yes,
            area_no
        );
    }

    #[test]
    fn morphological_close_merges_gap() {
        let mut img = image::RgbaImage::new(400, 200);
        let white = image::Rgba([255, 255, 255, 255]);
        for y in 50..150 {
            for x in 100..198 {
                img.put_pixel(x, y, white);
            }
        }
        for y in 50..150 {
            for x in 200..298 {
                img.put_pixel(x, y, white);
            }
        }
        let buf = encode_rgba_to_png(&img);
        let no_morph = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            morph_size: 0,
            ..DetectionParams::default()
        };
        let with_morph = DetectionParams {
            min_area: 0.005,
            min_vertices: 3,
            blur_radius: 0,
            detection_method: DetectionMethod::Threshold,
            threshold: 127,
            morph_size: 3,
            ..DetectionParams::default()
        };
        let det_no = detect_from_image(&buf, &no_morph).expect("no-morph detection failed");
        let det_yes = detect_from_image(&buf, &with_morph).expect("morph detection failed");
        assert_eq!(
            det_no.contours.len(),
            2,
            "Without morph close, expected 2 separate rects, got {}",
            det_no.contours.len()
        );
        assert_eq!(
            det_yes.contours.len(),
            1,
            "With morph close, expected 1 merged rect, got {}",
            det_yes.contours.len()
        );
    }
}
