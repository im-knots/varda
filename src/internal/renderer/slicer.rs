//! Dome slicer — auto-computes per-projector warp meshes from dome geometry
//! and projector placement. Generates `WarpMesh` instances that map each
//! projector's output rectangle onto the correct region of the domemaster.
//!
//! The algorithm:
//!   1. For each projector, define a grid of output pixels
//!   2. For each grid point, cast a ray from the projector through that pixel
//!   3. Intersect the ray with the dome hemisphere
//!   4. Convert the dome-surface hit point to equidistant azimuthal (domemaster) UV
//!   5. Store as WarpMesh { position = output grid, uv = domemaster coords }

use super::warp::{MeshPoint, WarpMesh};

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

impl Default for DomeGeometry {
    fn default() -> Self {
        Self {
            radius: 1.0,
            truncation_degrees: 90.0,
            tilt_degrees: 0.0,
            content_azimuth_degrees: 0.0,
            content_elevation_degrees: 0.0,
            content_roll_degrees: 0.0,
        }
    }
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

impl Default for ProjectorConfig {
    fn default() -> Self {
        Self {
            azimuth_degrees: 0.0,
            elevation_degrees: 30.0,
            distance: 0.5,
            fov_degrees: 90.0,
            aspect_ratio: 16.0 / 9.0,
            overlap_pct: 0.15,
        }
    }
}

/// A complete dome setup: geometry + projector array.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DomeSetup {
    pub geometry: DomeGeometry,
    pub projectors: Vec<ProjectorConfig>,
}

/// Mesh grid density for slicer output.
const SLICER_GRID_COLS: u32 = 17;
const SLICER_GRID_ROWS: u32 = 17;

/// Compute the warp mesh for a single projector aimed at a dome.
///
/// Returns a `WarpMesh` where:
/// - `position` = uniform grid in projector output space [0..1]²
/// - `uv` = corresponding domemaster texture coordinates [0..1]²
///
/// The domemaster is an equidistant azimuthal projection where the center
/// of the texture is the dome zenith and the edge of the inscribed circle
/// is the horizon.
pub fn compute_projector_mesh(
    geometry: &DomeGeometry,
    projector: &ProjectorConfig,
    cols: u32,
    rows: u32,
) -> WarpMesh {
    let half_fov_h = (projector.fov_degrees * 0.5).to_radians();
    let half_fov_v = ((projector.fov_degrees / projector.aspect_ratio) * 0.5).to_radians();
    let az = projector.azimuth_degrees.to_radians();
    let el = projector.elevation_degrees.to_radians();
    let dome_trunc = geometry.truncation_degrees.to_radians();
    let dome_tilt = geometry.tilt_degrees.to_radians();
    let content_az = geometry.content_azimuth_degrees.to_radians();
    let content_el = geometry.content_elevation_degrees.to_radians();
    let content_roll = geometry.content_roll_degrees.to_radians();

    let mut points = Vec::with_capacity((cols * rows) as usize);

    for row in 0..rows {
        let v = row as f32 / (rows - 1) as f32;
        let angle_v = half_fov_v * (1.0 - 2.0 * v);

        for col in 0..cols {
            let u = col as f32 / (cols - 1) as f32;
            let angle_h = half_fov_h * (2.0 * u - 1.0);

            // Ray direction in projector-local space (forward = +Z)
            let local_dir = normalize([angle_h.tan(), angle_v.tan(), 1.0]);

            // Rotate by projector elevation (around X axis)
            let after_el = rotate_x(local_dir, el);
            // Rotate by projector azimuth (around Y axis)
            let world_dir = rotate_y(after_el, az);

            // Convert ray direction to dome-surface polar coordinates
            let uv = ray_to_domemaster_uv(world_dir, dome_trunc, dome_tilt, content_az, content_el, content_roll);

            points.push(MeshPoint {
                position: [u, v],
                uv,
            });
        }
    }

    WarpMesh { cols, rows, points }
}

/// Convenience: compute meshes for all projectors in a dome setup.
pub fn compute_dome_meshes(setup: &DomeSetup) -> Vec<WarpMesh> {
    setup.projectors.iter()
        .map(|p| compute_projector_mesh(&setup.geometry, p, SLICER_GRID_COLS, SLICER_GRID_ROWS))
        .collect()
}

