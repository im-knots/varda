//! Surface action processing for VardaApp.

use super::VardaApp;
use crate::usecases::ui;

impl VardaApp {
    /// Apply surface actions from UI.
    /// `grid_size` is the UI-consumer-owned grid size for duplicate offset.
    pub fn apply_surface_actions(&mut self, ui_actions: &ui::UIActions, grid_size: f32) {
        for action in &ui_actions.surface_actions {
            match action {
                ui::SurfaceAction::Add { name, source } => {
                    let uuid = self.surface_manager.add_surface(name.clone(), source.clone());
                    log::info!("Added surface '{}' (uuid {})", name, uuid);
                }
                ui::SurfaceAction::AddPolygon { name, vertices, source } => {
                    let uuid = self.surface_manager.add_polygon_surface(name.clone(), vertices.clone(), source.clone());
                    log::info!("Added polygon surface '{}' with {} vertices (uuid {})", name, vertices.len(), uuid);
                }
                ui::SurfaceAction::Remove { uuid } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid(uuid) {
                        let name = surface.name.clone();
                        self.surface_manager.remove_surface(uuid);
                        log::info!("Removed surface '{}'", name);
                    }
                }
                ui::SurfaceAction::UpdateVertices { uuid, contour, vertices } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        if *contour == 0 {
                            if let Some(ref mut hint) = surface.circle_hint {
                                let n = vertices.len().max(1) as f32;
                                let sum = vertices.iter().fold([0.0f32, 0.0], |acc, v| {
                                    [acc[0] + v[0], acc[1] + v[1]]
                                });
                                hint.center = [sum[0] / n, sum[1] / n];
                            }
                            surface.vertices = vertices.clone();
                        } else if let Some(c) = surface.extra_contours.get_mut(*contour - 1) {
                            *c = vertices.clone();
                        }
                    }
                }
                ui::SurfaceAction::MoveDelta { uuid, dx, dy } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        surface.translate(*dx, *dy);
                        let n = surface.vertices.len().max(1) as f32;
                        let sum = surface.vertices.iter().fold([0.0f32, 0.0], |acc, v| {
                            [acc[0] + v[0], acc[1] + v[1]]
                        });
                        let new_center = [sum[0] / n, sum[1] / n];
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.center = new_center;
                        }
                    }
                }
                ui::SurfaceAction::SetSource { uuid, source } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        surface.source = source.clone();
                        log::info!("Surface '{}' source changed to: {}", surface.name, source);
                    }
                }
                ui::SurfaceAction::SetOutputType { uuid, output_type } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        surface.output_type = *output_type;
                        log::info!("Surface '{}' output type changed to: {}", surface.name, output_type);
                    }
                }
                ui::SurfaceAction::SetContentMapping { uuid, mapping } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        surface.content_mapping = *mapping;
                        log::info!("Surface '{}' content mapping changed to: {}", surface.name, mapping);
                    }
                }
                ui::SurfaceAction::Rename { uuid, name } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        log::info!("Surface '{}' renamed to '{}'", surface.name, name);
                        surface.name = name.clone();
                    }
                }
                ui::SurfaceAction::Duplicate { uuid } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid(uuid) {
                        let mut dup = surface.clone();
                        dup.uuid = crate::deck::generate_short_uuid();
                        dup.name = format!("{} (copy)", dup.name);
                        let offset = grid_size;
                        for v in &mut dup.vertices {
                            v[0] = (v[0] + offset).min(1.0);
                            v[1] = (v[1] + offset).min(1.0);
                        }
                        for contour in &mut dup.extra_contours {
                            for v in contour.iter_mut() {
                                v[0] = (v[0] + offset).min(1.0);
                                v[1] = (v[1] + offset).min(1.0);
                            }
                        }
                        if let Some(ref mut hint) = dup.circle_hint {
                            hint.center[0] = (hint.center[0] + offset).min(1.0);
                            hint.center[1] = (hint.center[1] + offset).min(1.0);
                        }
                        let orig_name = surface.name.clone();
                        let new_name = dup.name.clone();
                        self.surface_manager.surfaces.push(dup);
                        log::info!("Duplicated surface '{}' → '{}'", orig_name, new_name);
                    }
                }
                ui::SurfaceAction::FlipHorizontal { uuid } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        let bb = surface.bounding_box();
                        let cx = bb.x + bb.width / 2.0;
                        for v in &mut surface.vertices {
                            v[0] = cx + (cx - v[0]);
                        }
                        for contour in &mut surface.extra_contours {
                            for v in contour.iter_mut() {
                                v[0] = cx + (cx - v[0]);
                            }
                        }
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.center[0] = cx + (cx - hint.center[0]);
                        }
                        log::info!("Flipped surface '{}' horizontally", surface.name);
                    }
                }
                ui::SurfaceAction::FlipVertical { uuid } => {
                    if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                        let bb = surface.bounding_box();
                        let cy = bb.y + bb.height / 2.0;
                        for v in &mut surface.vertices {
                            v[1] = cy + (cy - v[1]);
                        }
                        for contour in &mut surface.extra_contours {
                            for v in contour.iter_mut() {
                                v[1] = cy + (cy - v[1]);
                            }
                        }
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.center[1] = cy + (cy - hint.center[1]);
                        }
                        log::info!("Flipped surface '{}' vertically", surface.name);
                    }
                }
                _ => { self.apply_surface_action_extended(action); }
            }
        }
    }


    fn apply_surface_action_extended(&mut self, action: &ui::SurfaceAction) {
        match action {
            ui::SurfaceAction::InsertVertex { uuid, after_vert_idx, position } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                    surface.convert_to_polygon();
                    if *after_vert_idx < surface.vertices.len() {
                        surface.vertices.insert(after_vert_idx + 1, *position);
                        log::info!("Inserted vertex on surface '{}' after vertex {}", surface.name, after_vert_idx);
                    }
                }
            }
            ui::SurfaceAction::AddCircle { name, hint, source } => {
                let uuid = self.surface_manager.add_circle_surface(name.clone(), *hint, source.clone());
                log::info!("Added circle surface '{}' (uuid {}, radius={:.3}, sides={})", name, uuid, hint.radius, hint.sides);
            }
            ui::SurfaceAction::SetCircleRadius { uuid, radius } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                    if let Some(ref mut hint) = surface.circle_hint {
                        hint.radius = *radius;
                        surface.vertices = hint.generate_vertices();
                        log::info!("Circle '{}' radius set to {:.3}", surface.name, radius);
                    }
                }
            }
            ui::SurfaceAction::SetCircleSides { uuid, sides } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                    if let Some(ref mut hint) = surface.circle_hint {
                        hint.sides = *sides;
                        surface.vertices = hint.generate_vertices();
                        log::info!("Circle '{}' sides set to {}", surface.name, sides);
                    }
                }
            }
            ui::SurfaceAction::ConvertToPolygon { uuid } => {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid_mut(uuid) {
                    surface.convert_to_polygon();
                    log::info!("Converted surface '{}' to polygon", surface.name);
                }
            }
            ui::SurfaceAction::Combine { uuids } => {
                if let Some(new_uuid) = self.surface_manager.combine_surfaces(uuids) {
                    if let Some((_, combined)) = self.surface_manager.find_by_uuid(&new_uuid) {
                        let name = combined.name.clone();
                        let contour_count = 1 + combined.extra_contours.len();
                        log::info!("Combined {} surfaces into '{}' ({} contours)", uuids.len(), name, contour_count);
                        self.notifications.info(format!("🔗 Combined {} surfaces → '{}'", uuids.len(), name));
                    }
                }
            }
            _ => {} // Already handled in main match
        }
    }
}