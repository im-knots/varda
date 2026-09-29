//! Surface auto-detection value types and the import error. The detection
//! pipeline lives in `internal::surface::detect` and `::import`.

/// A single detected contour with computed geometry metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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
    /// Curve outline from SVG import. `None` for raster and DXF detection.
    #[serde(default)]
    pub path: Option<super::surface::SurfacePath>,
}

/// Method used to produce the binary image for contour detection.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum DetectionMethod {
    /// Canny edge detector (good for line-art and SVG-like inputs).
    Canny,
    /// Binary threshold (camera feeds with controlled lighting).
    #[default]
    Threshold,
}

/// Post-processing hull mode applied after simplification.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum HullMode {
    /// Keep the simplified polygon as-is.
    #[default]
    None,
    /// Replace with convex hull (removes concavities).
    ConvexHull,
}

/// Parameters controlling the contour detection pipeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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

/// Result of running contour detection on an image.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct DetectionResult {
    /// Detected contours, sorted by area descending.
    pub contours: Vec<DetectedContour>,
    /// Width of the source image in pixels.
    pub source_width: u32,
    /// Height of the source image in pixels.
    pub source_height: u32,
}

// ── Import error ─────────────────────────────────────────────────────

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
