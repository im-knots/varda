use crate::mixer::CrossfadeEasing;
use crate::params::ParamValue;
use crate::modulation::{LFOWaveform, StepInterpolation};
use crate::{BlendMode, ScalingMode};
use crate::renderer::context::{OutputSource, OutputTarget};
use crate::surface::{CircleHint, ContentMapping, SurfaceOutputType};
use super::{UIData, UIActions, ParamUpdate, ModulationAction, CrossfaderAction, OutputAction, SurfaceAction, ModSourceUI, NotificationUI, modulator_color, LibraryDrag, EffectDrag};
use super::widgets;

// ── Polygon triangulation (ear-clipping) ────────────────────────────

/// Build an `egui::Shape` for an arbitrary (possibly concave) polygon using
/// ear-clipping triangulation. Falls back to `convex_polygon` for ≤4 vertices
/// where convexity is likely.
fn polygon_shape(
    verts: &[egui::Pos2],
    fill: egui::Color32,
    stroke: egui::Stroke,
) -> egui::Shape {
    if verts.len() < 3 {
        return egui::Shape::Noop;
    }

    // Triangulate
    let indices = triangulate_polygon(verts);
    if indices.is_empty() {
        // Fallback if triangulation fails
        return egui::Shape::convex_polygon(verts.to_vec(), fill, stroke);
    }

    // Build mesh for the filled area
    let mut mesh = egui::Mesh::default();
    mesh.texture_id = egui::TextureId::default();
    for &p in verts {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: p,
            uv: egui::pos2(0.0, 0.0),
            color: fill,
        });
    }
    mesh.indices = indices;

    let mut shapes = vec![egui::Shape::mesh(mesh)];

    // Draw outline on top
    if stroke.width > 0.0 {
        let mut outline = verts.to_vec();
        outline.push(verts[0]); // close the loop
        shapes.push(egui::Shape::line(outline, stroke));
    }

    egui::Shape::Vec(shapes)
}

/// Ear-clipping triangulation for a simple polygon.
/// Returns triangle indices into the vertex array.
fn triangulate_polygon(verts: &[egui::Pos2]) -> Vec<u32> {
    let n = verts.len();
    if n < 3 { return Vec::new(); }

    // Work with a mutable index list
    let mut idx: Vec<usize> = (0..n).collect();
    let mut result = Vec::with_capacity((n - 2) * 3);

    // Determine winding: positive = CCW
    let signed_area: f32 = idx.windows(2)
        .map(|w| {
            let a = verts[w[0]];
            let b = verts[w[1]];
            (b.x - a.x) * (b.y + a.y)
        })
        .sum::<f32>()
        + {
            let a = verts[*idx.last().unwrap()];
            let b = verts[idx[0]];
            (b.x - a.x) * (b.y + a.y)
        };
    let ccw = signed_area < 0.0; // screen coords: y-down, so negative area = CCW

    let mut remaining = idx.len();
    let mut fail_count = 0;
    let mut i = 0;

    while remaining > 2 && fail_count < remaining {
        let prev = idx[(i + remaining - 1) % remaining];
        let curr = idx[i % remaining];
        let next = idx[(i + 1) % remaining];

        if is_ear(verts, &idx, prev, curr, next, ccw) {
            result.push(prev as u32);
            result.push(curr as u32);
            result.push(next as u32);
            idx.remove(i % remaining);
            remaining -= 1;
            fail_count = 0;
            if i >= remaining && remaining > 0 {
                i = 0;
            }
        } else {
            i = (i + 1) % remaining;
            fail_count += 1;
        }
    }

    result
}

fn cross_2d(o: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn is_ear(verts: &[egui::Pos2], idx: &[usize], prev: usize, curr: usize, next: usize, ccw: bool) -> bool {
    let cross = cross_2d(verts[prev], verts[curr], verts[next]);
    // For CCW winding, an ear has positive cross product
    if ccw { if cross <= 0.0 { return false; } } else { if cross >= 0.0 { return false; } }

    // Check no other vertex is inside this triangle
    for &vi in idx {
        if vi == prev || vi == curr || vi == next { continue; }
        if point_in_triangle(verts[vi], verts[prev], verts[curr], verts[next]) {
            return false;
        }
    }
    true
}

fn point_in_triangle(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> bool {
    let d0 = cross_2d(a, b, p);
    let d1 = cross_2d(b, c, p);
    let d2 = cross_2d(c, a, p);
    let has_neg = (d0 < 0.0) || (d1 < 0.0) || (d2 < 0.0);
    let has_pos = (d0 > 0.0) || (d1 > 0.0) || (d2 > 0.0);
    !(has_neg && has_pos)
}

/// Render the complete UI and return all collected actions/intents.
pub fn render_ui(ctx: &egui::Context, data: &UIData) -> UIActions {
    let mut actions = UIActions::new();

    // === LEFT PANEL: Library (collapsible) ===
    if data.library_panel_open {
        egui::SidePanel::left("library_panel")
            .min_width(180.0)
            .default_width(220.0)
            .resizable(true)
            .show(ctx, |ui| {
                render_library_panel(ui, data, &mut actions);
            });
    }

    // === RIGHT PANEL: Main Output + Master Effects ===
    egui::SidePanel::right("master_panel")
        .min_width(280.0)
        .default_width(320.0)
        .show(ctx, |ui| {
            render_right_panel(ui, data, &mut actions);
        });

    // === BOTTOM PANEL: Audio, Modulation, Shader Browser ===
    egui::TopBottomPanel::bottom("bottom_panel")
        .min_height(80.0)
        .max_height(400.0)
        .default_height(180.0)
        .resizable(true)
        .show_separator_line(true)
        .show(ctx, |ui| {
            render_bottom_panel(ui, data, &mut actions);
        });

    // === TOP BAR: Save button + status ===
    egui::TopBottomPanel::top("top_bar")
        .exact_height(28.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if ui.button("💾 Save").on_hover_text("Save workspace (⌘S)").clicked() {
                    actions.save_requested = true;
                }
            });
        });

    // === CENTRAL AREA: Decks as columns ===
    egui::CentralPanel::default().show(ctx, |ui| {
        render_central_panel(ui, data, &mut actions);
    });

    // === LIBRARY DnD: deferred drop handler ===
    // We can't rely on egui's dnd_release_payload/contains_pointer because the drag ghost
    // (tooltip layer) covers drop targets. Instead, each frame we store which target the
    // pointer is over. When the payload disappears (mouse released), we use the last target.
    {
        let had_payload_id = egui::Id::new("__lib_dnd_had_payload");
        let hover_ch_id = egui::Id::new("__lib_dnd_hover_ch");
        // Effect chain targets: "deck", "channel", or "master"
        let hover_fx_target_id = egui::Id::new("__lib_dnd_hover_fx_target");
        let has_payload = egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx);

        if has_payload {
            if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                // Check channel column rects (for generator drops)
                let mut found_ch: Option<usize> = None;
                for ch_idx in 0..data.channels.len() {
                    let key = egui::Id::new("ch_drop_rect").with(ch_idx);
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                        if rect.contains(pos) {
                            found_ch = Some(ch_idx);
                            break;
                        }
                    }
                }

                // Check effect chain rects (for effect drops)
                // Format: (target_type, ch_idx, deck_idx) — "deck"/(ch,dk), "channel"/(ch,0), "master"/(0,0)
                // Only check the currently active bottom bar view to avoid stale rects
                // from previously-rendered views (they share the same screen area).
                let mut found_fx: Option<(String, usize, usize)> = None;

                if data.selected_master {
                    // Master effect chain
                    let master_key = egui::Id::new("master_fx_drop_rect");
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(master_key)) {
                        if rect.contains(pos) {
                            found_fx = Some(("master".to_string(), 0, 0));
                        }
                    }
                } else if let Some(ch_idx) = data.selected_channel {
                    // Channel effect chain (only the selected channel)
                    let key = egui::Id::new("ch_fx_drop_rect").with(ch_idx);
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                        if rect.contains(pos) {
                            found_fx = Some(("channel".to_string(), ch_idx, 0));
                        }
                    }
                } else if let Some((sel_ch, sel_dk)) = data.selected_deck {
                    // Deck effect chain
                    let key = egui::Id::new("deck_fx_drop_rect").with((sel_ch, sel_dk));
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                        if rect.contains(pos) {
                            found_fx = Some(("deck".to_string(), sel_ch, sel_dk));
                        }
                    }
                }

                ctx.memory_mut(|mem| {
                    mem.data.insert_temp(hover_ch_id, found_ch);
                    mem.data.insert_temp(hover_fx_target_id, found_fx);
                    mem.data.insert_temp::<bool>(had_payload_id, true);
                });
            }
        } else {
            // No payload this frame — check if we HAD one last frame (= drop just happened)
            let had_payload: bool = ctx.memory(|mem| mem.data.get_temp(had_payload_id).unwrap_or(false));
            if had_payload {
                let hover_ch: Option<usize> = ctx.memory(|mem| mem.data.get_temp(hover_ch_id).unwrap_or(None));
                let hover_fx: Option<(String, usize, usize)> = ctx.memory(|mem| mem.data.get_temp(hover_fx_target_id).unwrap_or(None));

                // Generator or Camera drop on channel
                if let Some(ch_idx) = hover_ch {
                    let gen_key = egui::Id::new("__lib_dnd_gen_idx");
                    let gen_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(gen_key));
                    if let Some(gen_idx) = gen_idx {
                        log::info!("Library drop (deferred): generator {} -> ch{}", gen_idx, ch_idx);
                        actions.shader_to_add = Some((ch_idx, gen_idx));
                    }

                    let cam_key = egui::Id::new("__lib_dnd_cam_id");
                    let cam_id: Option<crate::camera::CameraId> = ctx.memory(|mem| mem.data.get_temp(cam_key));
                    if let Some(cam_id) = cam_id {
                        log::info!("Library drop (deferred): camera {} -> ch{}", cam_id, ch_idx);
                        actions.camera_to_add = Some((ch_idx, cam_id));
                    }
                }

                // Effect drop on chain
                if let Some((target_type, ch_idx, deck_idx)) = hover_fx {
                    let fx_key = egui::Id::new("__lib_dnd_fx_idx");
                    let filter_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(fx_key));
                    if let Some(filter_idx) = filter_idx {
                        match target_type.as_str() {
                            "deck" => {
                                log::info!("Library drop (deferred): effect {} -> ch{} deck{}", filter_idx, ch_idx, deck_idx);
                                actions.effect_to_add = Some((ch_idx, deck_idx, filter_idx));
                            }
                            "channel" => {
                                log::info!("Library drop (deferred): effect {} -> ch{} channel fx", filter_idx, ch_idx);
                                actions.ch_effect_to_add = Some((ch_idx, filter_idx));
                            }
                            "master" => {
                                log::info!("Library drop (deferred): effect {} -> master fx", filter_idx);
                                actions.master_effect_to_add = Some(filter_idx);
                            }
                            _ => {}
                        }
                    }
                }

                // Clear state
                ctx.memory_mut(|mem| {
                    mem.data.remove::<bool>(had_payload_id);
                    mem.data.remove::<Option<usize>>(hover_ch_id);
                    mem.data.remove::<Option<(String, usize, usize)>>(hover_fx_target_id);
                    mem.data.remove::<usize>(egui::Id::new("__lib_dnd_gen_idx"));
                    mem.data.remove::<usize>(egui::Id::new("__lib_dnd_fx_idx"));
                    mem.data.remove::<crate::camera::CameraId>(egui::Id::new("__lib_dnd_cam_id"));
                });
            }
        }
    }

    // === EFFECT REORDER DnD: deferred drop handler ===
    // Same pattern as library drops — dnd_drag_source renders on tooltip layer blocking drop zones.
    // Each frame while EffectDrag is active, find which drop zone the pointer is over.
    // When the payload disappears, apply the move.
    {
        let had_eff_id = egui::Id::new("__eff_dnd_had_payload");
        let hover_dz_id = egui::Id::new("__eff_dnd_hover_dz"); // (chain_key, position)
        let has_eff_payload = egui::DragAndDrop::has_payload_of_type::<EffectDrag>(ctx);

        if has_eff_payload {
            if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                // Search all known drop zone rects to find which one the pointer is over
                let mut found_dz: Option<(String, usize)> = None;

                // Helper: check drop zones for a given chain key
                let check_chain = |chain_key: &str, ctx: &egui::Context, pos: egui::Pos2| -> Option<(String, usize)> {
                    let count_key = egui::Id::new("eff_dz_count").with(chain_key.to_string());
                    let count: usize = ctx.memory(|mem| mem.data.get_temp(count_key).unwrap_or(0));
                    for p in 0..count {
                        let rk = egui::Id::new("eff_dz_rect").with((chain_key.to_string(), p));
                        if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(rk)) {
                            if rect.contains(pos) {
                                return Some((chain_key.to_string(), p));
                            }
                        }
                    }
                    None
                };

                // Check deck chains
                if found_dz.is_none() {
                    if let Some((sel_ch, sel_dk)) = data.selected_deck {
                        found_dz = check_chain(&format!("deck_{}_{}", sel_ch, sel_dk), ctx, pos);
                    }
                }
                // Check master chain
                if found_dz.is_none() {
                    found_dz = check_chain("master", ctx, pos);
                }
                // Check channel chains
                if found_dz.is_none() {
                    for ch_idx in 0..data.channels.len() {
                        found_dz = check_chain(&format!("ch_{}", ch_idx), ctx, pos);
                        if found_dz.is_some() { break; }
                    }
                }

                ctx.memory_mut(|mem| {
                    mem.data.insert_temp(hover_dz_id, found_dz);
                    mem.data.insert_temp::<bool>(had_eff_id, true);
                });
            }
        } else {
            let had: bool = ctx.memory(|mem| mem.data.get_temp(had_eff_id).unwrap_or(false));
            if had {
                let hover_dz: Option<(String, usize)> = ctx.memory(|mem| mem.data.get_temp(hover_dz_id).unwrap_or(None));
                // Retrieve the source payload — stored by dnd_drag_source before cleanup
                // We need to read it from the last frame. Since egui clears it, we stored the
                // EffectDrag in temp memory during the drag.
                let src_key = egui::Id::new("__eff_dnd_src");
                let src: Option<EffectDrag> = ctx.memory(|mem| mem.data.get_temp(src_key));

                if let (Some((chain_key, target_pos)), Some(src_drag)) = (hover_dz, src) {
                    match src_drag {
                        EffectDrag::Deck(src_ch, src_dk, src_eff) => {
                            let expected_key = format!("deck_{}_{}", src_ch, src_dk);
                            if chain_key == expected_key {
                                let to = if src_eff < target_pos { target_pos - 1 } else { target_pos };
                                if to != src_eff {
                                    log::info!("Effect reorder (deferred): deck {}/{} effect {} -> {}", src_ch, src_dk, src_eff, to);
                                    actions.effect_to_move = Some((src_ch, src_dk, src_eff, to));
                                }
                            }
                        }
                        EffectDrag::Channel(src_ch, src_eff) => {
                            let expected_key = format!("ch_{}", src_ch);
                            if chain_key == expected_key {
                                let to = if src_eff < target_pos { target_pos - 1 } else { target_pos };
                                if to != src_eff {
                                    log::info!("Effect reorder (deferred): ch{} effect {} -> {}", src_ch, src_eff, to);
                                    actions.ch_effect_to_move = Some((src_ch, src_eff, to));
                                }
                            }
                        }
                        EffectDrag::Master(src_eff) => {
                            if chain_key == "master" {
                                let to = if src_eff < target_pos { target_pos - 1 } else { target_pos };
                                if to != src_eff {
                                    log::info!("Effect reorder (deferred): master effect {} -> {}", src_eff, to);
                                    actions.master_effect_to_move = Some((src_eff, to));
                                }
                            }
                        }
                    }
                }

                ctx.memory_mut(|mem| {
                    mem.data.remove::<bool>(had_eff_id);
                    mem.data.remove::<Option<(String, usize)>>(hover_dz_id);
                    mem.data.remove::<EffectDrag>(src_key);
                });
            }
        }
    }

    // === NOTIFICATION OVERLAY (rendered last, on top of everything) ===
    render_notifications(ctx, &data.notifications, &mut actions);

    // === GLOBAL RIGHT-CLICK: Toggle MIDI Learn Mode ===
    // Track popup state: position and "just opened" flag to avoid same-frame dismiss
    let popup_id = egui::Id::new("global_midi_learn_popup");
    let popup_fresh_id = egui::Id::new("global_midi_learn_popup_fresh");

    // Check if popup is currently open
    let popup_pos: Option<egui::Pos2> = ctx.memory(|mem| mem.data.get_temp(popup_id));
    let popup_fresh: bool = ctx.memory(|mem| mem.data.get_temp(popup_fresh_id).unwrap_or(false));

    if ctx.input(|i| i.pointer.secondary_clicked()) {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            if popup_pos.is_some() {
                // Close existing popup
                ctx.memory_mut(|mem| {
                    mem.data.remove::<egui::Pos2>(popup_id);
                    mem.data.remove::<bool>(popup_fresh_id);
                });
            } else {
                // Open new popup, mark as fresh (skip dismiss this frame)
                ctx.memory_mut(|mem| {
                    mem.data.insert_temp(popup_id, pos);
                    mem.data.insert_temp(popup_fresh_id, true);
                });
            }
        }
    }

    // Re-read after potential update
    let popup_pos: Option<egui::Pos2> = ctx.memory(|mem| mem.data.get_temp(popup_id));
    if let Some(pos) = popup_pos {
        let label = if data.midi_learn_active { "🎹 Exit MIDI Learn" } else { "🎹 Enter MIDI Learn" };

        let area_resp = egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if ui.button(label).clicked() {
                        actions.midi_learn_toggle = true;
                        ctx.memory_mut(|mem| {
                            mem.data.remove::<egui::Pos2>(popup_id);
                            mem.data.remove::<bool>(popup_fresh_id);
                        });
                    }
                });
            });

        // Dismiss on primary click outside the popup (but not on the fresh frame)
        if !popup_fresh {
            if ctx.input(|i| i.pointer.primary_clicked()) {
                let popup_rect = area_resp.response.rect;
                let click_pos = ctx.input(|i| i.pointer.interact_pos());
                if let Some(click) = click_pos {
                    if !popup_rect.contains(click) {
                        ctx.memory_mut(|mem| {
                            mem.data.remove::<egui::Pos2>(popup_id);
                            mem.data.remove::<bool>(popup_fresh_id);
                        });
                    }
                }
            }
        } else {
            // Clear the fresh flag for next frame
            ctx.memory_mut(|mem| {
                mem.data.insert_temp(popup_fresh_id, false);
            });
        }
    }

    // === KEYBOARD SHORTCUTS (global) ===
    // Ctrl+S / Cmd+S: save workspace
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
        actions.save_requested = true;
    }

    // L key: toggle library panel (only if no text field has focus)
    if !ctx.wants_keyboard_input() {
        if ctx.input(|i| i.key_pressed(egui::Key::L)) {
            actions.toggle_library_panel = true;
        }
    }

    actions
}

