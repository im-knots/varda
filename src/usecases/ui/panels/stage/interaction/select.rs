//! Select tool: vertex, edge, surface, radius and gizmo dragging, plus marquee
//! selection. The largest interaction handler — it owns every gesture that edits
//! existing geometry rather than creating it.

use super::super::super::super::SurfaceUI;
use super::super::super::super::{UIActions, UIData};
use super::super::gizmo::{RotateDrag, ScaleDrag, try_begin_gizmo_drag};
use super::super::hit_test::CanvasGeometry;
use super::super::state::HitTestResult;
use super::super::state::StageEditorState;
use crate::engine::EngineCommand;

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::similar_names
)]
pub(super) fn handle(
    ui: &egui::Ui,
    painter: &egui::Painter,
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
    geom: CanvasGeometry,
) {
    // Helper: pixel-space distance between a normalized point and a vertex
    let pixel_dist = |nx: f32, ny: f32, vx: f32, vy: f32| -> f32 {
        let dx_px = (nx - vx) * geom.width;
        let dy_px = (ny - vy) * geom.height;
        (dx_px * dx_px + dy_px * dy_px).sqrt()
    };

    // Helper: find the closest edge of a specific surface within a threshold.
    // Returns (contour_idx, edge_start_idx, projected_point, distance_px).
    let find_closest_edge = |nx: f32,
                             ny: f32,
                             surface: &SurfaceUI,
                             threshold: f32|
     -> Option<(usize, usize, [f32; 2], f32)> {
        let contours: Vec<&Vec<[f32; 2]>> = std::iter::once(&surface.vertices)
            .chain(surface.extra_contours.iter())
            .collect();
        let mut best: Option<(usize, usize, [f32; 2], f32)> = None;
        for (ci, verts) in contours.iter().enumerate() {
            let n = verts.len();
            for ei in 0..n {
                let ej = (ei + 1) % n;
                let (ax, ay) = (verts[ei][0], verts[ei][1]);
                let (bx, by) = (verts[ej][0], verts[ej][1]);
                let dx = (bx - ax) * geom.width;
                let dy = (by - ay) * geom.height;
                let len_sq = dx * dx + dy * dy;
                if len_sq < 1e-6 {
                    continue;
                }
                let px_nx = (nx - ax) * geom.width;
                let px_ny = (ny - ay) * geom.height;
                let t = ((px_nx * dx + px_ny * dy) / len_sq).clamp(0.0, 1.0);
                let proj_x = ax + t * (bx - ax);
                let proj_y = ay + t * (by - ay);
                let d = pixel_dist(nx, ny, proj_x, proj_y);
                if d < threshold && best.as_ref().is_none_or(|b| d < b.3) {
                    best = Some((ci, ei, [proj_x, proj_y], d));
                }
            }
        }
        best
    };

    // Helper: find what's under the cursor
    // vertex: (surface_uuid, contour_idx, vertex_idx)
    // edge: (surface_uuid, contour_idx, edge_start_idx, projected_point)
    // surface: (surface_uuid, nx, ny)
    let hit_test = |nx: f32, ny: f32| -> HitTestResult {
        let vertex_threshold_px = 14.0;
        let edge_threshold_px = 10.0;
        // Wider threshold for edges when cursor is inside the surface.
        // This ensures top/right edges are grabbable from inside.
        let edge_inner_threshold_px = 24.0;
        let mut found_vertex = None;
        let mut found_edge = None;
        let mut found_surface = None;

        for surface in data.surfaces.iter().rev() {
            let uid = &surface.uuid;
            // Path-backed surfaces edit via the Bezier tool; their
            // flattened vertices/edges are not directly grabbable here.
            let is_path = surface.path.is_some();
            // Check all contours for vertex/edge hits
            let contours: Vec<&Vec<[f32; 2]>> = std::iter::once(&surface.vertices)
                .chain(surface.extra_contours.iter())
                .collect();
            for (ci, verts) in contours.iter().enumerate() {
                for (vi, v) in verts.iter().enumerate() {
                    if !is_path && pixel_dist(nx, ny, v[0], v[1]) < vertex_threshold_px {
                        found_vertex = Some((uid.clone(), ci, vi));
                        return (found_vertex, None, None);
                    }
                }
            }

            // Standard edge detection (narrow threshold, works from outside)
            if !is_path
                && found_edge.is_none()
                && let Some((ci, ei, proj, _d)) =
                    find_closest_edge(nx, ny, surface, edge_threshold_px)
            {
                found_edge = Some((uid.clone(), ci, ei, proj));
            }

            // Point-in-polygon (any contour)
            if found_surface.is_none() {
                let point_in = |verts: &[[f32; 2]]| -> bool {
                    let n = verts.len();
                    if n < 3 {
                        return false;
                    }
                    let mut inside = false;
                    let mut j = n - 1;
                    for k in 0..n {
                        let (xi, yi) = (verts[k][0], verts[k][1]);
                        let (xj, yj) = (verts[j][0], verts[j][1]);
                        if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi)
                        {
                            inside = !inside;
                        }
                        j = k;
                    }
                    inside
                };
                if point_in(&surface.vertices) || surface.extra_contours.iter().any(|c| point_in(c))
                {
                    found_surface = Some((uid.clone(), nx, ny));
                    // If cursor is inside the surface but no edge found yet,
                    // try again with a wider threshold to catch edges from inside.
                    if !is_path
                        && found_edge.is_none()
                        && let Some((ci, ei, proj, _d)) =
                            find_closest_edge(nx, ny, surface, edge_inner_threshold_px)
                    {
                        found_edge = Some((uid.clone(), ci, ei, proj));
                    }
                }
            }
        }
        (found_vertex, found_edge, found_surface)
    };

    // Hover feedback: change cursor when over interactive elements.
    // Hit-testing uses the raw (un-snapped) cursor so off-grid vertices
    // and edges remain grabbable; snapping applies only to placement.
    if let Some(pos) = resp.hover_pos() {
        let [nx, ny] = geom.to_norm_raw(pos);
        let (found_vertex, found_edge, found_surface) = hit_test(nx, ny);
        if found_vertex.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if found_edge.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else if found_surface.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }

    let shift_held = ui.input(|i| i.modifiers.shift);

    // Click to select (without drag)
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm_raw(pos);
        let (found_vertex, _found_edge, found_surface) = hit_test(nx, ny);
        if let Some((si, _ci, _vi)) = found_vertex {
            if shift_held {
                // Toggle selection with shift
                if !state.selected_surfaces.remove(&si) {
                    state.selected_surfaces.insert(si);
                }
            } else {
                state.selected_surfaces.clear();
                state.selected_surfaces.insert(si);
            }
        } else if let Some((si, _lx, _ly)) = found_surface {
            if shift_held {
                if !state.selected_surfaces.remove(&si) {
                    state.selected_surfaces.insert(si);
                }
            } else {
                state.selected_surfaces.clear();
                state.selected_surfaces.insert(si);
            }
        } else if !shift_held {
            state.selected_surfaces.clear();
        }
    }

    // Double-click on edge to insert vertex
    if resp.double_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm_raw(pos);
        let (_found_vertex, found_edge, _found_surface) = hit_test(nx, ny);
        if let Some((uuid, _ci, ei, snap_pos)) = found_edge {
            let snapped = [geom.snap(snap_pos[0]), geom.snap(snap_pos[1])];
            actions.commands.push(EngineCommand::InsertSurfaceVertex {
                uuid: uuid.clone(),
                after_vert_idx: ei,
                position: snapped,
            });
            state.selected_surfaces.clear();
            state.selected_surfaces.insert(uuid);
        }
    }

    // Drag start: begin radius drag, vertex drag, surface move, or marquee selection
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let [nx, ny] = geom.to_norm(pos);
        // Raw cursor for hit-testing; off-grid vertices/edges (e.g.
        // after a gizmo scale/rotate) stay grabbable. Placement and
        // drag-reference math below stay in snapped space.
        let [rnx, rny] = geom.to_norm_raw(pos);

        // Transform gizmo handles take priority over vertex/edge/body.
        // The gizmo hit-tests in raw pixels; nx,ny only seed the
        // rotate start angle (kept snapped to match the drag loop).
        let gizmo_consumed = try_begin_gizmo_drag(
            state,
            &data.surfaces,
            pos,
            nx,
            ny,
            geom.rect,
            geom.width,
            geom.height,
        );

        if !gizmo_consumed {
            // Check for radius handle hit on selected circles first
            let mut found_radius_handle = None;
            for sel_uuid in &state.selected_surfaces {
                if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *sel_uuid)
                    && let Some(hint) = &surface.circle_hint
                {
                    let hx = hint.center[0] + hint.radius;
                    let hy = hint.center[1];
                    if pixel_dist(rnx, rny, hx, hy) < 14.0 {
                        found_radius_handle = Some(sel_uuid.clone());
                        break;
                    }
                }
            }

            if let Some(uuid) = found_radius_handle {
                state.dragging_radius = Some(uuid);
                state.dragging_vertex = None;
                state.moving_surface = None;
                state.selection_rect_start = None;
                state.dragging_edge = None;
            } else {
                let (found_vertex, found_edge, found_surface) = hit_test(rnx, rny);

                if let Some((uuid, ci, vi)) = found_vertex {
                    // If vertex drag on a circle, auto-convert to polygon first
                    if data
                        .surfaces
                        .iter()
                        .find(|s| s.uuid == uuid)
                        .is_some_and(|s| s.circle_hint.is_some())
                    {
                        actions
                            .commands
                            .push(EngineCommand::ConvertSurfaceToPolygon { uuid: uuid.clone() });
                    }
                    if !shift_held {
                        state.selected_surfaces.clear();
                    }
                    state.selected_surfaces.insert(uuid.clone());
                    state.dragging_vertex = Some((uuid, ci, vi));
                    state.moving_surface = None;
                    state.selection_rect_start = None;
                    state.dragging_edge = None;
                } else if let Some((uuid, ci, ei, _proj)) = found_edge {
                    // Edge drag: store original edge endpoints + grab point.
                    // Grab point is the snapped cursor so the drag loop
                    // (also snapped) starts with a zero delta — no jump.
                    if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == uuid) {
                        let verts = if ci == 0 {
                            &surface.vertices
                        } else {
                            &surface.extra_contours[ci - 1]
                        };
                        let ej = (ei + 1) % verts.len();
                        let v0 = verts[ei];
                        let v1 = verts[ej];
                        // Auto-convert circle to polygon before edge drag
                        if surface.circle_hint.is_some() {
                            actions
                                .commands
                                .push(EngineCommand::ConvertSurfaceToPolygon {
                                    uuid: uuid.clone(),
                                });
                        }
                        if !shift_held {
                            state.selected_surfaces.clear();
                        }
                        state.selected_surfaces.insert(uuid.clone());
                        state.dragging_edge = Some((uuid, ci, ei, v0, v1, [nx, ny]));
                        state.dragging_vertex = None;
                        state.moving_surface = None;
                        state.selection_rect_start = None;
                    }
                } else if let Some((uuid, _rx, _ry)) = found_surface {
                    if !shift_held && !state.selected_surfaces.contains(&uuid) {
                        state.selected_surfaces.clear();
                    }
                    state.selected_surfaces.insert(uuid.clone());
                    // Store the snapped grab point so the move loop
                    // (snapped) starts with a zero delta — no jump.
                    state.moving_surface = Some((uuid, nx, ny));
                    state.dragging_vertex = None;
                    state.selection_rect_start = None;
                    state.dragging_edge = None;
                } else {
                    if !shift_held {
                        state.selected_surfaces.clear();
                    }
                    state.selection_rect_start = Some([nx, ny]);
                    state.dragging_vertex = None;
                    state.moving_surface = None;
                    state.dragging_edge = None;
                }
            }
        }
    }

    if resp.dragged() {
        // A mutating drag (not marquee selection) is one undo gesture —
        // flag it so the runner collapses the drag into a single step.
        if state.dragging_rotate.is_some()
            || state.dragging_scale.is_some()
            || state.dragging_radius.is_some()
            || state.dragging_vertex.is_some()
            || state.dragging_edge.is_some()
            || state.moving_surface.is_some()
        {
            actions.session.gesture_active = true;
        }
        if let Some(pos) = resp.interact_pointer_pos() {
            let [nx, ny] = geom.to_norm(pos);

            if let Some(rot) = state.dragging_rotate {
                let angle = (ny - rot.center[1]).atan2(nx - rot.center[0]);
                let mut delta = angle - rot.last_angle;
                if delta > std::f32::consts::PI {
                    delta -= std::f32::consts::TAU;
                } else if delta < -std::f32::consts::PI {
                    delta += std::f32::consts::TAU;
                }
                for surf_uuid in &state.selected_surfaces {
                    if data.surfaces.iter().any(|s| s.uuid == *surf_uuid) {
                        actions.commands.push(EngineCommand::RotateSurface {
                            uuid: surf_uuid.clone(),
                            angle: delta,
                            pivot: rot.center,
                        });
                    }
                }
                state.dragging_rotate = Some(RotateDrag {
                    center: rot.center,
                    last_angle: angle,
                });
            } else if let Some(sc) = state.dragging_scale {
                let raw_sx = if sc.axis_x {
                    let d = sc.start_handle[0] - sc.pivot[0];
                    if d.abs() > 1e-5 {
                        (nx - sc.pivot[0]) / d
                    } else {
                        1.0
                    }
                } else {
                    1.0
                };
                let raw_sy = if sc.axis_y {
                    let d = sc.start_handle[1] - sc.pivot[1];
                    if d.abs() > 1e-5 {
                        (ny - sc.pivot[1]) / d
                    } else {
                        1.0
                    }
                } else {
                    1.0
                };
                let total_sx = raw_sx.max(0.05);
                let total_sy = raw_sy.max(0.05);
                let dsx = if sc.last_sx.abs() > 1e-5 {
                    total_sx / sc.last_sx
                } else {
                    1.0
                };
                let dsy = if sc.last_sy.abs() > 1e-5 {
                    total_sy / sc.last_sy
                } else {
                    1.0
                };
                for surf_uuid in &state.selected_surfaces {
                    if data.surfaces.iter().any(|s| s.uuid == *surf_uuid) {
                        actions.commands.push(EngineCommand::ScaleSurface {
                            uuid: surf_uuid.clone(),
                            sx: dsx,
                            sy: dsy,
                            pivot: sc.pivot,
                        });
                    }
                }
                state.dragging_scale = Some(ScaleDrag {
                    last_sx: total_sx,
                    last_sy: total_sy,
                    ..sc
                });
            } else if let Some(ref uuid) = state.dragging_radius {
                // Compute new radius from cursor distance to circle center
                if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid)
                    && let Some(hint) = &surface.circle_hint
                {
                    let dx = nx - hint.center[0];
                    let dy = ny - hint.center[1];
                    let new_radius = (dx * dx + dy * dy).sqrt().max(0.01);
                    actions.commands.push(EngineCommand::SetCircleRadius {
                        uuid: uuid.clone(),
                        radius: new_radius,
                    });
                }
            } else if let Some((ref uuid, ci, vi)) = state.dragging_vertex {
                if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                    let contour_verts = if ci == 0 {
                        Some(&surface.vertices)
                    } else {
                        surface.extra_contours.get(ci - 1)
                    };
                    if let Some(verts) = contour_verts {
                        let mut new_verts = verts.clone();
                        if vi < new_verts.len() {
                            new_verts[vi] = [nx, ny];
                            actions
                                .commands
                                .push(EngineCommand::UpdateSurfaceContourVertices {
                                    uuid: uuid.clone(),
                                    contour: ci,
                                    vertices: new_verts,
                                });
                        }
                    }
                }
            } else if let Some((ref uuid, ci, ei, orig_v0, orig_v1, grab_pt)) = state.dragging_edge
            {
                // Edge drag: move both edge endpoints by the cursor displacement
                // relative to where the user first grabbed the edge.
                let dx = nx - grab_pt[0];
                let dy = ny - grab_pt[1];
                if let Some(surface) = data.surfaces.iter().find(|s| s.uuid == *uuid) {
                    let contour_verts = if ci == 0 {
                        Some(&surface.vertices)
                    } else {
                        surface.extra_contours.get(ci - 1)
                    };
                    if let Some(verts) = contour_verts {
                        let mut new_verts = verts.clone();
                        let ej = (ei + 1) % new_verts.len();
                        new_verts[ei] = [
                            (orig_v0[0] + dx).clamp(0.0, 1.0),
                            (orig_v0[1] + dy).clamp(0.0, 1.0),
                        ];
                        new_verts[ej] = [
                            (orig_v1[0] + dx).clamp(0.0, 1.0),
                            (orig_v1[1] + dy).clamp(0.0, 1.0),
                        ];
                        actions
                            .commands
                            .push(EngineCommand::UpdateSurfaceContourVertices {
                                uuid: uuid.clone(),
                                contour: ci,
                                vertices: new_verts,
                            });
                    }
                }
            } else if let Some((ref moving_uuid, lx, ly)) = state.moving_surface {
                let dx = nx - lx;
                let dy = ny - ly;
                // Move ALL selected surfaces by the same delta
                for surf_uuid in &state.selected_surfaces {
                    if data.surfaces.iter().any(|s| s.uuid == *surf_uuid) {
                        actions.commands.push(EngineCommand::MoveSurface {
                            uuid: surf_uuid.clone(),
                            dx,
                            dy,
                        });
                    }
                }
                state.moving_surface = Some((moving_uuid.clone(), nx, ny));
            } else if let Some(start) = state.selection_rect_start {
                // Draw marquee selection rectangle
                let x0 = geom.rect.left() + start[0] * geom.width;
                let y0 = geom.rect.top() + start[1] * geom.height;
                let x1 = geom.rect.left() + nx * geom.width;
                let y1 = geom.rect.top() + ny * geom.height;
                let sel_rect = egui::Rect::from_two_pos(egui::pos2(x0, y0), egui::pos2(x1, y1));
                painter.rect_filled(
                    sel_rect,
                    0.0,
                    egui::Color32::from_rgba_premultiplied(80, 130, 255, 40),
                );
                painter.rect_stroke(
                    sel_rect,
                    0.0,
                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 130, 255)),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }

    if resp.drag_stopped() {
        // Finish marquee selection: select all surfaces that intersect the rect
        if let Some(start) = state.selection_rect_start
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let [nx, ny] = geom.to_norm(pos);
            let sel_min_x = start[0].min(nx);
            let sel_max_x = start[0].max(nx);
            let sel_min_y = start[1].min(ny);
            let sel_max_y = start[1].max(ny);

            for surface in &data.surfaces {
                // Compute bounding box of surface vertices
                let (mut bb_min_x, mut bb_min_y) = (f32::MAX, f32::MAX);
                let (mut bb_max_x, mut bb_max_y) = (f32::MIN, f32::MIN);
                for v in &surface.vertices {
                    bb_min_x = bb_min_x.min(v[0]);
                    bb_min_y = bb_min_y.min(v[1]);
                    bb_max_x = bb_max_x.max(v[0]);
                    bb_max_y = bb_max_y.max(v[1]);
                }
                // Check if surface bounding box overlaps the selection rect
                let intersects = bb_min_x < sel_max_x
                    && bb_max_x > sel_min_x
                    && bb_min_y < sel_max_y
                    && bb_max_y > sel_min_y;
                if intersects {
                    state.selected_surfaces.insert(surface.uuid.clone());
                }
            }
        }
        state.selection_rect_start = None;
        state.dragging_vertex = None;
        state.moving_surface = None;
        state.dragging_radius = None;
        state.dragging_edge = None;
        state.dragging_scale = None;
        state.dragging_rotate = None;
    }

    // Delete selected surfaces (handled below via keymap)
}
