//! Per-surface warp editor: the stage editor's bottom-bar mode.
//!
//! Rendered by the bottom-bar dispatcher while the stage editor is open. Lives
//! beside the stage editor because the two share the surface-selection key in
//! [`stage_selection_id`], which `stage/mod.rs` publishes and this module reads.

use super::super::super::{SurfaceUI, UIActions, UIData};
use crate::engine::EngineCommand;

/// Shared context-memory key: the stage editor publishes its current surface
/// selection here so the bottom detail bar can target it.
pub(crate) fn stage_selection_id() -> egui::Id {
    egui::Id::new("varda_stage_selected_surfaces")
}

/// Upper bound on grid resolution offered by the steppers (engine clamps to 64).
const UI_MAX_WARP_SUBDIVISIONS: u32 = 16;

/// `(cols, rows)` of a surface's warp. `None` or a corner-pin reads as 2×2.
fn warp_grid_dims(warp: Option<&crate::renderer::warp::WarpMode>) -> (u32, u32) {
    match warp {
        Some(crate::renderer::warp::WarpMode::Mesh(m)) => (m.cols, m.rows),
        _ => (2, 2),
    }
}

fn corner_to_rc(i: usize) -> (usize, usize) {
    match i {
        0 => (0, 0),
        1 => (0, 1),
        2 => (1, 1),
        _ => (1, 0),
    }
}
fn rc_to_corner(row: usize, col: usize) -> usize {
    match (row, col) {
        (0, 0) => 0,
        (0, 1) => 1,
        (1, 1) => 2,
        _ => 3,
    }
}

/// Axis-aligned bbox `(x, y, w, h)` of a surface's primary contour.
// minx/miny/maxx/maxy are the idiomatic bbox names; renaming them hurts readability.
#[allow(clippy::similar_names)]
fn surface_bbox(surface: &SurfaceUI) -> [f32; 4] {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for v in &surface.vertices {
        minx = minx.min(v[0]);
        miny = miny.min(v[1]);
        maxx = maxx.max(v[0]);
        maxy = maxy.max(v[1]);
    }
    if !minx.is_finite() {
        return [0.0, 0.0, 1.0, 1.0];
    }
    [minx, miny, (maxx - minx).max(1e-4), (maxy - miny).max(1e-4)]
}

/// Bottom-bar content while the stage editor is open: the per-surface warp editor
/// for the single selected surface, else a hint.
pub(crate) fn render_stage_bottom_bar(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let selected: Vec<String> = ui
        .ctx()
        .memory(|m| m.data.get_temp::<Vec<String>>(stage_selection_id()))
        .unwrap_or_default();
    let sel: Vec<&SurfaceUI> = selected
        .iter()
        .filter_map(|u| data.surfaces.iter().find(|s| &s.uuid == u))
        .collect();
    if sel.len() == 1 {
        render_surface_warp_editor(ui, sel[0], data, actions);
    } else {
        let msg = if sel.is_empty() {
            "Select a surface on the stage to edit its warp"
        } else {
            "Select a single surface to edit its warp"
        };
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new(msg).weak());
        });
    }
}