/// Render toast notifications as an overlay in the top-right corner
fn render_notifications(ctx: &egui::Context, notifications: &[NotificationUI], actions: &mut UIActions) {
    if notifications.is_empty() {
        return;
    }

    let screen_rect = ctx.content_rect();
    let toast_width = 360.0;
    let toast_height = 48.0;
    let margin = 12.0;
    let spacing = 6.0;

    for (i, notif) in notifications.iter().enumerate() {
        let x = screen_rect.right() - toast_width - margin;
        let y = screen_rect.top() + margin + (toast_height + spacing) * i as f32;

        let toast_rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(toast_width, toast_height),
        );

        let layer_id = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(format!("notif_{}", i)));
        let painter = ctx.layer_painter(layer_id);

        // Background
        painter.rect_filled(toast_rect, 6.0, egui::Color32::from_rgba_unmultiplied(20, 20, 30, 230));

        // Left accent bar
        let accent_color = notif.level.color();
        let bar_rect = egui::Rect::from_min_size(
            toast_rect.min,
            egui::vec2(4.0, toast_height),
        );
        painter.rect_filled(bar_rect, egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, accent_color);

        // Level label
        painter.text(
            egui::pos2(toast_rect.left() + 12.0, toast_rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            notif.level.label(),
            egui::FontId::proportional(10.0),
            accent_color,
        );

        // Message text (truncated)
        let max_msg_width = toast_width - 50.0;
        let msg = if notif.message.len() > 60 {
            format!("{}…", &notif.message[..59])
        } else {
            notif.message.clone()
        };
        painter.text(
            egui::pos2(toast_rect.left() + 12.0, toast_rect.top() + 22.0),
            egui::Align2::LEFT_TOP,
            &msg,
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(220),
        );

        // Progress bar (fade out indicator)
        let progress_width = toast_width * (1.0 - notif.progress);
        let progress_rect = egui::Rect::from_min_size(
            egui::pos2(toast_rect.left(), toast_rect.bottom() - 2.0),
            egui::vec2(progress_width, 2.0),
        );
        painter.rect_filled(progress_rect, 0.0, accent_color.linear_multiply(0.5));

        // Dismiss button ("✕") — use an Area so it's clickable
        let dismiss_id = egui::Id::new(format!("dismiss_notif_{}", i));
        egui::Area::new(dismiss_id)
            .fixed_pos(egui::pos2(toast_rect.right() - 24.0, toast_rect.top() + 4.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if ui.add(egui::Button::new(egui::RichText::new("✕").size(12.0).color(egui::Color32::GRAY))
                    .frame(false)
                ).clicked() {
                    actions.notifications_to_dismiss.push(i);
                }
            });
    }

    // Request repaint to animate progress bars
    ctx.request_repaint();
}

/// Render the left library panel — generators, effects, images, video, solid color
fn render_library_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.horizontal(|ui| {
        ui.heading("📚 Library");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("◀").on_hover_text("Close library (L)").clicked() {
                actions.toggle_library_panel = true;
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical().scroll_source(egui::scroll_area::ScrollSource {
        scroll_bar: true,
        drag: false,
        mouse_wheel: true,
    }).show(ui, |ui| {
        // === GENERATORS ===
        let gen_header = egui::RichText::new(format!("🎨 Generators ({})", data.generators.len())).strong();
        egui::CollapsingHeader::new(gen_header).default_open(true).show(ui, |ui| {
            for (name, gen_idx) in &data.generators {
                let item_id = egui::Id::new(("lib_gen", *gen_idx));
                let resp = ui.dnd_drag_source(item_id, LibraryDrag::Generator(*gen_idx), |ui| {
                    ui.label(egui::RichText::new(format!("  ◆ {}", name)).size(12.0));
                }).response;
                // Store generator index in temp memory so the deferred drop handler can use it
                if ui.ctx().is_being_dragged(item_id) {
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(egui::Id::new("__lib_dnd_gen_idx"), *gen_idx);
                    });
                }
                // Fallback: double-click adds to first channel
                if resp.double_clicked() {
                    actions.shader_to_add = Some((0, *gen_idx));
                }
                resp.on_hover_text("Drag to a channel to create a deck, or double-click to add to Ch 1");
            }
        });

        ui.add_space(4.0);

        // === EFFECTS ===
        let fx_header = egui::RichText::new(format!("🔮 Effects ({})", data.filters.len())).strong();
        egui::CollapsingHeader::new(fx_header).default_open(true).show(ui, |ui| {
            for (name, filter_idx) in &data.filters {
                let item_id = egui::Id::new(("lib_fx", *filter_idx));
                ui.dnd_drag_source(item_id, LibraryDrag::Effect(*filter_idx), |ui| {
                    ui.label(egui::RichText::new(format!("  ◇ {}", name)).size(12.0));
                });
                // Store effect filter index in temp memory for deferred drop handler
                if ui.ctx().is_being_dragged(item_id) {
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(egui::Id::new("__lib_dnd_fx_idx"), *filter_idx);
                    });
                }
            }
        });

        ui.add_space(4.0);

        // === IMAGES ===
        let img_header = egui::RichText::new("🖼 Images").strong();
        egui::CollapsingHeader::new(img_header).default_open(false).show(ui, |ui| {
            ui.label(egui::RichText::new("Load image files as deck sources").small().weak());
            for ch in &data.channels {
                if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                    actions.open_image_dialog_for_channel = Some(ch.ch_idx);
                }
            }
        });

        ui.add_space(4.0);

        // === VIDEO ===
        let vid_header = egui::RichText::new("🎬 Video").strong();
        egui::CollapsingHeader::new(vid_header).default_open(false).show(ui, |ui| {
            ui.label(egui::RichText::new("Load video files as deck sources").small().weak());
            for ch in &data.channels {
                if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                    actions.open_video_dialog_for_channel = Some(ch.ch_idx);
                }
            }
        });

        ui.add_space(4.0);

        // === SOLID COLOR ===
        let color_header = egui::RichText::new("🎨 Solid Color").strong();
        egui::CollapsingHeader::new(color_header).default_open(false).show(ui, |ui| {
            for ch in &data.channels {
                if ui.button(format!("Add to {}", ch.name)).clicked() {
                    actions.solid_color_to_add = Some((ch.ch_idx, [0.0, 0.0, 0.0, 1.0]));
                }
            }
        });

        ui.add_space(4.0);

        // === CAMERAS ===
        let cam_header = egui::RichText::new(format!("📹 Cameras ({})", data.cameras.len())).strong();
        egui::CollapsingHeader::new(cam_header).default_open(true).show(ui, |ui| {
            if ui.small_button("🔄 Rescan").clicked() {
                actions.camera_rescan = true;
            }
            if data.cameras.is_empty() {
                ui.label(egui::RichText::new("No cameras detected").small().weak());
            }
            for (name, cam_id) in &data.cameras {
                let item_id = egui::Id::new(("lib_cam", *cam_id));
                ui.dnd_drag_source(item_id, LibraryDrag::Camera(*cam_id), |ui| {
                    ui.label(egui::RichText::new(format!("  📹 {}", name)).size(12.0));
                }).response;
                if ui.ctx().is_being_dragged(item_id) {
                    ui.ctx().memory_mut(|mem| {
                        mem.data.insert_temp(egui::Id::new("__lib_dnd_cam_id"), *cam_id);
                    });
                }
            }
        });

        ui.add_space(4.0);

        // === AUDIO (placeholder) ===
        let audio_header = egui::RichText::new("🔊 Audio").strong();
        egui::CollapsingHeader::new(audio_header).default_open(false).show(ui, |ui| {
            ui.label(egui::RichText::new("Audio sources (coming soon)").small().weak());
        });
    });
}

/// Render the right side panel (main output + master effects only)
fn render_right_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Clickable heading — selects master for bottom bar
        let heading_response = ui.add(
            egui::Label::new(egui::RichText::new("🎬 Main Output").heading())
                .sense(egui::Sense::click()),
        );
        if heading_response.clicked() {
            actions.select_master = true;
        }

        // Main output preview (clickable to select master)
        let preview_width = ui.available_width() - 10.0;
        let preview_height = preview_width * 0.5625;
        let preview_size = egui::vec2(preview_width, preview_height);

        if let Some(texture_id) = data.main_output_texture {
            let img_response = ui.add(egui::Image::new(egui::load::SizedTexture::new(texture_id, preview_size))
                .corner_radius(4.0)
                .sense(egui::Sense::click()));
            if img_response.clicked() {
                actions.select_master = true;
            }
        } else {
            ui.allocate_ui(preview_size, |ui| {
                let (rect, response) = ui.allocate_exact_size(preview_size, egui::Sense::click());
                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 20, 30));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No Output",
                    egui::FontId::proportional(14.0),
                    egui::Color32::GRAY,
                );
                if response.clicked() {
                    actions.select_master = true;
                }
            });
        }

        ui.add_space(10.0);
        ui.heading("🔮 Master Effects");
        ui.label("(Apply to final composite)");

        ui.add_space(10.0);
        ui.separator();

        // Modulation sources
        render_modulation_section(ui, data, actions);

        ui.add_space(10.0);
        ui.separator();

        // Library panel toggle (if closed, show a button to reopen)
        if !data.library_panel_open {
            if ui.button("📚 Open Library (L)").clicked() {
                actions.toggle_library_panel = true;
            }
            ui.add_space(10.0);
            ui.separator();
        }

        // MIDI devices & mappings
        render_midi_section(ui, data, actions);

        ui.add_space(10.0);
        ui.separator();

        // Surface editor (2D stage layout)
        render_surface_editor(ui, data, actions);

        ui.add_space(10.0);
        ui.separator();

        // Output windows management
        render_output_section(ui, data, actions);
    });
}

