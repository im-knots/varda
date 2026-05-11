//! Shared UI utilities.

use super::super::EffectDrag;

/// Format seconds as MM:SS
pub(super) fn format_time(secs: f64) -> String {
    let m = (secs / 60.0).floor() as u32;
    let s = (secs % 60.0).floor() as u32;
    format!("{:02}:{:02}", m, s)
}

pub(super) fn render_collapsed_column(ui: &mut egui::Ui, label: &str, open_id: egui::Id) {
    let strip_width = 20.0;
    let min_height = ui.available_height().max(60.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(strip_width, min_height),
        egui::Sense::click(),
    );
    if response.clicked() {
        ui.ctx().memory_mut(|mem| mem.data.insert_temp(open_id, true));
    }
    let painter = ui.painter_at(rect);
    // Background
    let bg = if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };
    painter.rect_filled(rect, 4.0, bg);
    // Draw each character vertically, centered in the strip
    let font_id = egui::FontId::proportional(10.0);
    let text_color = ui.visuals().text_color();
    let chars: Vec<char> = label.chars().collect();
    let char_height = 12.0;
    let total_text_height = chars.len() as f32 * char_height;
    let start_y = rect.center().y - total_text_height / 2.0;
    for (i, ch) in chars.iter().enumerate() {
        let pos = egui::pos2(rect.center().x, start_y + i as f32 * char_height);
        painter.text(pos, egui::Align2::CENTER_TOP, ch.to_string(), font_id.clone(), text_color);
    }
}

/// `chain_key` identifies the chain (e.g. "deck_0_1", "ch_0", "master").
/// `position` is the insert index in the chain.
pub(super) fn render_effect_drop_zone(ui: &mut egui::Ui, chain_key: &str, position: usize) {
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

/// Render a drag handle that initiates effect drag-and-drop.
/// Returns the handle response. Uses painted dots instead of text to avoid selection.
pub(super) fn render_effect_drag_handle(ui: &mut egui::Ui, payload: EffectDrag) {
    let handle_size = egui::vec2(12.0, 16.0);
    let (handle_rect, handle_resp) = ui.allocate_exact_size(handle_size, egui::Sense::drag());
    let color = if handle_resp.dragged() || handle_resp.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    // Draw 6 grip dots (3 rows x 2 cols)
    let cx = handle_rect.center().x;
    let cy = handle_rect.center().y;
    let r = 1.5;
    let dx = 3.0;
    let dy = 4.0;
    for row in -1..=1 {
        for col in [-1.0_f32, 1.0] {
            let x = cx + col * dx;
            let y = cy + row as f32 * dy;
            ui.painter().circle_filled(egui::pos2(x, y), r, color);
        }
    }
    if handle_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if handle_resp.dragged() {
        egui::DragAndDrop::set_payload(ui.ctx(), payload);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
}

/// Show a floating ghost card while an effect is being dragged.
pub(super) fn render_effect_drag_ghost(ui: &mut egui::Ui, ghost_id: egui::Id, payload: EffectDrag, name: &str) {
    if egui::DragAndDrop::payload::<EffectDrag>(ui.ctx())
        .map(|p| *p == payload).unwrap_or(false)
    {
        // Store source in temp memory for deferred drop handler
        ui.ctx().memory_mut(|mem| {
            mem.data.insert_temp(egui::Id::new("__eff_dnd_src"), payload);
        });
        // Paint floating ghost at pointer using Area (avoids cross-order sublayer panic)
        if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
            egui::Area::new(ghost_id)
                .order(egui::Order::Tooltip)
                .fixed_pos(egui::pos2(pos.x + 12.0, pos.y + 12.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::default()
                        .inner_margin(4.0)
                        .corner_radius(4.0)
                        .fill(egui::Color32::from_rgba_premultiplied(40, 40, 55, 220))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 180, 255)))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(name).strong().size(11.0));
                        });
                });
        }
    }
}


/// Channel accent colors — infinite non-colliding colors via binary hue subdivision.
///
/// Hues are placed by halving the hue wheel: ch0 gets one hue, ch1 the opposite,
/// ch2–3 fill the quarter-points, ch4–7 the eighth-points, etc. This guarantees
/// maximum hue separation for any channel count. Each subdivision "ring" gets a
/// distinct saturation/lightness style so nearby hues in later rings still look
/// clearly different (vivid-dark vs pastel vs saturated, etc.).
pub(super) fn channel_color(ch_idx: usize) -> egui::Color32 {
    const HUE_OFFSET: f32 = 0.76; // start at purple to match original Ch 0

    // Saturation/lightness per ring — strongly varied so same-region hues differ
    const RING_STYLES: [(f32, f32); 6] = [
        (0.75, 0.58), // ring 0: vivid mid
        (0.70, 0.65), // ring 1: vivid light
        (0.80, 0.50), // ring 2: saturated dark
        (0.55, 0.72), // ring 3: soft light
        (0.85, 0.45), // ring 4: very saturated dark
        (0.50, 0.75), // ring 5+: pastel
    ];

    let (ring, hue_frac) = hue_subdivision(ch_idx);
    let hue = (HUE_OFFSET + hue_frac) % 1.0;
    let (sat, lit) = RING_STYLES[ring.min(RING_STYLES.len() - 1)];

    let (r, g, b) = hsl_to_rgb(hue, sat, lit);
    egui::Color32::from_rgb(
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
    )
}

/// Binary subdivision of the hue wheel. Returns (ring, hue_fraction).
/// Ring 0 → 1 slot (0/1), ring 1 → 1 slot (1/2), ring k≥2 → 2^(k-1) slots
/// at odd multiples of 1/2^k. Guarantees optimal minimum hue distance.
pub(crate) fn hue_subdivision(idx: usize) -> (usize, f32) {
    if idx == 0 {
        return (0, 0.0);
    }
    let mut remaining = idx - 1;
    let mut ring: usize = 1;
    let mut ring_size: usize = 1;
    while remaining >= ring_size {
        remaining -= ring_size;
        ring += 1;
        ring_size = 1 << (ring - 1); // 1, 2, 4, 8, …
    }
    let denom = 1u32 << ring as u32;            // 2, 4, 8, 16, …
    let numerator = (2 * remaining + 1) as u32; // odd: 1, 3, 5, 7, …
    (ring, numerator as f32 / denom as f32)
}

/// Convert HSL (all 0.0–1.0) to RGB (all 0.0–1.0).
pub(crate) fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    (
        hue_to_channel(p, q, h + 1.0 / 3.0),
        hue_to_channel(p, q, h),
        hue_to_channel(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_channel(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 { t += 1.0; }
    if t > 1.0 { t -= 1.0; }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 1.0 / 2.0 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    p
}
