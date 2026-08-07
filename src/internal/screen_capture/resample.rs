//! Crop/scale geometry and CPU resampling shared by the platform backends.
//!
//! macOS is the exception: `SCStreamConfiguration` crops and scales inside
//! `ScreenCaptureKit`, so a 4K display never crosses the process boundary at
//! full size. Windows Graphics Capture, X11, and `PipeWire` all hand back a
//! full-size surface, so the same crop-then-downscale has to happen here. One
//! implementation, so a fix to the filter is a fix on every platform.
//!
//! See spec/screen-capture.md § Platform Support.

use super::backend::CaptureConfig;

/// A resolved source rectangle and delivered output size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub src_x: u32,
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub out_w: u32,
    pub out_h: u32,
}

impl Geometry {
    /// Resolve a config against the target's native size.
    ///
    /// Crop selects a sub-rectangle of the captured surface, which genuinely
    /// shrinks the readback because backends copy only that region. `scale_to`
    /// then sets the delivered size, preserving the cropped aspect ratio so the
    /// deck's own scaling mode still has something sane to letterbox. Upscaling
    /// is refused: enlarging on the CPU costs bandwidth and adds no detail.
    pub fn resolve(native_w: u32, native_h: u32, config: &CaptureConfig) -> Self {
        let (nw, nh) = (native_w.max(1), native_h.max(1));
        let crop = config.crop.clamped();
        let src_x = ((nw as f32) * crop.x).round() as u32;
        let src_y = ((nh as f32) * crop.y).round() as u32;
        let src_w =
            (((nw as f32) * crop.w).round() as u32).clamp(1, nw.saturating_sub(src_x).max(1));
        let src_h =
            (((nh as f32) * crop.h).round() as u32).clamp(1, nh.saturating_sub(src_y).max(1));
        let (out_w, out_h) = config
            .scale_to
            .map_or((src_w, src_h), |(w, h)| fit_within(src_w, src_h, w, h));
        Self {
            src_x,
            src_y,
            src_w,
            src_h,
            out_w,
            out_h,
        }
    }

    /// Whether the resolved output already matches the source rectangle, in
    /// which case the caller can skip [`downscale`] entirely.
    pub fn is_identity_scale(self) -> bool {
        self.out_w == self.src_w && self.out_h == self.src_h
    }
}

/// Largest size with `w:h`'s aspect ratio that fits inside `max_w × max_h`,
/// never enlarging.
///
/// Also the right value to put in [`CaptureConfig::scale_to`] when opening a
/// capture for a deck: pass the target's native size and the deck's size. The
/// deck's size alone would be wrong, because backends treat `scale_to` as the
/// delivered extent — a window shaped differently from the stage would arrive
/// already fitted to the deck's aspect (letterboxed by `ScreenCaptureKit`, which
/// bakes the bars into the pixels), leaving the deck's own scaling mode with
/// identical source and target dimensions and therefore nothing to do.
pub fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    if max_w == 0 || max_h == 0 {
        return (w, h);
    }
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    if scale >= 1.0 {
        return (w, h);
    }
    (
        (((w as f32) * scale).round() as u32).max(1),
        (((h as f32) * scale).round() as u32).max(1),
    )
}

