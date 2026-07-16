//! Surface command state mutations.

use super::super::VardaApp;
use crate::engine::traits::SurfaceCommands;
use crate::engine::{CommandResult, ErrorCode};
use crate::surface::{CubicHandle, SurfacePath, SurfaceReorderOp};

impl VardaApp {
    /// Add a subtractive hole (8i.7) to a surface from a closed path.
    pub fn cmd_add_surface_hole(&mut self, uuid: &str, hole: SurfacePath) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.add_hole(hole);
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    /// Remove the hole at `hole_index` from a surface.
    pub fn cmd_remove_surface_hole(&mut self, uuid: &str, hole_index: usize) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            if surface.remove_hole(hole_index) {
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            } else {
                CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: format!("Hole index {} out of range", hole_index),
                }
            }
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    /// "Make Hole" (8i.7): convert `source_uuid` into a cut-out hole in the
    /// topmost *other* surface under its centroid, then remove the source
    /// surface (purging its output assignments). Atomic — target resolution,
    /// hole add, and source removal happen in one command.
    pub fn cmd_punch_surface_hole(&mut self, source_uuid: &str) -> CommandResult {
        let hole = match self.output.surface_manager.find_by_uuid(source_uuid) {
            Some((_, s)) => s.outline_as_path(),
            None => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: format!("Surface {} not found", source_uuid),
                }
            }
        };
        let target_uuid = match self.output.surface_manager.resolve_hole_target(source_uuid) {
            Some(t) => t,
            None => {
                return CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: "No surface beneath the selection to cut a hole into".into(),
                }
            }
        };
        if let Some((_, target)) = self.output.surface_manager.find_by_uuid_mut(&target_uuid) {
            target.add_hole(hole);
        }
        self.remove_surface(source_uuid);
        for output in &mut self.output.outputs {
            output
                .surface_assignments_mut()
                .retain(|a| a.surface_uuid != source_uuid);
        }
        self.recompute_auto_edge_blend();
        CommandResult::Ok
    }

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

    /// Change a surface's stacking order (8i.12). Geometry is unchanged, so no
    /// edge-blend recompute is needed.
    pub fn cmd_reorder_surface(&mut self, uuid: &str, op: SurfaceReorderOp) -> CommandResult {
        if self.output.surface_manager.reorder_surface(uuid, op) {
            CommandResult::Ok
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

    /// Rotate a surface by `angle` radians around `pivot` (normalized coords).
    /// Vertices, extra contours, curve path and circle hint are rotated in step.
    pub fn cmd_rotate_surface(&mut self, uuid: &str, angle: f32, pivot: [f32; 2]) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.rotate(angle, pivot);
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    /// Scale a surface by `(sx, sy)` around `pivot` (normalized coords).
    /// Vertices, extra contours, curve path and circle hint are scaled in step.
    pub fn cmd_scale_surface(
        &mut self,
        uuid: &str,
        sx: f32,
        sy: f32,
        pivot: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.scale(sx, sy, pivot);
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    /// Convert edge `edge_idx` of a surface's curve path to a cubic bezier or
    /// back to a straight line. Lazily builds a path from the polygon if needed.
    pub fn cmd_convert_surface_edge(
        &mut self,
        uuid: &str,
        edge_idx: usize,
        to_cubic: bool,
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.convert_edge(edge_idx, to_cubic);
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    /// Move a curve-path anchor to `pos` (normalized coords), regenerating verts.
    pub fn cmd_move_path_anchor(
        &mut self,
        uuid: &str,
        anchor_idx: usize,
        pos: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.move_path_anchor(anchor_idx, pos);
            self.recompute_auto_edge_blend();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {} not found", uuid),
            }
        }
    }

    /// Move a cubic control handle of segment `segment_idx` to `pos`.
    pub fn cmd_move_path_handle(
        &mut self,
        uuid: &str,
        segment_idx: usize,
        handle: CubicHandle,
        pos: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(uuid) {
            surface.move_path_handle(segment_idx, handle, pos);
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
            if !assignments.iter().any(|a| a.surface_uuid == surface_uuid)
                && self
                    .output
                    .surface_manager
                    .find_by_uuid(surface_uuid)
                    .is_some()
            {
                assignments.push(crate::renderer::context::SurfaceAssignment {
                    surface_uuid: surface_uuid.to_string(),
                    enabled: true,
                    overlap_zones: Default::default(),
                });
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