/// Render the 2D surface editor — interactive canvas for placing/naming surfaces
fn render_surface_editor(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🗺 Stage Layout");

    // Open Editor / Add Surface buttons
    ui.horizontal(|ui| {
        let editor_label = if data.stage_editor_open { "✏ Close Editor" } else { "✏ Open Editor" };
        if ui.button(editor_label).clicked() {
            actions.toggle_stage_editor = true;
        }
        if ui.button("+ Add Surface").clicked() {
            let idx = data.surfaces.len() + 1;
            actions.surface_actions.push(SurfaceAction::Add {
                name: format!("Surface {}", idx),
                source: OutputSource::Master,
            });
        }
    });

    ui.add_space(4.0);

    // 2D Canvas — draw surfaces as rectangles
    let canvas_width = ui.available_width() - 4.0;
    let canvas_height = canvas_width * 0.5625; // 16:9 aspect
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Canvas background (dark stage)
    painter.rect_filled(canvas_rect, 4.0, egui::Color32::from_rgb(15, 15, 25));
    painter.rect_stroke(canvas_rect, 4.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 80)), egui::StrokeKind::Outside);

    // Grid lines
    for i in 1..4 {
        let x = canvas_rect.left() + canvas_width * (i as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(x, canvas_rect.top()), egui::pos2(x, canvas_rect.bottom())],
            egui::Stroke::new(0.5, egui::Color32::from_rgb(30, 30, 45)),
        );
    }
    for i in 1..3 {
        let y = canvas_rect.top() + canvas_height * (i as f32 / 3.0);
        painter.line_segment(
            [egui::pos2(canvas_rect.left(), y), egui::pos2(canvas_rect.right(), y)],
            egui::Stroke::new(0.5, egui::Color32::from_rgb(30, 30, 45)),
        );
    }

    // Draw each surface
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
        let fill = egui::Color32::from_rgba_premultiplied(color.r() / 4, color.g() / 4, color.b() / 4, 160);

        // Convert normalized vertices to canvas pixel positions
        let pixel_verts: Vec<egui::Pos2> = surface.vertices.iter().map(|v| {
            egui::pos2(
                canvas_rect.left() + v[0] * canvas_width,
                canvas_rect.top() + v[1] * canvas_height,
            )
        }).collect();

        if pixel_verts.len() >= 3 {
            painter.add(polygon_shape(&pixel_verts, fill, egui::Stroke::new(1.5, color)));
        } else if pixel_verts.len() == 2 {
            painter.line_segment([pixel_verts[0], pixel_verts[1]], egui::Stroke::new(1.5, color));
        }
        // Draw extra contours (combined non-overlapping surfaces)
        for ec in &surface.extra_contours {
            let ec_verts: Vec<egui::Pos2> = ec.iter().map(|v| {
                egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
            }).collect();
            if ec_verts.len() >= 3 {
                painter.add(polygon_shape(&ec_verts, fill, egui::Stroke::new(1.5, color)));
            }
        }

        // Surface label at center
        let n = surface.vertices.len().max(1) as f32;
        let center = surface.vertices.iter().fold(egui::pos2(0.0, 0.0), |acc, v| {
            egui::pos2(acc.x + v[0] / n, acc.y + v[1] / n)
        });
        let center_px = egui::pos2(
            canvas_rect.left() + center.x * canvas_width,
            canvas_rect.top() + center.y * canvas_height,
        );
        let label = format!("{}\n{}", surface.name, surface.source);
        painter.text(center_px, egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(10.0), egui::Color32::WHITE);

        // Output type + mapping mode indicators
        let type_label = match surface.output_type {
            SurfaceOutputType::Projection => "📽",
            SurfaceOutputType::LEDDirect => "💡",
        };
        let mapping_label = match surface.content_mapping {
            ContentMapping::Fill => "▣",
            ContentMapping::Mapped => "▥",
        };
        // Place indicator near first vertex
        if let Some(v0) = pixel_verts.first() {
            painter.text(
                egui::pos2(v0.x + 4.0, v0.y + 4.0),
                egui::Align2::LEFT_TOP,
                &format!("{}{}", mapping_label, type_label),
                egui::FontId::proportional(9.0),
                egui::Color32::WHITE,
            );
        }

        // Vertex handles
        let handle_size = 5.0;
        for v in &pixel_verts {
            let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(handle_size, handle_size));
            painter.rect_filled(handle_rect, 1.0, color);
        }
    }

    // Handle drag interactions on the canvas
    let drag_id = ui.id().with("surface_drag");
    let _drag_state = ui.memory(|mem| {
        mem.data.get_temp::<SurfaceDragState>(drag_id)
    });

    if canvas_response.drag_started() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            let nx = (pos.x - canvas_rect.left()) / canvas_width;
            let ny = (pos.y - canvas_rect.top()) / canvas_height;

            // Check if near a vertex (drag vertex) or inside a surface (move whole shape)
            // Use pixel-space distance for correct hit detection on non-square canvas
            let vertex_threshold_px = 14.0;
            let mut found_vertex = None;
            let mut found_surface = None;

            for (i, surface) in data.surfaces.iter().enumerate().rev() {
                if let Some(vert_idx) = surface.vertices.iter().enumerate()
                    .map(|(vi, v)| {
                        let dx_px = (nx - v[0]) * canvas_width;
                        let dy_px = (ny - v[1]) * canvas_height;
                        (vi, (dx_px * dx_px + dy_px * dy_px).sqrt())
                    })
                    .filter(|(_, d)| *d < vertex_threshold_px)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .map(|(vi, _)| vi)
                {
                    found_vertex = Some((i, vert_idx));
                    break;
                }
                // Point-in-polygon test for move
                if found_surface.is_none() {
                    let verts = &surface.vertices;
                    let n = verts.len();
                    if n >= 3 {
                        let mut inside = false;
                        let mut j = n - 1;
                        for k in 0..n {
                            let (xi, yi) = (verts[k][0], verts[k][1]);
                            let (xj, yj) = (verts[j][0], verts[j][1]);
                            if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
                                inside = !inside;
                            }
                            j = k;
                        }
                        if inside {
                            found_surface = Some((i, nx, ny));
                        }
                    }
                }
            }

            let state = if let Some((surf_idx, vert_idx)) = found_vertex {
                SurfaceDragState::DraggingVertex { surf_idx, vert_idx }
            } else if let Some((surf_idx, start_x, start_y)) = found_surface {
                SurfaceDragState::Moving { surf_idx, last_x: start_x, last_y: start_y }
            } else {
                SurfaceDragState::None
            };

            ui.memory_mut(|mem| mem.data.insert_temp(drag_id, state));
        }
    }

    if canvas_response.dragged() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            let nx = ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0);
            let ny = ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0);

            let state = ui.memory(|mem| {
                mem.data.get_temp::<SurfaceDragState>(drag_id).unwrap_or(SurfaceDragState::None)
            });

            match state {
                SurfaceDragState::Moving { surf_idx, last_x, last_y } => {
                    if data.surfaces.get(surf_idx).is_some() {
                        let dx = nx - last_x;
                        let dy = ny - last_y;
                        actions.surface_actions.push(SurfaceAction::MoveDelta {
                            idx: surf_idx, dx, dy,
                        });
                        ui.memory_mut(|mem| mem.data.insert_temp(drag_id,
                            SurfaceDragState::Moving { surf_idx, last_x: nx, last_y: ny }));
                    }
                }
                SurfaceDragState::DraggingVertex { surf_idx, vert_idx } => {
                    if let Some(surface) = data.surfaces.get(surf_idx) {
                        let mut new_verts = surface.vertices.clone();
                        if vert_idx < new_verts.len() {
                            new_verts[vert_idx] = [nx, ny];
                            actions.surface_actions.push(SurfaceAction::UpdateVertices {
                                idx: surf_idx, contour: 0, vertices: new_verts,
                            });
                        }
                    }
                }
                SurfaceDragState::None => {}
            }
        }
    }

    if canvas_response.drag_stopped() {
        ui.memory_mut(|mem| mem.data.insert_temp(drag_id, SurfaceDragState::None));
    }

    ui.add_space(4.0);

    // Surface list with properties
    for (i, surface) in data.surfaces.iter().enumerate() {
        let color = surface_colors[i % surface_colors.len()];
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(3.0)
            .stroke(egui::Stroke::new(1.0, color.linear_multiply(0.5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Color swatch
                    let (swatch_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), egui::Sense::hover());
                    ui.painter().rect_filled(swatch_rect, 2.0, color);

                    ui.label(egui::RichText::new(&surface.name).strong().size(11.0));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").clicked() {
                            actions.surface_actions.push(SurfaceAction::Remove { idx: i });
                        }
                    });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Source:").weak().size(10.0));
                    let current_label = format!("{}", surface.source);
                    egui::ComboBox::from_id_salt(format!("surf_src_{}", i))
                        .selected_text(&current_label)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(surface.source == OutputSource::Master, "Master").clicked() {
                                actions.surface_actions.push(SurfaceAction::SetSource {
                                    idx: i,
                                    source: OutputSource::Master,
                                });
                            }
                            for ch in &data.channels {
                                if ui.selectable_label(
                                    surface.source == OutputSource::Channel(ch.ch_idx),
                                    &ch.name,
                                ).clicked() {
                                    actions.surface_actions.push(SurfaceAction::SetSource {
                                        idx: i,
                                        source: OutputSource::Channel(ch.ch_idx),
                                    });
                                }
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Mapping:").weak().size(10.0));
                    egui::ComboBox::from_id_salt(format!("surf_map_{}", i))
                        .selected_text(format!("{}", surface.content_mapping))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(
                                surface.content_mapping == ContentMapping::Fill,
                                "Fill",
                            ).on_hover_text("Entire source scaled to fill this surface")
                            .clicked() {
                                actions.surface_actions.push(SurfaceAction::SetContentMapping {
                                    idx: i,
                                    mapping: ContentMapping::Fill,
                                });
                            }
                            if ui.selectable_label(
                                surface.content_mapping == ContentMapping::Mapped,
                                "Mapped",
                            ).on_hover_text("Surface position on canvas = UV crop into source")
                            .clicked() {
                                actions.surface_actions.push(SurfaceAction::SetContentMapping {
                                    idx: i,
                                    mapping: ContentMapping::Mapped,
                                });
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Type:").weak().size(10.0));
                    egui::ComboBox::from_id_salt(format!("surf_type_{}", i))
                        .selected_text(format!("{}", surface.output_type))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(
                                surface.output_type == SurfaceOutputType::Projection,
                                "📽 Projection",
                            ).clicked() {
                                actions.surface_actions.push(SurfaceAction::SetOutputType {
                                    idx: i,
                                    output_type: SurfaceOutputType::Projection,
                                });
                            }
                            if ui.selectable_label(
                                surface.output_type == SurfaceOutputType::LEDDirect,
                                "💡 LED Direct",
                            ).clicked() {
                                actions.surface_actions.push(SurfaceAction::SetOutputType {
                                    idx: i,
                                    output_type: SurfaceOutputType::LEDDirect,
                                });
                            }
                        });
                });
            });
        ui.add_space(2.0);
    }

    if data.surfaces.is_empty() {
        ui.label(egui::RichText::new("No surfaces. Add one to define your stage layout.").weak().small());
    }
}

/// Drag state for the surface canvas editor
#[derive(Debug, Clone, Copy, Default)]
enum SurfaceDragState {
    #[default]
    None,
    Moving { surf_idx: usize, last_x: f32, last_y: f32 },
    DraggingVertex { surf_idx: usize, vert_idx: usize },
}

/// Drawing tool for the stage editor
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum DrawingTool {
    #[default]
    Select,
    Rectangle,
    Polygon,
    Circle,
}

/// State for active drawing operations in the stage editor
#[derive(Debug, Clone, Default)]
struct StageEditorState {
    tool: DrawingTool,
    /// For rectangle tool: start position of drag
    rect_start: Option<[f32; 2]>,
    /// For polygon tool: accumulated vertices
    polygon_verts: Vec<[f32; 2]>,
    /// For circle tool: center position
    circle_center: Option<[f32; 2]>,
    /// Number of sides for circle/N-gon approximation
    circle_sides: u32,
    /// Currently selected surface indices (supports multi-select)
    selected_surfaces: std::collections::BTreeSet<usize>,
    /// Drag state for vertex editing in select mode
    dragging_vertex: Option<(usize, usize, usize)>, // (surface_idx, contour_idx, vertex_idx)
    /// Drag state for moving whole surface in select mode
    moving_surface: Option<(usize, f32, f32)>, // (surface_idx, last_x, last_y)
    /// Marquee selection: start position of drag rectangle in normalized coords
    selection_rect_start: Option<[f32; 2]>,
    /// Drag state for radius handle on circle surfaces
    dragging_radius: Option<usize>, // surface_idx
}

