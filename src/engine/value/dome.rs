//! Dome slicer value types — geometry, projector, and preset data.
//!
//! Definitions moved from `internal::renderer::slicer` (see
//! /spec/engine-value-types.md). The slicing algorithm (ray casting,
//! equidistant-azimuthal projection) stays in `renderer::slicer` as
//! functions/inherent impls over these re-exported types.

/// Dome hemisphere geometry.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DomeGeometry {
    /// Dome radius in arbitrary units (only ratios matter).
    pub radius: f32,
    /// Truncation angle in degrees from zenith.
    /// 90° = full hemisphere, 60° = truncated dome.
    pub truncation_degrees: f32,
    /// Dome tilt in degrees (0 = zenith up, positive = tilted forward).
    pub tilt_degrees: f32,
    /// Content azimuth rotation in degrees. Rotates what content appears where
    /// around the dome's vertical axis (0 = no rotation).
    #[serde(default)]
    pub content_azimuth_degrees: f32,
    /// Content elevation rotation in degrees. Tilts the content sphere so the
    /// zenith content (e.g. a black hole at center) can be aimed at the wall
    /// instead of the top of the dome (0 = no tilt, 90 = zenith→horizon).
    #[serde(default)]
    pub content_elevation_degrees: f32,
    /// Content roll in degrees. Spins the content around the dome's zenith axis
    /// (like rotating the circular domemaster image around its center).
    #[serde(default)]
    pub content_roll_degrees: f32,
}

/// Projector placement and lens configuration.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct ProjectorConfig {
    /// Azimuth angle in degrees (0 = front, 90 = right, etc.)
    pub azimuth_degrees: f32,
    /// Elevation angle in degrees from the horizon.
    pub elevation_degrees: f32,
    /// Distance from dome center (normalized to dome radius).
    pub distance: f32,
    /// Horizontal field of view in degrees.
    pub fov_degrees: f32,
    /// Aspect ratio (width / height), e.g. 16.0/9.0
    pub aspect_ratio: f32,
    /// Overlap percentage with adjacent projectors (0.0–1.0).
    pub overlap_pct: f32,
}

/// A complete dome setup: geometry + projector array.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DomeSetup {
    pub geometry: DomeGeometry,
    pub projectors: Vec<ProjectorConfig>,
}

/// Standard dome projector arrangement presets.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DomePreset {
    /// Single projector (fisheye lens, aimed at zenith)
    Single,
    /// 2 projectors (front/back split)
    Dual,
    /// 3 projectors (120° apart)
    Triple,
    /// 4 projectors (90° apart)
    Quad,
    /// 5 projectors (72° apart)
    Penta,
    /// 6 projectors (60° apart)
    Hexa,
    /// 8 projectors (45° apart)
    Octa,
}
