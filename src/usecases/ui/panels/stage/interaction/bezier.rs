//! Bezier tool: curve authoring plus anchor and tangent-handle dragging.

use super::super::super::super::{UIActions, UIData};
use super::super::hit_test::CanvasGeometry;
use super::super::state::StageEditorState;
use crate::engine::EngineCommand;
use crate::surface::{CubicHandle, PathSegment};

#[allow(clippy::too_many_lines, clippy::similar_names)]
pub(super) fn handle(
    ui: &egui::Ui,
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
    geom: CanvasGeometry,
) {
    let pixel_dist = |nx: f32, ny: f32, vx: f32, vy: f32| -> f32 {
        let dx_px = (nx - vx) * geom.width;
        let dy_px = (ny - vy) * geom.height;
        (dx_px * dx_px + dy_px * dy_px).sqrt()
    };
    let seg_dist = |nx: f32, ny: f32, a: [f32; 2], b: [f32; 2]| -> f32 {
        let dx = (b[0] - a[0]) * geom.width;
        let dy = (b[1] - a[1]) * geom.height;
        let len_sq = dx * dx + dy * dy;
        if len_sq < 1e-6 {
            return pixel_dist(nx, ny, a[0], a[1]);
        }
        let px = (nx - a[0]) * geom.width;
        let py = (ny - a[1]) * geom.height;
        let t = ((px * dx + py * dy) / len_sq).clamp(0.0, 1.0);
        pixel_dist(nx, ny, a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]))
    };
    let polyline_dist = |nx: f32, ny: f32, pts: &[[f32; 2]]| -> f32 {
        pts.windows(2)
            .map(|w| seg_dist(nx, ny, w[0], w[1]))
            .fold(f32::MAX, f32::min)
    };
    let handle_hit_px = 16.0;
    let anchor_hit_px = 16.0;
    let edge_hit_px = 10.0;

    let hit_handle = |nx: f32, ny: f32| -> Option<(String, usize, CubicHandle)> {
        for surface in data.surfaces.iter().rev() {
            if let Some(path) = &surface.path {
                for (si, seg) in path.segments.iter().enumerate() {
                    if let PathSegment::Cubic { c1, c2, .. } = seg {
                        if pixel_dist(nx, ny, c1[0], c1[1]) < handle_hit_px {
                            return Some((surface.uuid.clone(), si, CubicHandle::C1));
                        }
                        if pixel_dist(nx, ny, c2[0], c2[1]) < handle_hit_px {
                            return Some((surface.uuid.clone(), si, CubicHandle::C2));
                        }
                    }
                }
            }
        }
        None
    };
    let hit_anchor = |nx: f32, ny: f32| -> Option<(String, usize)> {
        for surface in data.surfaces.iter().rev() {
            if let Some(path) = &surface.path {
                for ai in 0..path.anchor_count() {
                    let a = path.anchor_pos(ai);
                    if pixel_dist(nx, ny, a[0], a[1]) < anchor_hit_px {
                        return Some((surface.uuid.clone(), ai));
                    }
                }
            }
        }
        None
    };
    let hit_edge = |nx: f32, ny: f32| -> Option<(String, usize, bool)> {
        let mut best: Option<(String, usize, bool, f32)> = None;
        for surface in data.surfaces.iter().rev() {
            if let Some(path) = &surface.path {
                for ei in 0..path.edge_count() {
                    let d = polyline_dist(nx, ny, &path.sample_edge(ei, 12));
                    if d < edge_hit_px && best.as_ref().is_none_or(|b| d < b.3) {
                        best = Some((surface.uuid.clone(), ei, path.is_edge_cubic(ei), d));
                    }
                }
            } else {
                let verts = &surface.vertices;
                let n = verts.len();
                for ei in 0..n {
                    let ej = (ei + 1) % n;
                    let d = seg_dist(nx, ny, verts[ei], verts[ej]);
                    if d < edge_hit_px && best.as_ref().is_none_or(|b| d < b.3) {
                        best = Some((surface.uuid.clone(), ei, false, d));
                    }
                }
            }
        }
        best.map(|(u, e, c, _)| (u, e, c))
    };

    // Hover feedback.
    if let Some(pos) = resp.hover_pos() {
        let [nx, ny] = geom.to_norm_raw(pos);
        if hit_handle(nx, ny).is_some() || hit_anchor(nx, ny).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if hit_edge(nx, ny).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    // Click an edge to toggle line <-> cubic.
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm_raw(pos);
        if hit_handle(nx, ny).is_none()
            && hit_anchor(nx, ny).is_none()
            && let Some((uuid, edge_idx, is_cubic)) = hit_edge(nx, ny)
        {
            actions.commands.push(EngineCommand::ConvertSurfaceEdge {
                uuid: uuid.clone(),
                edge_idx,
                to_cubic: !is_cubic,
            });
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid);
        }
    }

    // Begin dragging a control handle or an anchor.
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm_raw(pos);
        if let Some((uuid, si, handle)) = hit_handle(nx, ny) {
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid.clone());
            state.dragging_handle = Some((uuid, si, handle));
            state.dragging_anchor = None;
        } else if let Some((uuid, ai)) = hit_anchor(nx, ny) {
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid.clone());
            state.dragging_anchor = Some((uuid, ai));
            state.dragging_handle = None;
        }
    }

    // Apply the active drag.
    if resp.dragged() {
        // Bezier anchor/handle drag is one undo gesture.
        if state.dragging_handle.is_some() || state.dragging_anchor.is_some() {
            actions.session.gesture_active = true;
        }
        if let Some(pos) = resp.interact_pointer_pos() {
            let [nx, ny] = geom.to_norm_raw(pos);
            if let Some((ref uuid, si, handle)) = state.dragging_handle {
                actions.commands.push(EngineCommand::MovePathHandle {
                    uuid: uuid.clone(),
                    segment_idx: si,
                    handle,
                    pos: [nx, ny],
                });
            } else if let Some((ref uuid, ai)) = state.dragging_anchor {
                actions.commands.push(EngineCommand::MovePathAnchor {
                    uuid: uuid.clone(),
                    anchor_idx: ai,
                    pos: [nx, ny],
                });
            }
        }
    }

    if resp.drag_stopped() {
        state.dragging_handle = None;
        state.dragging_anchor = None;
    }
}
