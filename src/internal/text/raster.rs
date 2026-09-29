//! Shaping and rasterizing one line or word into a coverage mask.
//!
//! Each unit becomes a one-element SVG `<text>` document, shaped by usvg
//! against the shared font database (rustybuzz handles kerning, ligatures and
//! font fallback) and rendered with resvg. The mask is cropped to the ink and
//! stored with its placement measurements.

use anyhow::{Context, Result};
use std::sync::Arc;

/// Weight steps rasters are made at. A modulated weight reuses these.
pub const WEIGHT_STEP: f32 = 25.0;

/// How text is styled, as it affects rasterizing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Style {
    pub family: String,
    /// CSS weight rounded to [`WEIGHT_STEP`].
    pub weight: u16,
    pub italic: bool,
}

impl Style {
    pub fn new(family: &str, weight: f32, italic: bool) -> Self {
        Self {
            family: family.to_string(),
            weight: weight_step(weight),
            italic,
        }
    }
}

/// `weight` rounded to the nearest raster step, within CSS's 100 to 900.
pub fn weight_step(weight: f32) -> u16 {
    ((weight.clamp(100.0, 900.0) / WEIGHT_STEP).round() * WEIGHT_STEP) as u16
}

/// Font sizes are rasterized at quarter-octave steps: bucket `b` is
/// `2^(b/4)` pixels. A unit drawn at any size uses the smallest bucket at or
/// above it and is scaled down at most 16% on the GPU.
pub fn size_bucket(px: f32) -> i32 {
    (px.max(1.0).log2() * 4.0).ceil() as i32
}

/// The font size in pixels of bucket `bucket`.
pub fn bucket_px(bucket: i32) -> f32 {
    (bucket as f32 / 4.0).exp2()
}

/// One unit's coverage mask and where it sits.
pub struct Raster {
    /// Coverage, one byte per pixel, rows top to bottom.
    pub coverage: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The font size it was drawn at, in pixels.
    pub px: f32,
    /// Where the mask's top-left sits relative to the text's origin (left end
    /// of the baseline), in ems.
    pub origin: [f32; 2],
    /// Layout width, in ems: from the origin to the right edge of the ink.
    pub advance: f32,
    /// The right end of each word, in ems from the origin.
    pub word_ends: Arc<[f32]>,
}

/// Room left around the text on the scratch canvas, in ems, so ascenders,
/// descenders and overhangs are never cut before the crop.
const MARGIN_EM: f32 = 1.0;

/// Shape and rasterize `text` at `px` pixels. `None` for text with no ink
/// (empty or all whitespace).
///
/// `max_dim` bounds the mask's size, as the device's texture limit does; a
/// unit wider than that is drawn smaller and magnified on the GPU.
///
/// # Errors
///
/// Fails when the generated document does not parse or a pixmap cannot be
/// allocated.
pub fn rasterize(
    fonts: &Arc<usvg::fontdb::Database>,
    text: &str,
    style: &Style,
    px: f32,
    max_dim: u32,
) -> Result<Option<Raster>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let tree = parse(fonts, text, style, px)?;
    let Some(node) = text_node(tree.root()) else {
        return Ok(None);
    };
    let bbox = node.abs_bounding_box();
    if bbox.width() <= 0.0 || bbox.height() <= 0.0 {
        return Ok(None);
    }
    let pad = 1.0;
    let (origin_x, baseline) = canvas_origin(px);
    let mut scale = 1.0_f32;
    let full_w = bbox.width() + 2.0 * pad;
    let full_h = bbox.height() + 2.0 * pad;
    let limit = max_dim.max(1) as f32;
    if full_w > limit || full_h > limit {
        scale = (limit / full_w).min(limit / full_h);
    }
    let width = ((full_w * scale).ceil() as u32).clamp(1, max_dim.max(1));
    let height = ((full_h * scale).ceil() as u32).clamp(1, max_dim.max(1));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("Failed to allocate a {width}x{height} text pixmap"))?;
    let transform = resvg::tiny_skia::Transform::from_translate(pad - bbox.x(), pad - bbox.y())
        .post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let coverage = pixmap.pixels().iter().map(|p| p.alpha()).collect();

    let em = px;
    let drawn_px = px * scale;
    let word_ends = word_ends(node, origin_x, bbox.right(), em);
    Ok(Some(Raster {
        coverage,
        width,
        height,
        px: drawn_px,
        origin: [
            (bbox.x() - pad - origin_x) / em,
            (bbox.y() - pad - baseline) / em,
        ],
        advance: (bbox.right() - origin_x) / em,
        word_ends,
    }))
}

/// The baseline origin on the scratch canvas.
fn canvas_origin(px: f32) -> (f32, f32) {
    (MARGIN_EM * px, (MARGIN_EM + 1.0) * px)
}

fn parse(
    fonts: &Arc<usvg::fontdb::Database>,
    text: &str,
    style: &Style,
    px: f32,
) -> Result<usvg::Tree> {
    let (x, y) = canvas_origin(px);
    // Wide enough for any line the texture limit allows; usvg does not clip
    // text to the canvas when measuring.
    let canvas_w = (text.chars().count() as f32 + 2.0 * MARGIN_EM) * px * 2.0;
    let canvas_h = (2.0 * MARGIN_EM + 2.0) * px;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{canvas_w}" height="{canvas_h}"><text x="{x}" y="{y}" font-family="{family}, {fallback}" font-size="{px}" font-weight="{weight}" font-style="{italic}" fill="#fff" xml:space="preserve">{text}</text></svg>"##,
        family = xml_attr_family(&style.family),
        fallback = crate::fonts::FALLBACK_FAMILY,
        weight = style.weight,
        italic = if style.italic { "italic" } else { "normal" },
        text = xml_escape(text),
    );
    let options = usvg::Options {
        fontdb: fonts.clone(),
        ..Default::default()
    };
    usvg::Tree::from_str(&svg, &options).context("Failed to lay out text")
}