// ── Vector math helpers ─────────────────────────────────────────────────

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 { return [0.0, 0.0, 1.0]; }
    [v[0] / len, v[1] / len, v[2] / len]
}

/// Rotate vector around X axis by angle (radians).
fn rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

/// Rotate vector around Y axis by angle (radians).
fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

/// Rotate vector around Z axis by angle (radians).
fn rotate_z(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

/// Convert a world-space ray direction to domemaster UV coordinates.
///
/// Uses equidistant azimuthal projection:
///   - Center of texture (0.5, 0.5) = dome zenith (+Y)
///   - Edge of inscribed circle = horizon
///   - Radius proportional to polar angle from zenith
///
/// `content_az` / `content_el` / `content_roll` rotate the content sphere so
/// that what was at the zenith can be aimed at any point on the dome.
///
/// Rays pointing below the dome truncation boundary are clamped to the edge.
fn ray_to_domemaster_uv(
    dir: [f32; 3],
    trunc_angle: f32,
    dome_tilt: f32,
    content_az: f32,
    content_el: f32,
    content_roll: f32,
) -> [f32; 2] {
    // Apply dome tilt (rotate the ray in the opposite direction)
    let dir = rotate_x(dir, -dome_tilt);

    // Apply content rotation: rotate the sampling ray in the *opposite*
    // direction of the desired content shift.
    // Order: roll (Z) → elevation (X) → azimuth (Y)
    let dir = rotate_z(dir, -content_roll);
    let dir = rotate_x(dir, -content_el);
    let dir = rotate_y(dir, -content_az);

    // Compute polar angle from zenith (+Y axis)
    // Y is up: polar angle = acos(y)
    let polar = dir[1].clamp(-1.0, 1.0).acos();

    // Compute azimuthal angle in XZ plane
    let azimuth = dir[2].atan2(dir[0]);

    // Equidistant azimuthal: radius = polar / max_angle
    // Normalized so that truncation angle maps to the edge of the circle
    let max_angle = trunc_angle.min(std::f32::consts::PI);
    let r = if max_angle > 1e-6 { (polar / max_angle).min(1.0) } else { 0.0 };

    // Convert polar to Cartesian UV (centered at 0.5, 0.5)
    // Scale by 0.5 so the circle inscribes the [0,1]² square
    let uv_x = 0.5 + r * 0.5 * azimuth.cos();
    let uv_y = 0.5 + r * 0.5 * azimuth.sin();

    [uv_x.clamp(0.0, 1.0), uv_y.clamp(0.0, 1.0)]
}

// ── Dome presets ────────────────────────────────────────────────────────

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

impl DomePreset {
    /// Number of projectors in this preset.
    pub fn count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Dual => 2,
            Self::Triple => 3,
            Self::Quad => 4,
            Self::Penta => 5,
            Self::Hexa => 6,
            Self::Octa => 8,
        }
    }

    /// Generate a DomeSetup with default geometry and evenly-spaced projectors.
    pub fn to_setup(self) -> DomeSetup {
        self.to_setup_with_geometry(DomeGeometry::default())
    }

    /// Generate a DomeSetup with custom geometry and evenly-spaced projectors.
    pub fn to_setup_with_geometry(self, geometry: DomeGeometry) -> DomeSetup {
        let n = self.count();

        if n == 1 {
            // Single projector: aimed straight up (zenith), wide FOV
            return DomeSetup {
                geometry,
                projectors: vec![ProjectorConfig {
                    azimuth_degrees: 0.0,
                    elevation_degrees: 90.0,
                    distance: 0.0,
                    fov_degrees: 180.0,
                    aspect_ratio: 1.0,
                    overlap_pct: 0.0,
                }],
            };
        }

        // Multi-projector: evenly spaced around the dome at moderate elevation
        let angle_step = 360.0 / n as f32;
        let fov = angle_step + angle_step * 0.15; // 15% overlap default
        let overlap = 0.15;

        let projectors = (0..n)
            .map(|i| ProjectorConfig {
                azimuth_degrees: i as f32 * angle_step,
                elevation_degrees: 30.0,
                distance: 0.5,
                fov_degrees: fov,
                aspect_ratio: 16.0 / 9.0,
                overlap_pct: overlap,
            })
            .collect();

        DomeSetup { geometry, projectors }
    }
}

