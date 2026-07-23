//! Surface action processing for VardaApp.

use super::VardaApp;
use crate::engine::EngineCommand;
use crate::renderer::context::OutputSource;
use crate::renderer::slicer::compute_dome_meshes;
use crate::renderer::warp::WarpMode;

impl VardaApp {
    /// Generate dome slices: remove old "Dome P*" surfaces, compute warp meshes,
    /// create new surfaces with pre-computed WarpMesh per projector.
    pub(crate) fn generate_dome_slices(&mut self, setup: &crate::renderer::slicer::DomeSetup) {
        // Remove existing dome-generated surfaces (named "Dome P*")
        let dome_uuids: Vec<String> = self
            .output
            .surface_manager
            .surfaces
            .iter()
            .filter(|s| s.name.starts_with("Dome P"))
            .map(|s| s.uuid.clone())
            .collect();
        for uuid in &dome_uuids {
            self.execute_command(EngineCommand::RemoveSurface { uuid: uuid.clone() });
        }

        // Compute warp meshes for all projectors
        let meshes = compute_dome_meshes(setup);

        // Create a surface per projector with Domemaster source and pre-computed warp mesh
        for (i, mesh) in meshes.iter().enumerate() {
            let name = format!("Dome P{}", i + 1);
            // Compute the convex hull of the warp mesh UVs as the 2D surface polygon
            let vertices = convex_hull_of_uvs(mesh);
            let uuid = self.output.surface_manager.add_polygon_surface(
                name.clone(),
                vertices,
                OutputSource::Domemaster,
            );
            // Store the pre-computed warp mesh on the surface (per-surface warp).
            // Unbind from auto-warp so the dome's mesh is authoritative.
            if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(&uuid) {
                surface.warp = Some(WarpMode::Mesh(mesh.clone()));
                surface.warp_bound = false;
            }
            log::info!(
                "Created dome surface '{}' (uuid {}) with {}x{} warp mesh",
                name,
                uuid,
                mesh.cols,
                mesh.rows
            );
        }

        // Store dome setup on surface manager
        self.output.surface_manager.dome_setup = Some(setup.clone());

        // Ensure the domemaster renderer is created and enabled
        self.ensure_domemaster();
    }
}

