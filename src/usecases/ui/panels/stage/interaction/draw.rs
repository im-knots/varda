//! Rectangle, polygon and circle tools: the create-new-surface gestures.

use super::super::super::super::{UIActions, UIData};
use super::super::hit_test::{CanvasGeometry, point_in_any_surface as hit_surface};
use super::super::state::DrawingTool;
use super::super::state::StageEditorState;
use crate::engine::EngineCommand;
use crate::renderer::context::OutputSource;

pub(super) fn rectangle(
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
    geom: CanvasGeometry,
) {
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm(pos);
        if let Some(uuid) = hit_surface(&data.surfaces, nx, ny) {
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid.clone());
            state.moving_surface = Some((uuid, nx, ny));
            state.tool = DrawingTool::Select;
        } else {
            state.rect_start = Some([nx, ny]);
        }
    }
    if resp.drag_stopped()
        && let Some(start) = state.rect_start.take()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let end = geom.to_norm(pos);
        let x0 = start[0].min(end[0]);
        let y0 = start[1].min(end[1]);
        let x1 = start[0].max(end[0]);
        let y1 = start[1].max(end[1]);
        if (x1 - x0) > 0.01 && (y1 - y0) > 0.01 {
            let idx = data.surfaces.len() + 1;
            actions.commands.push(EngineCommand::AddPolygonSurface {
                name: format!("Surface {idx}"),
                vertices: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                source: OutputSource::Master,
            });
        }
    }
}

pub(super) fn polygon(
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
    geom: CanvasGeometry,
) {
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let pt = geom.to_norm(pos);

        // If no polygon in progress and clicking inside existing surface, select it
        let mut handled = false;
        if state.polygon_verts.is_empty()
            && let Some(uuid) = hit_surface(&data.surfaces, pt[0], pt[1])
        {
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid);
            state.tool = DrawingTool::Select;
            handled = true;
        }

        // Check if clicking near first vertex to close
        if !handled && state.polygon_verts.len() >= 3 {
            let first = state.polygon_verts[0];
            let dx = pt[0] - first[0];
            let dy = pt[1] - first[1];
            let close_threshold = 15.0 / geom.width;
            if (dx * dx + dy * dy).sqrt() < close_threshold {
                // Close polygon
                let idx = data.surfaces.len() + 1;
                actions.commands.push(EngineCommand::AddPolygonSurface {
                    name: format!("Surface {idx}"),
                    vertices: state.polygon_verts.clone(),
                    source: OutputSource::Master,
                });
                state.polygon_verts.clear();
            } else {
                state.polygon_verts.push(pt);
            }
        } else if !handled {
            state.polygon_verts.push(pt);
        }
    }
    if resp.double_clicked() {
        // Finish polygon on double-click
        if state.polygon_verts.len() >= 3 {
            let idx = data.surfaces.len() + 1;
            actions.commands.push(EngineCommand::AddPolygonSurface {
                name: format!("Surface {idx}"),
                vertices: state.polygon_verts.clone(),
                source: OutputSource::Master,
            });
        }
        state.polygon_verts.clear();
    }
}

pub(super) fn circle(
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
    geom: CanvasGeometry,
) {
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm(pos);
        if let Some(uuid) = hit_surface(&data.surfaces, nx, ny) {
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid.clone());
            state.moving_surface = Some((uuid, nx, ny));
            state.tool = DrawingTool::Select;
        } else {
            state.circle_center = Some([nx, ny]);
        }
    }
    if resp.drag_stopped()
        && let Some(center) = state.circle_center.take()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let end = geom.to_norm(pos);
        let rx = (end[0] - center[0]).abs();
        let ry = (end[1] - center[1]).abs();
        let radius = (rx.max(ry)).max(0.02);
        let sides = state.circle_sides.max(3);
        let aspect_ratio = geom.width / geom.height;
        let idx = data.surfaces.len() + 1;
        actions.commands.push(EngineCommand::AddCircleSurface {
            name: format!("Surface {idx}"),
            center,
            radius,
            sides,
            aspect_ratio,
            source: OutputSource::Master,
        });
    }
}
