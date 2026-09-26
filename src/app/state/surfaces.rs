//! Surface commands. Surfaces and the outputs they are assigned to live
//! together in [`Outputs`], so a surface removed or reshaped updates the
//! assignments and edge blending in the same place.

use super::super::Outputs;
use crate::engine::{CommandResult, ErrorCode};
use crate::surface::SurfaceReorderOp;

impl Outputs {
    /// Apply `f` to the surface a UUID names.
    pub(crate) fn edit_surface(
        &mut self,
        uuid: &str,
        f: impl FnOnce(&mut crate::surface::Surface),
    ) -> CommandResult {
        match self.surface_manager.surface_mut(uuid) {
            Ok(surface) => {
                f(surface);
                CommandResult::Ok
            }
            Err(e) => e.into(),
        }
    }

    /// [`Self::edit_surface`] for a change to a surface's shape, after which
    /// the outputs' automatic edge blending is derived again.
    pub(crate) fn reshape_surface(
        &mut self,
        uuid: &str,
        f: impl FnOnce(&mut crate::surface::Surface),
    ) -> CommandResult {
        let result = self.edit_surface(uuid, f);
        if matches!(result, CommandResult::Ok) {
            self.recompute_auto_edge_blend();
        }
        result
    }

    /// "Make Hole" (8i.7): convert `source_uuid` into a cut-out hole in the
    /// topmost *other* surface under its centroid, then remove the source
    /// surface (purging its output assignments). Atomic — target resolution,
    /// hole add, and source removal happen in one command.
    pub fn cmd_punch_surface_hole(&mut self, source_uuid: &str) -> CommandResult {
        let hole = match self.surface_manager.find_by_uuid(source_uuid) {
            Some((_, s)) => s.outline_as_path(),
            None => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: format!("Surface {source_uuid} not found"),
                };
            }
        };
        let Some(target_uuid) = self.surface_manager.resolve_hole_target(source_uuid) else {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "No surface beneath the selection to cut a hole into".into(),
            };
        };
        if let Some((_, target)) = self.surface_manager.find_by_uuid_mut(&target_uuid) {
            target.add_hole(hole);
        }
        self.surface_manager.remove_surface(source_uuid);
        for output in &mut self.outputs {
            output
                .surface_assignments_mut()
                .retain(|a| a.surface_uuid != source_uuid);
        }
        self.recompute_auto_edge_blend();
        CommandResult::Ok
    }

    pub fn cmd_remove_surface(&mut self, uuid: &str) -> CommandResult {
        self.surface_manager.remove_surface(uuid);
        // Purge dangling surface assignments from all outputs
        for output in &mut self.outputs {
            output
                .surface_assignments_mut()
                .retain(|a| a.surface_uuid != uuid);
        }
        self.recompute_auto_edge_blend();
        CommandResult::Ok
    }

    /// Remove the hole at `hole_index` from a surface.
    pub fn cmd_remove_surface_hole(&mut self, uuid: &str, hole_index: usize) -> CommandResult {
        if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
            if surface.remove_hole(hole_index) {
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            } else {
                CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: format!("Hole index {hole_index} out of range"),
                }
            }
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {uuid} not found"),
            }
        }
    }

    pub fn cmd_duplicate_surface(&mut self, uuid: &str) -> CommandResult {
        if let Some(new_uuid) = self.surface_manager.duplicate_surface(uuid) {
            CommandResult::OkWithId { uuid: new_uuid }
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {uuid} not found"),
            }
        }
    }

    /// Change a surface's stacking order (8i.12). Geometry is unchanged, so no
    /// edge-blend recompute is needed.
    pub fn cmd_reorder_surface(&mut self, uuid: &str, op: SurfaceReorderOp) -> CommandResult {
        if self.surface_manager.reorder_surface(uuid, op) {
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Surface {uuid} not found"),
            }
        }
    }

    pub fn cmd_combine_surfaces(&mut self, uuids: &[String]) -> CommandResult {
        if let Some(new_uuid) = self.surface_manager.combine_surfaces(uuids) {
            // Purge dangling assignments for combined (removed) surfaces
            for output in &mut self.outputs {
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
}