/// Compute the convex hull of a warp mesh's UV coordinates.
/// Returns polygon vertices in CCW order for use as a 2D surface shape.
fn convex_hull_of_uvs(mesh: &crate::renderer::warp::WarpMesh) -> Vec<[f32; 2]> {
    let mut points: Vec<[f32; 2]> = mesh.points.iter().map(|p| p.uv).collect();
    if points.len() < 3 {
        return points;
    }

    // Andrew's monotone chain convex hull algorithm
    points.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });
    points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);

    if points.len() < 3 {
        return points;
    }

    let n = points.len();
    let mut hull: Vec<[f32; 2]> = Vec::with_capacity(2 * n);

    // Lower hull
    for &p in &points {
        while hull.len() >= 2 && cross_2d(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }

    // Upper hull
    let lower_len = hull.len() + 1;
    for &p in points.iter().rev() {
        while hull.len() >= lower_len
            && cross_2d(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
        {
            hull.pop();
        }
        hull.push(p);
    }

    hull.pop(); // remove duplicate last point
    hull
}

/// 2D cross product of vectors OA and OB (positive = CCW turn).
fn cross_2d(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::warp::{MeshPoint, WarpMesh};

    #[test]
    fn convex_hull_of_unit_square_mesh() {
        let mesh = WarpMesh::identity(3, 3);
        let hull = convex_hull_of_uvs(&mesh);
        // Should be 4 corners of the unit square
        assert_eq!(hull.len(), 4);
        // Verify bounding box covers [0,0] to [1,1]
        let min_x = hull.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let max_x = hull.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let min_y = hull.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let max_y = hull.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
        assert!((min_x - 0.0).abs() < 1e-5);
        assert!((max_x - 1.0).abs() < 1e-5);
        assert!((min_y - 0.0).abs() < 1e-5);
        assert!((max_y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn convex_hull_triangle() {
        let mesh = WarpMesh {
            cols: 2,
            rows: 2,
            points: vec![
                MeshPoint {
                    position: [0.0, 0.0],
                    uv: [0.5, 0.0],
                },
                MeshPoint {
                    position: [1.0, 0.0],
                    uv: [1.0, 1.0],
                },
                MeshPoint {
                    position: [0.0, 1.0],
                    uv: [0.0, 1.0],
                },
                MeshPoint {
                    position: [1.0, 1.0],
                    uv: [0.5, 0.5],
                }, // UV inside triangle
            ],
        };
        let hull = convex_hull_of_uvs(&mesh);
        // The interior point (0.5, 0.5) should be excluded
        assert_eq!(hull.len(), 3);
    }

    #[test]
    fn convex_hull_small_mesh() {
        let mesh = WarpMesh {
            cols: 2,
            rows: 1,
            points: vec![
                MeshPoint {
                    position: [0.0, 0.0],
                    uv: [0.2, 0.3],
                },
                MeshPoint {
                    position: [1.0, 0.0],
                    uv: [0.8, 0.7],
                },
            ],
        };
        let hull = convex_hull_of_uvs(&mesh);
        assert_eq!(hull.len(), 2);
    }

    // ── Offensive: NaN float sort must not panic ──────────────────────

    #[test]
    fn convex_hull_nan_uv_does_not_panic() {
        let mesh = WarpMesh {
            cols: 3,
            rows: 2,
            points: vec![
                MeshPoint {
                    position: [0.0, 0.0],
                    uv: [f32::NAN, 0.0],
                },
                MeshPoint {
                    position: [1.0, 0.0],
                    uv: [0.5, f32::NAN],
                },
                MeshPoint {
                    position: [0.0, 1.0],
                    uv: [0.0, 1.0],
                },
                MeshPoint {
                    position: [1.0, 1.0],
                    uv: [1.0, 1.0],
                },
                MeshPoint {
                    position: [0.5, 0.5],
                    uv: [f32::NAN, f32::NAN],
                },
                MeshPoint {
                    position: [0.5, 0.0],
                    uv: [0.3, 0.7],
                },
            ],
        };
        // Must not panic — NaN comparisons fall back to Equal
        let _hull = convex_hull_of_uvs(&mesh);
    }

    #[test]
    fn convex_hull_all_nan_does_not_panic() {
        let mesh = WarpMesh {
            cols: 2,
            rows: 2,
            points: vec![
                MeshPoint {
                    position: [0.0, 0.0],
                    uv: [f32::NAN, f32::NAN],
                },
                MeshPoint {
                    position: [1.0, 0.0],
                    uv: [f32::NAN, f32::NAN],
                },
                MeshPoint {
                    position: [0.0, 1.0],
                    uv: [f32::NAN, f32::NAN],
                },
                MeshPoint {
                    position: [1.0, 1.0],
                    uv: [f32::NAN, f32::NAN],
                },
            ],
        };
        let _hull = convex_hull_of_uvs(&mesh);
    }

    #[test]
    fn convex_hull_infinity_uv_does_not_panic() {
        let mesh = WarpMesh {
            cols: 2,
            rows: 2,
            points: vec![
                MeshPoint {
                    position: [0.0, 0.0],
                    uv: [f32::INFINITY, 0.0],
                },
                MeshPoint {
                    position: [1.0, 0.0],
                    uv: [f32::NEG_INFINITY, 1.0],
                },
                MeshPoint {
                    position: [0.0, 1.0],
                    uv: [0.0, f32::INFINITY],
                },
                MeshPoint {
                    position: [1.0, 1.0],
                    uv: [1.0, 1.0],
                },
            ],
        };
        let _hull = convex_hull_of_uvs(&mesh);
    }
}