/// Per-surface warp editor: subdivide steppers + a draggable grid canvas.
// r/c (row/col) and x/y/w/h (bbox) are the clearest names for this grid geometry.
#[allow(clippy::many_single_char_names)]
fn render_surface_warp_editor(
    ui: &mut egui::Ui,
    surface: &SurfaceUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    use crate::renderer::warp::WarpMode;
    let uuid = surface.uuid.clone();
    let bound = surface.warp_bound;
    let is_bezier = matches!(surface.warp, Some(WarpMode::Bezier(_)));
    // In bezier mode the steppers control the anchor cage; otherwise the mesh grid.
    let (cols, rows) = if let Some(WarpMode::Bezier(b)) = &surface.warp {
        (b.anchor_cols, b.anchor_rows)
    } else {
        warp_grid_dims(surface.warp.as_ref())
    };

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("⊞ Warp — {}", surface.name)).strong());
        ui.separator();
        let mut bind = bound;
        if ui
            .checkbox(&mut bind, "🔗 Bind to shape")
            .on_hover_text(
                "Auto-conform the warp grid to the surface outline; uncheck to fine-tune",
            )
            .changed()
        {
            actions.commands.push(EngineCommand::SetWarpBound {
                surface_uuid: uuid.clone(),
                bound: bind,
            });
        }
        ui.separator();
        ui.add_enabled_ui(!bound, |ui| {
            ui.label(
                egui::RichText::new(if is_bezier { "cage" } else { "grid" })
                    .weak()
                    .small(),
            );
            if ui
                .small_button("−")
                .on_hover_text("Fewer columns")
                .clicked()
            {
                actions.commands.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    cols.saturating_sub(1).max(2),
                    rows,
                ));
            }
            ui.label(egui::RichText::new(format!("{cols}")).monospace().small());
            if ui.small_button("+").on_hover_text("More columns").clicked() {
                actions.commands.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    (cols + 1).min(UI_MAX_WARP_SUBDIVISIONS),
                    rows,
                ));
            }
            ui.label(egui::RichText::new("×").weak().small());
            if ui.small_button("−").on_hover_text("Fewer rows").clicked() {
                actions.commands.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    cols,
                    rows.saturating_sub(1).max(2),
                ));
            }
            ui.label(egui::RichText::new(format!("{rows}")).monospace().small());
            if ui.small_button("+").on_hover_text("More rows").clicked() {
                actions.commands.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    cols,
                    (rows + 1).min(UI_MAX_WARP_SUBDIVISIONS),
                ));
            }
            ui.separator();
            // Curve ↔ grid mode toggle (8i.6).
            if is_bezier {
                if ui
                    .small_button("⊞ Grid")
                    .on_hover_text("Switch back to a straight mesh warp")
                    .clicked()
                {
                    actions.commands.push(EngineCommand::SetWarpSubdivisions {
                        surface_uuid: uuid.clone(),
                        cols,
                        rows,
                    });
                }
            } else if ui
                .small_button("〰 Curve")
                .on_hover_text("Convert to a smooth bezier warp with tangent handles")
                .clicked()
            {
                actions.commands.push(EngineCommand::ConvertWarpToBezier {
                    surface_uuid: uuid.clone(),
                });
            }
            ui.separator();
            if ui
                .small_button("↺ Reset")
                .on_hover_text("Clear warp")
                .clicked()
            {
                actions.commands.push(EngineCommand::ResetWarp {
                    surface_uuid: uuid.clone(),
                });
            }
        });
    });

    // Canvas sized to the remaining bottom-bar space, at the render aspect: the
    // warp cage is in normalised output coordinates, so a 16:9 canvas would put
    // the control points somewhere other than where they land on the output.
    let avail = ui.available_size();
    let canvas = crate::usecases::ui::panels::utils::preview_size(
        egui::vec2((avail.x - 8.0).max(64.0), (avail.y - 4.0).max(64.0)),
        data.render_width,
        data.render_height,
    );
    let canvas_width = canvas.x;
    let canvas_height = canvas.y;
    let (canvas_rect, resp) = ui.allocate_exact_size(canvas, egui::Sense::click_and_drag());
    let painter = ui.painter_at(canvas_rect);
    painter.rect_filled(canvas_rect, 2.0, egui::Color32::from_rgb(15, 15, 25));

    let to_screen = |nx: f32, ny: f32| {
        egui::pos2(
            canvas_rect.left() + nx * canvas_width,
            canvas_rect.top() + ny * canvas_height,
        )
    };
    let from_screen = |p: egui::Pos2| {
        [
            ((p.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0),
            ((p.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0),
        ]
    };

    // Bezier warp: dedicated cage editor (anchors + tangent handles).
    if let Some(WarpMode::Bezier(b)) = &surface.warp {
        render_bezier_canvas(ui, b, &uuid, bound, canvas_rect, &resp, actions);
        return;
    }

    // Handles as (row, col, pos). A `None` warp shows identity corners from bbox.
    let is_mesh = matches!(surface.warp, Some(WarpMode::Mesh(_)));
    let mut handles: Vec<(usize, usize, [f32; 2])> = Vec::new();
    match &surface.warp {
        Some(WarpMode::Mesh(mesh)) => {
            for r in 0..mesh.rows as usize {
                for c in 0..mesh.cols as usize {
                    handles.push((r, c, mesh.points[r * mesh.cols as usize + c].position));
                }
            }
        }
        Some(WarpMode::CornerPin { corners }) => {
            for (i, corner) in corners.iter().enumerate() {
                let (r, c) = corner_to_rc(i);
                handles.push((r, c, *corner));
            }
        }
        // Bezier cage handles are drawn/edited by a dedicated overlay (8i.6);
        // no mesh-point handles here.
        Some(WarpMode::Bezier(_)) => {}
        None => {
            let [x, y, w, h] = surface_bbox(surface);
            let corners = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
            for (i, corner) in corners.iter().enumerate() {
                let (r, c) = corner_to_rc(i);
                handles.push((r, c, *corner));
            }
        }
    }

    let grid_stroke = egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(120, 160, 200));
    let outline_stroke = egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(200, 200, 200));
    if let Some(WarpMode::Mesh(mesh)) = &surface.warp {
        let cc = mesh.cols as usize;
        let rr = mesh.rows as usize;
        let at = |r: usize, c: usize| {
            let p = mesh.points[r * cc + c].position;
            to_screen(p[0], p[1])
        };
        for r in 0..rr {
            for c in 0..cc {
                if c + 1 < cc {
                    painter.line_segment([at(r, c), at(r, c + 1)], grid_stroke);
                }
                if r + 1 < rr {
                    painter.line_segment([at(r, c), at(r + 1, c)], grid_stroke);
                }
            }
        }
    } else {
        // Corner-pin quad outline (TL, TR, BR, BL).
        let order = [(0usize, 0usize), (0, 1), (1, 1), (1, 0)];
        let pts: Vec<egui::Pos2> = order
            .iter()
            .map(|(r, c)| {
                let p = handles
                    .iter()
                    .find(|h| h.0 == *r && h.1 == *c)
                    .map_or([0.0, 0.0], |h| h.2);
                to_screen(p[0], p[1])
            })
            .collect();
        for i in 0..4 {
            painter.line_segment([pts[i], pts[(i + 1) % 4]], outline_stroke);
        }
    }

    let state_id = ui.id().with("surface_warp").with(uuid.as_str());
    let mut dragging: Option<(usize, usize)> = ui
        .memory(|m| m.data.get_temp::<Option<(usize, usize)>>(state_id))
        .flatten();

    let corner_colors = [
        egui::Color32::from_rgb(255, 100, 100),
        egui::Color32::from_rgb(100, 255, 100),
        egui::Color32::from_rgb(100, 100, 255),
        egui::Color32::from_rgb(255, 255, 100),
    ];
    for &(row, col, position) in &handles {
        let pos = to_screen(position[0], position[1]);
        let active = dragging == Some((row, col));
        if is_mesh {
            let is_corner =
                (row == 0 || row + 1 == rows as usize) && (col == 0 || col + 1 == cols as usize);
            let base = if is_corner { 5.5 } else { 4.0 };
            let r = if active { base + 2.0 } else { base };
            painter.circle_filled(pos, r, egui::Color32::from_rgb(120, 220, 255));
        } else {
            let ci = rc_to_corner(row, col);
            let r = if active { 10.0 } else { 8.0 };
            painter.circle_filled(pos, r, corner_colors[ci]);
        }
    }

    if resp.drag_started() && !bound {
        if let Some(p) = resp.interact_pointer_pos() {
            let mut best: Option<(usize, usize, f32)> = None;
            for &(row, col, position) in &handles {
                let d = p.distance(to_screen(position[0], position[1]));
                if d < 18.0 && best.is_none_or(|(_, _, bd)| d < bd) {
                    best = Some((row, col, d));
                }
            }
            if let Some((row, col, _)) = best {
                dragging = Some((row, col));
            }
        }
    }

    if let Some((row, col)) = dragging {
        if resp.dragged() {
            // Warp point drag is one undo gesture.
            actions.session.gesture_active = true;
            if let Some(p) = resp.interact_pointer_pos() {
                let np = from_screen(p);
                if is_mesh {
                    actions.commands.push(EngineCommand::SetWarpMeshPoint {
                        surface_uuid: uuid.clone(),
                        row,
                        col,
                        position: np,
                    });
                } else {
                    actions.commands.push(EngineCommand::SetWarpCorner {
                        surface_uuid: uuid.clone(),
                        corner_idx: rc_to_corner(row, col),
                        position: np,
                    });
                }
            }
        }
    }
    if resp.drag_stopped() {
        dragging = None;
    }
    ui.memory_mut(|m| m.data.insert_temp(state_id, dragging));
}

