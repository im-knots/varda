//! Shared UI utilities.

use super::super::{ChannelUIInfo, EffectDrag};

/// Shorten `s` to at most `max` characters, ending in an ellipsis when cut.
///
/// Counts characters, not bytes. Source names carry em dashes and emoji (a
/// window capture is labelled `🖥 Firefox — Title`), and byte-slicing one of
/// those panics on a char boundary rather than merely rendering oddly.
pub(super) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Format seconds as MM:SS
pub(super) fn format_time(secs: f64) -> String {
    let m = (secs / 60.0).floor() as u32;
    let s = (secs % 60.0).floor() as u32;
    format!("{m:02}:{s:02}")
}

/// The size to draw a preview or output-space canvas at so that it keeps the
/// render resolution's aspect ratio while fitting inside `budget`.
///
/// Every preview widget used to hardcode 16:9, so a portrait project — a phone
/// video at 1080×1920, say — rendered correctly but was squashed into a
/// landscape rectangle everywhere in the UI. The GPU side was never the
/// problem: `PreviewEncoder` already downscales with the aspect preserved, and
/// recordings always came out right. Only the widgets lied.
///
/// The result is scaled to *contain*: it never exceeds `budget` on either axis,
/// and touches it on whichever axis binds first. Callers pass the space they are
/// willing to give up, which for a width-driven panel means passing a height cap
/// as well, or a tall project would push everything below it off the panel.
///
/// This also governs the stage surface and warp canvases. They are not preview
/// images, but they map normalised output coordinates onto a rectangle, so a
/// 16:9 canvas draws a square surface as a wide one whenever the output is not
/// 16:9 — the same lie in a place where it misleads about geometry the user is
/// editing.
pub(super) fn preview_size(
    budget: egui::Vec2,
    render_width: u32,
    render_height: u32,
) -> egui::Vec2 {
    let budget = egui::vec2(budget.x.max(1.0), budget.y.max(1.0));
    // A render resolution is never zero in practice; the fallback keeps a
    // malformed scene file from producing a NaN-sized widget.
    let aspect = if render_width == 0 || render_height == 0 {
        16.0 / 9.0
    } else {
        render_width as f32 / render_height as f32
    };
    let height_at_full_width = budget.x / aspect;
    if height_at_full_width <= budget.y {
        egui::vec2(budget.x, height_at_full_width)
    } else {
        egui::vec2(budget.y * aspect, budget.y)
    }
}

pub(super) fn render_collapsed_column(ui: &mut egui::Ui, label: &str, open_id: egui::Id) {
    let strip_width = 20.0;
    let min_height = ui.available_height().max(60.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(strip_width, min_height), egui::Sense::click());
    if response.clicked() {
        ui.ctx()
            .memory_mut(|mem| mem.data.insert_temp(open_id, true));
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
        painter.text(
            pos,
            egui::Align2::CENTER_TOP,
            ch.to_string(),
            font_id.clone(),
            text_color,
        );
    }
}

/// Resolve a channel UUID to its ordinal and display name. The ordinal is only
/// used for palette lookup; callers must treat `None` as "no longer exists"
/// rather than falling back to a position.
pub(super) fn resolve_channel(channels: &[ChannelUIInfo], uuid: &str) -> Option<(usize, String)> {
    channels
        .iter()
        .position(|c| c.uuid == uuid)
        .map(|i| (i, channels[i].name.clone()))
}