/// Box-filter downscale of a tightly-packed 4-bytes-per-pixel image.
///
/// Averaging rather than nearest-neighbour matters here: desktop content is
/// full of one-pixel text stems, and point sampling a 4K display down to deck
/// resolution makes them shimmer as the source scrolls. Channel order is
/// irrelevant, so this serves both RGBA and BGRA callers.
pub fn downscale(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let (dst_w, dst_h) = (dst_w.max(1), dst_h.max(1));
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    let x_ratio = src_w as f32 / dst_w as f32;
    let y_ratio = src_h as f32 / dst_h as f32;
    for y in 0..dst_h {
        let y0 = ((y as f32) * y_ratio) as u32;
        let y1 = ((((y + 1) as f32) * y_ratio) as u32).clamp(y0 + 1, src_h);
        for x in 0..dst_w {
            let x0 = ((x as f32) * x_ratio) as u32;
            let x1 = ((((x + 1) as f32) * x_ratio) as u32).clamp(x0 + 1, src_w);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for sy in y0..y1 {
                let row = (sy as usize) * (src_w as usize) * 4;
                for sx in x0..x1 {
                    let i = row + (sx as usize) * 4;
                    if i + 3 < src.len() {
                        acc[0] += u32::from(src[i]);
                        acc[1] += u32::from(src[i + 1]);
                        acc[2] += u32::from(src[i + 2]);
                        acc[3] += u32::from(src[i + 3]);
                        n += 1;
                    }
                }
            }
            let o = ((y as usize) * (dst_w as usize) + (x as usize)) * 4;
            let n = n.max(1);
            out[o] = (acc[0] / n) as u8;
            out[o + 1] = (acc[1] / n) as u8;
            out[o + 2] = (acc[2] / n) as u8;
            out[o + 3] = (acc[3] / n) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_capture::backend::CropRect;

    #[test]
    fn full_frame_config_resolves_to_the_native_rect() {
        let g = Geometry::resolve(1920, 1080, &CaptureConfig::default());
        assert_eq!((g.src_x, g.src_y, g.src_w, g.src_h), (0, 0, 1920, 1080));
        assert_eq!((g.out_w, g.out_h), (1920, 1080));
        assert!(g.is_identity_scale());
    }

    #[test]
    fn crop_selects_a_sub_rect_that_stays_inside_the_surface() {
        let cfg = CaptureConfig {
            crop: CropRect {
                x: 0.5,
                y: 0.5,
                w: 0.75,
                h: 0.75,
            },
            ..Default::default()
        };
        let g = Geometry::resolve(1000, 1000, &cfg);
        assert_eq!((g.src_x, g.src_y), (500, 500));
        assert!(g.src_x + g.src_w <= 1000, "crop ran past the right edge");
        assert!(g.src_y + g.src_h <= 1000, "crop ran past the bottom edge");
    }

    #[test]
    fn scale_to_preserves_the_cropped_aspect_ratio() {
        let cfg = CaptureConfig {
            scale_to: Some((640, 640)),
            ..Default::default()
        };
        let g = Geometry::resolve(1920, 1080, &cfg);
        assert_eq!((g.out_w, g.out_h), (640, 360));
    }

    #[test]
    fn scale_to_never_upscales() {
        let cfg = CaptureConfig {
            scale_to: Some((4096, 4096)),
            ..Default::default()
        };
        let g = Geometry::resolve(640, 480, &cfg);
        assert_eq!((g.out_w, g.out_h), (640, 480));
        assert!(g.is_identity_scale());
    }

    #[test]
    fn crop_shrinks_the_source_rect_so_the_readback_shrinks_with_it() {
        let cfg = CaptureConfig {
            crop: CropRect {
                x: 0.25,
                y: 0.5,
                w: 0.5,
                h: 0.5,
            },
            ..Default::default()
        };
        let g = Geometry::resolve(1920, 1080, &cfg);
        assert_eq!((g.src_x, g.src_y), (480, 540));
        assert_eq!((g.src_w, g.src_h), (960, 540));
        // This is the whole bandwidth argument: a crop must not read the full frame.
        assert!(g.src_w * g.src_h < 1920 * 1080);
    }

    #[test]
    fn scale_to_preserves_aspect_after_an_asymmetric_crop() {
        let cfg = CaptureConfig {
            crop: CropRect {
                x: 0.0,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            },
            scale_to: Some((1920, 1080)),
            ..Default::default()
        };
        let g = Geometry::resolve(3840, 2160, &cfg);
        let src_aspect = g.src_w as f32 / g.src_h as f32;
        let out_aspect = g.out_w as f32 / g.out_h as f32;
        assert!(
            (src_aspect - out_aspect).abs() < 0.01,
            "aspect drifted: {src_aspect} vs {out_aspect}"
        );
    }

    #[test]
    fn fit_within_keeps_a_windows_shape_instead_of_the_decks() {
        // The bug this guards: opening a 1000×800 window for a 16:9 deck used to
        // pass the deck size straight through as scale_to, so the capture came
        // back already fitted to 16:9 and the deck's scaling mode had identical
        // source and target dimensions — every mode collapsed to identity.
        let (w, h) = fit_within(1000, 800, 1920, 1080);
        let native_aspect = 1000.0 / 800.0;
        let capped_aspect = w as f32 / h as f32;
        assert!(
            (native_aspect - capped_aspect).abs() < 0.01,
            "capture must keep the window's shape, got {w}×{h}"
        );
        assert_ne!(
            (w, h),
            (1920, 1080),
            "a smaller window must not be blown up to the deck size"
        );
    }

    #[test]
    fn fit_within_caps_a_larger_target_to_the_deck() {
        // The bandwidth argument still has to hold: 4K must not arrive at 4K.
        assert_eq!(fit_within(3840, 2160, 1920, 1080), (1920, 1080));
        // An ultrawide is bounded by width and keeps its shape rather than
        // being squashed into the deck's 16:9.
        assert_eq!(fit_within(3440, 1440, 1920, 1080), (1920, 804));
    }

    #[test]
    fn fit_within_never_upscales_or_divides_by_zero() {
        assert_eq!(fit_within(640, 480, 4096, 4096), (640, 480));
        assert_eq!(fit_within(640, 480, 0, 0), (640, 480));
        assert_eq!(fit_within(0, 0, 1920, 1080), (1, 1));
    }

    #[test]
    fn geometry_is_never_degenerate() {
        let cfg = CaptureConfig {
            crop: CropRect {
                x: 0.0,
                y: 0.0,
                w: 0.0001,
                h: 0.0001,
            },
            ..Default::default()
        };
        let g = Geometry::resolve(1920, 1080, &cfg);
        assert!(g.src_w >= 1 && g.src_h >= 1);
        assert!(g.out_w >= 1 && g.out_h >= 1);
    }

    #[test]
    fn downscale_halving_averages_rather_than_dropping_pixels() {
        let src: Vec<u8> = vec![
            0, 0, 0, 255, 100, 100, 100, 255, //
            200, 200, 200, 255, 255, 255, 255, 255,
        ];
        let out = downscale(&src, 2, 2, 1, 1);
        assert_eq!(out.len(), 4);
        assert_eq!(u32::from(out[0]), (100 + 200 + 255) / 4);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn downscale_output_is_tightly_packed() {
        let src = vec![0u8; 8 * 8 * 4];
        let out = downscale(&src, 8, 8, 3, 5);
        assert_eq!(out.len(), 3 * 5 * 4, "manager requires width*4 rows");
    }

    #[test]
    fn downscale_never_reads_past_a_short_source() {
        // A truncated readback must not panic — the backend clamps and delivers
        // what it has rather than taking the whole app down.
        let src = vec![7u8; 4 * 4 * 4 / 2];
        let out = downscale(&src, 4, 4, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
    }
}
