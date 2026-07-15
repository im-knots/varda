//! Bottom panel and deck detail.

use super::super::{
    widgets, AutoTransitionAction, EffectDrag, LibraryDrag, ModulationAction, ParamUpdate,
    SurfaceAction, SurfaceUI, UIActions, UIData, VideoAction,
};
use super::effects::{render_channel_effect_detail, render_master_effect_detail};
use super::sequence::{render_sequence_step_editor, render_timeline_strip};
use super::utils::{
    channel_color, format_time, render_collapsed_column, render_effect_drag_ghost,
    render_effect_drag_handle, render_effect_drop_zone,
};
use crate::channel::DeckRenderFps;
use crate::params::ParamValue;
use crate::{BlendMode, ScalingMode};

/// Apply MIDI + keyboard learn affordances (glow + click-to-select) to a just-drawn
/// control. `path` is the parameter-router path the control binds to. The two learn
/// modes are mutually exclusive, so at most one overlay is active at a time.
fn learn_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: String,
    data: &UIData,
    actions: &mut UIActions,
) {
    if data.midi_learn_active {
        if data.midi_learn_target.as_deref() == Some(path.as_str()) {
            widgets::draw_midi_learn_selected(ui, rect);
        } else {
            widgets::draw_midi_learn_glow(ui, rect);
        }
        let id = ui.id().with(("midi_learn", path.as_str()));
        if ui.interact(rect, id, egui::Sense::click()).clicked() {
            actions.midi_learn_select = Some(path);
        }
    } else if data.keyboard_learn_active {
        if data.keyboard_learn_target.as_deref() == Some(path.as_str()) {
            widgets::draw_keyboard_learn_selected(ui, rect);
        } else {
            widgets::draw_keyboard_learn_glow(ui, rect);
        }
        let id = ui.id().with(("kb_learn", path.as_str()));
        if ui.interact(rect, id, egui::Sense::click()).clicked() {
            actions.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(path));
        }
    }
}

pub(super) fn render_bottom_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // MIDI learn status indicator
    if data.midi_learn_active {
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(4.0)
            .fill(egui::Color32::from_rgb(180, 80, 220))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(target) = &data.midi_learn_target {
                        ui.label(
                            egui::RichText::new(format!(
                                "🎹 MIDI LEARN — Move a control to map: {}",
                                target
                            ))
                            .strong()
                            .color(egui::Color32::WHITE),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("🎹 MIDI LEARN — Click a parameter to select it")
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("x Exit MIDI Learn").clicked() {
                            actions.midi_learn_toggle = true;
                        }
                    });
                });
            });
    }

    // While the stage editor is open the bottom bar hosts the per-surface warp
    // editor for the selected surface (8i.5).
    if data.stage_editor_open {
        render_stage_bottom_bar(ui, data, actions);
        return;
    }

    // Context-sensitive bottom bar: master effects, channel effects, sequence, or deck detail
    if data.selected_master {
        render_master_effect_detail(ui, data, actions);
    } else if let Some(ch_idx) = data.selected_channel {
        render_channel_effect_detail(ui, ch_idx, data, actions);
    } else if let Some(seq_idx) = data.selected_sequence {
        render_sequence_detail(ui, seq_idx, data, actions);
    } else {
        render_selected_deck_detail(ui, data, actions);
    }
}

/// Shared context-memory key: the stage editor publishes its current surface
/// selection here so the bottom detail bar can target it.
pub(super) fn stage_selection_id() -> egui::Id {
    egui::Id::new("varda_stage_selected_surfaces")
}

/// Upper bound on grid resolution offered by the steppers (engine clamps to 64).
const UI_MAX_WARP_SUBDIVISIONS: u32 = 16;