fn text_node(group: &usvg::Group) -> Option<&usvg::Text> {
    group.children().iter().find_map(|node| match node {
        usvg::Node::Text(text) => Some(text.as_ref()),
        usvg::Node::Group(inner) => text_node(inner),
        _ => None,
    })
}

/// The right end of each word: where the whitespace after it starts, or the
/// ink's right edge for the last.
fn word_ends(text: &usvg::Text, origin_x: f32, ink_right: f32, em: f32) -> Arc<[f32]> {
    let mut ends = Vec::new();
    let mut in_word = false;
    for glyph in text
        .layouted()
        .iter()
        .flat_map(|span| &span.positioned_glyphs)
    {
        let blank = glyph.text.chars().all(char::is_whitespace);
        if blank && in_word {
            ends.push((glyph.transform().tx - origin_x) / em);
        }
        in_word = !blank;
    }
    if in_word {
        ends.push((ink_right - origin_x) / em);
    }
    ends.into()
}

/// A family name quoted for a CSS `font-family` list inside an XML attribute.
fn xml_attr_family(family: &str) -> String {
    let cleaned: String = family
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '<' | '>' | '&'))
        .collect();
    format!("'{cleaned}'")
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Arc<usvg::fontdb::Database> {
        let mut db = usvg::fontdb::Database::new();
        crate::fonts::add_bundled(&mut db);
        Arc::new(db)
    }

    fn style() -> Style {
        Style::new("Ubuntu", 400.0, false)
    }

    #[test]
    fn a_line_has_ink_and_a_width() {
        let r = rasterize(&fonts(), "Hello", &style(), 64.0, 8192)
            .unwrap()
            .expect("ink");
        assert!(r.coverage.iter().any(|&c| c > 200), "some opaque pixels");
        assert_eq!(r.coverage.len(), (r.width * r.height) as usize);
        assert!(r.advance > 1.0 && r.advance < 5.0, "{}", r.advance);
        // The mask starts above the baseline.
        assert!(r.origin[1] < -0.5, "{:?}", r.origin);
    }

    #[test]
    fn a_longer_string_is_wider_in_proportion() {
        let f = fonts();
        let one = rasterize(&f, "abcd", &style(), 48.0, 8192)
            .unwrap()
            .unwrap();
        let two = rasterize(&f, "abcdabcd", &style(), 48.0, 8192)
            .unwrap()
            .unwrap();
        let ratio = two.advance / one.advance;
        assert!((ratio - 2.0).abs() < 0.2, "{ratio}");
    }

    #[test]
    fn widths_are_in_ems_whatever_the_size() {
        let f = fonts();
        let small = rasterize(&f, "Varda", &style(), 20.0, 8192)
            .unwrap()
            .unwrap();
        let large = rasterize(&f, "Varda", &style(), 80.0, 8192)
            .unwrap()
            .unwrap();
        assert!((small.advance - large.advance).abs() < 0.1);
        assert!(large.width > 3 * small.width);
    }

    #[test]
    fn word_ends_are_ordered_and_end_at_the_ink() {
        let r = rasterize(&fonts(), "one two three", &style(), 40.0, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(r.word_ends.len(), 3);
        assert!(
            r.word_ends.windows(2).all(|w| w[0] < w[1]),
            "{:?}",
            r.word_ends
        );
        assert!((r.word_ends[2] - r.advance).abs() < 1e-3);
    }

    #[test]
    fn markup_characters_are_text_not_markup() {
        let r = rasterize(&fonts(), "a < b & \"c\"", &style(), 40.0, 8192).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn blank_text_has_no_raster() {
        assert!(
            rasterize(&fonts(), "   ", &style(), 40.0, 8192)
                .unwrap()
                .is_none()
        );
        assert!(
            rasterize(&fonts(), "", &style(), 40.0, 8192)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_texture_limit_shrinks_a_wide_unit() {
        let r = rasterize(&fonts(), "a very long line of text", &style(), 200.0, 1024)
            .unwrap()
            .unwrap();
        assert!(r.width <= 1024);
        assert!(r.px < 200.0);
    }

    #[test]
    fn a_missing_family_falls_back_to_sans_serif() {
        let s = Style::new("No Such Family", 400.0, false);
        assert!(
            rasterize(&fonts(), "fallback", &s, 40.0, 8192)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn buckets_are_a_quarter_octave_apart_and_cover_the_size() {
        for px in [8.0, 33.0, 100.0, 517.0] {
            let b = size_bucket(px);
            assert!(bucket_px(b) >= px - 1e-3);
            assert!(bucket_px(b) / px <= 2f32.powf(0.25) + 1e-3);
        }
    }

    #[test]
    fn weights_round_to_steps_of_25() {
        assert_eq!(weight_step(412.0), 400);
        assert_eq!(weight_step(413.0), 425);
        assert_eq!(weight_step(50.0), 100);
        assert_eq!(weight_step(1000.0), 900);
    }
}
