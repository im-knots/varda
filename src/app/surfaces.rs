//! Surface action processing for VardaApp.

use super::VardaApp;
use crate::engine::EngineCommand;
use crate::usecases::ui;

impl VardaApp {
    /// Apply surface actions from UI.
    /// `grid_size` is the UI-consumer-owned grid size for duplicate offset.
    pub fn apply_surface_actions(&mut self, ui_actions: &ui::UIActions, grid_size: f32) {
        for action in &ui_actions.surface_actions {
            match action {
                ui::SurfaceAction::Add { name, source } =>
                    { self.execute_command(EngineCommand::AddSurface { name: name.clone(), source: source.clone() }); }
                ui::SurfaceAction::AddPolygon { name, vertices, source } =>
                    { self.execute_command(EngineCommand::AddPolygonSurface { name: name.clone(), vertices: vertices.clone(), source: source.clone() }); }
                ui::SurfaceAction::AddCircle { name, hint, source } =>
                    { self.execute_command(EngineCommand::AddCircleSurface { name: name.clone(), center: hint.center, radius: hint.radius, sides: hint.sides, aspect_ratio: hint.aspect_ratio, source: source.clone() }); }
                ui::SurfaceAction::Remove { uuid } =>
                    { self.execute_command(EngineCommand::RemoveSurface { uuid: uuid.clone() }); }
                ui::SurfaceAction::UpdateVertices { uuid, contour, vertices } =>
                    { self.execute_command(EngineCommand::UpdateSurfaceContourVertices { uuid: uuid.clone(), contour: *contour, vertices: vertices.clone() }); }
                ui::SurfaceAction::MoveDelta { uuid, dx, dy } =>
                    { self.execute_command(EngineCommand::MoveSurface { uuid: uuid.clone(), dx: *dx, dy: *dy }); }
                ui::SurfaceAction::SetSource { uuid, source } =>
                    { self.execute_command(EngineCommand::SetSurfaceSource { uuid: uuid.clone(), source: source.clone() }); }
                ui::SurfaceAction::SetOutputType { uuid, output_type } =>
                    { self.execute_command(EngineCommand::SetSurfaceOutputType { uuid: uuid.clone(), output_type: *output_type }); }
                ui::SurfaceAction::SetContentMapping { uuid, mapping } =>
                    { self.execute_command(EngineCommand::SetSurfaceContentMapping { uuid: uuid.clone(), mapping: *mapping }); }
                ui::SurfaceAction::Rename { uuid, name } =>
                    { self.execute_command(EngineCommand::RenameSurface { uuid: uuid.clone(), name: name.clone() }); }
                ui::SurfaceAction::Duplicate { uuid } => {
                    // Duplicate uses grid_size for offset, then MoveSurface
                    self.execute_command(EngineCommand::DuplicateSurface { uuid: uuid.clone() });
                    // Move the duplicate by grid_size offset
                    // DuplicateSurface returns OkWithId but we don't have the new UUID here,
                    // so we apply the offset using the last surface (just added)
                    if let Some(new_surface) = self.surface_manager.surfaces.last() {
                        let new_uuid = new_surface.uuid.clone();
                        self.execute_command(EngineCommand::MoveSurface { uuid: new_uuid, dx: grid_size, dy: grid_size });
                    }
                }
                ui::SurfaceAction::FlipHorizontal { uuid } =>
                    { self.execute_command(EngineCommand::FlipSurfaceHorizontal { uuid: uuid.clone() }); }
                ui::SurfaceAction::FlipVertical { uuid } =>
                    { self.execute_command(EngineCommand::FlipSurfaceVertical { uuid: uuid.clone() }); }
                ui::SurfaceAction::InsertVertex { uuid, after_vert_idx, position } =>
                    { self.execute_command(EngineCommand::InsertSurfaceVertex { uuid: uuid.clone(), after_vert_idx: *after_vert_idx, position: *position }); }
                ui::SurfaceAction::SetCircleRadius { uuid, radius } =>
                    { self.execute_command(EngineCommand::SetCircleRadius { uuid: uuid.clone(), radius: *radius }); }
                ui::SurfaceAction::SetCircleSides { uuid, sides } =>
                    { self.execute_command(EngineCommand::SetCircleSides { uuid: uuid.clone(), sides: *sides }); }
                ui::SurfaceAction::ConvertToPolygon { uuid } =>
                    { self.execute_command(EngineCommand::ConvertSurfaceToPolygon { uuid: uuid.clone() }); }
                ui::SurfaceAction::Combine { uuids } =>
                    { self.execute_command(EngineCommand::CombineSurfaces { uuids: uuids.clone() }); }
            }
        }
    }
}