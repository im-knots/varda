//! File import pipelines for surface auto-detection (raster, SVG, DXF).

use std::io::Cursor;

use usvg::tiny_skia_path;

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
    let img = image::load_from_memory(image_data)
        .map_err(|e| ImportError::ImageLoad(e.to_string()))?;
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
    let gray = image::GrayImage::from_raw(w, h, gray_pixels)
        .ok_or_else(|| ImportError::ImageLoad("Failed to create grayscale image from RGBA".into()))?;

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
pub fn detect_from_svg(svg_data: &[u8]) -> Result<DetectionResult, ImportError> {
    let tree = usvg::Tree::from_data(svg_data, &usvg::Options::default())
        .map_err(|e| ImportError::SvgParse(e.to_string()))?;

    let mut polylines: Vec<Vec<[f32; 2]>> = Vec::new();
    walk_svg_group(tree.root(), &mut polylines);

    if polylines.is_empty() {
        return Err(ImportError::NoContours);
    }

    // Compute bounding box of all points for normalization
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

    let mut contours = Vec::new();
    for (i, poly) in polylines.iter().enumerate() {
        let normalized: Vec<[f32; 2]> = poly
            .iter()
            .map(|pt| [(pt[0] - min_x) / width, (pt[1] - min_y) / height])
            .collect();
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
        contours.push(DetectedContour {
            vertices: normalized,
            area,
            is_circular,
            circle_fit,
            suggested_name,
        });
    }

    // Sort by area descending
    contours.sort_by(|a, b| b.area.partial_cmp(&a.area).unwrap_or(std::cmp::Ordering::Equal));

    if contours.is_empty() {
        return Err(ImportError::NoContours);
    }
    Ok(DetectionResult {
        contours,
        source_width: width as u32,
        source_height: height as u32,
    })
}

/// Recursively walk a usvg Group, extracting polylines from Path nodes.
fn walk_svg_group(group: &usvg::Group, out: &mut Vec<Vec<[f32; 2]>>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(ref g) => walk_svg_group(g, out),
            usvg::Node::Path(ref p) => {
                let polyline = flatten_svg_path(p.data());
                if polyline.len() >= 3 {
                    out.push(polyline);
                }
            }
            _ => {}
        }
    }
}

/// Flatten a tiny_skia_path::Path into a polyline by sampling curve segments.
fn flatten_svg_path(path: &tiny_skia_path::Path) -> Vec<[f32; 2]> {
    let mut points: Vec<[f32; 2]> = Vec::new();
    let mut last = [0.0f32; 2];

    for seg in path.segments() {
        match seg {
            tiny_skia_path::PathSegment::MoveTo(pt) => {
                last = [pt.x, pt.y];
                points.push(last);
            }
            tiny_skia_path::PathSegment::LineTo(pt) => {
                last = [pt.x, pt.y];
                points.push(last);
            }
            tiny_skia_path::PathSegment::QuadTo(ctrl, pt) => {
                // Sample quadratic Bézier at intermediate points
                const STEPS: usize = 8;
                for i in 1..=STEPS {
                    let t = i as f32 / STEPS as f32;
                    let inv = 1.0 - t;
                    let x = inv * inv * last[0]
                        + 2.0 * inv * t * ctrl.x
                        + t * t * pt.x;
                    let y = inv * inv * last[1]
                        + 2.0 * inv * t * ctrl.y
                        + t * t * pt.y;
                    points.push([x, y]);
                }
                last = [pt.x, pt.y];
            }
            tiny_skia_path::PathSegment::CubicTo(c1, c2, pt) => {
                // Sample cubic Bézier at intermediate points
                const STEPS: usize = 12;
                for i in 1..=STEPS {
                    let t = i as f32 / STEPS as f32;
                    let inv = 1.0 - t;
                    let x = inv * inv * inv * last[0]
                        + 3.0 * inv * inv * t * c1.x
                        + 3.0 * inv * t * t * c2.x
                        + t * t * t * pt.x;
                    let y = inv * inv * inv * last[1]
                        + 3.0 * inv * inv * t * c1.y
                        + 3.0 * inv * t * t * c2.y
                        + t * t * t * pt.y;
                    points.push([x, y]);
                }
                last = [pt.x, pt.y];
            }
            tiny_skia_path::PathSegment::Close => {
                // Close the path by connecting back to the first point
                if let Some(&first) = points.first() {
                    if (last[0] - first[0]).abs() > 1e-4
                        || (last[1] - first[1]).abs() > 1e-4
                    {
                        points.push(first);
                    }
                }
            }
        }
    }
    points
}

// ── DXF import ─────────────────────────────────────────────────────

const DXF_MIN_AREA: f32 = 0.001;
const ARC_SEGMENTS: usize = 32;
const CLOSE_TOLERANCE: f64 = 1e-4;

/// Detect surfaces from DXF data.
pub fn detect_from_dxf(dxf_data: &[u8]) -> Result<DetectionResult, ImportError> {
    let mut cursor = Cursor::new(dxf_data);
    let drawing = dxf::Drawing::load(&mut cursor)
        .map_err(|e| ImportError::DxfParse(e.to_string()))?;

    let mut polylines: Vec<(Vec<[f64; 2]>, bool)> = Vec::new(); // (points, is_circular)

    for entity in drawing.entities() {
        match &entity.specific {
            dxf::entities::EntityType::Line(line) => {
                polylines.push((
                    vec![[line.p1.x, line.p1.y], [line.p2.x, line.p2.y]],
                    false,
                ));
            }
            dxf::entities::EntityType::LwPolyline(poly) => {
                let pts: Vec<[f64; 2]> =
                    poly.vertices.iter().map(|v| [v.x, v.y]).collect();
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
                let pts = approximate_circle(
                    circle.center.x,
                    circle.center.y,
                    circle.radius,
                );
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
            .map(|pt| [
                ((pt[0] - min_x) / width) as f32,
                ((pt[1] - min_y) / height) as f32,
            ])
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
        });
    }

    // Sort by area descending
    contours.sort_by(|a, b| b.area.partial_cmp(&a.area).unwrap_or(std::cmp::Ordering::Equal));

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
    (first[0] - last[0]).abs() < CLOSE_TOLERANCE
        && (first[1] - last[1]).abs() < CLOSE_TOLERANCE
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

fn approximate_arc(
    cx: f64,
    cy: f64,
    r: f64,
    start_deg: f64,
    end_deg: f64,
) -> Vec<[f64; 2]> {
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
        image::ImageEncoder::write_image(
            encoder,
            &img,
            200,
            200,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();

        let params = DetectionParams {
            min_area: 0.01,
            min_vertices: 3,
            ..DetectionParams::default()
        };
        let result = detect_from_image(&buf, &params);
        assert!(result.is_ok(), "Expected detection to succeed: {:?}", result.err());
        let det = result.unwrap();
        assert!(!det.contours.is_empty(), "Expected at least one contour");
    }

    #[test]
    fn detect_from_svg_simple_rect() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <rect x="10" y="10" width="80" height="80" fill="black" stroke="black"/>
        </svg>"#;
        let result = detect_from_svg(svg);
        assert!(result.is_ok(), "Expected SVG detection to succeed: {:?}", result.err());
        let det = result.unwrap();
        assert!(!det.contours.is_empty(), "Expected at least one contour from SVG rect");
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
        assert!(result.is_ok(), "Expected detection to succeed: {:?}", result.err());
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
}