/// Full-screen stage editor — replaces the deck view
fn render_stage_editor(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let state_id = ui.id().with("stage_editor_state");
    let mut state = ui.memory(|mem| {
        mem.data.get_temp::<StageEditorState>(state_id).unwrap_or_default()
    });

    // Toolbar at top
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("🎨 Stage Editor").strong().size(16.0));
        ui.separator();

        // Tool buttons
        let tools = [
            (DrawingTool::Select, "⬚ Select", "Select and edit surfaces (S)"),
            (DrawingTool::Rectangle, "▭ Rectangle", "Draw rectangle surfaces (R)"),
            (DrawingTool::Polygon, "⬠ Polygon", "Draw polygon surfaces — click to add vertices, double-click to finish (P)"),
            (DrawingTool::Circle, "⬤ Circle", "Draw circle/N-gon surfaces (C)"),
        ];
        for (tool, label, tooltip) in &tools {
            let selected = state.tool == *tool;
            let btn = ui.selectable_label(selected, *label);
            if btn.on_hover_text(*tooltip).clicked() {
                state.tool = *tool;
                // Clear any in-progress drawing
                state.rect_start = None;
                state.polygon_verts.clear();
                state.circle_center = None;
            }
        }

        ui.separator();

        // Grid controls
        let snap_label = if data.stage_editor_snap { "🧲 Snap: ON" } else { "🧲 Snap: OFF" };
        if ui.button(snap_label).clicked() {
            actions.toggle_snap = true;
        }

        // Grid size selector
        let grid_sizes = [
            (0.1, "10%"),
            (0.05, "5%"),
            (0.025, "2.5%"),
            (0.0125, "1.25%"),
        ];
        egui::ComboBox::from_id_salt("grid_size")
            .selected_text(format!("Grid: {:.1}%", data.stage_editor_grid_size * 100.0))
            .width(90.0)
            .show_ui(ui, |ui| {
                for (size, label) in &grid_sizes {
                    if ui.selectable_value(&mut actions.set_grid_size, Some(*size), *label).clicked() {
                        // handled by set_grid_size
                    }
                }
            });

        // Circle sides (only when circle tool selected)
        if state.tool == DrawingTool::Circle {
            ui.separator();
            ui.label("Sides:");
            if state.circle_sides == 0 { state.circle_sides = 32; }
            ui.add(egui::DragValue::new(&mut state.circle_sides).range(3..=128).speed(1));
        }

        // Circle-specific toolbar: when exactly one circle is selected, show radius/sides/convert
        let selected_circle = if state.selected_surfaces.len() == 1 {
            let idx = *state.selected_surfaces.iter().next().unwrap();
            data.surfaces.get(idx).and_then(|s| s.circle_hint.map(|h| (idx, h)))
        } else {
            None
        };
        if let Some((sel_idx, hint)) = selected_circle {
            ui.separator();
            ui.label("⬤ Circle:");
            let mut radius = hint.radius;
            if ui.add(egui::DragValue::new(&mut radius).prefix("R: ").range(0.01..=1.0).speed(0.005)).changed() {
                actions.surface_actions.push(SurfaceAction::SetCircleRadius { idx: sel_idx, radius });
            }
            let mut sides = hint.sides;
            if ui.add(egui::DragValue::new(&mut sides).prefix("Sides: ").range(3..=128).speed(1)).changed() {
                actions.surface_actions.push(SurfaceAction::SetCircleSides { idx: sel_idx, sides });
            }
            if ui.button("⬠ Convert to Polygon").on_hover_text("Drop circle identity, keep vertices as polygon").clicked() {
                actions.surface_actions.push(SurfaceAction::ConvertToPolygon { idx: sel_idx });
            }
        }

        // Duplicate & flip (enabled when any surfaces are selected)
        ui.separator();
        let has_sel = !state.selected_surfaces.is_empty();
        ui.add_enabled_ui(has_sel, |ui| {
            if ui.button("📋 Dup").on_hover_text("Duplicate selected (D)").clicked() {
                for &idx in &state.selected_surfaces {
                    actions.surface_actions.push(SurfaceAction::Duplicate { idx });
                }
            }
            if ui.button("↔ Flip H").on_hover_text("Flip horizontal (H)").clicked() {
                for &idx in &state.selected_surfaces {
                    actions.surface_actions.push(SurfaceAction::FlipHorizontal { idx });
                }
            }
            if ui.button("↕ Flip V").on_hover_text("Flip vertical (V)").clicked() {
                for &idx in &state.selected_surfaces {
                    actions.surface_actions.push(SurfaceAction::FlipVertical { idx });
                }
            }
            if state.selected_surfaces.len() >= 2 {
                if ui.button("🔗 Combine").on_hover_text("Combine selected surfaces (G)").clicked() {
                    let indices: Vec<usize> = state.selected_surfaces.iter().copied().collect();
                    actions.surface_actions.push(SurfaceAction::Combine { indices });
                    state.selected_surfaces.clear();
                }
            }
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕ Close Editor").clicked() {
                actions.toggle_stage_editor = true;
            }
        });
    });

    ui.add_space(4.0);

    // Main canvas — fill available space
    let canvas_width = ui.available_width();
    let canvas_height = ui.available_height().max(200.0);
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Canvas background
    painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_rgb(10, 10, 18));

    // Grid lines
    let grid_size = data.stage_editor_grid_size;
    if grid_size > 0.001 {
        let steps = (1.0 / grid_size).round() as usize;
        for i in 1..steps {
            let t = i as f32 * grid_size;
            let x = canvas_rect.left() + t * canvas_width;
            let y = canvas_rect.top() + t * canvas_height;
            if x < canvas_rect.right() {
                painter.line_segment(
                    [egui::pos2(x, canvas_rect.top()), egui::pos2(x, canvas_rect.bottom())],
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(25, 25, 38)),
                );
            }
            if y < canvas_rect.bottom() {
                painter.line_segment(
                    [egui::pos2(canvas_rect.left(), y), egui::pos2(canvas_rect.right(), y)],
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(25, 25, 38)),
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
        let is_selected = state.selected_surfaces.contains(&i);
        let fill_alpha = if is_selected { 120 } else { 60 };
        let fill = egui::Color32::from_rgba_premultiplied(color.r() / 3, color.g() / 3, color.b() / 3, fill_alpha);
        let stroke_width = if is_selected { 2.5 } else { 1.5 };

        let pixel_verts: Vec<egui::Pos2> = surface.vertices.iter().map(|v| {
            egui::pos2(
                canvas_rect.left() + v[0] * canvas_width,
                canvas_rect.top() + v[1] * canvas_height,
            )
        }).collect();

        if pixel_verts.len() >= 3 {
            painter.add(polygon_shape(&pixel_verts, fill, egui::Stroke::new(stroke_width, color)));
        }
        // Draw extra contours (combined non-overlapping surfaces)
        for ec in &surface.extra_contours {
            let ec_verts: Vec<egui::Pos2> = ec.iter().map(|v| {
                egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
            }).collect();
            if ec_verts.len() >= 3 {
                painter.add(polygon_shape(&ec_verts, fill, egui::Stroke::new(stroke_width, color)));
            }
        }

        // Label
        let n = surface.vertices.len().max(1) as f32;
        let center = surface.vertices.iter().fold([0.0f32, 0.0], |acc, v| {
            [acc[0] + v[0] / n, acc[1] + v[1] / n]
        });
        let center_px = egui::pos2(
            canvas_rect.left() + center[0] * canvas_width,
            canvas_rect.top() + center[1] * canvas_height,
        );
        painter.text(center_px, egui::Align2::CENTER_CENTER, &surface.name, egui::FontId::proportional(13.0), egui::Color32::WHITE);

        // For circles: render radius handle instead of vertex handles
        if is_selected && surface.circle_hint.is_some() {
            let hint = surface.circle_hint.unwrap();
            let cx_px = canvas_rect.left() + hint.center[0] * canvas_width;
            let cy_px = canvas_rect.top() + hint.center[1] * canvas_height;
            let center_pos = egui::pos2(cx_px, cy_px);
            // Radius ring — compute the pixel radius at angle=0
            let radius_px_x = hint.radius * canvas_width;
            let radius_px_y = hint.radius * hint.aspect_ratio * canvas_height;
            let avg_radius_px = (radius_px_x + radius_px_y) / 2.0;
            // Center dot (white)
            painter.circle_filled(center_pos, 4.0, egui::Color32::WHITE);
            // Radius ring (yellow, dashed look via stroke)
            painter.circle_stroke(center_pos, avg_radius_px, egui::Stroke::new(1.0, egui::Color32::YELLOW));
            // Radius handle at angle=0 (yellow dot on the right)
            let handle_pos = egui::pos2(cx_px + radius_px_x, cy_px);
            painter.circle_filled(handle_pos, 6.0, egui::Color32::YELLOW);
            painter.circle_stroke(handle_pos, 6.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
        } else {
            // Regular vertex handles (primary + extra contours)
            let handle_size = if is_selected { 10.0 } else { 7.0 };
            let handle_color = if is_selected { egui::Color32::WHITE } else { color };
            let draw_handles = |verts: &[egui::Pos2]| {
                for v in verts {
                    let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(handle_size, handle_size));
                    painter.rect_filled(handle_rect, 2.0, handle_color);
                    painter.rect_stroke(handle_rect, 2.0, egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Outside);
                }
            };
            draw_handles(&pixel_verts);
            for ec in &surface.extra_contours {
                let ec_px: Vec<egui::Pos2> = ec.iter().map(|v| {
                    egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
                }).collect();
                draw_handles(&ec_px);
            }
        }
    }

    // Draw in-progress polygon
    if !state.polygon_verts.is_empty() && state.tool == DrawingTool::Polygon {
        let pixel_verts: Vec<egui::Pos2> = state.polygon_verts.iter().map(|v| {
            egui::pos2(canvas_rect.left() + v[0] * canvas_width, canvas_rect.top() + v[1] * canvas_height)
        }).collect();
        for i in 0..pixel_verts.len() - 1 {
            painter.line_segment([pixel_verts[i], pixel_verts[i + 1]], egui::Stroke::new(2.0, egui::Color32::YELLOW));
        }
        // Draw line from last vertex to cursor
        if let Some(pos) = canvas_response.hover_pos() {
            if let Some(last) = pixel_verts.last() {
                painter.line_segment([*last, pos], egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 255, 0, 128)));
            }
        }
        for v in &pixel_verts {
            let handle_rect = egui::Rect::from_center_size(*v, egui::vec2(8.0, 8.0));
            painter.rect_filled(handle_rect, 2.0, egui::Color32::YELLOW);
        }
    }

    // Draw in-progress rectangle preview
    if let Some(start) = state.rect_start {
        if state.tool == DrawingTool::Rectangle {
            if let Some(pos) = canvas_response.hover_pos() {
                let end_x = (pos.x - canvas_rect.left()) / canvas_width;
                let end_y = (pos.y - canvas_rect.top()) / canvas_height;
                let (sx, sy) = (start[0], start[1]);
                let preview_rect = egui::Rect::from_two_pos(
                    egui::pos2(canvas_rect.left() + sx * canvas_width, canvas_rect.top() + sy * canvas_height),
                    egui::pos2(canvas_rect.left() + end_x * canvas_width, canvas_rect.top() + end_y * canvas_height),
                );
                painter.rect_stroke(preview_rect, 0.0, egui::Stroke::new(2.0, egui::Color32::YELLOW), egui::StrokeKind::Outside);
            }
        }
    }

    // Draw in-progress circle preview
    if let Some(center) = state.circle_center {
        if state.tool == DrawingTool::Circle {
            if let Some(pos) = canvas_response.hover_pos() {
                let cx_px = canvas_rect.left() + center[0] * canvas_width;
                let cy_px = canvas_rect.top() + center[1] * canvas_height;
                let radius = ((pos.x - cx_px).powi(2) + (pos.y - cy_px).powi(2)).sqrt();
                painter.circle_stroke(egui::pos2(cx_px, cy_px), radius, egui::Stroke::new(2.0, egui::Color32::YELLOW));
            }
        }
    }

    // --- Interaction handling ---
    let snap = |v: f32| -> f32 {
        if data.stage_editor_snap && grid_size > 0.001 {
            (v / grid_size).round() * grid_size
        } else {
            v
        }
    };

    let to_norm = |pos: egui::Pos2| -> [f32; 2] {
        let nx = ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0);
        let ny = ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0);
        [snap(nx), snap(ny)]
    };

    // Helper: check if a point is inside any existing surface (returns surface index)
    let point_in_any_surface = |nx: f32, ny: f32| -> Option<usize> {
        for (i, surface) in data.surfaces.iter().enumerate().rev() {
            let verts = &surface.vertices;
            let n = verts.len();
            if n >= 3 {
                let mut inside = false;
                let mut j = n - 1;
                for k in 0..n {
                    let (xi, yi) = (verts[k][0], verts[k][1]);
                    let (xj, yj) = (verts[j][0], verts[j][1]);
                    if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
                        inside = !inside;
                    }
                    j = k;
                }
                if inside { return Some(i); }
            }
        }
        None
    };

    match state.tool {
        DrawingTool::Select => {
            // Helper: pixel-space distance between a normalized point and a vertex
            let pixel_dist = |nx: f32, ny: f32, vx: f32, vy: f32| -> f32 {
                let dx_px = (nx - vx) * canvas_width;
                let dy_px = (ny - vy) * canvas_height;
                (dx_px * dx_px + dy_px * dy_px).sqrt()
            };

            // Helper: find what's under the cursor
            // vertex: (surface_idx, contour_idx, vertex_idx)
            // edge: (surface_idx, contour_idx, edge_start_idx, projected_point)
            // surface: (surface_idx, nx, ny)
            let hit_test = |nx: f32, ny: f32| -> (Option<(usize, usize, usize)>, Option<(usize, usize, usize, [f32; 2])>, Option<(usize, f32, f32)>) {
                let vertex_threshold_px = 14.0;
                let edge_threshold_px = 10.0;
                let mut found_vertex = None;
                let mut found_edge = None;
                let mut found_surface = None;

                for (i, surface) in data.surfaces.iter().enumerate().rev() {
                    // Check all contours for vertex/edge hits
                    let contours: Vec<&Vec<[f32; 2]>> = std::iter::once(&surface.vertices)
                        .chain(surface.extra_contours.iter()).collect();
                    for (ci, verts) in contours.iter().enumerate() {
                        for (vi, v) in verts.iter().enumerate() {
                            if pixel_dist(nx, ny, v[0], v[1]) < vertex_threshold_px {
                                found_vertex = Some((i, ci, vi));
                                return (found_vertex, None, None);
                            }
                        }
                    }

                    if found_edge.is_none() {
                        for (ci, verts) in contours.iter().enumerate() {
                            let n = verts.len();
                            for ei in 0..n {
                                let ej = (ei + 1) % n;
                                let (ax, ay) = (verts[ei][0], verts[ei][1]);
                                let (bx, by) = (verts[ej][0], verts[ej][1]);
                                let dx = (bx - ax) * canvas_width;
                                let dy = (by - ay) * canvas_height;
                                let len_sq = dx * dx + dy * dy;
                                if len_sq < 1e-6 { continue; }
                                let px_nx = (nx - ax) * canvas_width;
                                let px_ny = (ny - ay) * canvas_height;
                                let t = (px_nx * dx + px_ny * dy) / len_sq;
                                let t = t.clamp(0.0, 1.0);
                                let proj_x = ax + t * (bx - ax);
                                let proj_y = ay + t * (by - ay);
                                if pixel_dist(nx, ny, proj_x, proj_y) < edge_threshold_px {
                                    found_edge = Some((i, ci, ei, [proj_x, proj_y]));
                                    break;
                                }
                            }
                            if found_edge.is_some() { break; }
                        }
                    }

                    // Point-in-polygon (any contour)
                    if found_surface.is_none() {
                        let point_in = |verts: &[[f32; 2]]| -> bool {
                            let n = verts.len();
                            if n < 3 { return false; }
                            let mut inside = false;
                            let mut j = n - 1;
                            for k in 0..n {
                                let (xi, yi) = (verts[k][0], verts[k][1]);
                                let (xj, yj) = (verts[j][0], verts[j][1]);
                                if ((yi > ny) != (yj > ny)) && (nx < (xj - xi) * (ny - yi) / (yj - yi) + xi) {
                                    inside = !inside;
                                }
                                j = k;
                            }
                            inside
                        };
                        if point_in(&surface.vertices) || surface.extra_contours.iter().any(|c| point_in(c)) {
                            found_surface = Some((i, nx, ny));
                        }
                    }
                }
                (found_vertex, found_edge, found_surface)
            };

            // Hover feedback: change cursor when over interactive elements
            if let Some(pos) = canvas_response.hover_pos() {
                let [nx, ny] = to_norm(pos);
                let (found_vertex, _found_edge, found_surface) = hit_test(nx, ny);
                if found_vertex.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if found_surface.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }

            let shift_held = ui.input(|i| i.modifiers.shift);

            // Click to select (without drag)
            if canvas_response.clicked() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
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
            }

            // Double-click on edge to insert vertex
            if canvas_response.double_clicked() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    let (_found_vertex, found_edge, _found_surface) = hit_test(nx, ny);
                    if let Some((si, _ci, ei, snap_pos)) = found_edge {
                        let snapped = [snap(snap_pos[0]), snap(snap_pos[1])];
                        actions.surface_actions.push(SurfaceAction::InsertVertex {
                            idx: si,
                            after_vert_idx: ei,
                            position: snapped,
                        });
                        state.selected_surfaces.clear();
                        state.selected_surfaces.insert(si);
                    }
                }
            }

            // Drag start: begin radius drag, vertex drag, surface move, or marquee selection
            if canvas_response.drag_started() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);

                    // Check for radius handle hit on selected circles first
                    let mut found_radius_handle = None;
                    for &si in &state.selected_surfaces {
                        if let Some(surface) = data.surfaces.get(si) {
                            if let Some(hint) = &surface.circle_hint {
                                // Radius handle is at angle=0: (center_x + radius, center_y)
                                let hx = hint.center[0] + hint.radius;
                                let hy = hint.center[1];
                                if pixel_dist(nx, ny, hx, hy) < 14.0 {
                                    found_radius_handle = Some(si);
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(si) = found_radius_handle {
                        state.dragging_radius = Some(si);
                        state.dragging_vertex = None;
                        state.moving_surface = None;
                        state.selection_rect_start = None;
                    } else {
                        let (found_vertex, _found_edge, found_surface) = hit_test(nx, ny);

                        if let Some((si, ci, vi)) = found_vertex {
                            // If vertex drag on a circle, auto-convert to polygon first
                            if data.surfaces.get(si).map_or(false, |s| s.circle_hint.is_some()) {
                                actions.surface_actions.push(SurfaceAction::ConvertToPolygon { idx: si });
                            }
                            if !shift_held {
                                state.selected_surfaces.clear();
                            }
                            state.selected_surfaces.insert(si);
                            state.dragging_vertex = Some((si, ci, vi));
                            state.moving_surface = None;
                            state.selection_rect_start = None;
                        } else if let Some((si, lx, ly)) = found_surface {
                            if !shift_held && !state.selected_surfaces.contains(&si) {
                                state.selected_surfaces.clear();
                            }
                            state.selected_surfaces.insert(si);
                            state.moving_surface = Some((si, lx, ly));
                            state.dragging_vertex = None;
                            state.selection_rect_start = None;
                        } else {
                            if !shift_held {
                                state.selected_surfaces.clear();
                            }
                            state.selection_rect_start = Some([nx, ny]);
                            state.dragging_vertex = None;
                            state.moving_surface = None;
                        }
                    }
                }
            }

            if canvas_response.dragged() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);

                    if let Some(si) = state.dragging_radius {
                        // Compute new radius from cursor distance to circle center
                        if let Some(surface) = data.surfaces.get(si) {
                            if let Some(hint) = &surface.circle_hint {
                                let dx = nx - hint.center[0];
                                let dy = ny - hint.center[1];
                                // Use x-distance as primary radius (consistent with angle=0 handle)
                                let new_radius = (dx * dx + dy * dy).sqrt().max(0.01);
                                actions.surface_actions.push(SurfaceAction::SetCircleRadius {
                                    idx: si, radius: new_radius,
                                });
                            }
                        }
                    } else if let Some((si, ci, vi)) = state.dragging_vertex {
                        if let Some(surface) = data.surfaces.get(si) {
                            let contour_verts = if ci == 0 { Some(&surface.vertices) } else { surface.extra_contours.get(ci - 1) };
                            if let Some(verts) = contour_verts {
                                let mut new_verts = verts.clone();
                                if vi < new_verts.len() {
                                    new_verts[vi] = [nx, ny];
                                    actions.surface_actions.push(SurfaceAction::UpdateVertices {
                                        idx: si, contour: ci, vertices: new_verts,
                                    });
                                }
                            }
                        }
                    } else if let Some((_si, lx, ly)) = state.moving_surface {
                        let dx = nx - lx;
                        let dy = ny - ly;
                        // Move ALL selected surfaces by the same delta
                        for &surf_idx in &state.selected_surfaces {
                            if data.surfaces.get(surf_idx).is_some() {
                                actions.surface_actions.push(SurfaceAction::MoveDelta {
                                    idx: surf_idx, dx, dy,
                                });
                            }
                        }
                        state.moving_surface = Some((_si, nx, ny));
                    } else if let Some(start) = state.selection_rect_start {
                        // Draw marquee selection rectangle
                        let x0 = canvas_rect.left() + start[0] * canvas_width;
                        let y0 = canvas_rect.top() + start[1] * canvas_height;
                        let x1 = canvas_rect.left() + nx * canvas_width;
                        let y1 = canvas_rect.top() + ny * canvas_height;
                        let sel_rect = egui::Rect::from_two_pos(
                            egui::pos2(x0, y0),
                            egui::pos2(x1, y1),
                        );
                        painter.rect_filled(sel_rect, 0.0, egui::Color32::from_rgba_premultiplied(80, 130, 255, 40));
                        painter.rect_stroke(sel_rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 130, 255)), egui::StrokeKind::Outside);
                    }
                }
            }

            if canvas_response.drag_stopped() {
                // Finish marquee selection: select all surfaces that intersect the rect
                if let Some(start) = state.selection_rect_start {
                    if let Some(pos) = canvas_response.interact_pointer_pos() {
                        let [nx, ny] = to_norm(pos);
                        let sel_min_x = start[0].min(nx);
                        let sel_max_x = start[0].max(nx);
                        let sel_min_y = start[1].min(ny);
                        let sel_max_y = start[1].max(ny);

                        for (i, surface) in data.surfaces.iter().enumerate() {
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
                            let intersects = bb_min_x < sel_max_x && bb_max_x > sel_min_x &&
                                bb_min_y < sel_max_y && bb_max_y > sel_min_y;
                            if intersects {
                                state.selected_surfaces.insert(i);
                            }
                        }
                    }
                }
                state.selection_rect_start = None;
                state.dragging_vertex = None;
                state.moving_surface = None;
                state.dragging_radius = None;
            }

            // Delete selected surfaces with delete key
            if !state.selected_surfaces.is_empty() {
                if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
                    // Remove in reverse order to preserve indices
                    let indices: Vec<usize> = state.selected_surfaces.iter().copied().rev().collect();
                    for idx in indices {
                        actions.surface_actions.push(SurfaceAction::Remove { idx });
                    }
                    state.selected_surfaces.clear();
                }
            }
        }

        DrawingTool::Rectangle => {
            if canvas_response.drag_started() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    if let Some(si) = point_in_any_surface(nx, ny) {
                        state.selected_surfaces.clear();
                        state.selected_surfaces.insert(si);
                        state.moving_surface = Some((si, nx, ny));
                        state.tool = DrawingTool::Select;
                    } else {
                        state.rect_start = Some([nx, ny]);
                    }
                }
            }
            if canvas_response.drag_stopped() {
                if let Some(start) = state.rect_start.take() {
                    if let Some(pos) = canvas_response.interact_pointer_pos() {
                        let end = to_norm(pos);
                        let x0 = start[0].min(end[0]);
                        let y0 = start[1].min(end[1]);
                        let x1 = start[0].max(end[0]);
                        let y1 = start[1].max(end[1]);
                        if (x1 - x0) > 0.01 && (y1 - y0) > 0.01 {
                            let idx = data.surfaces.len() + 1;
                            actions.surface_actions.push(SurfaceAction::AddPolygon {
                                name: format!("Surface {}", idx),
                                vertices: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                                source: OutputSource::Master,
                            });
                        }
                    }
                }
            }
        }

        DrawingTool::Polygon => {
            if canvas_response.clicked() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let pt = to_norm(pos);

                    // If no polygon in progress and clicking inside existing surface, select it
                    let mut handled = false;
                    if state.polygon_verts.is_empty() {
                        if let Some(si) = point_in_any_surface(pt[0], pt[1]) {
                            state.selected_surfaces.clear();
                            state.selected_surfaces.insert(si);
                            state.tool = DrawingTool::Select;
                            handled = true;
                        }
                    }

                    // Check if clicking near first vertex to close
                    if !handled && state.polygon_verts.len() >= 3 {
                        let first = state.polygon_verts[0];
                        let dx = pt[0] - first[0];
                        let dy = pt[1] - first[1];
                        let close_threshold = 15.0 / canvas_width;
                        if (dx * dx + dy * dy).sqrt() < close_threshold {
                            // Close polygon
                            let idx = data.surfaces.len() + 1;
                            actions.surface_actions.push(SurfaceAction::AddPolygon {
                                name: format!("Surface {}", idx),
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
            }
            if canvas_response.double_clicked() {
                // Finish polygon on double-click
                if state.polygon_verts.len() >= 3 {
                    let idx = data.surfaces.len() + 1;
                    actions.surface_actions.push(SurfaceAction::AddPolygon {
                        name: format!("Surface {}", idx),
                        vertices: state.polygon_verts.clone(),
                        source: OutputSource::Master,
                    });
                }
                state.polygon_verts.clear();
            }
        }

        DrawingTool::Circle => {
            if canvas_response.drag_started() {
                if let Some(pos) = canvas_response.interact_pointer_pos() {
                    let [nx, ny] = to_norm(pos);
                    if let Some(si) = point_in_any_surface(nx, ny) {
                        state.selected_surfaces.clear();
                        state.selected_surfaces.insert(si);
                        state.moving_surface = Some((si, nx, ny));
                        state.tool = DrawingTool::Select;
                    } else {
                        state.circle_center = Some([nx, ny]);
                    }
                }
            }
            if canvas_response.drag_stopped() {
                if let Some(center) = state.circle_center.take() {
                    if let Some(pos) = canvas_response.interact_pointer_pos() {
                        let end = to_norm(pos);
                        let rx = (end[0] - center[0]).abs();
                        let ry = (end[1] - center[1]).abs();
                        let radius = (rx.max(ry)).max(0.02);
                        let sides = state.circle_sides.max(3);
                        let aspect_ratio = canvas_width / canvas_height;
                        let idx = data.surfaces.len() + 1;
                        actions.surface_actions.push(SurfaceAction::AddCircle {
                            name: format!("Surface {}", idx),
                            hint: CircleHint {
                                center,
                                radius,
                                sides,
                                aspect_ratio,
                            },
                            source: OutputSource::Master,
                        });
                    }
                }
            }
        }
    }

    // Keyboard shortcuts
    if ui.input(|i| i.key_pressed(egui::Key::S)) { state.tool = DrawingTool::Select; }
    if ui.input(|i| i.key_pressed(egui::Key::R)) { state.tool = DrawingTool::Rectangle; }
    if ui.input(|i| i.key_pressed(egui::Key::P)) { state.tool = DrawingTool::Polygon; }
    if ui.input(|i| i.key_pressed(egui::Key::C)) { state.tool = DrawingTool::Circle; }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.polygon_verts.clear();
        state.rect_start = None;
        state.circle_center = None;
    }
    // Duplicate & flip shortcuts (when any surfaces are selected)
    if !state.selected_surfaces.is_empty() {
        if ui.input(|i| i.key_pressed(egui::Key::D)) {
            for &idx in &state.selected_surfaces {
                actions.surface_actions.push(SurfaceAction::Duplicate { idx });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::H)) {
            for &idx in &state.selected_surfaces {
                actions.surface_actions.push(SurfaceAction::FlipHorizontal { idx });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::V)) {
            for &idx in &state.selected_surfaces {
                actions.surface_actions.push(SurfaceAction::FlipVertical { idx });
            }
        }
        if state.selected_surfaces.len() >= 2 && ui.input(|i| i.key_pressed(egui::Key::G)) {
            let indices: Vec<usize> = state.selected_surfaces.iter().copied().collect();
            actions.surface_actions.push(SurfaceAction::Combine { indices });
            state.selected_surfaces.clear();
        }
    }

    // Persist state
    ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
}

