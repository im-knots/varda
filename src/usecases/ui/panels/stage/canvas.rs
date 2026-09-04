//! Stage canvas painting: background, grid, surfaces, the transform gizmo, and
//! in-progress shape previews.
//!
//! Read-only with respect to editor state — every gesture that mutates state
//! lives in [`super::interaction`]. Takes the frame's [`CanvasGeometry`] rather
//! than recomputing the screen mapping.

use super::super::super::UIData;
use super::geometry::polygon_shape;
use super::gizmo::{
    GIZMO_MARGIN_PX, GIZMO_ROTATE_OFFSET_PX, gizmo_scale_handles, selection_bounds,
};
use super::hit_test::CanvasGeometry;
use super::state::{DrawingTool, StageEditorState};
use crate::surface::PathSegment;

#[allow(clippy::too_many_lines, clippy::similar_names)]
pub(super) fn paint(
    painter: &egui::Painter,
    resp: &egui::Response,
    data: &UIData,
    state: &StageEditorState,
    geom: CanvasGeometry,
) {
    // Canvas background
    painter.rect_filled(geom.rect, 0.0, egui::Color32::from_rgb(10, 10, 18));

    // Grid lines
    if geom.grid_size > 0.001 {
        let steps = (1.0 / geom.grid_size).round() as usize;
        for i in 1..steps {
            let t = i as f32 * geom.grid_size;
            let x = geom.rect.left() + t * geom.width;
            let y = geom.rect.top() + t * geom.height;
            if x < geom.rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(x, geom.rect.top()),
                        egui::pos2(x, geom.rect.bottom()),
                    ],
                    egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(25, 25, 38)),
                );
            }
            if y < geom.rect.bottom() {
                painter.line_segment(
                    [
                        egui::pos2(geom.rect.left(), y),
                        egui::pos2(geom.rect.right(), y),
                    ],
                    egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(25, 25, 38)),
                );
            }
        }
    }

    // Draw surfaces
    let surface_colors = [
        egui::Color32::from_rgb(80, 140, 220),
        egui::Color32::from_rgb(220, 120, 80),
        egui::Color32::from_rgb(80, 200, 120),
        egui::Color32::from_rgb(200, 80, 200),
        egui::Color32::from_rgb(200, 200, 80),
        egui::Color32::from_rgb(80, 200, 200),
    ];

    for (i, surface) in data.surfaces.iter().enumerate() {
        let color = surface_colors[i % surface_colors.len()];
        let is_selected = state.selected_surfaces.contains(&surface.uuid);
        let fill_alpha = if is_selected { 120 } else { 60 };
        let fill = egui::Color32::from_rgba_premultiplied(
            color.r() / 3,
            color.g() / 3,
            color.b() / 3,
            fill_alpha,
        );
        let stroke_width = if is_selected { 2.5_f32 } else { 1.5_f32 };

        let pixel_verts: Vec<egui::Pos2> = surface
            .vertices
            .iter()
            .map(|v| {
                egui::pos2(
                    geom.rect.left() + v[0] * geom.width,
                    geom.rect.top() + v[1] * geom.height,
                )
            })
            .collect();

        if pixel_verts.len() >= 3 {
            painter.add(polygon_shape(
                &pixel_verts,
                fill,
                egui::Stroke::new(stroke_width, color),
            ));
        }
        // Draw extra contours (combined non-overlapping surfaces)
        for ec in &surface.extra_contours {
            let ec_verts: Vec<egui::Pos2> = ec
                .iter()
                .map(|v| {
                    egui::pos2(
                        geom.rect.left() + v[0] * geom.width,
                        geom.rect.top() + v[1] * geom.height,
                    )
                })
                .collect();
            if ec_verts.len() >= 3 {
                painter.add(polygon_shape(
                    &ec_verts,
                    fill,
                    egui::Stroke::new(stroke_width, color),
                ));
            }
        }

        // Label
        let n = surface.vertices.len().max(1) as f32;
        let center = surface.vertices.iter().fold([0.0f32, 0.0], |acc, v| {
            [acc[0] + v[0] / n, acc[1] + v[1] / n]
        });
        let center_px = egui::pos2(
            geom.rect.left() + center[0] * geom.width,
            geom.rect.top() + center[1] * geom.height,
        );
        painter.text(
            center_px,
            egui::Align2::CENTER_CENTER,
            &surface.name,
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

        // For path-backed (bezier) surfaces: draw the anchor/handle overlay
        // instead of the dense flattened-vertex handles.
        if let Some(path) = &surface.path {
            let anchor_color = egui::Color32::from_rgb(90, 220, 220);
            let handle_color = egui::Color32::from_rgb(255, 180, 60);
            // Control handles + connector lines (Bezier tool only).
            if state.tool == DrawingTool::Bezier {
                for (si, seg) in path.segments.iter().enumerate() {
                    if let PathSegment::Cubic { c1, c2, .. } = seg {
                        let s_px = geom.to_screen(path.segment_start(si));
                        let e_px = geom.to_screen(seg.end());
                        let (c1_px, c2_px) = (geom.to_screen(*c1), geom.to_screen(*c2));
                        let line = egui::Stroke::new(1.0_f32, egui::Color32::GRAY);
                        painter.line_segment([s_px, c1_px], line);
                        painter.line_segment([e_px, c2_px], line);
                        for cp in [c1_px, c2_px] {
                            painter.circle_filled(cp, 5.0, handle_color);
                            painter.circle_stroke(
                                cp,
                                5.0,
                                egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
                            );
                        }
                    }
                }
            }
            // Anchors (always shown for path surfaces).
            for ai in 0..path.anchor_count() {
                let a_px = geom.to_screen(path.anchor_pos(ai));
                let r = egui::Rect::from_center_size(a_px, egui::vec2(9.0, 9.0));
                painter.rect_filled(r, 2.0, anchor_color);
                painter.rect_stroke(
                    r,
                    2.0,
                    egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
                    egui::StrokeKind::Outside,
                );
            }
        } else if is_selected && surface.circle_hint.is_some() {
            let Some(hint) = surface.circle_hint else {
                continue;
            };
            let cx_px = geom.rect.left() + hint.center[0] * geom.width;
            let cy_px = geom.rect.top() + hint.center[1] * geom.height;
            let center_pos = egui::pos2(cx_px, cy_px);
            // Radius ring — compute the pixel radius at angle=0
            let radius_px_x = hint.radius * geom.width;
            let radius_px_y = hint.radius * hint.aspect_ratio * geom.height;
            let avg_radius_px = f32::midpoint(radius_px_x, radius_px_y);
            // Center dot (white)
            painter.circle_filled(center_pos, 4.0, egui::Color32::WHITE);
            // Radius ring (yellow, dashed look via stroke)
            painter.circle_stroke(
                center_pos,
                avg_radius_px,
                egui::Stroke::new(1.0_f32, egui::Color32::YELLOW),
            );
            // Radius handle at angle=0 (yellow dot on the right)
            let handle_pos = egui::pos2(cx_px + radius_px_x, cy_px);
            painter.circle_filled(handle_pos, 6.0, egui::Color32::YELLOW);
            painter.circle_stroke(
                handle_pos,
                6.0,
                egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
            );
        } else {
            // Regular vertex handles (primary + extra contours)
            let handle_size = if is_selected { 10.0 } else { 7.0 };
            let handle_color = if is_selected {
                egui::Color32::WHITE
            } else {
                color
            };
            let draw_handles = |verts: &[egui::Pos2]| {
                for v in verts {
                    let handle_rect =
                        egui::Rect::from_center_size(*v, egui::vec2(handle_size, handle_size));
                    painter.rect_filled(handle_rect, 2.0, handle_color);
                    painter.rect_stroke(
                        handle_rect,
                        2.0,
                        egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
                        egui::StrokeKind::Outside,
                    );
                }
            };
            draw_handles(&pixel_verts);
            for ec in &surface.extra_contours {
                let ec_px: Vec<egui::Pos2> = ec
                    .iter()
                    .map(|v| {
                        egui::pos2(
                            geom.rect.left() + v[0] * geom.width,
                            geom.rect.top() + v[1] * geom.height,
                        )
                    })
                    .collect();
                draw_handles(&ec_px);
            }
        }
    }

    // ── Transform gizmo (Select tool, non-empty selection) ───────────
    if state.tool == DrawingTool::Select
        && let Some((bx, by, bw, bh)) = selection_bounds(&data.surfaces, &state.selected_surfaces)
    {
        let mx = GIZMO_MARGIN_PX / geom.width;
        let my = GIZMO_MARGIN_PX / geom.height;
        let (gx, gy, gw, gh) = (bx - mx, by - my, bw + 2.0 * mx, bh + 2.0 * my);
        let gcolor = egui::Color32::from_rgb(90, 200, 255);
        let box_rect =
            egui::Rect::from_two_pos(geom.to_screen([gx, gy]), geom.to_screen([gx + gw, gy + gh]));
        painter.rect_stroke(
            box_rect,
            0.0,
            egui::Stroke::new(1.0_f32, gcolor),
            egui::StrokeKind::Outside,
        );
        for (handle, _pivot, _ax, _ay) in gizmo_scale_handles(gx, gy, gw, gh) {
            let hr = egui::Rect::from_center_size(geom.to_screen(handle), egui::vec2(8.0, 8.0));
            painter.rect_filled(hr, 1.0, gcolor);
            painter.rect_stroke(
                hr,
                1.0,
                egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
                egui::StrokeKind::Outside,
            );
        }
        let top_mid = geom.to_screen([gx + gw * 0.5, gy]);
        let knob = egui::pos2(top_mid.x, top_mid.y - GIZMO_ROTATE_OFFSET_PX);
        painter.line_segment([top_mid, knob], egui::Stroke::new(1.0_f32, gcolor));
        painter.circle_filled(knob, 5.0, gcolor);
        painter.circle_stroke(knob, 5.0, egui::Stroke::new(1.0_f32, egui::Color32::BLACK));
    }

    // Draw in-progress polygon
    if !state.polygon_verts.is_empty() && state.tool == DrawingTool::Polygon {
        let pixel_verts: Vec<egui::Pos2> = state
            .polygon_verts
            .iter()
            .map(|v| {
                egui::pos2(
                    geom.rect.left() + v[0] * geom.width,
                    geom.rect.top() + v[1] * geom.height,
                )
            })
            .collect();
        for i in 0..pixel_verts.len() - 1 {
            painter.line_segment(
                [pixel_verts[i], pixel_verts[i + 1]],
                egui::Stroke::new(2.0_f32, egui::Color32::YELLOW),
            );
        }
        // Draw line from last vertex to cursor
        if let Some(pos) = resp.hover_pos()
            && let Some(last) = pixel_verts.last()
        {
            painter.line_segment(
                [*last, pos],
                egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_premultiplied(255, 255, 0, 128),
                ),
            );
        }
        for v in &pixel_verts {
            let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(8.0, 8.0));
            painter.rect_filled(handle_rect, 2.0, egui::Color32::YELLOW);
        }
    }

    // Draw subtractive holes (8i.7) as red outlines on every surface.
    let hole_color = egui::Color32::from_rgb(255, 80, 80);
    for surface in &data.surfaces {
        for contour in &surface.hole_contours {
            if contour.len() < 2 {
                continue;
            }
            let pts: Vec<egui::Pos2> = contour
                .iter()
                .map(|v| {
                    egui::pos2(
                        geom.rect.left() + v[0] * geom.width,
                        geom.rect.top() + v[1] * geom.height,
                    )
                })
                .collect();
            for i in 0..pts.len() {
                painter.line_segment(
                    [pts[i], pts[(i + 1) % pts.len()]],
                    egui::Stroke::new(1.5_f32, hole_color),
                );
            }
        }
    }

    // Draw in-progress rectangle preview
    if let Some(start) = state.rect_start
        && state.tool == DrawingTool::Rectangle
        && let Some(pos) = resp.hover_pos()
    {
        let end_x = (pos.x - geom.rect.left()) / geom.width;
        let end_y = (pos.y - geom.rect.top()) / geom.height;
        let (sx, sy) = (start[0], start[1]);
        let preview_rect = egui::Rect::from_two_pos(
            egui::pos2(
                geom.rect.left() + sx * geom.width,
                geom.rect.top() + sy * geom.height,
            ),
            egui::pos2(
                geom.rect.left() + end_x * geom.width,
                geom.rect.top() + end_y * geom.height,
            ),
        );
        painter.rect_stroke(
            preview_rect,
            0.0,
            egui::Stroke::new(2.0_f32, egui::Color32::YELLOW),
            egui::StrokeKind::Outside,
        );
    }

    // Draw in-progress circle preview
    if let Some(center) = state.circle_center
        && state.tool == DrawingTool::Circle
        && let Some(pos) = resp.hover_pos()
    {
        let cx_px = geom.rect.left() + center[0] * geom.width;
        let cy_px = geom.rect.top() + center[1] * geom.height;
        let radius = ((pos.x - cx_px).powi(2) + (pos.y - cy_px).powi(2)).sqrt();
        painter.circle_stroke(
            egui::pos2(cx_px, cy_px),
            radius,
            egui::Stroke::new(2.0_f32, egui::Color32::YELLOW),
        );
    }
}
