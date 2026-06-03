//! Surface command state mutations.

use super::super::VardaApp;
use crate::engine::traits::SurfaceCommands;
use crate::engine::{CommandResult, ErrorCode};

impl VardaApp {
    pub fn cmd_remove_surface(&mut self, uuid: &str) -> CommandResult {
        self.remove_surface(uuid);
        // Purge dangling surface assignments from all outputs
        for output in &mut self.output.outputs {
            output
                .surface_assignments_mut()
                .retain(|a| a.surface_uuid != uuid);
        }
        self.recompute_auto_edge_blend();
        CommandResult::Ok
    }

    pub fn cmd_update_surface_vertices(
        &mut self,
        uuid: &str,
        vertices: Vec<[f32; 2]>,
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.vertices = vertices;
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_duplicate_surface(&mut self, uuid: &str) -> CommandResult {
        if let Some(new_uuid) = self.output.surface_manager.duplicate_surface(uuid) {
            CommandResult::OkWithId { uuid: new_uuid }
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_flip_surface_horizontal(&mut self, uuid: &str) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            for v in &mut surface.vertices {
                v[0] = 1.0 - v[0];
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_flip_surface_vertical(&mut self, uuid: &str) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            for v in &mut surface.vertices {
                v[1] = 1.0 - v[1];
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_insert_surface_vertex(
        &mut self,
        uuid: &str,
        after_vert_idx: usize,
        position: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.convert_to_polygon();
            if after_vert_idx < surface.vertices.len() {
                surface.vertices.insert(after_vert_idx + 1, position);
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_set_circle_radius(&mut self, uuid: &str, radius: f32) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            if let Some(ref mut hint) = surface.circle_hint {
                hint.radius = radius;
                surface.vertices = hint.generate_vertices();
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_set_circle_sides(&mut self, uuid: &str, sides: u32) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            if let Some(ref mut hint) = surface.circle_hint {
                hint.sides = sides;
                surface.vertices = hint.generate_vertices();
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_convert_surface_to_polygon(&mut self, uuid: &str) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.convert_to_polygon();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_combine_surfaces(&mut self, uuids: &[String]) -> CommandResult {
        if let Some(new_uuid) = self.output.surface_manager.combine_surfaces(uuids) {
            // Purge dangling assignments for combined (removed) surfaces
            for output in &mut self.output.outputs {
                output
                    .surface_assignments_mut()
                    .retain(|a| !uuids.contains(&a.surface_uuid) || a.surface_uuid == new_uuid);
            }
            self.recompute_auto_edge_blend();
            CommandResult::OkWithId { uuid: new_uuid }
        } else {
            CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "Failed to combine surfaces".into(),
            }
        }
    }

    pub fn cmd_move_surface(&mut self, uuid: &str, dx: f32, dy: f32) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.translate(dx, dy);
            if let Some(ref mut hint) = surface.circle_hint {
                let n = surface.vertices.len().max(1) as f32;
                let sum = surface
                    .vertices
                    .iter()
                    .fold([0.0f32, 0.0], |acc, v| [acc[0] + v[0], acc[1] + v[1]]);
                hint.center = [sum[0] / n, sum[1] / n];
            }
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_update_surface_contour_vertices(
        &mut self,
        uuid: &str,
        contour: usize,
        vertices: Vec<[f32; 2]>,
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            if contour == 0 {
                if let Some(ref mut hint) = surface.circle_hint {
                    let n = vertices.len().max(1) as f32;
                    let sum = vertices
                        .iter()
                        .fold([0.0f32, 0.0], |acc, v| [acc[0] + v[0], acc[1] + v[1]]);
                    hint.center = [sum[0] / n, sum[1] / n];
                }
                surface.vertices = vertices;
            } else if let Some(c) = surface.extra_contours.get_mut(contour - 1) {
                *c = vertices;
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    pub fn cmd_assign_surface_to_output_by_idx(
        &mut self,
        output_idx: usize,
        surface_uuid: &str,
    ) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(output_idx) {
            let assignments = output.surface_assignments_mut();
            if !assignments.iter().any(|a| a.surface_uuid == surface_uuid) {
                if let Some((_, surface)) = self.output.surface_manager.find_by_uuid(surface_uuid) {
                    let bb = surface.bounding_box();
                    let assignment = crate::renderer::context::SurfaceAssignment {
                        surface_uuid: surface_uuid.to_string(),
                        warp_mode: crate::renderer::warp::WarpMode::identity_corners([
                            bb.x, bb.y, bb.width, bb.height,
                        ]),
                        enabled: true,
                        overlap_zones: Default::default(),
                    };
                    assignments.push(assignment);
                }
            }
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }

    pub fn cmd_unassign_surface_from_output_by_idx(
        &mut self,
        output_idx: usize,
        assignment_idx: usize,
    ) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(output_idx) {
            let assignments = output.surface_assignments_mut();
            if assignment_idx < assignments.len() {
                assignments.remove(assignment_idx);
            }
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }
}