/// `(cols, rows)` of a surface's warp. `None` or a corner-pin reads as 2×2.
fn warp_grid_dims(warp: &Option<crate::renderer::warp::WarpMode>) -> (u32, u32) {
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
fn render_stage_bottom_bar(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let selected: Vec<String> = ui
        .ctx()
        .memory(|m| m.data.get_temp::<Vec<String>>(stage_selection_id()))
        .unwrap_or_default();
    let sel: Vec<&SurfaceUI> = selected
        .iter()
        .filter_map(|u| data.surfaces.iter().find(|s| &s.uuid == u))
        .collect();
    if sel.len() == 1 {
        render_surface_warp_editor(ui, sel[0], actions);
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
fn render_surface_warp_editor(ui: &mut egui::Ui, surface: &SurfaceUI, actions: &mut UIActions) {
    use crate::renderer::warp::WarpMode;
    let uuid = surface.uuid.clone();
    let bound = surface.warp_bound;
    let is_bezier = matches!(surface.warp, Some(WarpMode::Bezier(_)));
    // In bezier mode the steppers control the anchor cage; otherwise the mesh grid.
    let (cols, rows) = if let Some(WarpMode::Bezier(b)) = &surface.warp {
        (b.anchor_cols, b.anchor_rows)
    } else {
        warp_grid_dims(&surface.warp)
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
            actions.surface_actions.push(SurfaceAction::SetWarpBound {
                uuid: uuid.clone(),
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
                actions.surface_actions.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    cols.saturating_sub(1).max(2),
                    rows,
                ));
            }
            ui.label(egui::RichText::new(format!("{cols}")).monospace().small());
            if ui.small_button("+").on_hover_text("More columns").clicked() {
                actions.surface_actions.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    (cols + 1).min(UI_MAX_WARP_SUBDIVISIONS),
                    rows,
                ));
            }
            ui.label(egui::RichText::new("×").weak().small());
            if ui.small_button("−").on_hover_text("Fewer rows").clicked() {
                actions.surface_actions.push(subdiv_action(
                    is_bezier,
                    uuid.clone(),
                    cols,
                    rows.saturating_sub(1).max(2),
                ));
            }
            ui.label(egui::RichText::new(format!("{rows}")).monospace().small());
            if ui.small_button("+").on_hover_text("More rows").clicked() {
                actions.surface_actions.push(subdiv_action(
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
                    actions
                        .surface_actions
                        .push(SurfaceAction::SetWarpSubdivisions {
                            uuid: uuid.clone(),
                            cols,
                            rows,
                        });
                }
            } else if ui
                .small_button("〰 Curve")
                .on_hover_text("Convert to a smooth bezier warp with tangent handles")
                .clicked()
            {
                actions
                    .surface_actions
                    .push(SurfaceAction::ConvertWarpToBezier { uuid: uuid.clone() });
            }
            ui.separator();
            if ui
                .small_button("↺ Reset")
                .on_hover_text("Clear warp")
                .clicked()
            {
                actions
                    .surface_actions
                    .push(SurfaceAction::ResetWarp { uuid: uuid.clone() });
            }
        });
    });

    // Canvas sized to remaining bottom-bar space, capped to 16:9.
    let avail = ui.available_size();
    let canvas_width = (avail.x - 8.0).max(64.0);
    let canvas_height = (canvas_width * 0.5625).min((avail.y - 4.0).max(64.0));
    let (canvas_rect, resp) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );
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
                    .map(|h| h.2)
                    .unwrap_or([0.0, 0.0]);
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
            if let Some(p) = resp.interact_pointer_pos() {
                let np = from_screen(p);
                if is_mesh {
                    actions
                        .surface_actions
                        .push(SurfaceAction::SetWarpMeshPoint {
                            uuid: uuid.clone(),
                            row,
                            col,
                            position: np,
                        });
                } else {
                    actions.surface_actions.push(SurfaceAction::SetWarpCorner {
                        uuid: uuid.clone(),
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
fn subdiv_action(is_bezier: bool, uuid: String, cols: u32, rows: u32) -> SurfaceAction {
    if is_bezier {
        SurfaceAction::SetBezierCageSubdivisions { uuid, cols, rows }
    } else {
        SurfaceAction::SetWarpSubdivisions { uuid, cols, rows }
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
            if let Some(p) = resp.interact_pointer_pos() {
                let np = from_screen(p);
                match t {
                    BezDrag::Anchor { r, c } => {
                        actions.surface_actions.push(SurfaceAction::MoveWarpAnchor {
                            uuid: uuid.to_string(),
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
                        actions.surface_actions.push(SurfaceAction::MoveWarpHandle {
                            uuid: uuid.to_string(),
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

/// Render the selected deck's full details (params, effects, blend, scaling) in the bottom bar
pub(super) fn render_selected_deck_detail(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
) {
    ui.heading("🎛 Selected Deck");

    let Some((ch_idx, deck_idx)) = data.selected_deck else {
        ui.label(
            egui::RichText::new("Click a deck thumbnail to see its controls here")
                .weak()
                .small(),
        );
        return;
    };

    // Find the deck data
    let Some(ch) = data.channels.get(ch_idx) else {
        ui.label(egui::RichText::new("Channel not found").weak());
        return;
    };
    let Some(deck) = ch.decks.iter().find(|d| d.deck_idx == deck_idx) else {
        ui.label(egui::RichText::new("Deck not found").weak());
        return;
    };

    let accent = channel_color(ch_idx);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} / Deck {} — {}",
                ch.name,
                deck_idx + 1,
                deck.name
            ))
            .strong()
            .color(accent),
        );

        // Save as preset — inline name prompt
        let prompt_id = egui::Id::new("deck_preset_name_prompt");
        let name_id = egui::Id::new("deck_preset_name_input");
        let is_prompting: bool = ui.data(|d| d.get_temp(prompt_id)).unwrap_or(false);

        if is_prompting {
            let cleared_id = egui::Id::new("deck_preset_name_cleared");
            let was_cleared: bool = ui.data(|d| d.get_temp(cleared_id)).unwrap_or(false);
            let mut name: String = ui
                .data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| deck.name.clone());
            let response = ui.text_edit_singleline(&mut name);
            if response.gained_focus() && !was_cleared {
                name.clear();
                ui.data_mut(|d| d.insert_temp(cleared_id, true));
            }
            if ui.small_button("✓ Save").clicked() && !name.is_empty() {
                actions.save_deck_preset = Some((ch_idx, deck_idx, name.clone()));
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            if ui.small_button("✕").clicked() {
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            ui.data_mut(|d| d.insert_temp(name_id, name));
        } else if ui.small_button("💾 Save Preset").clicked() {
            ui.data_mut(|d| {
                d.insert_temp(prompt_id, true);
                d.remove_temp::<String>(name_id);
                d.insert_temp(egui::Id::new("deck_preset_name_cleared"), false);
            });
        }
    });

    // Horizontal columns: Preview | Generator | Effect 1 | Effect 2 | ... | Add Effect
    egui::ScrollArea::horizontal().id_salt("selected_deck_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            // Column 0: Deck preview — scales with bottom bar height
            if let Some(tex_id) = data.deck_preview_textures.get(&(ch_idx, deck_idx)) {
                let available_height = ui.available_height() - 12.0; // margin
                let preview_height = available_height.max(60.0);
                let preview_width = preview_height * 16.0 / 9.0;
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(preview_width + 12.0);
                        ui.set_max_width(preview_width + 12.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.image(egui::load::SizedTexture::new(*tex_id, egui::vec2(preview_width, preview_height)));
                            ui.label(egui::RichText::new(&deck.name).small().color(accent));
                        });
                    });
                ui.separator();
            }

            // Column: HTML source controls (only for HTML decks)
            if deck.is_html {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(140.0);
                        ui.set_max_width(200.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.label(egui::RichText::new("🌐 HTML").strong());
                            let reload_resp = ui.button("⟳ Reload");
                            if reload_resp.clicked() {
                                actions.html_to_reload.push((ch_idx, deck_idx));
                            }
                            learn_overlay(
                                ui,
                                reload_resp.rect,
                                format!("deck/{}/html/reload", deck.uuid),
                                data,
                                actions,
                            );
                            let interactive_label = if deck.is_html_interactive {
                                "🖱 Exit Interactive"
                            } else {
                                "🖱 Interactive"
                            };
                            let interactive_resp = ui.button(interactive_label);
                            if interactive_resp.clicked() {
                                actions.html_set_interactive.push((
                                    ch_idx,
                                    deck_idx,
                                    !deck.is_html_interactive,
                                ));
                            }
                            learn_overlay(
                                ui,
                                interactive_resp.rect,
                                format!("deck/{}/html/interactive", deck.uuid),
                                data,
                                actions,
                            );
                            let mut transparent = deck.transparent;
                            let transparent_resp =
                                ui.checkbox(&mut transparent, "Transparent BG");
                            if transparent_resp.changed() {
                                actions.transparent_updates.push((
                                    ch_idx,
                                    deck_idx,
                                    transparent,
                                ));
                            }
                            learn_overlay(
                                ui,
                                transparent_resp.rect,
                                format!("deck/{}/transparent", deck.uuid),
                                data,
                                actions,
                            );
                        });
                    });
                ui.separator();
            }

            // Column: Video playback controls (only for video decks)
            if let Some(ref vp) = deck.video_playback {
                egui::Frame::default()
                    .inner_margin(6.0)
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.set_min_width(220.0);
                        ui.set_max_width(280.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.label(egui::RichText::new("▶ Playback").strong());

                            // Play/Pause button
                            let play_label = if vp.playing { "⏸ Pause" } else { "▶ Play" };
                            let play_resp = ui.button(play_label);
                            if play_resp.clicked() {
                                actions.video_actions.push((ch_idx, deck_idx, VideoAction::TogglePlay));
                            }
                            learn_overlay(ui, play_resp.rect, format!("deck/{}/video/play", deck.uuid), data, actions);

                            // Position scrub bar
                            let duration = vp.duration.max(0.001);
                            let mut pos = vp.position as f32;
                            ui.horizontal(|ui| {
                                ui.label(format_time(vp.position));
                                let slider = egui::Slider::new(&mut pos, 0.0..=duration as f32)
                                    .show_value(false)
                                    .trailing_fill(true);
                                let resp = ui.add(slider);
                                if resp.changed() {
                                    actions.video_actions.push((ch_idx, deck_idx, VideoAction::Seek(pos as f64)));
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/seek", deck.uuid), data, actions);
                                ui.label(format_time(duration));
                            });

                            // Speed control
                            let mut speed = vp.speed as f32;
                            ui.horizontal(|ui| {
                                ui.label("Speed:");
                                let resp = ui.add(egui::Slider::new(&mut speed, 0.1..=4.0).step_by(0.05).suffix("x"));
                                if resp.changed() {
                                    actions.video_actions.push((ch_idx, deck_idx, VideoAction::SetSpeed(speed as f64)));
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/speed", deck.uuid), data, actions);
                            });

                            // Loop mode
                            let loop_resp = ui.horizontal(|ui| {
                                ui.label("Loop:");
                                let modes = [
                                    ("🔁", crate::video::LoopMode::Loop, "Loop"),
                                    ("🔄", crate::video::LoopMode::PingPong, "Ping-Pong"),
                                    ("1️⃣", crate::video::LoopMode::OneShot, "One Shot"),
                                    ("⏹", crate::video::LoopMode::HoldLast, "Hold Last"),
                                ];
                                for (icon, mode, tooltip) in &modes {
                                    let selected = vp.loop_mode == *mode;
                                    let btn = egui::Button::new(*icon).selected(selected);
                                    if ui.add(btn).on_hover_text(*tooltip).clicked() && !selected {
                                        actions.video_actions.push((ch_idx, deck_idx, VideoAction::SetLoopMode(*mode)));
                                    }
                                }
                            });
                            learn_overlay(ui, loop_resp.response.rect, format!("deck/{}/video/loop_mode", deck.uuid), data, actions);

                            // In/Out points (bookshelf)
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("📐 In/Out Points").strong());
                            let effective_out = if vp.out_point > 0.0 { vp.out_point } else { duration };
                            let has_range = vp.in_point > 0.0 || vp.out_point > 0.0;

                            // In-point
                            let mut in_pt = vp.in_point as f32;
                            ui.horizontal(|ui| {
                                ui.label("In:");
                                let resp = ui.add(egui::Slider::new(&mut in_pt, 0.0..=duration as f32)
                                    .show_value(false).trailing_fill(true));
                                if resp.changed()
                                {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetInPoint(in_pt as f64)));
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/in_point", deck.uuid), data, actions);
                                ui.label(format_time(in_pt as f64));
                            });

                            // Out-point
                            let mut out_pt = effective_out as f32;
                            ui.horizontal(|ui| {
                                ui.label("Out:");
                                let resp = ui.add(egui::Slider::new(&mut out_pt, 0.0..=duration as f32)
                                    .show_value(false).trailing_fill(true));
                                if resp.changed()
                                {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetOutPoint(out_pt as f64)));
                                }
                                learn_overlay(ui, resp.rect, format!("deck/{}/video/out_point", deck.uuid), data, actions);
                                ui.label(format_time(out_pt as f64));
                            });

                            // Set from current / clear buttons
                            ui.horizontal(|ui| {
                                if ui.small_button("[ Set In").on_hover_text("Set in-point to current position").clicked() {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetInPoint(vp.position)));
                                }
                                if ui.small_button("Set Out ]").on_hover_text("Set out-point to current position").clicked() {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::SetOutPoint(vp.position)));
                                }
                                // Clear is always shown (disabled when no range) so it stays MIDI/keyboard-mappable.
                                let clear_resp = ui
                                    .add_enabled(has_range, egui::Button::new("x Clear").small())
                                    .on_hover_text("Reset to full clip");
                                if clear_resp.clicked() {
                                    actions.video_actions.push((ch_idx, deck_idx,
                                        VideoAction::ClearInOutPoints));
                                }
                                learn_overlay(ui, clear_resp.rect, format!("deck/{}/video/clear", deck.uuid), data, actions);
                            });

                            if has_range {
                                ui.label(egui::RichText::new(format!(
                                    "Range: {} → {} ({})",
                                    format_time(vp.in_point),
                                    format_time(effective_out),
                                    format_time(effective_out - vp.in_point),
                                )).small().weak());
                            }

                            // Info line
                            ui.label(egui::RichText::new(format!(
                                "{:.0} fps • {}", vp.frame_rate, format_time(duration)
                            )).small().weak());
                        });
                    });
                ui.separator();
            }

            // Column: Auto-Transition controls (collapsible column, default closed)
            {
                let at_open_id = egui::Id::new("at_col_open").with((ch_idx, deck_idx));
                let at_open = ui.ctx().memory(|mem| mem.data.get_temp::<bool>(at_open_id).unwrap_or(false));
                if at_open {
                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(200.0);
                            ui.set_max_width(260.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                // Clickable full-width header to collapse
                                let header_rect = ui.available_rect_before_wrap();
                                let header_rect = egui::Rect::from_min_size(header_rect.min, egui::vec2(ui.available_width(), 20.0));
                                let header_resp = ui.allocate_rect(header_rect, egui::Sense::click());
                                ui.painter().text(header_rect.left_center(), egui::Align2::LEFT_CENTER, "Auto Transition", egui::FontId::proportional(13.0), ui.visuals().strong_text_color());
                                if header_resp.clicked() {
                                    ui.ctx().memory_mut(|mem| mem.data.insert_temp(at_open_id, false));
                                }
                                if header_resp.hovered() {
                                    ui.painter().rect_filled(header_rect, 2.0, ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.3));
                                }
                                ui.separator();
                                // Enable toggle
                                ui.horizontal(|ui| {
                                    let enabled = deck.auto_transition.as_ref().is_some_and(|at| at.enabled);
                                    let mut en = enabled;
                                    ui.checkbox(&mut en, "Enabled");
                                    if en != enabled {
                                        actions.auto_transition_actions.push((ch_idx, deck_idx,
                                            AutoTransitionAction::SetEnabled(en)));
                                    }
                                });

                                if let Some(ref at) = deck.auto_transition {
                                    if at.enabled {
                                        ui.horizontal(|ui| {
                                            ui.label("Trigger:");
                                            let mut clip_end = at.trigger_is_clip_end;
                                            if ui.selectable_label(!clip_end, "Timer").clicked() && clip_end {
                                                clip_end = false;
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::SetTrigger(false)));
                                            }
                                            if ui.selectable_label(clip_end, "Clip End").clicked() && !clip_end {
                                                actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                    AutoTransitionAction::SetTrigger(true)));
                                            }
                                        });
                                        let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                                        ui.horizontal(|ui| {
                                            ui.label("Play:");
                                            let mut val = at.play_duration_value as f32;
                                            let max = if at.play_duration_is_beats { 128.0 } else { 300.0 };
                                            let play_path = format!("deck/{}/at/play_duration", deck.uuid);
                                            let slider_rect = if any_learn {
                                                let inner = ui.scope(|ui| {
                                                    ui.disable();
                                                    ui.add(egui::Slider::new(&mut val, 0.5..=max)
                                                        .logarithmic(true)
                                                        .suffix(if at.play_duration_is_beats { " beats" } else { " sec" }))
                                                });
                                                inner.inner.rect
                                            } else {
                                                let resp = ui.add(egui::Slider::new(&mut val, 0.5..=max)
                                                    .logarithmic(true)
                                                    .suffix(if at.play_duration_is_beats { " beats" } else { " sec" }));
                                                if resp.changed() {
                                                    actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                        AutoTransitionAction::SetPlayDuration(val as f64)));
                                                }
                                                resp.rect
                                            };
                                            if data.midi_learn_active {
                                                let is_target = data.midi_learn_target.as_deref() == Some(play_path.as_str());
                                                if is_target { widgets::draw_midi_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_midi_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("midi_learn_at_play", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.midi_learn_select = Some(play_path.clone());
                                                }
                                            }
                                            if data.keyboard_learn_active {
                                                let is_target = data.keyboard_learn_target.as_deref() == Some(play_path.as_str());
                                                if is_target { widgets::draw_keyboard_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_keyboard_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("kb_learn_at_play", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(play_path));
                                                }
                                            }
                                            if !any_learn
                                                && ui.small_button(if at.play_duration_is_beats { "♩" } else { "⏱" })
                                                    .on_hover_text("Toggle beats/seconds").clicked()
                                                {
                                                    actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                        AutoTransitionAction::TogglePlayDurationUnit));
                                                }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Trans:");
                                            let mut val = at.transition_duration_value as f32;
                                            let max = if at.transition_duration_is_beats { 32.0 } else { 30.0 };
                                            let trans_path = format!("deck/{}/at/trans_duration", deck.uuid);
                                            let slider_rect = if any_learn {
                                                let inner = ui.scope(|ui| {
                                                    ui.disable();
                                                    ui.add(egui::Slider::new(&mut val, 0.1..=max)
                                                        .logarithmic(true)
                                                        .suffix(if at.transition_duration_is_beats { " beats" } else { " sec" }))
                                                });
                                                inner.inner.rect
                                            } else {
                                                let resp = ui.add(egui::Slider::new(&mut val, 0.1..=max)
                                                    .logarithmic(true)
                                                    .suffix(if at.transition_duration_is_beats { " beats" } else { " sec" }));
                                                if resp.changed() {
                                                    actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                        AutoTransitionAction::SetTransitionDuration(val as f64)));
                                                }
                                                resp.rect
                                            };
                                            if data.midi_learn_active {
                                                let is_target = data.midi_learn_target.as_deref() == Some(trans_path.as_str());
                                                if is_target { widgets::draw_midi_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_midi_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("midi_learn_at_trans", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.midi_learn_select = Some(trans_path.clone());
                                                }
                                            }
                                            if data.keyboard_learn_active {
                                                let is_target = data.keyboard_learn_target.as_deref() == Some(trans_path.as_str());
                                                if is_target { widgets::draw_keyboard_learn_selected(ui, slider_rect); }
                                                else { widgets::draw_keyboard_learn_glow(ui, slider_rect); }
                                                let click_id = ui.id().with(("kb_learn_at_trans", ch_idx, deck_idx));
                                                if ui.interact(slider_rect, click_id, egui::Sense::click()).clicked() {
                                                    actions.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(trans_path));
                                                }
                                            }
                                            if !any_learn
                                                && ui.small_button(if at.transition_duration_is_beats { "♩" } else { "⏱" })
                                                    .on_hover_text("Toggle beats/seconds").clicked()
                                                {
                                                    actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                        AutoTransitionAction::ToggleTransitionDurationUnit));
                                                }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Shader:");
                                            let current = at.transition_shader_name.as_deref().unwrap_or("(fade)");
                                            egui::ComboBox::from_id_salt(format!("at_shader_{}_{}", ch_idx, deck_idx))
                                                .selected_text(current)
                                                .width(120.0)
                                                .show_ui(ui, |ui| {
                                                    if ui.selectable_label(at.transition_shader_name.is_none(), "(fade)").clicked() {
                                                        actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                            AutoTransitionAction::SetTransitionShader(None)));
                                                    }
                                                    for name in &data.transition_names {
                                                        let selected = at.transition_shader_name.as_deref() == Some(name.as_str());
                                                        if ui.selectable_label(selected, name).clicked() && !selected {
                                                            actions.auto_transition_actions.push((ch_idx, deck_idx,
                                                                AutoTransitionAction::SetTransitionShader(Some(name.clone()))));
                                                        }
                                                    }
                                                });
                                        });
                                    }
                                }
                            });
                        });
                } else {
                    // Collapsed: narrow vertical strip with vertical text
                    render_collapsed_column(ui, "Auto Transition", at_open_id);
                }
                ui.separator();
            }

            // Column: Generator parameters + blend/scale (collapsible column, default open)
            {
                let params_open_id = egui::Id::new("params_col_open").with((ch_idx, deck_idx));
                let params_open = ui.ctx().memory(|mem| mem.data.get_temp::<bool>(params_open_id).unwrap_or(true));
                if params_open {
                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(200.0);
                            ui.set_max_width(280.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                // Clickable full-width header to collapse
                                let header_rect = ui.available_rect_before_wrap();
                                let header_rect = egui::Rect::from_min_size(header_rect.min, egui::vec2(ui.available_width(), 20.0));
                                let header_resp = ui.allocate_rect(header_rect, egui::Sense::click());
                                let params_label = format!("Params: {}", deck.generator.shader_name);
                                ui.painter().text(header_rect.left_center(), egui::Align2::LEFT_CENTER, &params_label, egui::FontId::proportional(13.0), ui.visuals().strong_text_color());
                                if header_resp.clicked() {
                                    ui.ctx().memory_mut(|mem| mem.data.insert_temp(params_open_id, false));
                                }
                                if header_resp.hovered() {
                                    ui.painter().rect_filled(header_rect, 2.0, ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.3));
                                }
                                ui.separator();
                            let max_h = (ui.available_height() - 8.0).max(100.0);
                            egui::ScrollArea::vertical().id_salt("deck_gen_scroll").max_height(max_h).show(ui, |ui| {
                                // Blend mode
                                let all_modes = BlendMode::all();
                                let current_blend = all_modes.iter().position(|m| *m == deck.blend_mode).unwrap_or(0);
                                let mut selected = current_blend;
                                ui.horizontal(|ui| {
                                    ui.label("Blend:");
                                    egui::ComboBox::from_id_salt("sel_deck_blend")
                                        .selected_text(all_modes[selected].short_name())
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for (i, mode) in all_modes.iter().enumerate() {
                                                ui.selectable_value(&mut selected, i, mode.short_name());
                                            }
                                        });
                                });
                                if selected != current_blend {
                                    let new_blend = all_modes[selected];
                                    actions.deck_updates.push((ch_idx, deck_idx, deck.opacity, new_blend, deck.solo, deck.mute));
                                }

                                // Scaling mode
                                if let Some(current_scaling) = deck.scaling_mode {
                                    let scaling_modes = ["Fill", "Fit", "Stretch", "Center"];
                                    let current_idx = match current_scaling {
                                        ScalingMode::Fill => 0, ScalingMode::Fit => 1,
                                        ScalingMode::Stretch => 2, ScalingMode::Center => 3,
                                    };
                                    let mut selected_scaling = current_idx;
                                    ui.horizontal(|ui| {
                                        ui.label("Scale:");
                                        let combo = egui::ComboBox::from_id_salt("sel_deck_scale")
                                            .selected_text(scaling_modes[selected_scaling])
                                            .width(60.0)
                                            .show_ui(ui, |ui| {
                                                for (i, mode_name) in scaling_modes.iter().enumerate() {
                                                    ui.selectable_value(&mut selected_scaling, i, *mode_name);
                                                }
                                            });
                                        learn_overlay(ui, combo.response.rect, format!("deck/{}/scaling_mode", deck.uuid), data, actions);
                                    });
                                    if selected_scaling != current_idx {
                                        let new_scaling = match selected_scaling {
                                            1 => ScalingMode::Fit, 2 => ScalingMode::Stretch,
                                            3 => ScalingMode::Center, _ => ScalingMode::Fill,
                                        };
                                        actions.scaling_mode_updates.push((ch_idx, deck_idx, new_scaling));
                                    }
                                }

                                // Render FPS
                                ui.horizontal(|ui| {
                                    ui.label("Render:");
                                    let options = ["Auto", "60", "30", "15"];
                                    let current_idx = match deck.render_fps {
                                        DeckRenderFps::Auto => 0,
                                        DeckRenderFps::Fixed(60) => 1,
                                        DeckRenderFps::Fixed(30) => 2,
                                        DeckRenderFps::Fixed(15) => 3,
                                        DeckRenderFps::Fixed(_) => 0, // fallback
                                    };
                                    let mut selected = current_idx;
                                    egui::ComboBox::from_id_salt("sel_deck_render_fps")
                                        .selected_text(options[selected])
                                        .width(50.0)
                                        .show_ui(ui, |ui| {
                                            for (i, opt) in options.iter().enumerate() {
                                                ui.selectable_value(&mut selected, i, *opt);
                                            }
                                        });
                                    if selected != current_idx {
                                        let new_fps = match selected {
                                            1 => DeckRenderFps::Fixed(60),
                                            2 => DeckRenderFps::Fixed(30),
                                            3 => DeckRenderFps::Fixed(15),
                                            _ => DeckRenderFps::Auto,
                                        };
                                        actions.render_fps_updates.push((ch_idx, deck_idx, new_fps));
                                    }
                                    // Show render cost
                                    if deck.gpu_render_cost_us > 0.0 {
                                        let ms = deck.gpu_render_cost_us / 1000.0;
                                        ui.label(egui::RichText::new(format!("⚡{:.1}ms GPU", ms)).small().weak());
                                    } else if deck.render_cost_us > 0.0 {
                                        let ms = deck.render_cost_us / 1000.0;
                                        ui.label(egui::RichText::new(format!("⚡{:.1}ms", ms)).small().weak());
                                    }
                                });

                                // Generator parameters
                                let gen_params = &deck.generator;
                                if !gen_params.params.is_empty() {
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(&gen_params.shader_name).strong());
                                    let deck_uuid = deck.uuid.clone();
                                    let midi_path_prefix = format!("deck/{}", deck_uuid);
                                    let deck_uuid_assign = deck_uuid.clone();
                                    let deck_uuid_remove = deck_uuid.clone();
                                    widgets::render_params(
                                        ui,
                                        &gen_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| match val {
                                            ParamValue::Float(v) => ParamUpdate::GeneratorFloat { ch_idx, deck_idx, name: name.to_string(), value: v },
                                            ParamValue::Bool(v) => ParamUpdate::GeneratorBool { ch_idx, deck_idx, name: name.to_string(), value: v },
                                            ParamValue::Color(v) => ParamUpdate::GeneratorColor { ch_idx, deck_idx, name: name.to_string(), value: v },
                                            _ => unreachable!(),
                                        },
                                        Some(&|name: &str, source_uuid: &str| ModulationAction::AssignModulation {
                                            deck_uuid: deck_uuid_assign.clone(), param_name: name.to_string(), source_id: source_uuid.to_string(), amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveAssignment {
                                            deck_uuid: deck_uuid_remove.clone(), param_name: name.to_string(), source_id: String::new(),
                                        }),
                                        &mut actions.param_updates,
                                        &mut actions.modulation_actions,
                                        &format!("sel_{}_{}", ch_idx, deck_idx),
                                        Some(&midi_path_prefix),
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("deck_{}", deck_uuid),
                                        data.keyboard_learn_active,
                                        &mut actions.keyboard_learn_select,
                                        data.keyboard_learn_target.as_deref(),
                                    );
                                    ui.add_space(4.0);
                                    if ui.button("Reset").clicked() {
                                        actions.param_updates.push(ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx });
                                    }
                                }
                            });
                            });
                        });
                } else {
                    // Collapsed: narrow vertical strip with vertical text
                    render_collapsed_column(ui, &format!("Params: {}", deck.generator.shader_name), params_open_id);
                }
            }

            ui.separator();

            // Effect chain: drag-and-drop reordering + library drops
            {
                for (eff_idx, (eff_uuid, eff_name, eff_enabled, eff_params)) in deck.effects.iter().enumerate() {
                    // Drop zone before this effect (for reordering)
                    render_effect_drop_zone(ui, &format!("deck_{}_{}", ch_idx, deck_idx), eff_idx);

                    // Effect card with drag handle in header only
                    let card_resp = egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            let max_h = (ui.available_height() - 8.0).max(100.0);
                            egui::ScrollArea::vertical().id_salt(format!("deck_fx_scroll_{}_{}_{}",ch_idx,deck_idx,eff_idx)).max_height(max_h).scroll_source(egui::scroll_area::ScrollSource { drag: false, scroll_bar: true, mouse_wheel: true }).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    render_effect_drag_handle(ui, EffectDrag::Deck(ch_idx, deck_idx, eff_idx));
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.effect_to_toggle = Some((ch_idx, deck_idx, eff_idx));
                                    }
                                    ui.label(egui::RichText::new(eff_name).strong());
                                });

                                if !eff_params.params.is_empty() {
                                    let ch_copy = ch_idx;
                                    let deck_copy = deck_idx;
                                    let eff_idx_copy = eff_idx;
                                    let deck_uuid_eff = deck.uuid.clone();
                                    let eff_uuid_assign = eff_uuid.clone();
                                    let eff_uuid_remove = eff_uuid.clone();
                                    let eff_midi_prefix = format!("deck/{}/effect/{}", deck_uuid_eff, eff_idx_copy);
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| match val {
                                            ParamValue::Float(v) => ParamUpdate::EffectFloat { ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Bool(v) => ParamUpdate::EffectBool { ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Color(v) => ParamUpdate::EffectColor { ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            _ => unreachable!(),
                                        },
                                        Some(&|name: &str, source_uuid: &str| ModulationAction::AssignEffectModulation {
                                            effect_uuid: eff_uuid_assign.clone(),
                                            param_name: name.to_string(), source_id: source_uuid.to_string(), amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveEffectAssignment {
                                            effect_uuid: eff_uuid_remove.clone(),
                                            param_name: name.to_string(),
                                        }),
                                        &mut actions.param_updates,
                                        &mut actions.modulation_actions,
                                        &format!("fx_{}_{}_{}", ch_copy, deck_copy, eff_idx_copy),
                                        Some(&eff_midi_prefix),
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("fx_{}", eff_uuid),
                                        data.keyboard_learn_active,
                                        &mut actions.keyboard_learn_select,
                                        data.keyboard_learn_target.as_deref(),
                                    );
                                }
                            });
                            });
                        });
                    // X button overlay at top-right of card
                    {
                        let card_rect = card_resp.response.rect;
                        let btn_size = egui::vec2(16.0, 16.0);
                        let btn_pos = egui::pos2(card_rect.right() - btn_size.x - 4.0, card_rect.top() + 4.0);
                        let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                        let btn_resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                        let color = if btn_resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                        ui.painter().text(btn_rect.center(), egui::Align2::CENTER_CENTER, "x", egui::FontId::proportional(12.0), color);
                        if btn_resp.clicked() {
                            actions.effect_to_remove = Some((ch_idx, deck_idx, eff_idx));
                        }
                    }
                    render_effect_drag_ghost(
                        ui,
                        egui::Id::new(("eff_ghost", ch_idx, deck_idx, eff_idx)),
                        EffectDrag::Deck(ch_idx, deck_idx, eff_idx),
                        eff_name,
                    );
                    ui.separator();
                }

                // Drop zone after last effect (for reordering to end)
                if !deck.effects.is_empty() {
                    let num_effects = deck.effects.len();
                    render_effect_drop_zone(ui, &format!("deck_{}_{}", ch_idx, deck_idx), num_effects);
                }

                // Remaining space: always present drop target that fills remaining width
                let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                    .map(|p| matches!(&*p, LibraryDrag::Effect(_))).unwrap_or(false);
                let remaining_w = ui.available_width().max(80.0);
                let remaining_h = ui.available_height().max(40.0);
                let stroke = if has_fx_drag { egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
                let fill = if has_fx_drag { egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20) } else { egui::Color32::TRANSPARENT };
                egui::Frame::default()
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(remaining_w - 16.0, remaining_h - 16.0));
                        ui.centered_and_justified(|ui| {
                            ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                        });
                    });
            }

            // Store the entire horizontal_top area as the drop rect for deferred library effect drops
            let chain_rect = ui.min_rect();
            let deck_chain_key = format!("deck_{}_{}", ch_idx, deck_idx);
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("deck_fx_drop_rect").with((ch_idx, deck_idx)), chain_rect);
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with(deck_chain_key), deck.effects.len() + 1);
            });
        });
    });
}