/// Build the subdivision action for the warp steppers: in bezier mode this
/// resizes the anchor cage, otherwise the mesh grid.
fn subdiv_action(is_bezier: bool, uuid: String, cols: u32, rows: u32) -> EngineCommand {
    if is_bezier {
        EngineCommand::SetBezierCageSubdivisions {
            surface_uuid: uuid,
            cols,
            rows,
        }
    } else {
        EngineCommand::SetWarpSubdivisions {
            surface_uuid: uuid,
            cols,
            rows,
        }
    }
}

/// A draggable target in the bezier warp cage editor.
#[derive(Clone, Copy, PartialEq)]
enum BezDrag {
    Anchor {
        r: usize,
        c: usize,
    },
    Handle {
        horizontal: bool,
        r: usize,
        c: usize,
        which: usize,
    },
}

/// Bezier warp cage editor (8i.6): faint tessellated grid + control cage
/// (anchors, tangent handles, connector lines) with drag interaction.
fn render_bezier_canvas(
    ui: &mut egui::Ui,
    b: &crate::renderer::warp::BezierWarp,
    uuid: &str,
    bound: bool,
    canvas_rect: egui::Rect,
    resp: &egui::Response,
    actions: &mut UIActions,
) {
    let (cw, ch) = (canvas_rect.width(), canvas_rect.height());
    let painter = ui.painter_at(canvas_rect);
    let to_screen = |p: [f32; 2]| {
        egui::pos2(
            canvas_rect.left() + p[0] * cw,
            canvas_rect.top() + p[1] * ch,
        )
    };
    let from_screen = |p: egui::Pos2| {
        [
            ((p.x - canvas_rect.left()) / cw).clamp(0.0, 1.0),
            ((p.y - canvas_rect.top()) / ch).clamp(0.0, 1.0),
        ]
    };
    let ac = b.anchor_cols as usize;
    let ar = b.anchor_rows as usize;

    // 1. Faint tessellated grid — shows the actual smooth warped surface.
    let mesh = b.tessellate();
    let (cc, rr) = (mesh.cols as usize, mesh.rows as usize);
    let grid = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 90, 120));
    let mat = |r: usize, c: usize| to_screen(mesh.points[r * cc + c].position);
    for r in 0..rr {
        for c in 0..cc {
            if c + 1 < cc {
                painter.line_segment([mat(r, c), mat(r, c + 1)], grid);
            }
            if r + 1 < rr {
                painter.line_segment([mat(r, c), mat(r + 1, c)], grid);
            }
        }
    }

    // 2. Handle positions (drawn + hit-tested with priority over anchors).
    let mut handles: Vec<(BezDrag, [f32; 2])> = Vec::new();
    for r in 0..ar {
        for c in 0..ac - 1 {
            let h = b.h_horiz[r * (ac - 1) + c];
            handles.push((
                BezDrag::Handle {
                    horizontal: true,
                    r,
                    c,
                    which: 0,
                },
                h[0],
            ));
            handles.push((
                BezDrag::Handle {
                    horizontal: true,
                    r,
                    c,
                    which: 1,
                },
                h[1],
            ));
        }
    }
    for r in 0..ar - 1 {
        for c in 0..ac {
            let h = b.h_vert[r * ac + c];
            handles.push((
                BezDrag::Handle {
                    horizontal: false,
                    r,
                    c,
                    which: 0,
                },
                h[0],
            ));
            handles.push((
                BezDrag::Handle {
                    horizontal: false,
                    r,
                    c,
                    which: 1,
                },
                h[1],
            ));
        }
    }

    // 3. Connector lines (anchor → its tangent handles).
    let hstroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(150, 150, 90));
    for r in 0..ar {
        for c in 0..ac - 1 {
            let h = b.h_horiz[r * (ac - 1) + c];
            painter.line_segment([to_screen(b.anchor(r, c)), to_screen(h[0])], hstroke);
            painter.line_segment([to_screen(b.anchor(r, c + 1)), to_screen(h[1])], hstroke);
        }
    }
    for r in 0..ar - 1 {
        for c in 0..ac {
            let h = b.h_vert[r * ac + c];
            painter.line_segment([to_screen(b.anchor(r, c)), to_screen(h[0])], hstroke);
            painter.line_segment([to_screen(b.anchor(r + 1, c)), to_screen(h[1])], hstroke);
        }
    }

    // 4. Handles (small yellow squares).
    for (_, p) in &handles {
        painter.rect_filled(
            egui::Rect::from_center_size(to_screen(*p), egui::vec2(7.0, 7.0)),
            1.0,
            egui::Color32::from_rgb(230, 210, 90),
        );
    }

    // 5. Anchors (cyan circles; corners larger).
    for r in 0..ar {
        for c in 0..ac {
            let is_corner = (r == 0 || r + 1 == ar) && (c == 0 || c + 1 == ac);
            painter.circle_filled(
                to_screen(b.anchor(r, c)),
                if is_corner { 6.0 } else { 4.5 },
                egui::Color32::from_rgb(120, 220, 255),
            );
        }
    }

    // 6. Drag interaction (handles take priority over anchors on tie).
    let state_id = ui.id().with("surface_bezier_warp").with(uuid);
    let mut drag: Option<BezDrag> = ui
        .memory(|m| m.data.get_temp::<Option<BezDrag>>(state_id))
        .flatten();
    if resp.drag_started() && !bound {
        if let Some(p) = resp.interact_pointer_pos() {
            let mut best: Option<(BezDrag, f32)> = None;
            for (t, pos) in handles.iter().copied() {
                let d = p.distance(to_screen(pos));
                if d < 16.0 && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((t, d));
                }
            }
            for r in 0..ar {
                for c in 0..ac {
                    let d = p.distance(to_screen(b.anchor(r, c)));
                    if d < 16.0 && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((BezDrag::Anchor { r, c }, d));
                    }
                }
            }
            if let Some((t, _)) = best {
                drag = Some(t);
            }
        }
    }
    if let Some(t) = drag {
        if resp.dragged() {
            // Bezier warp cage drag is one undo gesture.
            actions.session.gesture_active = true;
            if let Some(p) = resp.interact_pointer_pos() {
                let np = from_screen(p);
                match t {
                    BezDrag::Anchor { r, c } => {
                        actions.commands.push(EngineCommand::MoveWarpAnchor {
                            surface_uuid: uuid.to_string(),
                            row: r,
                            col: c,
                            position: np,
                        });
                    }
                    BezDrag::Handle {
                        horizontal,
                        r,
                        c,
                        which,
                    } => {
                        actions.commands.push(EngineCommand::MoveWarpHandle {
                            surface_uuid: uuid.to_string(),
                            horizontal,
                            row: r,
                            col: c,
                            which,
                            position: np,
                        });
                    }
                }
            }
        }
    }
    if resp.drag_stopped() {
        drag = None;
    }
    ui.memory_mut(|m| m.data.insert_temp(state_id, drag));
}