impl std::fmt::Display for DomePreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single => write!(f, "Single (1)"),
            Self::Dual => write!(f, "Dual (2)"),
            Self::Triple => write!(f, "Triple (3)"),
            Self::Quad => write!(f, "Quad (4)"),
            Self::Penta => write!(f, "Penta (5)"),
            Self::Hexa => write!(f, "Hexa (6)"),
            Self::Octa => write!(f, "Octa (8)"),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dome_geometry() {
        let g = DomeGeometry::default();
        assert!((g.radius - 1.0).abs() < f32::EPSILON);
        assert!((g.truncation_degrees - 90.0).abs() < f32::EPSILON);
        assert!((g.tilt_degrees - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn default_projector_config() {
        let p = ProjectorConfig::default();
        assert!((p.azimuth_degrees - 0.0).abs() < f32::EPSILON);
        assert!((p.elevation_degrees - 30.0).abs() < f32::EPSILON);
        assert!((p.fov_degrees - 90.0).abs() < f32::EPSILON);
        assert!((p.overlap_pct - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn preset_counts() {
        assert_eq!(DomePreset::Single.count(), 1);
        assert_eq!(DomePreset::Dual.count(), 2);
        assert_eq!(DomePreset::Triple.count(), 3);
        assert_eq!(DomePreset::Quad.count(), 4);
        assert_eq!(DomePreset::Penta.count(), 5);
        assert_eq!(DomePreset::Hexa.count(), 6);
        assert_eq!(DomePreset::Octa.count(), 8);
    }

    #[test]
    fn preset_generates_correct_projector_count() {
        for preset in [DomePreset::Single, DomePreset::Dual, DomePreset::Triple,
                       DomePreset::Quad, DomePreset::Penta, DomePreset::Hexa, DomePreset::Octa] {
            let setup = preset.to_setup();
            assert_eq!(setup.projectors.len(), preset.count(),
                "Preset {:?} should generate {} projectors", preset, preset.count());
        }
    }

    #[test]
    fn preset_display() {
        assert_eq!(format!("{}", DomePreset::Single), "Single (1)");
        assert_eq!(format!("{}", DomePreset::Octa), "Octa (8)");
    }

    #[test]
    fn mesh_has_correct_dimensions() {
        let mesh = compute_projector_mesh(
            &DomeGeometry::default(),
            &ProjectorConfig::default(),
            5, 5,
        );
        assert_eq!(mesh.cols, 5);
        assert_eq!(mesh.rows, 5);
        assert_eq!(mesh.points.len(), 25);
    }

    #[test]
    fn mesh_positions_form_uniform_grid() {
        let mesh = compute_projector_mesh(
            &DomeGeometry::default(),
            &ProjectorConfig::default(),
            3, 3,
        );
        // Corners of position grid should be at [0,0], [1,0], [0,1], [1,1]
        let tl = &mesh.points[0];
        let tr = &mesh.points[2];
        let bl = &mesh.points[6];
        let br = &mesh.points[8];
        assert!((tl.position[0] - 0.0).abs() < 1e-6);
        assert!((tl.position[1] - 0.0).abs() < 1e-6);
        assert!((tr.position[0] - 1.0).abs() < 1e-6);
        assert!((tr.position[1] - 0.0).abs() < 1e-6);
        assert!((bl.position[0] - 0.0).abs() < 1e-6);
        assert!((bl.position[1] - 1.0).abs() < 1e-6);
        assert!((br.position[0] - 1.0).abs() < 1e-6);
        assert!((br.position[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mesh_uvs_are_in_unit_square() {
        let mesh = compute_projector_mesh(
            &DomeGeometry::default(),
            &ProjectorConfig::default(),
            9, 9,
        );
        for pt in &mesh.points {
            assert!(pt.uv[0] >= 0.0 && pt.uv[0] <= 1.0,
                "UV x out of range: {}", pt.uv[0]);
            assert!(pt.uv[1] >= 0.0 && pt.uv[1] <= 1.0,
                "UV y out of range: {}", pt.uv[1]);
        }
    }

    #[test]
    fn zenith_ray_maps_to_center() {
        // A ray pointing straight up should map to domemaster center
        let uv = ray_to_domemaster_uv(
            [0.0, 1.0, 0.0], // straight up (+Y)
            std::f32::consts::FRAC_PI_2, // 90° truncation
            0.0, // no tilt
            0.0, 0.0, 0.0, // no content rotation
        );
        assert!((uv[0] - 0.5).abs() < 1e-4, "Zenith x should be 0.5, got {}", uv[0]);
        assert!((uv[1] - 0.5).abs() < 1e-4, "Zenith y should be 0.5, got {}", uv[1]);
    }

    #[test]
    fn horizon_ray_maps_to_edge() {
        // A ray pointing at the horizon (+Z) with 90° truncation should map to edge
        let uv = ray_to_domemaster_uv(
            [0.0, 0.0, 1.0], // forward (horizon)
            std::f32::consts::FRAC_PI_2, // 90° truncation
            0.0,
            0.0, 0.0, 0.0, // no content rotation
        );
        // Should be at radius 0.5 from center (full edge of inscribed circle)
        let dx = uv[0] - 0.5;
        let dy = uv[1] - 0.5;
        let r = (dx * dx + dy * dy).sqrt();
        assert!((r - 0.5).abs() < 1e-3, "Horizon radius should be 0.5, got {}", r);
    }

    #[test]
    fn content_elevation_moves_zenith_to_edge() {
        // With 90° content elevation, the zenith content should end up at the horizon
        let uv = ray_to_domemaster_uv(
            [0.0, 0.0, 1.0], // forward (horizon ray)
            std::f32::consts::FRAC_PI_2, // 90° truncation
            0.0,
            0.0,
            std::f32::consts::FRAC_PI_2, // 90° content elevation
            0.0,
        );
        // The zenith content (center of domemaster) should now map near center
        // because we rotated the content sphere 90° so what was at zenith is now at horizon
        // and the horizon ray should now sample near the domemaster center
        assert!((uv[0] - 0.5).abs() < 0.05, "Expected near center x, got {}", uv[0]);
        assert!((uv[1] - 0.5).abs() < 0.05, "Expected near center y, got {}", uv[1]);
    }

    #[test]
    fn content_rotation_zero_is_identity() {
        // With zero content rotation, result should match no-rotation case
        let uv_no_rot = ray_to_domemaster_uv(
            [0.3, 0.8, 0.5],
            std::f32::consts::FRAC_PI_2,
            0.0, 0.0, 0.0, 0.0,
        );
        let uv_zero = ray_to_domemaster_uv(
            [0.3, 0.8, 0.5],
            std::f32::consts::FRAC_PI_2,
            0.0, 0.0, 0.0, 0.0,
        );
        assert!((uv_no_rot[0] - uv_zero[0]).abs() < 1e-6);
        assert!((uv_no_rot[1] - uv_zero[1]).abs() < 1e-6);
    }

    #[test]
    fn normalize_works() {
        let v = normalize([3.0, 4.0, 0.0]);
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rotate_y_90_degrees() {
        let v = rotate_y([1.0, 0.0, 0.0], std::f32::consts::FRAC_PI_2);
        // [1,0,0] rotated 90° around Y -> [0,0,-1]
        assert!((v[0] - 0.0).abs() < 1e-6);
        assert!((v[2] - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn compute_dome_meshes_returns_correct_count() {
        let setup = DomePreset::Quad.to_setup();
        let meshes = compute_dome_meshes(&setup);
        assert_eq!(meshes.len(), 4);
        for mesh in &meshes {
            assert_eq!(mesh.cols, SLICER_GRID_COLS);
            assert_eq!(mesh.rows, SLICER_GRID_ROWS);
        }
    }

    #[test]
    fn config_serialization_roundtrip() {
        let setup = DomePreset::Triple.to_setup();
        let json = serde_json::to_string(&setup).unwrap();
        let deserialized: DomeSetup = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.projectors.len(), 3);
        assert!((deserialized.geometry.radius - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn quad_projectors_evenly_spaced() {
        let setup = DomePreset::Quad.to_setup();
        let azimuths: Vec<f32> = setup.projectors.iter().map(|p| p.azimuth_degrees).collect();
        assert!((azimuths[0] - 0.0).abs() < 1e-6);
        assert!((azimuths[1] - 90.0).abs() < 1e-6);
        assert!((azimuths[2] - 180.0).abs() < 1e-6);
        assert!((azimuths[3] - 270.0).abs() < 1e-6);
    }
}