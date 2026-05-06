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
                    let idx = self.surface_manager.add_surface(name.clone(), source.clone());
                    log::info!("Added surface '{}' (index {})", name, idx);
                }
                ui::SurfaceAction::AddPolygon { name, vertices, source } => {
                    let idx = self.surface_manager.add_polygon_surface(name.clone(), vertices.clone(), source.clone());
                    log::info!("Added polygon surface '{}' with {} vertices (index {})", name, vertices.len(), idx);
                }
                ui::SurfaceAction::Remove { idx } => {
                    if *idx < self.surface_manager.surfaces.len() {
                        let name = self.surface_manager.surfaces[*idx].name.clone();
                        self.surface_manager.remove_surface(*idx);
                        log::info!("Removed surface '{}'", name);
                    }
                }
                ui::SurfaceAction::UpdateVertices { idx, contour, vertices } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
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
                ui::SurfaceAction::MoveDelta { idx, dx, dy } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
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
                ui::SurfaceAction::SetSource { idx, source } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.source = source.clone();
                        log::info!("Surface '{}' source changed to: {}", surface.name, source);
                    }
                }
                ui::SurfaceAction::SetOutputType { idx, output_type } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.output_type = *output_type;
                        log::info!("Surface '{}' output type changed to: {}", surface.name, output_type);
                    }
                }
                ui::SurfaceAction::SetContentMapping { idx, mapping } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.content_mapping = *mapping;
                        log::info!("Surface '{}' content mapping changed to: {}", surface.name, mapping);
                    }
                }
                ui::SurfaceAction::Rename { idx, name } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        log::info!("Surface '{}' renamed to '{}'", surface.name, name);
                        surface.name = name.clone();
                    }
                }
                ui::SurfaceAction::Duplicate { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get(*idx).cloned() {
                        let mut dup = surface;
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
                        let name = dup.name.clone();
                        self.surface_manager.surfaces.push(dup);
                        log::info!("Duplicated surface '{}' → '{}'", self.surface_manager.surfaces[*idx].name, name);
                    }
                }
                ui::SurfaceAction::FlipHorizontal { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
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
                ui::SurfaceAction::FlipVertical { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
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
            ui::SurfaceAction::InsertVertex { idx, after_vert_idx, position } => {
                if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                    surface.convert_to_polygon();
                    if *after_vert_idx < surface.vertices.len() {
                        surface.vertices.insert(after_vert_idx + 1, *position);
                        log::info!("Inserted vertex on surface '{}' after vertex {}", surface.name, after_vert_idx);
                    }
                }
            }
            ui::SurfaceAction::AddCircle { name, hint, source } => {
                let idx = self.surface_manager.add_circle_surface(name.clone(), *hint, source.clone());
                log::info!("Added circle surface '{}' (index {}, radius={:.3}, sides={})", name, idx, hint.radius, hint.sides);
            }
            ui::SurfaceAction::SetCircleRadius { idx, radius } => {
                if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                    if let Some(ref mut hint) = surface.circle_hint {
                        hint.radius = *radius;
                        surface.vertices = hint.generate_vertices();
                        log::info!("Circle '{}' radius set to {:.3}", surface.name, radius);
                    }
                }
            }
            ui::SurfaceAction::SetCircleSides { idx, sides } => {
                if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                    if let Some(ref mut hint) = surface.circle_hint {
                        hint.sides = *sides;
                        surface.vertices = hint.generate_vertices();
                        log::info!("Circle '{}' sides set to {}", surface.name, sides);
                    }
                }
            }
            ui::SurfaceAction::ConvertToPolygon { idx } => {
                if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                    surface.convert_to_polygon();
                    log::info!("Converted surface '{}' to polygon", surface.name);
                }
            }
            ui::SurfaceAction::Combine { indices } => {
                if let Some(new_idx) = self.surface_manager.combine_surfaces(indices) {
                    let name = self.surface_manager.surfaces[new_idx].name.clone();
                    let contour_count = 1 + self.surface_manager.surfaces[new_idx].extra_contours.len();
                    log::info!("Combined {} surfaces into '{}' ({} contours)", indices.len(), name, contour_count);
                    self.notifications.info(format!("🔗 Combined {} surfaces → '{}'", indices.len(), name));
                }
            }
            _ => {} // Already handled in main match
        }
    }
}