/// Bottom bar: full sequence editor when a sequence is selected.
fn render_sequence_detail(
    ui: &mut egui::Ui,
    seq_idx: usize,
    data: &UIData,
    actions: &mut UIActions,
) {
    use super::super::{SequenceAction, SequenceStepDrag, SequenceStepKindUI};

    let Some(seq) = data.sequences.get(seq_idx) else {
        ui.label(egui::RichText::new("Sequence not found").weak());
        return;
    };

    // Header: name, enable, play/stop, delete
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            egui::RichText::new(format!("🎬 {}", seq.name))
                .strong()
                .size(14.0),
        );

        let (en_label, en_color) = if seq.enabled {
            ("On", egui::Color32::from_rgb(80, 200, 80))
        } else {
            ("Off", egui::Color32::from_rgb(120, 120, 120))
        };
        if ui
            .button(egui::RichText::new(en_label).color(en_color))
            .on_hover_text("Toggle enabled")
            .clicked()
        {
            actions
                .sequence_actions
                .push(SequenceAction::ToggleEnabled(seq_idx));
        }

        if seq.playing {
            if ui.button("⏹ Stop").on_hover_text("Stop playback").clicked() {
                actions.sequence_actions.push(SequenceAction::Stop(seq_idx));
            }
        } else if seq.enabled
            && !seq.steps.is_empty()
            && ui
                .button("▶ Play")
                .on_hover_text("Start playback")
                .clicked()
        {
            actions.sequence_actions.push(SequenceAction::Play(seq_idx));
        }

        if ui
            .button("🗑 Delete")
            .on_hover_text("Delete sequence")
            .clicked()
        {
            actions
                .sequence_actions
                .push(SequenceAction::Delete(seq_idx));
        }
    });

    ui.add_space(4.0);

    // Interactive timeline strip (larger, clickable)
    let selected_step_idx = data
        .selected_sequence_step
        .filter(|(si, _)| *si == seq_idx)
        .map(|(_, step)| step);

    if seq.steps.is_empty() {
        ui.label(egui::RichText::new("No steps yet — add steps below").weak());
    } else {
        let (clicked_step, _) = render_timeline_strip(
            ui,
            seq,
            &data.channel_names,
            true,
            selected_step_idx,
            data.clock_bpm,
        );
        if let Some(clicked) = clicked_step {
            actions.select_sequence_step = Some((seq_idx, clicked));
        }
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);

    // Two-column layout: step list (left) | step editor (right)
    let target_id = egui::Id::new("__seq_step_dnd_target");
    ui.horizontal_top(|ui| {
        // ── Left column: stacked step list + add buttons ──
        let list_width = 280.0;
        ui.vertical(|ui| {
            ui.set_width(list_width);

            // Scrollable step list with visual-gap drag-and-drop
            egui::ScrollArea::vertical()
                .id_salt("seq_step_list")
                .max_height(ui.available_height() - 30.0)
                .show(ui, |ui| {
                    let src_id = egui::Id::new("__seq_step_dnd_src");
                    let is_dragging =
                        egui::DragAndDrop::has_payload_of_type::<SequenceStepDrag>(ui.ctx());
                    let drag_src: Option<SequenceStepDrag> = if is_dragging {
                        ui.ctx().memory(|mem| mem.data.get_temp(src_id))
                    } else {
                        None
                    };
                    let dragged_idx = drag_src.map(|d| d.step_idx);

                    // Compute drop target from pointer position BEFORE rendering,
                    // using fixed row heights to avoid oscillation from gap insertion.
                    let row_height = 22.0;
                    let gap_height = row_height;
                    let step_count = seq.steps.len();
                    let list_top = ui.cursor().top();

                    let drop_target: Option<usize> = match (is_dragging, dragged_idx) {
                        (true, Some(src)) => {
                            if let Some(pos) = ui.ctx().input(|inp| inp.pointer.hover_pos()) {
                                // Compute pointer offset from list top, in terms of
                                // the *logical* list (source item removed).
                                let rel_y = pos.y - list_top;
                                if rel_y >= 0.0 {
                                    // Visible items = all except the dragged one
                                    let visible_count = step_count - 1;
                                    // Which slot the pointer is over (0-based)
                                    let slot = ((rel_y / row_height) as usize).min(visible_count);
                                    // Map slot back to original index, re-inserting the gap for the source
                                    let target = if slot < src { slot } else { slot + 1 };
                                    Some(target.min(step_count))
                                } else {
                                    Some(0)
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    // Store the computed target in memory for the deferred handler
                    if let Some(t) = drop_target {
                        ui.ctx().memory_mut(|mem| {
                            mem.data.insert_temp::<usize>(target_id, t);
                        });
                    }

                    for (i, step) in seq.steps.iter().enumerate() {
                        // Hide the step being dragged from its original position
                        if dragged_idx == Some(i) {
                            continue;
                        }

                        // Insert gap BEFORE this item if it's the drop target
                        if drop_target == Some(i) {
                            let (gap_rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), gap_height),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                gap_rect,
                                2.0,
                                egui::Color32::from_rgba_premultiplied(255, 200, 80, 30),
                            );
                            ui.painter().rect_stroke(
                                gap_rect,
                                2.0,
                                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(255, 200, 80)),
                                egui::StrokeKind::Outside,
                            );
                        }

                        let is_selected = selected_step_idx == Some(i);
                        let is_current = seq.playing && i == seq.current_step;

                        let (icon, summary) = match &step.kind {
                            SequenceStepKindUI::Fade {
                                from_ch,
                                to_ch,
                                duration_val,
                                duration_unit,
                                ..
                            } => {
                                let from_name = data
                                    .channel_names
                                    .get(*from_ch)
                                    .map(|s| s.as_str())
                                    .unwrap_or("?");
                                let to_name = data
                                    .channel_names
                                    .get(*to_ch)
                                    .map(|s| s.as_str())
                                    .unwrap_or("?");
                                (
                                    "🔀",
                                    format!(
                                        "{} → {}  {:.1}{}",
                                        from_name,
                                        to_name,
                                        duration_val,
                                        duration_unit.label()
                                    ),
                                )
                            }
                            SequenceStepKindUI::Wait {
                                duration_val,
                                duration_unit,
                            } => ("⏸", format!("{:.1}{}", duration_val, duration_unit.label())),
                            SequenceStepKindUI::GoTo { step_index } => {
                                ("↺", format!("→ Step {}", step_index + 1))
                            }
                        };

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;

                            // Drag handle (grip dots)
                            let handle_size = egui::vec2(12.0, 16.0);
                            let (handle_rect, handle_resp) =
                                ui.allocate_exact_size(handle_size, egui::Sense::drag());
                            let grip_color = if handle_resp.dragged() || handle_resp.hovered() {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            let cx = handle_rect.center().x;
                            let cy = handle_rect.center().y;
                            for row in -1..=1 {
                                for col in [-1.0_f32, 1.0] {
                                    ui.painter().circle_filled(
                                        egui::pos2(cx + col * 3.0, cy + row as f32 * 4.0),
                                        1.5,
                                        grip_color,
                                    );
                                }
                            }
                            if handle_resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            if handle_resp.dragged() {
                                let drag = SequenceStepDrag {
                                    seq_idx,
                                    step_idx: i,
                                };
                                egui::DragAndDrop::set_payload(ui.ctx(), drag);
                                ui.ctx().memory_mut(|mem| {
                                    mem.data
                                        .insert_temp(egui::Id::new("__seq_step_dnd_src"), drag);
                                });
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            }

                            // Clickable label
                            let label_text =
                                format!("{} {}. {} {}", icon, i + 1, step.label, summary);
                            let text = if is_current {
                                egui::RichText::new(&label_text)
                                    .color(egui::Color32::from_rgb(80, 200, 80))
                            } else if is_selected {
                                egui::RichText::new(&label_text).strong()
                            } else {
                                egui::RichText::new(&label_text)
                            };

                            if ui.selectable_label(is_selected, text).clicked() {
                                actions.select_sequence_step = Some((seq_idx, i));
                            }
                        });
                    }

                    // Gap at the end of the list (drop after last item)
                    if drop_target == Some(step_count) {
                        let (gap_rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), gap_height),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(
                            gap_rect,
                            2.0,
                            egui::Color32::from_rgba_premultiplied(255, 200, 80, 30),
                        );
                        ui.painter().rect_stroke(
                            gap_rect,
                            2.0,
                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(255, 200, 80)),
                            egui::StrokeKind::Outside,
                        );
                    }
                });

            // Add step buttons at bottom of list
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let from = 0;
                let to = 1.min(data.channel_count.saturating_sub(1));
                if ui.small_button("+Fade").clicked() {
                    actions.sequence_actions.push(SequenceAction::AddFade {
                        seq_idx,
                        from_ch: from,
                        to_ch: to,
                    });
                }
                if ui.small_button("+Wait").clicked() {
                    actions
                        .sequence_actions
                        .push(SequenceAction::AddWait(seq_idx));
                }
                if ui.small_button("+Loop").clicked() {
                    actions.sequence_actions.push(SequenceAction::AddGoTo {
                        seq_idx,
                        step_index: 0,
                    });
                }
            });
        });

        ui.separator();

        // ── Right column: selected step editor ──
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            if let Some(step_idx) = selected_step_idx {
                if let Some(step) = seq.steps.get(step_idx) {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("Step {} — {}", step_idx + 1, step.label))
                                .strong(),
                        );
                        if ui
                            .small_button("🗑 Remove")
                            .on_hover_text("Remove this step")
                            .clicked()
                        {
                            actions
                                .sequence_actions
                                .push(SequenceAction::RemoveStep { seq_idx, step_idx });
                        }
                    });
                    ui.add_space(4.0);
                    render_sequence_step_editor(ui, seq_idx, step_idx, step, data, actions);
                } else {
                    ui.label(egui::RichText::new("Step not found").weak());
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("← Select a step to edit").weak());
                });
            }
        });
    });

    // Animate playhead
    if seq.playing {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_bottom_panel_smoke_deck_selected() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_channel_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = Some(0);
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_master_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_master = true;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_nothing_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = None;
        data.selected_master = false;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_sequence_selected() {
        use super::super::super::{SequenceStepKindUI, SequenceStepUI, SequenceUIData};
        use crate::channel::DurationUnit;
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.sequences.push(SequenceUIData {
            name: "Test Seq".to_string(),
            enabled: true,
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
            steps: vec![SequenceStepUI {
                label: "Fade".into(),
                kind: SequenceStepKindUI::Fade {
                    from_ch: 0,
                    to_ch: 1,
                    duration_val: 5.0,
                    duration_unit: DurationUnit::Seconds,
                    easing: "Linear".into(),
                    transition_shader: None,
                    target_amount: 1.0,
                },
            }],
        });
        data.selected_sequence = Some(0);
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_bottom_panel_smoke_sequence_with_step_selected() {
        use super::super::super::{SequenceStepKindUI, SequenceStepUI, SequenceUIData};
        use crate::channel::DurationUnit;
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.sequences.push(SequenceUIData {
            name: "Test Seq".to_string(),
            enabled: true,
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
            steps: vec![
                SequenceStepUI {
                    label: "Fade".into(),
                    kind: SequenceStepKindUI::Fade {
                        from_ch: 0,
                        to_ch: 1,
                        duration_val: 5.0,
                        duration_unit: DurationUnit::Seconds,
                        easing: "Linear".into(),
                        transition_shader: None,
                        target_amount: 1.0,
                    },
                },
                SequenceStepUI {
                    label: "Wait".into(),
                    kind: SequenceStepKindUI::Wait {
                        duration_val: 2.0,
                        duration_unit: DurationUnit::Seconds,
                    },
                },
            ],
        });
        data.selected_sequence = Some(0);
        data.selected_sequence_step = Some((0, 1));
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_bottom_panel(ui, &data, &mut actions);
        });
    }
}
