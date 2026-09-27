//! SVG parsing and rasterization for image decks.
//!
//! An SVG has no native pixel size, only a shape. Rasterizing it once at
//! whatever the artwork's `width`/`height` attributes happen to say would waste
//! the one advantage vector art has: a 512-unit logo would arrive as a 512 px
//! bitmap and be blown up to a 4K deck like any other small PNG. So decks keep
//! the parsed tree and re-render it whenever the master resolution changes,
//! which is what makes the same file sharp on a laptop preview and on a wall.
//!
//! See /spec/deck-sources.md § 3 (Image / Still).

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// System fonts, loaded at most once for the life of the process.
///
/// `usvg` turns `<text>` into paths at parse time, so this is only paid by the
/// first SVG loaded and never again — including on re-rasterization, which
/// works from the already-resolved tree.
fn system_fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            log::debug!("Loaded {} system font faces for SVG text", db.len());
            Arc::new(db)
        })
        .clone()
}

/// Whether a path should be treated as vector art rather than handed to the
/// raster image decoder. `.svgz` is a gzipped `.svg`, which `usvg` unwraps.
pub fn is_svg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
}

/// Parse an SVG file into a resolved tree.
///
/// Relative `<image>` and font references resolve against the file's own
/// directory, so artwork exported next to its assets loads the way it looks in
/// the drawing program.
///
/// # Errors
///
/// Returns an error if the file cannot be read or is not valid SVG.
pub fn parse_file(path: &Path) -> Result<usvg::Tree> {
    let data =
        std::fs::read(path).with_context(|| format!("Failed to read SVG: {}", path.display()))?;
    let options = usvg::Options {
        resources_dir: path.parent().map(std::path::Path::to_path_buf),
        fontdb: system_fonts(),
        ..Default::default()
    };
    usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("Failed to parse SVG: {}", path.display()))
}

/// The pixel size to rasterize `tree` at so it fills a `deck_w × deck_h` deck.
///
/// Fits rather than stretches, and scales up as readily as down — the point of
/// vector art is that enlarging costs nothing but memory. The deck's own
/// scaling mode then treats the result like any other image, so an SVG shaped
/// unlike the stage letterboxes or crops the same way a photograph would.
/// Bounded by the deck in both axes, so a long thin drawing cannot blow up the
/// texture budget on the axis it is not constrained by.
pub fn raster_size(tree: &usvg::Tree, deck_w: u32, deck_h: u32) -> (u32, u32) {
    let size = tree.size();
    let (svg_w, svg_h) = (size.width(), size.height());
    if svg_w <= 0.0 || svg_h <= 0.0 {
        return (deck_w.max(1), deck_h.max(1));
    }
    let scale = (deck_w as f32 / svg_w).min(deck_h as f32 / svg_h);
    (
        ((svg_w * scale).round() as u32).max(1),
        ((svg_h * scale).round() as u32).max(1),
    )
}

/// Rasterize `tree` to straight-alpha RGBA at the size [`raster_size`] picks.
///
/// resvg composites onto a premultiplied pixmap; the pixels are demultiplied on
/// the way out so an SVG behaves exactly like a PNG with an alpha channel once
/// it reaches the deck's blit.
///
/// # Errors
///
/// Returns an error if a pixmap of the required size cannot be allocated.
pub fn rasterize(tree: &usvg::Tree, deck_w: u32, deck_h: u32) -> Result<image::RgbaImage> {
    let (width, height) = raster_size(tree, deck_w, deck_h);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("Failed to allocate a {width}×{height} SVG pixmap"))?;

    let size = tree.size();
    let scale = if size.width() > 0.0 && size.height() > 0.0 {
        (width as f32 / size.width()).min(height as f32 / size.height())
    } else {
        1.0
    };
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let mut rgba = image::RgbaImage::new(width, height);
    for (dst, src) in rgba.pixels_mut().zip(pixmap.pixels()) {
        let c = src.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"
        width="100" height="100"><rect width="100" height="100" fill="#ff0000"/></svg>"##;
    const WIDE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 50"
        width="200" height="50"><rect width="200" height="50" fill="#00ff00"/></svg>"##;

    fn tree(data: &[u8]) -> usvg::Tree {
        usvg::Tree::from_data(data, &usvg::Options::default()).expect("valid SVG")
    }

    #[test]
    fn svg_paths_are_recognised_case_insensitively() {
        assert!(is_svg_path(Path::new("/art/logo.svg")));
        assert!(is_svg_path(Path::new("/art/logo.SVG")));
        assert!(is_svg_path(Path::new("/art/logo.svgz")));
        assert!(!is_svg_path(Path::new("/art/logo.png")));
        assert!(!is_svg_path(Path::new("/art/logo")));
    }

    #[test]
    fn a_small_drawing_is_enlarged_to_the_deck_rather_than_left_at_its_own_size() {
        // The whole reason to keep the tree: a 100-unit logo must not arrive as
        // a 100 px bitmap on a 1080p stage.
        assert_eq!(raster_size(&tree(SQUARE), 1920, 1080), (1080, 1080));
    }

    #[test]
    fn raster_size_keeps_the_drawings_shape() {
        // 4:1 artwork on a 16:9 deck is bounded by width, not stretched to fill.
        assert_eq!(raster_size(&tree(WIDE), 1920, 1080), (1920, 480));
    }

    #[test]
    fn raster_size_tracks_the_master_resolution() {
        let t = tree(SQUARE);
        assert_eq!(raster_size(&t, 3840, 2160), (2160, 2160));
        assert_eq!(raster_size(&t, 640, 360), (360, 360));
    }

    #[test]
    fn rasterize_fills_the_pixels_it_promised() {
        let img = rasterize(&tree(SQUARE), 256, 256).expect("rasterized");
        assert_eq!(img.dimensions(), (256, 256));
        let px = img.get_pixel(128, 128);
        assert_eq!(px.0[3], 255, "an opaque rect must rasterize opaque");
        assert!(px.0[0] > 200 && px.0[1] < 50, "expected red, got {px:?}");
    }

    #[test]
    fn transparent_areas_survive_as_straight_alpha() {
        // A half-covered canvas: the empty half must stay fully transparent and
        // not carry premultiplied colour into the deck's blit.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"
            width="100" height="100"><rect width="50" height="100" fill="#ffffff"/></svg>"##;
        let img = rasterize(&tree(svg), 100, 100).expect("rasterized");
        assert_eq!(
            img.get_pixel(90, 50).0[3],
            0,
            "uncovered area must be clear"
        );
        assert_eq!(img.get_pixel(10, 50).0, [255, 255, 255, 255]);
    }

    #[test]
    fn a_degenerate_drawing_does_not_produce_a_zero_sized_texture() {
        let t = tree(WIDE);
        let (w, h) = raster_size(&t, 1, 1);
        assert!(w >= 1 && h >= 1, "got {w}×{h}");
        assert!(rasterize(&t, 1, 1).is_ok());
    }

    #[test]
    fn a_parsed_tree_can_cross_thread_boundaries() {
        // Decks are built on a background loader thread and sent to the render
        // thread; holding the tree for re-rasterization only works if it is Send.
        fn assert_send<T: Send>() {}
        assert_send::<usvg::Tree>();
    }
}