/// Render the output windows management section
fn render_output_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("📺 Outputs");

    // New output button
    ui.horizontal(|ui| {
        if ui.button("+ New Output").clicked() {
            actions.output_actions.push(OutputAction::Create);
        }
    });

    // List existing output windows
    if data.output_windows.is_empty() {
        ui.label(egui::RichText::new("No output windows").small().color(egui::Color32::GRAY));
    } else {
        for (idx, output) in data.output_windows.iter().enumerate() {
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(egui::Color32::from_rgb(30, 30, 45))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&output.name).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("Close output").clicked() {
                                actions.output_actions.push(OutputAction::Close { idx });
                            }
                        });
                    });

                    // Display target selector
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Display:").small());
                        egui::ComboBox::from_id_salt(format!("output_target_{}", idx))
                            .selected_text(egui::RichText::new(&output.target_label).small())
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                // Windowed option
                                if ui.selectable_label(!output.is_on_display, "Windowed").clicked() {
                                    actions.output_actions.push(OutputAction::SetTarget {
                                        idx,
                                        target: OutputTarget::Windowed,
                                    });
                                }
                                // Available display monitors
                                for monitor in &data.available_monitors {
                                    let label = format!("{} ({}x{})", monitor.name, monitor.width, monitor.height);
                                    if ui.selectable_label(false, &label).clicked() {
                                        actions.output_actions.push(OutputAction::SetTarget {
                                            idx,
                                            target: OutputTarget::Display {
                                                name: monitor.name.clone(),
                                                monitor_index: monitor.index,
                                            },
                                        });
                                    }
                                }
                            });
                    });

                    // Calibration toggle
                    ui.horizontal(|ui| {
                        let cal_label = if output.calibration_mode { "🔧 Done" } else { "🔧 Calibrate" };
                        if ui.button(egui::RichText::new(cal_label).small()).clicked() {
                            actions.output_actions.push(OutputAction::ToggleCalibration { idx });
                        }
                    });

                    // Surface assignments
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Surfaces:").small().strong());

                    // Assign surface dropdown
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(format!("assign_surf_{}", idx))
                            .selected_text("+ Assign Surface")
                            .width(140.0)
                            .show_ui(ui, |ui| {
                                for (si, surface) in data.surfaces.iter().enumerate() {
                                    let already_assigned = output.surface_assignments.iter()
                                        .any(|a| a.surface_idx == si);
                                    if !already_assigned {
                                        if ui.selectable_label(false, &surface.name).clicked() {
                                            actions.output_actions.push(OutputAction::AssignSurface {
                                                output_idx: idx,
                                                surface_idx: si,
                                            });
                                        }
                                    }
                                }
                            });
                    });

                    // List assigned surfaces with warp controls
                    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&assignment.surface_name).small());
                            if ui.small_button("↺").on_hover_text("Reset warp").clicked() {
                                actions.output_actions.push(OutputAction::ResetWarp {
                                    output_idx: idx,
                                    assignment_idx: ai,
                                });
                            }
                            if ui.small_button("✕").on_hover_text("Unassign").clicked() {
                                actions.output_actions.push(OutputAction::UnassignSurface {
                                    output_idx: idx,
                                    assignment_idx: ai,
                                });
                            }
                        });
                    }

                    // Warp calibration mini-canvas (when in calibration mode)
                    if output.calibration_mode && !output.surface_assignments.is_empty() {
                        render_warp_calibration(ui, idx, output, actions);
                    }
                });
            ui.add_space(4.0);
        }
    }
}

/// Render the warp calibration mini-canvas for an output.
/// Shows surface assignments as quads with draggable corner handles.
fn render_warp_calibration(
    ui: &mut egui::Ui,
    output_idx: usize,
    output: &super::OutputWindowUI,
    actions: &mut UIActions,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("⊞ Drag corners to warp").small().color(egui::Color32::YELLOW));

    let canvas_width = ui.available_width() - 4.0;
    let canvas_height = canvas_width * 0.5625; // 16:9
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Dark background
    painter.rect_filled(canvas_rect, 2.0, egui::Color32::from_rgb(15, 15, 25));

    // Convert normalized [0..1] to canvas pixel position
    let to_screen = |nx: f32, ny: f32| -> egui::Pos2 {
        egui::pos2(
            canvas_rect.left() + nx * canvas_width,
            canvas_rect.top() + ny * canvas_height,
        )
    };
    let from_screen = |pos: egui::Pos2| -> [f32; 2] {
        [
            ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0),
            ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0),
        ]
    };

    let corner_colors = [
        egui::Color32::from_rgb(255, 100, 100), // TL - red
        egui::Color32::from_rgb(100, 255, 100), // TR - green
        egui::Color32::from_rgb(100, 100, 255), // BR - blue
        egui::Color32::from_rgb(255, 255, 100), // BL - yellow
    ];
    let corner_labels = ["TL", "TR", "BR", "BL"];

    // State for dragging — store as Option<(usize, usize)> consistently
    let state_id = ui.id().with("warp_cal").with(output_idx);
    let mut dragging: Option<(usize, usize)> = ui.memory(|mem| {
        mem.data.get_temp::<Option<(usize, usize)>>(state_id).flatten()
    });

    // Draw each assigned surface's warp quad and handles
    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        let corners = assignment.warp_corners;

        // Draw the warp quad outline
        let screen_corners: Vec<egui::Pos2> = corners.iter()
            .map(|c| to_screen(c[0], c[1]))
            .collect();
        for i in 0..4 {
            let j = (i + 1) % 4;
            painter.line_segment(
                [screen_corners[i], screen_corners[j]],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(200, 200, 200)),
            );
        }

        // Draw surface name at center
        let cx = corners.iter().map(|c| c[0]).sum::<f32>() / 4.0;
        let cy = corners.iter().map(|c| c[1]).sum::<f32>() / 4.0;
        painter.text(
            to_screen(cx, cy),
            egui::Align2::CENTER_CENTER,
            &assignment.surface_name,
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );

        // Draw corner handles
        let handle_radius = 8.0;
        for (ci, corner) in corners.iter().enumerate() {
            let pos = to_screen(corner[0], corner[1]);
            let is_dragging = dragging == Some((ai, ci));
            let r = if is_dragging { handle_radius + 2.0 } else { handle_radius };
            painter.circle_filled(pos, r, corner_colors[ci]);
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                corner_labels[ci],
                egui::FontId::proportional(9.0),
                egui::Color32::BLACK,
            );
        }
    }

    // Handle dragging
    if canvas_response.drag_started() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            // Find nearest corner
            let mut best: Option<(usize, usize, f32)> = None;
            for (ai, assignment) in output.surface_assignments.iter().enumerate() {
                for (ci, corner) in assignment.warp_corners.iter().enumerate() {
                    let screen_pos = to_screen(corner[0], corner[1]);
                    let dist = pos.distance(screen_pos);
                    if dist < 20.0 {
                        if best.is_none() || dist < best.unwrap().2 {
                            best = Some((ai, ci, dist));
                        }
                    }
                }
            }
            if let Some((ai, ci, _)) = best {
                dragging = Some((ai, ci));
            }
        }
    }

    if let Some((ai, ci)) = dragging {
        if canvas_response.dragged() {
            if let Some(pos) = canvas_response.interact_pointer_pos() {
                let new_pos = from_screen(pos);
                actions.output_actions.push(OutputAction::SetWarpCorner {
                    output_idx,
                    assignment_idx: ai,
                    corner_idx: ci,
                    position: new_pos,
                });
            }
        }
    }

    if canvas_response.drag_stopped() {
        dragging = None;
    }

    ui.memory_mut(|mem| mem.data.insert_temp(state_id, dragging));
}

/// Render the bottom panel (crossfader strip + audio, modulation, shader browser)
fn render_bottom_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // MIDI learn status indicator
    if data.midi_learn_active {
        egui::Frame::default()
            .inner_margin(4.0)
            .corner_radius(4.0)
            .fill(egui::Color32::from_rgb(180, 80, 220))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(target) = &data.midi_learn_target {
                        ui.label(egui::RichText::new(format!("🎹 MIDI LEARN — Move a control to map: {}", target))
                            .strong().color(egui::Color32::WHITE));
                    } else {
                        ui.label(egui::RichText::new("🎹 MIDI LEARN — Click a parameter to select it")
                            .strong().color(egui::Color32::WHITE));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕ Exit MIDI Learn").clicked() {
                            actions.midi_learn_toggle = true;
                        }
                    });
                });
            });
    }

    // Context-sensitive bottom bar: master effects, channel effects, or deck detail
    if data.selected_master {
        render_master_effect_detail(ui, data, actions);
    } else if let Some(ch_idx) = data.selected_channel {
        render_channel_effect_detail(ui, ch_idx, data, actions);
    } else {
        render_selected_deck_detail(ui, data, actions);
    }
}

/// Render the selected deck's full details (params, effects, blend, scaling) in the bottom bar
fn render_selected_deck_detail(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎛 Selected Deck");

    let Some((ch_idx, deck_idx)) = data.selected_deck else {
        ui.label(egui::RichText::new("Click a deck thumbnail to see its controls here").weak().small());
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
    ui.label(egui::RichText::new(format!("Ch {} / Deck {} — {}", ch.name, deck_idx + 1, deck.name))
        .strong().color(accent));

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

            // Column 1: Generator parameters + blend/scale
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(ui.visuals().faint_bg_color)
                .show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.set_max_width(280.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    egui::ScrollArea::vertical().id_salt("deck_gen_scroll").show(ui, |ui| {
                        // Blend mode
                        let blend_modes = ["Norm", "Add", "Mult", "Scrn", "Ovly", "Diff"];
                        let current_blend = match deck.blend_mode {
                            BlendMode::Normal => 0, BlendMode::Add => 1, BlendMode::Multiply => 2,
                            BlendMode::Screen => 3, BlendMode::Overlay => 4, BlendMode::Difference => 5,
                        };
                        let mut selected = current_blend;
                        ui.horizontal(|ui| {
                            ui.label("Blend:");
                            egui::ComboBox::from_id_salt("sel_deck_blend")
                                .selected_text(blend_modes[selected])
                                .width(60.0)
                                .show_ui(ui, |ui| {
                                    for (i, mode_name) in blend_modes.iter().enumerate() {
                                        ui.selectable_value(&mut selected, i, *mode_name);
                                    }
                                });
                        });
                        if selected != current_blend {
                            let new_blend = match selected {
                                1 => BlendMode::Add, 2 => BlendMode::Multiply, 3 => BlendMode::Screen,
                                4 => BlendMode::Overlay, 5 => BlendMode::Difference, _ => BlendMode::Normal,
                            };
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
                                egui::ComboBox::from_id_salt("sel_deck_scale")
                                    .selected_text(scaling_modes[selected_scaling])
                                    .width(60.0)
                                    .show_ui(ui, |ui| {
                                        for (i, mode_name) in scaling_modes.iter().enumerate() {
                                            ui.selectable_value(&mut selected_scaling, i, *mode_name);
                                        }
                                    });
                            });
                            if selected_scaling != current_idx {
                                let new_scaling = match selected_scaling {
                                    1 => ScalingMode::Fit, 2 => ScalingMode::Stretch,
                                    3 => ScalingMode::Center, _ => ScalingMode::Fill,
                                };
                                actions.scaling_mode_updates.push((ch_idx, deck_idx, new_scaling));
                            }
                        }

                        // Generator parameters
                        let gen_params = &deck.generator;
                        if !gen_params.params.is_empty() {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("🎨 {}", gen_params.shader_name)).strong());
                            let midi_path_prefix = format!("ch/{}/deck/{}", ch_idx, deck_idx);
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
                                Some(&|name: &str, src_idx: usize| ModulationAction::AssignModulation {
                                    ch_idx, deck_idx, param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                }),
                                Some(&|name: &str| ModulationAction::RemoveAssignment {
                                    ch_idx, deck_idx, param_name: name.to_string(), source_idx: 0,
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
                                &format!("ch{}_deck{}", ch_idx, deck_idx),
                            );
                            ui.add_space(4.0);
                            if ui.button("🔄 Reset").clicked() {
                                actions.param_updates.push(ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx });
                            }
                        }
                    });
                    });
                });

            ui.separator();

            // Effect chain: drag-and-drop reordering + library drops
            {
                for (eff_idx, (eff_name, eff_enabled, eff_params)) in deck.effects.iter().enumerate() {
                    // Drop zone before this effect (for reordering)
                    render_effect_drop_zone(ui, &format!("deck_{}_{}", ch_idx, deck_idx), eff_idx);

                    // Effect card
                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            egui::ScrollArea::vertical().id_salt(format!("deck_fx_scroll_{}_{}_{}",ch_idx,deck_idx,eff_idx)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let grip = ui.add(egui::Label::new(egui::RichText::new("⠿").weak().size(14.0)).sense(egui::Sense::drag()));
                                    if grip.drag_started() {
                                        egui::DragAndDrop::set_payload(ui.ctx(), EffectDrag::Deck(ch_idx, deck_idx, eff_idx));
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), EffectDrag::Deck(ch_idx, deck_idx, eff_idx));
                                        });
                                    } else if grip.dragged() {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), EffectDrag::Deck(ch_idx, deck_idx, eff_idx));
                                        });
                                    }
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.effect_to_toggle = Some((ch_idx, deck_idx, eff_idx));
                                    }
                                    ui.label(egui::RichText::new(format!("🔮 {}", eff_name)).strong());
                                    if ui.small_button("✕").clicked() {
                                        actions.effect_to_remove = Some((ch_idx, deck_idx, eff_idx));
                                    }
                                });

                                if !eff_params.params.is_empty() {
                                    let ch_copy = ch_idx;
                                    let deck_copy = deck_idx;
                                    let eff_idx_copy = eff_idx;
                                    let eff_midi_prefix = format!("ch/{}/deck/{}/effect/{}", ch_copy, deck_copy, eff_idx_copy);
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
                                        Some(&|name: &str, src_idx: usize| ModulationAction::AssignEffectModulation {
                                            ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy,
                                            param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveEffectAssignment {
                                            ch_idx: ch_copy, deck_idx: deck_copy, effect_idx: eff_idx_copy,
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
                                        &format!("ch{}_deck{}_fx{}", ch_copy, deck_copy, eff_idx_copy),
                                    );
                                }
                            });
                            });
                        });
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
                let stroke = if has_fx_drag { egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
                let fill = if has_fx_drag { egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20) } else { egui::Color32::TRANSPARENT };
                egui::Frame::default()
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(remaining_w - 16.0, remaining_h - 16.0));
                        if deck.effects.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                            });
                        }
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