/// `chain_key` identifies the chain (e.g. "deck_<uuid>", "ch_<uuid>", "master").
/// `position` is the insert index in the chain.
pub(super) fn render_effect_drop_zone(ui: &mut egui::Ui, chain_key: &str, position: usize) {
    let dz = ui.allocate_response(
        egui::vec2(8.0, ui.available_height().max(40.0)),
        egui::Sense::hover(),
    );
    let has_drag = egui::DragAndDrop::has_payload_of_type::<EffectDrag>(ui.ctx());
    // Store rect for deferred handler to find
    let key = egui::Id::new("eff_dz_rect").with((chain_key.to_string(), position));
    ui.ctx().memory_mut(|mem| {
        mem.data.insert_temp(key, dz.rect);
    });
    // Visual highlight: check if pointer is actually over this zone
    if has_drag
        && let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos())
        && dz.rect.contains(pos)
    {
        ui.painter()
            .rect_filled(dz.rect, 2.0, egui::Color32::from_rgb(100, 200, 255));
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
pub(super) fn render_effect_drag_ghost(
    ui: &mut egui::Ui,
    ghost_id: egui::Id,
    payload: EffectDrag,
    name: &str,
) {
    if egui::DragAndDrop::payload::<EffectDrag>(ui.ctx()).is_some_and(|p| *p == payload) {
        // Store source in temp memory for deferred drop handler
        ui.ctx().memory_mut(|mem| {
            mem.data
                .insert_temp(egui::Id::new("__eff_dnd_src"), payload);
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
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgb(100, 180, 255),
                        ))
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
    egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Binary subdivision of the hue wheel. Returns (ring, `hue_fraction`).
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
    let denom = 1u32 << ring as u32; // 2, 4, 8, 16, …
    let numerator = (2 * remaining + 1) as u32; // odd: 1, 3, 5, 7, …
    (ring, numerator as f32 / denom as f32)
}

/// Convert HSL (all 0.0–1.0) to RGB (all 0.0–1.0).
// h/s/l/p/q are the standard symbols in the HSL→RGB formula.
#[allow(clippy::many_single_char_names)]
pub(crate) fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    (
        hue_to_channel(p, q, h + 1.0 / 3.0),
        hue_to_channel(p, q, h),
        hue_to_channel(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_channel(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_time_covers_boundaries_and_truncates() {
        assert_eq!(format_time(0.0), "00:00");
        assert_eq!(format_time(30.0), "00:30");
        assert_eq!(format_time(60.0), "01:00");
        assert_eq!(format_time(125.0), "02:05");
        // Minutes are not wrapped at 60 — an hour reads as 60:00.
        assert_eq!(format_time(3600.0), "60:00");
        // Fractional seconds floor, not round.
        assert_eq!(format_time(30.7), "00:30");
        assert_eq!(format_time(59.9), "00:59");
        assert_eq!(format_time(60.1), "01:00");
    }

    /// The historical 16:9 sizing must be reproduced exactly, or every existing
    /// layout shifts the moment this helper is wired in.
    #[test]
    fn preview_size_leaves_a_landscape_project_where_it_was() {
        let size = preview_size(egui::vec2(100.0, 100.0), 1920, 1080);
        assert!((size.x - 100.0).abs() < 1e-3);
        assert!((size.y - 56.25).abs() < 1e-3, "got {}", size.y);
    }

    #[test]
    fn preview_size_makes_a_portrait_project_tall_and_narrow() {
        let size = preview_size(egui::vec2(100.0, 100.0), 1080, 1920);
        assert!((size.x - 56.25).abs() < 1e-3, "got {}", size.x);
        assert!((size.y - 100.0).abs() < 1e-3);
    }

    #[test]
    fn preview_size_never_exceeds_its_budget() {
        for (w, h) in [(1920u32, 1080u32), (1080, 1920), (1080, 1080), (2560, 1080)] {
            let budget = egui::vec2(320.0, 180.0);
            let size = preview_size(budget, w, h);
            assert!(
                size.x <= budget.x + 1e-3 && size.y <= budget.y + 1e-3,
                "{w}x{h} produced {size:?} for budget {budget:?}"
            );
        }
    }

    #[test]
    fn preview_size_preserves_the_render_aspect() {
        for (w, h) in [(1920u32, 1080u32), (1080, 1920), (1080, 1350), (1080, 1080)] {
            let size = preview_size(egui::vec2(200.0, 200.0), w, h);
            let want = w as f32 / h as f32;
            assert!(
                (size.x / size.y - want).abs() < 1e-3,
                "{w}x{h} produced aspect {}",
                size.x / size.y
            );
        }
    }

    /// A zero from a malformed scene must not reach the layout as a NaN.
    #[test]
    fn preview_size_falls_back_when_the_resolution_is_degenerate() {
        let size = preview_size(egui::vec2(160.0, 160.0), 0, 0);
        assert!(size.x.is_finite() && size.y.is_finite());
        assert!((size.x / size.y - 16.0 / 9.0).abs() < 1e-3);
    }

    /// The portrait bug was one mistake made independently in seven places:
    /// every preview widget and both stage canvases baked in 16:9 rather than
    /// asking what the project actually renders at. They are all routed through
    /// [`preview_size`] now, and this stops the eighth copy from landing.
    ///
    /// Matching on the literals is crude but it is what the bug looked like
    /// every time, and it costs nothing to keep honest.
    #[test]
    fn no_panel_hardcodes_a_16_by_9_widget() {
        // The camera feed is sized to the camera, not to the render resolution;
        // it has its own aspect problem and its own fix.
        const EXEMPT: &[&str] = &["camera_detect.rs"];
        const LITERALS: &[&str] = &["0.5625", "16.0 / 9.0", "9.0 / 16.0"];

        let panels =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/usecases/ui/panels");
        let mut offenders = Vec::new();
        let mut stack = vec![panels];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read the panels directory") {
                let path = entry.expect("read a directory entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if path.extension().is_none_or(|e| e != "rs")
                    || name == "utils.rs"
                    || EXEMPT.contains(&name)
                {
                    continue;
                }
                let body = std::fs::read_to_string(&path).expect("read a panel source file");
                for (n, line) in body.lines().enumerate() {
                    if LITERALS.iter().any(|lit| line.contains(lit)) {
                        offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these size a widget from a hardcoded 16:9 instead of the render \
             resolution — use utils::preview_size:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn hue_subdivision_places_indices_on_expected_ring_fractions() {
        assert_eq!(hue_subdivision(0), (0, 0.0));
        assert_eq!(hue_subdivision(1), (1, 0.5));
        assert_eq!(hue_subdivision(2), (2, 0.25));
        assert_eq!(hue_subdivision(3), (2, 0.75));
        // Ring 3 holds four slots at the odd eighths.
        assert_eq!(hue_subdivision(4), (3, 0.125));
        assert_eq!(hue_subdivision(5), (3, 0.375));
        assert_eq!(hue_subdivision(6), (3, 0.625));
        assert_eq!(hue_subdivision(7), (3, 0.875));
    }

    #[test]
    fn hue_subdivision_fractions_are_unique_across_channels() {
        // Every channel index must map to a distinct hue fraction so palette
        // colours never collide.
        let mut seen = std::collections::HashSet::new();
        for idx in 0..16 {
            let (_ring, frac) = hue_subdivision(idx);
            assert!(
                seen.insert(frac.to_bits()),
                "duplicate hue fraction {frac} at idx {idx}"
            );
        }
    }

    #[test]
    fn hue_subdivision_is_monotonic_within_each_ring() {
        // idx ranges per ring: ring2=[2,3], ring3=[4,7].
        for (start, end) in [(2usize, 4usize), (4, 8)] {
            let mut prev = -1.0;
            for idx in start..end {
                let (_ring, frac) = hue_subdivision(idx);
                assert!(frac > prev, "ring not monotonic at idx {idx}: {frac}");
                prev = frac;
            }
        }
    }

    // ── truncate_chars ──────────────────────────────────────────────

    #[test]
    fn truncate_chars_leaves_short_strings_alone() {
        assert_eq!(truncate_chars("Deck 1", 16), "Deck 1");
        // Exactly at the limit is not a cut.
        assert_eq!(truncate_chars("abcd", 4), "abcd");
    }

    #[test]
    fn truncate_chars_cuts_to_the_limit_with_an_ellipsis() {
        assert_eq!(truncate_chars("abcdefgh", 4), "abc…");
        assert_eq!(truncate_chars("abcdefgh", 4).chars().count(), 4);
    }

    /// The crash this replaces: a window-capture deck is named
    /// `🖥 Firefox — Title`, and byte-slicing it panicked mid-em-dash.
    #[test]
    fn truncate_chars_never_splits_a_multibyte_character() {
        let name = "🖥 Firefox — + ##sre | Libera.Chat";
        for max in 0..=name.chars().count() + 2 {
            let out = truncate_chars(name, max);
            assert!(out.chars().count() <= max.max(1));
            assert!(name.starts_with(out.trim_end_matches('…')));
        }
    }

    #[test]
    fn truncate_chars_handles_a_zero_limit() {
        assert_eq!(truncate_chars("abc", 0), "…");
    }

    fn approx(a: (f32, f32, f32), b: (f32, f32, f32)) {
        assert!(
            (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5 && (a.2 - b.2).abs() < 1e-5,
            "expected {b:?}, got {a:?}"
        );
    }

    #[test]
    fn hsl_to_rgb_zero_saturation_is_grayscale() {
        approx(hsl_to_rgb(0.5, 0.0, 0.5), (0.5, 0.5, 0.5));
        approx(hsl_to_rgb(0.2, 0.0, 0.25), (0.25, 0.25, 0.25));
    }

    #[test]
    fn hsl_to_rgb_primaries() {
        approx(hsl_to_rgb(0.0, 1.0, 0.5), (1.0, 0.0, 0.0));
        approx(hsl_to_rgb(1.0 / 3.0, 1.0, 0.5), (0.0, 1.0, 0.0));
        approx(hsl_to_rgb(2.0 / 3.0, 1.0, 0.5), (0.0, 0.0, 1.0));
    }

    #[test]
    fn hsl_to_rgb_lightness_extremes_are_black_and_white() {
        approx(hsl_to_rgb(0.5, 0.8, 0.0), (0.0, 0.0, 0.0));
        approx(hsl_to_rgb(0.5, 0.8, 1.0), (1.0, 1.0, 1.0));
    }

    #[test]
    fn hsl_to_rgb_hue_wraps_at_one() {
        // h=1.0 must resolve to the same colour as h=0.0 (red).
        approx(hsl_to_rgb(1.0, 1.0, 0.5), hsl_to_rgb(0.0, 1.0, 0.5));
    }

    #[test]
    fn channel_color_is_deterministic_and_distinct_across_channels() {
        assert_eq!(channel_color(0), channel_color(0));
        let colors: Vec<_> = (0..4).map(channel_color).collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(colors[i], colors[j], "channels {i} and {j} share a colour");
            }
        }
    }
}