/// Render master effect chain detail in the bottom bar (when master is selected)
fn render_master_effect_detail(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎬 Master Effects");

    egui::ScrollArea::horizontal().id_salt("master_fx_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            {
                for (eff_idx, (eff_name, eff_enabled, eff_params)) in data.master_effect_info.iter().enumerate() {
                    render_effect_drop_zone(ui, "master", eff_idx);

                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            egui::ScrollArea::vertical().id_salt(format!("master_fx_scroll_{}", eff_idx)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let grip = ui.add(egui::Label::new(egui::RichText::new("⠿").weak().size(14.0)).sense(egui::Sense::drag()));
                                    if grip.drag_started() {
                                        egui::DragAndDrop::set_payload(ui.ctx(), EffectDrag::Master(eff_idx));
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), EffectDrag::Master(eff_idx));
                                        });
                                    } else if grip.dragged() {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), EffectDrag::Master(eff_idx));
                                        });
                                    }
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.master_effect_to_toggle = Some(eff_idx);
                                    }
                                    ui.label(egui::RichText::new(format!("🔮 {}", eff_name)).strong());
                                    if ui.small_button("✕").clicked() {
                                        actions.master_effect_to_remove = Some(eff_idx);
                                    }
                                });

                                if !eff_params.params.is_empty() {
                                    let eff_idx_copy = eff_idx;
                                    let midi_prefix = format!("master/effect/{}", eff_idx_copy);
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| match val {
                                            ParamValue::Float(v) => ParamUpdate::MasterEffectFloat { effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Bool(v) => ParamUpdate::MasterEffectBool { effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Color(v) => ParamUpdate::MasterEffectColor { effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            _ => unreachable!(),
                                        },
                                        Some(&|name: &str, src_idx: usize| ModulationAction::AssignMasterEffectModulation {
                                            effect_idx: eff_idx_copy,
                                            param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveMasterEffectAssignment {
                                            effect_idx: eff_idx_copy,
                                            param_name: name.to_string(),
                                        }),
                                        &mut actions.param_updates,
                                        &mut actions.modulation_actions,
                                        &format!("master_fx_{}", eff_idx_copy),
                                        Some(&midi_prefix),
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("master_fx{}", eff_idx_copy),
                                    );
                                }
                            });
                            });
                        });
                    ui.separator();
                }

                // Drop zone after last effect (for reordering)
                if !data.master_effect_info.is_empty() {
                    let num_effects = data.master_effect_info.len();
                    render_effect_drop_zone(ui, "master", num_effects);
                }

                // Remaining space: always present drop target
                let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                    .map(|p| matches!(&*p, LibraryDrag::Effect(_))).unwrap_or(false);
                let remaining_w = ui.available_width().max(80.0);
                let remaining_h = ui.available_height().max(40.0);
                let stroke = if has_fx_drag { egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
                let fill = if has_fx_drag { egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20) } else { egui::Color32::TRANSPARENT };
                egui::Frame::default()
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(remaining_w - 16.0, remaining_h - 16.0));
                        if data.master_effect_info.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                            });
                        }
                    });
            }

            // Store master effect chain rect for deferred library drops
            let chain_rect = ui.min_rect();
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("master_fx_drop_rect"), chain_rect);
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with("master".to_string()), data.master_effect_info.len() + 1);
            });
        });
    });
}


/// Render channel effect chain detail in the bottom bar
fn render_channel_effect_detail(ui: &mut egui::Ui, ch_idx: usize, data: &UIData, actions: &mut UIActions) {
    let Some(ch) = data.channels.get(ch_idx) else {
        ui.label(egui::RichText::new("Channel not found").weak());
        return;
    };

    let accent = channel_color(ch_idx);
    ui.heading(egui::RichText::new(format!("🔮 Channel {} Effects", ch.name)).color(accent));

    egui::ScrollArea::horizontal().id_salt("channel_fx_hscroll").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            {
                for (eff_idx, (eff_name, eff_enabled, eff_params)) in ch.effects.iter().enumerate() {
                    render_effect_drop_zone(ui, &format!("ch_{}", ch_idx), eff_idx);

                    egui::Frame::default()
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .fill(ui.visuals().faint_bg_color)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(250.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            egui::ScrollArea::vertical().id_salt(format!("ch_fx_scroll_{}_{}", ch_idx, eff_idx)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let grip = ui.add(egui::Label::new(egui::RichText::new("⠿").weak().size(14.0)).sense(egui::Sense::drag()));
                                    if grip.drag_started() {
                                        egui::DragAndDrop::set_payload(ui.ctx(), EffectDrag::Channel(ch_idx, eff_idx));
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), EffectDrag::Channel(ch_idx, eff_idx));
                                        });
                                    } else if grip.dragged() {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), EffectDrag::Channel(ch_idx, eff_idx));
                                        });
                                    }
                                    let mut enabled = *eff_enabled;
                                    if ui.checkbox(&mut enabled, "").changed() {
                                        actions.ch_effect_to_toggle = Some((ch_idx, eff_idx));
                                    }
                                    ui.label(egui::RichText::new(format!("🔮 {}", eff_name)).strong().color(accent));
                                    if ui.small_button("✕").clicked() {
                                        actions.ch_effect_to_remove = Some((ch_idx, eff_idx));
                                    }
                                });

                                if !eff_params.params.is_empty() {
                                    let ch_copy = ch_idx;
                                    let eff_idx_copy = eff_idx;
                                    let midi_prefix = format!("ch/{}/effect/{}", ch_copy, eff_idx_copy);
                                    widgets::render_effect_params(
                                        ui,
                                        &eff_params.params,
                                        &data.modulation_sources,
                                        &|name: &str, val: ParamValue| match val {
                                            ParamValue::Float(v) => ParamUpdate::ChannelEffectFloat { ch_idx: ch_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Bool(v) => ParamUpdate::ChannelEffectBool { ch_idx: ch_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            ParamValue::Color(v) => ParamUpdate::ChannelEffectColor { ch_idx: ch_copy, effect_idx: eff_idx_copy, name: name.to_string(), value: v },
                                            _ => unreachable!(),
                                        },
                                        Some(&|name: &str, src_idx: usize| ModulationAction::AssignChannelEffectModulation {
                                            ch_idx: ch_copy, effect_idx: eff_idx_copy,
                                            param_name: name.to_string(), source_idx: src_idx, amount: 0.5,
                                        }),
                                        Some(&|name: &str| ModulationAction::RemoveChannelEffectAssignment {
                                            ch_idx: ch_copy, effect_idx: eff_idx_copy,
                                            param_name: name.to_string(),
                                        }),
                                        &mut actions.param_updates,
                                        &mut actions.modulation_actions,
                                        &format!("ch_fx_{}_{}", ch_copy, eff_idx_copy),
                                        Some(&midi_prefix),
                                        data.midi_learn_active,
                                        &mut actions.midi_learn_select,
                                        data.midi_learn_target.as_deref(),
                                        &data.modulation_assignments,
                                        &data.modulation_current_values,
                                        &format!("ch{}_fx{}", ch_copy, eff_idx_copy),
                                    );
                                }
                            });
                            });
                        });
                    ui.separator();
                }

                // Drop zone after last effect (for reordering)
                if !ch.effects.is_empty() {
                    let num_effects = ch.effects.len();
                    render_effect_drop_zone(ui, &format!("ch_{}", ch_idx), num_effects);
                }

                // Remaining space: always present drop target
                let has_fx_drag = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx())
                    .map(|p| matches!(&*p, LibraryDrag::Effect(_))).unwrap_or(false);
                let remaining_w = ui.available_width().max(80.0);
                let remaining_h = ui.available_height().max(40.0);
                let stroke = if has_fx_drag { egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)) } else { egui::Stroke::NONE };
                let fill = if has_fx_drag { egui::Color32::from_rgba_unmultiplied(100, 200, 255, 20) } else { egui::Color32::TRANSPARENT };
                egui::Frame::default()
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(remaining_w - 16.0, remaining_h - 16.0));
                        if ch.effects.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(egui::RichText::new("🔮 Drag effects here").weak());
                            });
                        }
                    });
            }

            // Store channel effect chain rect for deferred library drops
            let chain_rect = ui.min_rect();
            let ch_chain_key = format!("ch_{}", ch_idx);
            ui.ctx().memory_mut(|mem| {
                mem.data.insert_temp(egui::Id::new("ch_fx_drop_rect").with(ch_idx), chain_rect);
                mem.data.insert_temp(egui::Id::new("eff_dz_count").with(ch_chain_key), ch.effects.len() + 1);
            });
        });
    });
}

/// Render modulation sources section
fn render_modulation_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎛 Modulation");

    // Audio levels (integrated into modulation)
    ui.collapsing("🎵 Audio Input", |ui| {
        widgets::render_audio_levels(ui, &data.audio);
    });
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        if ui.button("➕ LFO").clicked() {
            actions.modulation_actions.push(ModulationAction::AddLFO {
                waveform: LFOWaveform::Sine,
                frequency: 1.0,
            });
        }
        if ui.button("➕ Audio").clicked() {
            actions.modulation_actions.push(ModulationAction::AddAudioBand {
                band: crate::modulation::AudioBand::Bass,
            });
        }
        if ui.button("➕ ADSR").clicked() {
            actions.modulation_actions.push(ModulationAction::AddADSR {
                attack: 0.1, decay: 0.3, sustain: 0.7, release: 0.5,
            });
        }
        if ui.button("➕ StepSeq").clicked() {
            actions.modulation_actions.push(ModulationAction::AddStepSequencer {
                num_steps: 8, rate: 2.0,
            });
        }
    });


    if data.modulation_sources.is_empty() {
        ui.label(egui::RichText::new("No modulation sources").small().weak());
    } else {
        egui::ScrollArea::horizontal().id_salt("mod_sources_hscroll").max_height(220.0).show(ui, |ui| {
            ui.horizontal_top(|ui| {
            for (idx, src) in data.modulation_sources.iter().enumerate() {
                let mod_color = modulator_color(idx);
                let dim_color = egui::Color32::from_rgba_premultiplied(
                    mod_color.r() / 4, mod_color.g() / 4, mod_color.b() / 4, 40
                );
                egui::Frame::default()
                    .inner_margin(4.0)
                    .corner_radius(4.0)
                    .fill(dim_color)
                    .stroke(egui::Stroke::new(1.0, mod_color))
                    .show(ui, |ui| {
                        ui.set_min_width(140.0);
                        ui.set_max_width(190.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        match src {
                            ModSourceUI::LFO { waveform, frequency, phase, amplitude, bipolar } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("LFO {}", idx + 1)).strong().color(mod_color));
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                let waveforms = ["Sine", "Square", "Triangle", "Saw", "Random"];
                                let current_wf = match waveform {
                                    LFOWaveform::Sine => 0,
                                    LFOWaveform::Square => 1,
                                    LFOWaveform::Triangle => 2,
                                    LFOWaveform::Sawtooth => 3,
                                    LFOWaveform::Random => 4,
                                };
                                let mut selected_wf = current_wf;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Wave:").small());
                                    egui::ComboBox::from_id_salt(format!("wf_{}", idx))
                                        .selected_text(waveforms[selected_wf])
                                        .width(70.0)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in waveforms.iter().enumerate() {
                                                if ui.selectable_value(&mut selected_wf, i, *name).changed() {
                                                    let new_wf = match i {
                                                        0 => LFOWaveform::Sine,
                                                        1 => LFOWaveform::Square,
                                                        2 => LFOWaveform::Triangle,
                                                        3 => LFOWaveform::Sawtooth,
                                                        _ => LFOWaveform::Random,
                                                    };
                                                    actions.modulation_actions.push(ModulationAction::UpdateLFOWaveform { idx, waveform: new_wf });
                                                }
                                            }
                                        });
                                });
                                let mut freq = *frequency;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Freq:").small());
                                    let path = format!("mod/{}/frequency", idx);
                                    if render_mod_learn_slider(ui, &mut freq, 0.01..=10.0, |s| s.logarithmic(true).show_value(true).suffix("Hz"), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOFrequency { idx, frequency: freq });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "frequency");
                                });
                                let mut amp = *amplitude;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Amp:").small());
                                    let path = format!("mod/{}/amplitude", idx);
                                    if render_mod_learn_slider(ui, &mut amp, 0.0..=1.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOAmplitude { idx, amplitude: amp });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "amplitude");
                                });
                                let mut ph = *phase;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Phase:").small());
                                    let path = format!("mod/{}/phase", idx);
                                    if render_mod_learn_slider(ui, &mut ph, 0.0..=1.0, |s| s.show_value(false), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOPhase { idx, phase: ph });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "phase");
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar (-1 to 1)").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateLFOBipolar { idx, bipolar: bp });
                                }
                                // LFO waveform visualization
                                let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(180.0), 30.0), egui::Sense::hover());
                                let rect = response.rect;
                                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                                let n_points = 60;
                                let points: Vec<egui::Pos2> = (0..=n_points).map(|i| {
                                    let t = i as f32 / n_points as f32;
                                    let raw = match waveform {
                                        LFOWaveform::Sine => (t * std::f32::consts::TAU).sin(),
                                        LFOWaveform::Square => if t < 0.5 { 1.0 } else { -1.0 },
                                        LFOWaveform::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                                        LFOWaveform::Sawtooth => 2.0 * t - 1.0,
                                        LFOWaveform::Random => {
                                            let step = (t * 8.0).floor();
                                            let seed = step as u32;
                                            let hash = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                                            (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
                                        }
                                    };
                                    let y = rect.center().y - raw * *amplitude * rect.height() * 0.4;
                                    egui::pos2(rect.left() + t * rect.width(), y)
                                }).collect();
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    let y = rect.center().y - cur_val * rect.height() * 0.4;
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::Audio { band, smoothing } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("Audio {}", idx + 1)).strong().color(mod_color));
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                let band_name = match band {
                                    crate::modulation::AudioBand::Level => "Level",
                                    crate::modulation::AudioBand::Bass => "Bass",
                                    crate::modulation::AudioBand::Mid => "Mid",
                                    crate::modulation::AudioBand::Treble => "Treble",
                                };
                                ui.label(egui::RichText::new(format!("Band: {}", band_name)).small());
                                let mut sm = *smoothing;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Smooth:").small());
                                    let path = format!("mod/{}/smoothing", idx);
                                    if render_mod_learn_slider(ui, &mut sm, 0.0..=0.99, |s| s.show_value(false), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioSmoothing { idx, smoothing: sm });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "smoothing");
                                });
                                // Audio level bar
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    ui.add(egui::ProgressBar::new(cur_val).desired_width(140.0).fill(mod_color));
                                }
                            }
                            ModSourceUI::ADSR { attack, decay, sustain, release, stage } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("ADSR {}", idx + 1)).strong().color(mod_color));
                                    let stage_text = match stage {
                                        crate::modulation::ADSRStage::Idle => "⏹",
                                        crate::modulation::ADSRStage::Attack => "▲",
                                        crate::modulation::ADSRStage::Decay => "▼",
                                        crate::modulation::ADSRStage::Sustain => "━",
                                        crate::modulation::ADSRStage::Release => "↘",
                                    };
                                    ui.label(egui::RichText::new(stage_text).small());
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                ui.horizontal(|ui| {
                                    if ui.button(egui::RichText::new("▶ Gate").small()).clicked() {
                                        actions.modulation_actions.push(ModulationAction::TriggerADSR { idx });
                                    }
                                    if ui.button(egui::RichText::new("⏹ Release").small()).clicked() {
                                        actions.modulation_actions.push(ModulationAction::ReleaseADSR { idx });
                                    }
                                });
                                let mut a = *attack;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("A:").small());
                                    let path = format!("mod/{}/attack", idx);
                                    if render_mod_learn_slider(ui, &mut a, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRAttack { idx, attack: a });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "attack");
                                });
                                let mut d = *decay;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("D:").small());
                                    let path = format!("mod/{}/decay", idx);
                                    if render_mod_learn_slider(ui, &mut d, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRDecay { idx, decay: d });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "decay");
                                });
                                let mut s = *sustain;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("S:").small());
                                    let path = format!("mod/{}/sustain", idx);
                                    if render_mod_learn_slider(ui, &mut s, 0.0..=1.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRSustain { idx, sustain: s });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "sustain");
                                });
                                let mut r = *release;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("R:").small());
                                    let path = format!("mod/{}/release", idx);
                                    if render_mod_learn_slider(ui, &mut r, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRRelease { idx, release: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "release");
                                });
                                // ADSR envelope visualization
                                let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(180.0), 30.0), egui::Sense::hover());
                                let rect = response.rect;
                                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                                let total_time = a + d + 0.3 + r; // sustain shown as ~0.3 segment
                                let ax = rect.left() + (a / total_time) * rect.width();
                                let dx = ax + (d / total_time) * rect.width();
                                let sx = dx + (0.3 / total_time) * rect.width();
                                let rx = sx + (r / total_time) * rect.width();
                                let top = rect.top() + 2.0;
                                let bot = rect.bottom() - 2.0;
                                let sus_y = top + (1.0 - s) * (bot - top);
                                let points = vec![
                                    egui::pos2(rect.left(), bot),  // start at 0
                                    egui::pos2(ax, top),           // attack peak
                                    egui::pos2(dx, sus_y),         // decay to sustain
                                    egui::pos2(sx, sus_y),         // sustain hold
                                    egui::pos2(rx, bot),           // release to 0
                                ];
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    let y = bot - cur_val * (bot - top);
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::StepSequencer { steps, rate, interpolation, bipolar } => {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("StepSeq {}", idx + 1)).strong().color(mod_color));
                                    if ui.small_button("✕").clicked() {
                                        actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                                    }
                                });
                                let mut r = *rate;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Rate:").small());
                                    let path = format!("mod/{}/rate", idx);
                                    if render_mod_learn_slider(ui, &mut r, 0.1..=20.0, |s| s.logarithmic(true).suffix("Hz").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateStepRate { idx, rate: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "rate");
                                });
                                // Interpolation mode
                                let interp_names = ["None", "Linear", "Smooth"];
                                let current_interp = match interpolation {
                                    StepInterpolation::None => 0,
                                    StepInterpolation::Linear => 1,
                                    StepInterpolation::Smooth => 2,
                                };
                                let mut selected_interp = current_interp;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Interp:").small());
                                    egui::ComboBox::from_id_salt(format!("step_interp_{}", idx))
                                        .selected_text(interp_names[selected_interp])
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in interp_names.iter().enumerate() {
                                                if ui.selectable_value(&mut selected_interp, i, *name).changed() {
                                                    let new_interp = match i {
                                                        0 => StepInterpolation::None,
                                                        1 => StepInterpolation::Linear,
                                                        _ => StepInterpolation::Smooth,
                                                    };
                                                    actions.modulation_actions.push(ModulationAction::UpdateStepInterpolation { idx, interpolation: new_interp });
                                                }
                                            }
                                        });
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateStepBipolar { idx, bipolar: bp });
                                }
                                // Step value sliders (compact)
                                ui.horizontal_wrapped(|ui| {
                                    for (step_idx, step_val) in steps.iter().enumerate() {
                                        let mut val = *step_val;
                                        let slider = egui::Slider::new(&mut val, 0.0..=1.0)
                                            .vertical()
                                            .show_value(false);
                                        if data.midi_learn_active {
                                            let inner = ui.scope(|ui| {
                                                ui.disable();
                                                ui.add_sized([12.0, 30.0], slider)
                                            });
                                            let step_path = format!("mod/{}/step/{}", idx, step_idx);
                                            let is_target = data.midi_learn_target.as_deref() == Some(step_path.as_str());
                                            if is_target {
                                                widgets::draw_midi_learn_selected(ui, inner.inner.rect);
                                            } else {
                                                widgets::draw_midi_learn_glow(ui, inner.inner.rect);
                                            }
                                            let click_id = ui.id().with(("midi_learn_step", idx, step_idx));
                                            if ui.interact(inner.inner.rect, click_id, egui::Sense::click()).clicked() {
                                                actions.midi_learn_select = Some(step_path);
                                            }
                                        } else {
                                            if ui.add_sized([12.0, 30.0], slider).on_hover_text(format!("Step {}", step_idx + 1)).changed() {
                                                actions.modulation_actions.push(ModulationAction::UpdateStepValue { idx, step_idx, value: val });
                                            }
                                        }
                                    }
                                });
                            }
                        }
                        });
                    });
                ui.separator();
            }
            });
        });
    }
}

/// Render a modulation source slider with MIDI learn support.
/// Returns true if the slider value changed (only in non-learn mode).
fn render_mod_learn_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    slider_opts: impl FnOnce(egui::Slider<'_>) -> egui::Slider<'_>,
    midi_path: &str,
    data: &UIData,
    actions: &mut UIActions,
) -> bool {
    let mut changed = false;
    if data.midi_learn_active {
        let inner = ui.scope(|ui| {
            ui.disable();
            ui.add(slider_opts(egui::Slider::new(value, range)))
        });
        let slider_rect = inner.inner.rect;
        let is_target = data.midi_learn_target.as_deref() == Some(midi_path);
        if is_target {
            widgets::draw_midi_learn_selected(ui, slider_rect);
        } else {
            widgets::draw_midi_learn_glow(ui, slider_rect);
        }
        let click_id = ui.id().with(("midi_learn_mod", midi_path));
        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
        if click_resp.clicked() {
            actions.midi_learn_select = Some(midi_path.to_string());
        }
    } else {
        if ui.add(slider_opts(egui::Slider::new(value, range))).changed() {
            changed = true;
        }
    }
    changed
}

/// Render a mod-on-mod assignment dropdown for a modulator's parameter.
/// `target_idx` is the modulator whose parameter is being targeted.
/// `param_name` is the parameter name (e.g., "frequency", "amplitude", "phase").
/// Shows a 🎛 combo listing all other modulators that can modulate this parameter.
fn render_mod_on_mod_dropdown(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    target_idx: usize,
    param_name: &str,
) {
    let key = format!("mod:{}:{}", target_idx, param_name);
    let has_assignment = data.modulation_assignments.get(&key).map_or(false, |v| !v.is_empty());
    let btn_text = if has_assignment { "🎛" } else { "🎛" };
    let btn_color = if has_assignment {
        modulator_color(data.modulation_assignments.get(&key)
            .and_then(|v| v.first())
            .map(|a| a.source_idx)
            .unwrap_or(0))
    } else {
        egui::Color32::GRAY
    };

    egui::ComboBox::from_id_salt(format!("mom_{}_{}", target_idx, param_name))
        .selected_text(egui::RichText::new(btn_text).color(btn_color).small())
        .width(30.0)
        .show_ui(ui, |ui| {
            ui.label(egui::RichText::new(format!("Modulate {}", param_name)).small().strong());
            for (src_idx, src) in data.modulation_sources.iter().enumerate() {
                if src_idx == target_idx { continue; } // can't modulate yourself
                let color = modulator_color(src_idx);
                let src_name = match src {
                    ModSourceUI::LFO { .. } => format!("LFO {}", src_idx + 1),
                    ModSourceUI::Audio { band, .. } => format!("Audio {:?}", band),
                    ModSourceUI::ADSR { .. } => format!("ADSR {}", src_idx + 1),
                    ModSourceUI::StepSequencer { .. } => format!("StepSeq {}", src_idx + 1),
                };
                if ui.button(egui::RichText::new(format!("+ {}", src_name)).color(color).small()).clicked() {
                    actions.modulation_actions.push(ModulationAction::AssignModOnMod {
                        target_source_idx: target_idx,
                        param_name: param_name.to_string(),
                        modulator_idx: src_idx,
                        amount: 1.0,
                    });
                }
            }
            if has_assignment {
                ui.separator();
                if ui.button(egui::RichText::new("✕ Remove").small().color(egui::Color32::from_rgb(255, 100, 100))).clicked() {
                    actions.modulation_actions.push(ModulationAction::RemoveModOnMod {
                        target_source_idx: target_idx,
                        param_name: param_name.to_string(),
                    });
                }
            }
        });
}

/// Render MIDI devices and mappings section
fn render_midi_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.heading("🎹 MIDI");

    ui.horizontal(|ui| {
        ui.label(format!("{} device(s)", data.midi_devices.len()));
        if ui.button("🔄 Rescan").clicked() {
            actions.midi_rescan = true;
        }
    });

    // Device list
    if !data.midi_devices.is_empty() {
        ui.collapsing(format!("Devices ({})", data.midi_devices.len()), |ui| {
            for dev in &data.midi_devices {
                ui.horizontal(|ui| {
                    let mut enabled = dev.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        actions.midi_device_toggles.push((dev.id, enabled));
                    }
                    let status = if dev.enabled { "●" } else { "○" };
                    let color = if dev.enabled {
                        egui::Color32::from_rgb(100, 255, 100)
                    } else {
                        egui::Color32::GRAY
                    };
                    ui.label(egui::RichText::new(status).color(color));
                    ui.label(&dev.name);
                    if dev.has_output {
                        ui.label(egui::RichText::new("⇄").small().color(egui::Color32::from_rgb(100, 200, 255)))
                            .on_hover_text("Has output (LED feedback)");
                    }
                    ui.label(egui::RichText::new(&dev.profile).small().weak());
                });
            }
        });
    }

    // Mappings list
    if !data.midi_mappings.is_empty() {
        ui.collapsing(format!("Mappings ({})", data.midi_mappings.len()), |ui| {
            ui.horizontal(|ui| {
                if ui.button("🗑 Clear All").clicked() {
                    actions.midi_clear_mappings = true;
                }
            });
            egui::ScrollArea::vertical().max_height(150.0).id_salt("midi_mappings_scroll").show(ui, |ui| {
                for mapping in &data.midi_mappings {
                    ui.horizontal(|ui| {
                        if ui.small_button("✕").clicked() {
                            actions.midi_remove_mapping.push(mapping.key);
                        }
                        ui.label(egui::RichText::new(&mapping.device_name).small().color(egui::Color32::from_rgb(180, 180, 255)));
                        ui.label(egui::RichText::new(&mapping.key_display).small().strong());
                        ui.label(egui::RichText::new("→").small());
                        ui.label(egui::RichText::new(&mapping.param_path).small().color(egui::Color32::from_rgb(255, 200, 100)));
                    });
                }
            });
        });
    } else {
        ui.label(egui::RichText::new("No mappings. Right-click anywhere → Enter MIDI Learn.").small().weak());
    }
}


/// Render a thin drop zone for effect reordering within a chain.
/// Stores the zone rect in temp memory for deferred drop detection.
/// `chain_key` identifies the chain (e.g. "deck_0_1", "ch_0", "master").
/// `position` is the insert index in the chain.
fn render_effect_drop_zone(ui: &mut egui::Ui, chain_key: &str, position: usize) {
    let dz = ui.allocate_response(egui::vec2(8.0, ui.available_height().max(40.0)), egui::Sense::hover());
    let has_drag = egui::DragAndDrop::has_payload_of_type::<EffectDrag>(ui.ctx());
    // Store rect for deferred handler to find
    let key = egui::Id::new("eff_dz_rect").with((chain_key.to_string(), position));
    ui.ctx().memory_mut(|mem| {
        mem.data.insert_temp(key, dz.rect);
    });
    // Visual highlight: check if pointer is actually over this zone
    if has_drag {
        if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
            if dz.rect.contains(pos) {
                ui.painter().rect_filled(dz.rect, 2.0, egui::Color32::from_rgb(100, 200, 255));
            }
        }
    }
}

/// Channel accent colors
fn channel_color(ch_idx: usize) -> egui::Color32 {
    match ch_idx {
        0 => egui::Color32::from_rgb(160, 100, 255), // Purple — Ch 1
        1 => egui::Color32::from_rgb(100, 160, 255), // Blue — Ch 2
        2 => egui::Color32::from_rgb(255, 160, 60),  // Orange
        3 => egui::Color32::from_rgb(80, 200, 120),   // Green
        _ => egui::Color32::from_rgb(180, 180, 180),  // Gray for extras
    }
}

/// Render the central panel: Left channels | Mixer Box | Right channels
/// With 2 channels: A | Mixer | B
/// With 4 channels: A,B | Mixer | C,D
/// Generalizes to N channels split evenly across both sides.
fn render_central_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    if data.stage_editor_open {
        render_stage_editor(ui, data, actions);
        return;
    }

    let available = ui.available_width();
    let mixer_width = 160.0;
    let num_channels = data.channels.len();
    let left_count = (num_channels + 1) / 2; // ceil(N/2)
    let right_count = num_channels / 2;       // floor(N/2)
    let side_width = ((available - mixer_width) / 2.0) - 8.0;

    ui.horizontal_top(|ui| {
        // Left side channels
        if left_count > 0 {
            ui.vertical(|ui| {
                ui.set_width(side_width);
                let per_ch_width = side_width / left_count as f32 - 4.0;
                ui.horizontal_top(|ui| {
                    for i in 0..left_count {
                        if let Some(ch) = data.channels.get(i) {
                            ui.vertical(|ui| {
                                ui.set_width(per_ch_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                            if i < left_count - 1 { ui.separator(); }
                        }
                    }
                });
            });
        }

        ui.separator();

        // Center mixer box
        ui.vertical(|ui| {
            ui.set_width(mixer_width);
            render_mixer_box(ui, data, actions);
        });

        ui.separator();

        // Right side channels
        if right_count > 0 {
            ui.vertical(|ui| {
                ui.set_width(side_width);
                let per_ch_width = side_width / right_count as f32 - 4.0;
                ui.horizontal_top(|ui| {
                    for i in 0..right_count {
                        let ch_idx = left_count + i;
                        if let Some(ch) = data.channels.get(ch_idx) {
                            ui.vertical(|ui| {
                                ui.set_width(per_ch_width);
                                render_channel_column(ui, ch, data, actions);
                            });
                            if i < right_count - 1 { ui.separator(); }
                        }
                    }
                });
            });
        }
    });
}

/// Render the center mixer box (DJ console style)
/// Supports N channels: 2 channels = crossfader mode, 3+ = per-channel opacity mode
fn render_mixer_box(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let num_channels = data.channels.len();
    let use_crossfader = num_channels == 2;

    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(egui::Color32::from_rgb(20, 20, 30))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 80)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🎚 Mixer").strong().size(13.0));
                if ui.small_button("➕ Ch").on_hover_text("Add a new channel").clicked() {
                    actions.add_channel = true;
                }
            });
            ui.add_space(4.0);

            // Channel volume faders (vertical, side by side) — N channels
            let fader_height = 100.0;
            let mut opacities: Vec<f32> = data.channels.iter().map(|c| c.opacity).collect();
            let blend_modes_orig: Vec<BlendMode> = data.channels.iter().map(|c| c.blend_mode).collect();

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // Center faders: estimate total width and add leading space
                let per_fader_width = 30.0_f32; // label + slider column width
                let spacing = 4.0;
                let total_faders_width = num_channels as f32 * per_fader_width
                    + (num_channels.saturating_sub(1)) as f32 * spacing;
                let avail = ui.available_width();
                if avail > total_faders_width {
                    ui.add_space((avail - total_faders_width) / 2.0);
                }
                for (ch_idx, ch) in data.channels.iter().enumerate() {
                    let color = channel_color(ch_idx);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&ch.name).strong().color(color).size(11.0));
                            // Show remove button only if more than 2 channels
                            if num_channels > 2 {
                                if ui.small_button("✕").on_hover_text(format!("Remove channel {}", ch.name)).clicked() {
                                    actions.remove_channel = Some(ch_idx);
                                }
                            }
                        });
                        // Render slider — disabled in learn mode
                        let slider_rect;
                        if data.midi_learn_active {
                            let inner = ui.scope(|ui| {
                                ui.disable();
                                let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                                    .vertical()
                                    .show_value(false);
                                ui.add_sized([18.0, fader_height], slider)
                            });
                            slider_rect = inner.inner.rect;
                        } else {
                            let slider = egui::Slider::new(&mut opacities[ch_idx], 0.0..=1.0)
                                .vertical()
                                .show_value(false);
                            let resp = ui.add_sized([18.0, fader_height], slider);
                            slider_rect = resp.rect;
                        }
                        // MIDI learn: glow + click overlay
                        if data.midi_learn_active {
                            let path = format!("ch/{}/opacity", ch_idx);
                            let is_target = data.midi_learn_target.as_deref() == Some(path.as_str());
                            if is_target {
                                widgets::draw_midi_learn_selected(ui, slider_rect);
                            } else {
                                widgets::draw_midi_learn_glow(ui, slider_rect);
                            }
                            let click_id = ui.id().with(("midi_learn_ch_opacity", ch_idx));
                            let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                            if click_resp.clicked() {
                                actions.midi_learn_select = Some(path);
                            }
                        }
                    });
                }
            });

            // Only emit channel updates when opacity actually changed (via the fader)
            // This avoids overwriting blend mode changes made by the blend mode selector below
            for (ch_idx, ch) in data.channels.iter().enumerate() {
                if (opacities[ch_idx] - ch.opacity).abs() > f32::EPSILON {
                    actions.channel_updates.push((ch_idx, opacities[ch_idx], blend_modes_orig[ch_idx]));
                }
            }

            ui.add_space(4.0);
            ui.separator();

            // Crossfader — only shown for exactly 2 channels
            if use_crossfader {
                let color_a = channel_color(0);
                let color_b = channel_color(1);
                ui.label(egui::RichText::new("Crossfader").small());
                let name_a = data.channels.first().map(|c| c.name.as_str()).unwrap_or("A");
                let name_b = data.channels.get(1).map(|c| c.name.as_str()).unwrap_or("B");
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name_a).small().color(color_a));
                    let mut crossfader = data.crossfader;
                    let slider_rect;
                    if data.midi_learn_active {
                        let inner = ui.scope(|ui| {
                            ui.disable();
                            let slider = egui::Slider::new(&mut crossfader, 0.0..=1.0)
                                .show_value(false);
                            ui.add_sized([ui.available_width() - 16.0, 18.0], slider)
                        });
                        slider_rect = inner.inner.rect;
                    } else {
                        let slider = egui::Slider::new(&mut crossfader, 0.0..=1.0)
                            .show_value(false);
                        let resp = ui.add_sized([ui.available_width() - 16.0, 18.0], slider);
                        if resp.changed() {
                            actions.crossfader_action = Some(CrossfaderAction::SetPosition(crossfader));
                        }
                        slider_rect = resp.rect;
                    }
                    if data.midi_learn_active {
                        let is_target = data.midi_learn_target.as_deref() == Some("crossfader");
                        if is_target {
                            widgets::draw_midi_learn_selected(ui, slider_rect);
                        } else {
                            widgets::draw_midi_learn_glow(ui, slider_rect);
                        }
                        let click_id = ui.id().with("midi_learn_crossfader");
                        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
                        if click_resp.clicked() {
                            actions.midi_learn_select = Some("crossfader".to_string());
                        }
                    }
                    ui.label(egui::RichText::new(name_b).small().color(color_b));
                });

                // Snap buttons
                ui.horizontal(|ui| {
                    if ui.small_button(format!("⏮ {}", name_a)).clicked() {
                        actions.crossfader_action = Some(CrossfaderAction::SnapA);
                    }
                    if ui.small_button(format!("{} ⏭", name_b)).clicked() {
                        actions.crossfader_action = Some(CrossfaderAction::SnapB);
                    }
                });

                ui.add_space(2.0);
                ui.separator();

                // Auto-transition
                let auto_target = if data.crossfader < 0.5 { 1.0 } else { 0.0 };
                let auto_label = if data.crossfader < 0.5 { format!("→{}", name_b) } else { format!("→{}", name_a) };

                if data.auto_crossfade_active {
                    ui.add(egui::ProgressBar::new(data.auto_crossfade_progress)
                        .text("Transitioning..."));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui.small_button(format!("{} 1s", auto_label)).clicked() {
                            actions.crossfader_action = Some(CrossfaderAction::AutoTransition {
                                target: auto_target, duration_secs: 1.0,
                                easing: CrossfadeEasing::EaseInOut,
                            });
                        }
                        if ui.small_button("2s").clicked() {
                            actions.crossfader_action = Some(CrossfaderAction::AutoTransition {
                                target: auto_target, duration_secs: 2.0,
                                easing: CrossfadeEasing::EaseInOut,
                            });
                        }
                        if ui.small_button("4s").clicked() {
                            actions.crossfader_action = Some(CrossfaderAction::AutoTransition {
                                target: auto_target, duration_secs: 4.0,
                                easing: CrossfadeEasing::EaseInOut,
                            });
                        }
                        if data.audio.bpm.is_some() {
                            if ui.small_button("4beat").clicked() {
                                actions.crossfader_action = Some(CrossfaderAction::BeatTransition {
                                    target: auto_target, beats: 4.0,
                                });
                            }
                        }
                    });
                }

                ui.add_space(2.0);
                ui.separator();

                // Transition shader selector
                let current_label = data.active_transition_name.as_deref().unwrap_or("Opacity");
                egui::ComboBox::from_id_salt("transition_selector")
                    .selected_text(egui::RichText::new(format!("🔀 {}", current_label)).small())
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        let is_opacity = data.active_transition_name.is_none();
                        if ui.selectable_label(is_opacity, "Opacity (default)").clicked() {
                            actions.set_transition = Some(None);
                        }
                        ui.separator();
                        for name in &data.transition_names {
                            let selected = data.active_transition_name.as_ref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() {
                                actions.set_transition = Some(Some(name.clone()));
                            }
                        }
                    });
            }

            // Blend mode selectors for each channel
            ui.add_space(2.0);
            ui.separator();
            ui.label(egui::RichText::new("Blend").small());
            let blend_mode_names = ["Norm", "Add", "Mult", "Scrn", "Ovly", "Diff"];
            for (ch_idx, ch) in data.channels.iter().enumerate() {
                let color = channel_color(ch_idx);
                let current = match ch.blend_mode {
                    BlendMode::Normal => 0, BlendMode::Add => 1, BlendMode::Multiply => 2,
                    BlendMode::Screen => 3, BlendMode::Overlay => 4, BlendMode::Difference => 5,
                };
                let mut selected = current;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&ch.name).small().color(color));
                    egui::ComboBox::from_id_salt(format!("mix_blend_{}", ch_idx))
                        .selected_text(blend_mode_names[selected])
                        .width(55.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in blend_mode_names.iter().enumerate() {
                                ui.selectable_value(&mut selected, i, *name);
                            }
                        });
                });
                if selected != current {
                    let new_blend = match selected {
                        1 => BlendMode::Add, 2 => BlendMode::Multiply, 3 => BlendMode::Screen,
                        4 => BlendMode::Overlay, 5 => BlendMode::Difference, _ => BlendMode::Normal,
                    };
                    // Either update an existing entry (if opacity also changed this frame)
                    // or push a new one for blend-mode-only changes
                    if let Some(entry) = actions.channel_updates.iter_mut().find(|e| e.0 == ch_idx) {
                        entry.2 = new_blend;
                    } else {
                        actions.channel_updates.push((ch_idx, ch.opacity, new_blend));
                    }
                }
            }
        });
}

/// Render a channel column with header and its decks
fn render_channel_column(ui: &mut egui::Ui, ch: &super::ChannelUIInfo, data: &UIData, actions: &mut UIActions) {
    let accent = channel_color(ch.ch_idx);
    let ch_idx = ch.ch_idx;

    ui.push_id(format!("ch_{}", ch_idx), |ui| {
        // Visual feedback: highlight when a library drag hovers over this channel
        let has_library_drag = egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ui.ctx());
        let is_hovering = has_library_drag && ui.rect_contains_pointer(ui.max_rect());

        let frame = if is_hovering {
            egui::Frame::default()
                .fill(accent.linear_multiply(0.08))
                .stroke(egui::Stroke::new(2.0, accent.linear_multiply(0.5)))
                .corner_radius(4.0)
        } else {
            egui::Frame::NONE
        };

        frame.show(ui, |ui| {
        ui.vertical(|ui| {
            // Channel header
            let is_ch_selected = data.selected_channel == Some(ch_idx);
            let header_frame = if is_ch_selected {
                egui::Frame::default().fill(accent.linear_multiply(0.15)).corner_radius(3.0).inner_margin(2.0)
            } else {
                egui::Frame::default().inner_margin(2.0)
            };
            header_frame.show(ui, |ui| {
                let header_resp = ui.label(egui::RichText::new(format!("▌ Channel {}", ch.name)).strong().color(accent).size(16.0));
                let header_resp = header_resp.interact(egui::Sense::click());
                if header_resp.clicked() {
                    actions.select_channel = Some(ch_idx);
                }
            });

            ui.separator();

            // Deck grid
            egui::ScrollArea::vertical()
                .id_salt(format!("ch_scroll_{}", ch_idx))
                .drag_to_scroll(false)
                .show(ui, |ui| {
                if ch.decks.is_empty() {
                    ui.label(egui::RichText::new("No decks — drag generator here").weak().small());
                }
                let card_width = 134.0;
                let available_w = ui.available_width();
                let cols = ((available_w / card_width).floor() as usize).max(1);
                let mut deck_iter = ch.decks.iter().peekable();
                while deck_iter.peek().is_some() {
                    ui.horizontal(|ui| {
                        for _ in 0..cols {
                            if let Some(deck) = deck_iter.next() {
                                render_deck_thumbnail(ui, ch_idx, deck, accent, data, actions);
                            }
                        }
                    });
                    ui.add_space(2.0);
                }

                // Remaining space for deck-to-deck moves
                let drop_resp = ui.allocate_response(
                    egui::vec2(ui.available_width(), 20.0_f32.max(ui.available_height())),
                    egui::Sense::hover(),
                );
                if let Some(payload) = drop_resp.dnd_release_payload::<(usize, usize)>() {
                    let (src_ch, src_deck) = *payload;
                    if src_ch != ch_idx {
                        actions.deck_to_move = Some((src_ch, src_deck, ch_idx));
                    }
                }

                // Channel FX chain (compact)
                if !ch.effects.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("🔮 Ch FX").small().color(accent));
                    for (_i, (name, enabled, _)) in ch.effects.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let label = if *enabled {
                                egui::RichText::new(name).small()
                            } else {
                                egui::RichText::new(name).small().strikethrough().weak()
                            };
                            ui.label(label);
                        });
                    }
                }
            });
        });
        }); // frame.show

        // Store this channel's screen rect for the deferred DnD drop handler
        let ch_rect = ui.min_rect();
        ui.ctx().memory_mut(|mem| {
            mem.data.insert_temp(egui::Id::new("ch_drop_rect").with(ch_idx), ch_rect);
        });
    });
}

/// Render a compact deck thumbnail (clickable cell in the deck grid)
/// Layout: [ preview | opacity slider (vertical) ]
///         [ name                                 ]
///         [ M  S  ✕                              ]
fn render_deck_thumbnail(
    ui: &mut egui::Ui,
    ch_idx: usize,
    deck: &super::DeckUIInfo,
    accent: egui::Color32,
    data: &UIData,
    actions: &mut UIActions,
) {
    let idx = deck.deck_idx;
    let mut opacity = deck.opacity;
    let mut solo = deck.solo;
    let mut mute = deck.mute;
    let is_selected = data.selected_deck == Some((ch_idx, idx));
    let preview_width = 100.0;
    let preview_height = preview_width * 0.5625;
    let slider_width = 18.0;
    let card_width = preview_width + slider_width + 8.0; // preview + slider + padding

    ui.push_id(format!("deck_{}_{}", ch_idx, idx), |ui| {
        let border_color = if is_selected { accent } else { accent.linear_multiply(0.3) };
        let border_width = if is_selected { 2.0 } else { 1.0 };

        // Use manual rect-based painting to avoid egui layout overlap issues.
        // Total card height = preview_height + name_row(16) + button_row(20) + spacing(8) + padding(8)
        let name_row_h = 16.0;
        let button_row_h = 20.0;
        let spacing = 4.0;
        let padding = 4.0;
        let total_h = padding + preview_height + spacing + name_row_h + spacing + button_row_h + padding;

        let card_size = egui::vec2(card_width + padding * 2.0, total_h);
        let (card_rect, card_resp) = ui.allocate_exact_size(
            card_size,
            egui::Sense::click_and_drag(),
        );

        // MIDI learn mode: glow on deck card, click to select trigger
        if data.midi_learn_active {
            let trigger_path = format!("ch/{}/deck/{}/trigger", ch_idx, idx);
            let is_target = data.midi_learn_target.as_deref() == Some(trigger_path.as_str());
            if is_target {
                widgets::draw_midi_learn_selected(ui, card_rect);
            } else {
                widgets::draw_midi_learn_glow(ui, card_rect);
            }
            if card_resp.clicked() {
                actions.midi_learn_select = Some(trigger_path);
            }
        }

        // Start drag: set payload for deck move between channels
        if card_resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), (ch_idx, idx));
        }

        // While dragging, show a translucent ghost at the cursor
        if card_resp.dragged() {
            if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                let ghost_rect = egui::Rect::from_center_size(pointer_pos, card_size);
                let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("deck_drag_ghost"));
                let painter = ui.ctx().layer_painter(layer);
                painter.rect_filled(ghost_rect, 4.0, egui::Color32::from_rgba_unmultiplied(80, 120, 200, 120));
                painter.text(
                    ghost_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &deck.name,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Draw card background + border
        let bg_alpha = if card_resp.dragged() { 100 } else { 255 };
        ui.painter().rect_filled(card_rect, 4.0, egui::Color32::from_rgba_unmultiplied(25, 25, 35, bg_alpha));
        ui.painter().rect_stroke(card_rect, 4.0, egui::Stroke::new(border_width, border_color), egui::StrokeKind::Outside);

        // Row 1: Preview image (left) + vertical opacity slider (right)
        let preview_rect = egui::Rect::from_min_size(
            card_rect.min + egui::vec2(padding, padding),
            egui::vec2(preview_width, preview_height),
        );
        let slider_rect = egui::Rect::from_min_size(
            egui::pos2(preview_rect.max.x + 2.0, card_rect.min.y + padding),
            egui::vec2(slider_width, preview_height),
        );

        // Draw preview
        if let Some(&texture_id) = data.deck_preview_textures.get(&(ch_idx, idx)) {
            ui.painter().image(
                texture_id,
                preview_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            ui.painter().rect_filled(preview_rect, 3.0, egui::Color32::from_rgb(30, 30, 40));
            ui.painter().text(
                preview_rect.center(),
                egui::Align2::CENTER_CENTER,
                "No Preview",
                egui::FontId::proportional(9.0),
                egui::Color32::GRAY,
            );
        }

        // Click on card to select deck (only when not in learn mode — learn mode click selects trigger)
        if !data.midi_learn_active && card_resp.clicked() {
            actions.select_deck = Some((ch_idx, idx));
        }

        // Vertical opacity slider — use a child ui placed at the slider rect
        let mut slider_ui = ui.new_child(egui::UiBuilder::new().max_rect(slider_rect));
        let op_slider_rect;
        if data.midi_learn_active {
            let inner = slider_ui.scope(|ui| {
                ui.disable();
                let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
                    .vertical()
                    .show_value(false);
                ui.add_sized([slider_width, preview_height], slider)
            });
            op_slider_rect = inner.inner.rect;
        } else {
            let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
                .vertical()
                .show_value(false);
            let resp = slider_ui.add_sized([slider_width, preview_height], slider);
            op_slider_rect = resp.rect;
        }
        if data.midi_learn_active {
            let opacity_path = format!("ch/{}/deck/{}/opacity", ch_idx, idx);
            let is_target = data.midi_learn_target.as_deref() == Some(opacity_path.as_str());
            if is_target {
                widgets::draw_midi_learn_selected(&slider_ui, op_slider_rect);
            } else {
                widgets::draw_midi_learn_glow(&slider_ui, op_slider_rect);
            }
            let click_id = slider_ui.id().with(("midi_learn_deck_opacity", ch_idx, idx));
            let click_resp = slider_ui.interact(op_slider_rect, click_id, egui::Sense::click());
            if click_resp.clicked() {
                actions.midi_learn_select = Some(opacity_path);
            }
        }

        // Row 2: Deck name
        let name_y = card_rect.min.y + padding + preview_height + spacing;
        let display_name = if deck.name.len() > 16 {
            format!("{}…", &deck.name[..15])
        } else {
            deck.name.clone()
        };
        ui.painter().text(
            egui::pos2(card_rect.min.x + padding, name_y),
            egui::Align2::LEFT_TOP,
            &display_name,
            egui::FontId::proportional(11.0),
            accent,
        );

        // Row 3: M S ✕ buttons — use a child ui placed at the button row
        let btn_y = name_y + name_row_h + spacing;
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(card_rect.min.x + padding, btn_y),
            egui::vec2(card_width, button_row_h),
        );
        let mut btn_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect));
        btn_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ui.selectable_label(mute, egui::RichText::new("M").small()).clicked() {
                mute = !mute;
            }
            if ui.selectable_label(solo, egui::RichText::new("S").small()).clicked() {
                solo = !solo;
            }
            if ui.small_button(egui::RichText::new("✕").small()).clicked() {
                actions.deck_to_remove = Some((ch_idx, idx));
            }
        });

        // Only push deck updates when something actually changed from the UI controls here
        // (opacity slider, solo/mute buttons). This avoids overwriting blend mode changes
        // made in the detail panel's blend mode selector.
        if (opacity - deck.opacity).abs() > f32::EPSILON || solo != deck.solo || mute != deck.mute {
            actions.deck_updates.push((ch_idx, idx, opacity, deck.blend_mode, solo, mute));
        }
    });
}

