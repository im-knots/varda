//! X11 screen/window capture via `GetImage`.
//!
//! X11 has no event-driven capture API: nothing tells a client that the screen
//! changed, so the only way to get frames is to ask for them. The backend is
//! therefore polled — [`ScreenCaptureBackend::is_self_paced`] is `false` and the
//! capture loop drives it at `CaptureConfig.rate`. That is the opposite of every
//! other platform here, and it is why the CPU cost of an X11 capture scales
//! linearly with the configured rate.
//!
//! Crop is pushed down into the request rather than applied afterwards: a
//! `GetImage` for a sub-rectangle genuinely transfers fewer bytes. `scale_to` is
//! a CPU resample after the fact, so it saves GPU upload bandwidth but not X11
//! wire bandwidth.
//!
//! ## Known limitations, both inherent to X11
//!
//! - **`exclude_varda` is not honoured.** X11 has no compositing filter, so a
//!   display capture necessarily includes Varda's own windows. Pointing a deck
//!   at the display Varda is on will produce a feedback mirror. The capture rate
//!   defaulting below the render rate keeps that stable rather than seizing, but
//!   it cannot be prevented the way `SCContentFilter` prevents it on macOS.
//! - **Window captures read the window's front buffer.** Without the Composite
//!   extension redirecting the window to an offscreen pixmap, regions covered by
//!   another window come back as whatever is physically on screen there. Most
//!   modern desktops run a compositor, in which case this is a non-issue.
//!
//! See spec/screen-capture.md § Platform Support.

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, Drawable, ImageFormat, MapState, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureFrame, CapturePixelFormat, CaptureTargetInfo,
    CaptureTargetKind, ScreenCaptureBackend,
};
use crate::screen_capture::resample::{Geometry, downscale};

pub const BACKEND_NAME: &str = "X11";

/// Every plane. X11 lets a client mask off bit planes; we always want all of them.
const ALL_PLANES: u32 = !0;

/// Upper bound on a `_NET_CLIENT_LIST` read, in 32-bit words. A desktop with
/// more than this many managed windows is pathological, and an unbounded length
/// would let a hostile root property allocate arbitrarily.
const MAX_CLIENT_LIST_WORDS: u32 = 1024;

/// Upper bound on a text property read, in 32-bit words.
const MAX_TEXT_WORDS: u32 = 256;

/// Where the R, G and B bytes sit within one 32-bit `ZPixmap` pixel.
///
/// Derived from the visual's channel masks combined with the server's image
/// byte order, so a big-endian or unusually-masked server is repacked rather
/// than rendered with swapped channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteLayout {
    /// Bytes arrive as B, G, R, X. The overwhelmingly common case (little-endian
    /// server, `TrueColor` 24/32-bit visual) and the one that needs no swizzle.
    Bgrx,
    /// Anything else. Repacked to RGBA on the CPU using these byte offsets.
    Other { r: usize, g: usize, b: usize },
}

impl ByteLayout {
    pub fn format(self) -> CapturePixelFormat {
        match self {
            Self::Bgrx => CapturePixelFormat::Bgra8UnormSrgb,
            Self::Other { .. } => CapturePixelFormat::Rgba8UnormSrgb,
        }
    }
}

/// Byte offset of a single 8-bit channel within a 32-bit pixel.
///
/// Returns `None` for a mask that is not one byte-aligned octet, which rules out
/// 16-bit and paletted visuals — those are repacked by the caller's fallback
/// rather than mis-decoded.
fn channel_byte(mask: u32, msb_first: bool) -> Option<usize> {
    if mask == 0 {
        return None;
    }
    let shift = mask.trailing_zeros() as usize;
    if !shift.is_multiple_of(8) || (mask >> shift) != 0xFF {
        return None;
    }
    let index = shift / 8;
    Some(if msb_first { 3 - index } else { index })
}

/// Resolve the pixel layout of a `ZPixmap` from a visual's masks.
pub fn resolve_layout(red: u32, green: u32, blue: u32, msb_first: bool) -> ByteLayout {
    match (
        channel_byte(red, msb_first),
        channel_byte(green, msb_first),
        channel_byte(blue, msb_first),
    ) {
        (Some(2), Some(1), Some(0)) => ByteLayout::Bgrx,
        (Some(r), Some(g), Some(b)) => ByteLayout::Other { r, g, b },
        // A visual we cannot decode. Assume the common layout rather than
        // failing the capture outright: wrong colours beat a black deck, and
        // this is unreachable on any TrueColor desktop.
        _ => ByteLayout::Bgrx,
    }
}

/// Normalize a `GetImage` payload into tightly packed 4-byte pixels.
///
/// Alpha is forced opaque. A depth-24 drawable leaves the fourth byte
/// undefined, and uploading that as alpha makes a capture of an ordinary
/// desktop come out randomly transparent.
///
/// 32-bit `ZPixmap` rows need no unpadding: the scanline pad is 32 bits and a
/// row is already `width * 32` bits wide.
pub fn repack(data: &[u8], width: u32, height: u32, layout: ByteLayout) -> Vec<u8> {
    let pixels = (width as usize) * (height as usize);
    let mut out = vec![0u8; pixels * 4];
    match layout {
        ByteLayout::Bgrx => {
            let n = (pixels * 4).min(data.len());
            out[..n].copy_from_slice(&data[..n]);
            for px in out.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
        ByteLayout::Other { r, g, b } => {
            for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let base = i * 4;
                if base + 3 >= data.len() {
                    break;
                }
                px[0] = data[base + r];
                px[1] = data[base + g];
                px[2] = data[base + b];
                px[3] = 255;
            }
        }
    }
    out
}

fn connect() -> Result<(RustConnection, usize), CaptureError> {
    x11rb::connect(None)
        .map_err(|e| CaptureError::Backend(format!("cannot reach the X server: {e}")))
}

fn atom(conn: &RustConnection, name: &str) -> Option<Atom> {
    conn.intern_atom(true, name.as_bytes())
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
        .filter(|a| *a != 0)
}

/// Read a UTF-8 or Latin-1 text property.
fn text_property(conn: &RustConnection, window: Window, property: Atom) -> Option<String> {
    let reply = conn
        .get_property(false, window, property, AtomEnum::ANY, 0, MAX_TEXT_WORDS)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    // Latin-1 `WM_NAME` is not valid UTF-8, so fall back per byte instead of
    // dropping the title.
    let text = String::from_utf8(reply.value.clone())
        .unwrap_or_else(|_| reply.value.iter().map(|b| char::from(*b)).collect());
    let text = text.trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn window_title(conn: &RustConnection, window: Window) -> Option<String> {
    atom(conn, "_NET_WM_NAME")
        .and_then(|a| text_property(conn, window, a))
        .or_else(|| text_property(conn, window, AtomEnum::WM_NAME.into()))
}

/// `WM_CLASS` is two NUL-separated strings: instance name then class name. The
/// class is the stable, human-recognisable one ("Firefox", not "Navigator").
fn window_class(conn: &RustConnection, window: Window) -> Option<String> {
    let reply = conn
        .get_property(
            false,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            0,
            MAX_TEXT_WORDS,
        )
        .ok()?
        .reply()
        .ok()?;
    let mut parts = reply.value.split(|b| *b == 0);
    let instance = parts.next();
    let class = parts.next().filter(|s| !s.is_empty()).or(instance)?;
    let class = String::from_utf8_lossy(class).trim().to_string();
    (!class.is_empty()).then_some(class)
}

fn window_pid(conn: &RustConnection, window: Window) -> Option<u32> {
    let a = atom(conn, "_NET_WM_PID")?;
    let reply = conn
        .get_property(false, window, a, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32().and_then(|mut v| v.next())
}

/// Enumerate `RandR` monitors as displays and managed top-level windows.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if the X display cannot be reached.
pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
    let (conn, screen_num) = connect()?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let our_pid = std::process::id();
    let mut targets = Vec::new();

    for (i, monitor) in monitors(&conn, root).into_iter().enumerate() {
        targets.push(CaptureTargetInfo {
            kind: CaptureTargetKind::Display,
            // Index, not a RandR id: the identity that survives a restart is the
            // label, and `open` re-resolves by label with this as the fallback.
            platform_id: i as u64,
            label: monitor.label,
            app: None,
            title: None,
            width: u32::from(monitor.width),
            height: u32::from(monitor.height),
            is_varda: false,
        });
    }

    for window in client_list(&conn, root) {
        let Ok(Ok(attrs)) = conn
            .get_window_attributes(window)
            .map(x11rb::cookie::Cookie::reply)
        else {
            continue;
        };
        if attrs.map_state != MapState::VIEWABLE {
            continue;
        }
        let Ok(Ok(geom)) = conn.get_geometry(window).map(x11rb::cookie::Cookie::reply) else {
            continue;
        };
        // 1x1 keep-alive and IPC windows are managed but not capturable content.
        if geom.width < 2 || geom.height < 2 {
            continue;
        }
        let title = window_title(&conn, window);
        let app = window_class(&conn, window);
        let label = match (&app, &title) {
            (Some(app), Some(title)) => format!("{app} — {title}"),
            (Some(app), None) => app.clone(),
            (None, Some(title)) => title.clone(),
            (None, None) => format!("Window {window:#x}"),
        };
        targets.push(CaptureTargetInfo {
            kind: CaptureTargetKind::Window,
            platform_id: u64::from(window),
            label,
            app,
            title,
            width: u32::from(geom.width),
            height: u32::from(geom.height),
            is_varda: window_pid(&conn, window) == Some(our_pid),
        });
    }

    Ok(targets)
}

/// A `RandR` monitor, or the whole root window if `RandR` is unavailable.
struct MonitorRect {
    label: String,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

fn monitors(conn: &RustConnection, root: Window) -> Vec<MonitorRect> {
    let listed = conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|reply| {
            reply
                .monitors
                .into_iter()
                .enumerate()
                .map(|(i, m)| MonitorRect {
                    label: conn
                        .get_atom_name(m.name)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("Display {}", i + 1)),
                    x: m.x,
                    y: m.y,
                    width: m.width,
                    height: m.height,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !listed.is_empty() {
        return listed;
    }
    // No RandR (or a server that reports no active monitors): the root window is
    // the only display there is.
    conn.get_geometry(root)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|g| {
            vec![MonitorRect {
                label: "Display 1".to_string(),
                x: 0,
                y: 0,
                width: g.width,
                height: g.height,
            }]
        })
        .unwrap_or_default()
}

fn client_list(conn: &RustConnection, root: Window) -> Vec<Window> {
    let Some(a) = atom(conn, "_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    conn.get_property(false, root, a, AtomEnum::WINDOW, 0, MAX_CLIENT_LIST_WORDS)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(Iterator::collect))
        .unwrap_or_default()
}

/// Open a capture for `target`.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if the X display cannot be reached, or
/// [`CaptureError::TargetNotFound`] if the display or window has gone away
/// since enumeration.
pub fn open(
    target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    Ok(Box::new(X11Backend::new(target, config)?))
}

pub struct X11Backend {
    conn: RustConnection,
    label: String,
    drawable: Drawable,
    /// Whether the drawable is a window, whose size can change under us.
    is_window: bool,
    /// Origin of the captured area within the drawable. Non-zero for a monitor
    /// on a multi-head root window.
    origin_x: i16,
    origin_y: i16,
    native_w: u32,
    native_h: u32,
    layout: ByteLayout,
    geometry: Geometry,
    config: CaptureConfig,
}

impl X11Backend {
    fn new(target: &CaptureTargetInfo, config: &CaptureConfig) -> Result<Self, CaptureError> {
        let (conn, screen_num) = connect()?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let msb_first =
            conn.setup().image_byte_order == x11rb::protocol::xproto::ImageOrder::MSB_FIRST;
        let layout = root_visual_layout(&conn, screen_num, msb_first);

        let (drawable, is_window, origin_x, origin_y, native_w, native_h) = match target.kind {
            CaptureTargetKind::Display => {
                let all = monitors(&conn, root);
                // Match by label first: a rescan or a restart reorders the list,
                // and the label is what persistence stores.
                let monitor = all
                    .iter()
                    .find(|m| m.label == target.label)
                    .or_else(|| all.get(target.platform_id as usize))
                    .ok_or_else(|| CaptureError::TargetNotFound(target.label.clone()))?;
                (
                    Drawable::from(root),
                    false,
                    monitor.x,
                    monitor.y,
                    u32::from(monitor.width),
                    u32::from(monitor.height),
                )
            }
            CaptureTargetKind::Window => {
                let window = Window::try_from(target.platform_id)
                    .map_err(|_| CaptureError::TargetNotFound(target.label.clone()))?;
                let geom = conn
                    .get_geometry(Drawable::from(window))
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .ok_or_else(|| CaptureError::TargetNotFound(target.label.clone()))?;
                (
                    Drawable::from(window),
                    true,
                    0,
                    0,
                    u32::from(geom.width),
                    u32::from(geom.height),
                )
            }
        };

        let config = config.clone().sanitized();
        let geometry = Geometry::resolve(native_w, native_h, &config);
        log::info!(
            "X11 capture started for '{}' at {}x{} ({:?})",
            target.label,
            geometry.out_w,
            geometry.out_h,
            layout
        );
        Ok(Self {
            conn,
            label: target.label.clone(),
            drawable,
            is_window,
            origin_x,
            origin_y,
            native_w,
            native_h,
            layout,
            geometry,
            config,
        })
    }

    /// Re-read a window's size, which the user can change at any time, and
    /// re-resolve the crop against it.
    fn refresh_window_size(&mut self) -> bool {
        let Ok(Ok(geom)) = self
            .conn
            .get_geometry(self.drawable)
            .map(x11rb::cookie::Cookie::reply)
        else {
            return false;
        };
        let (w, h) = (u32::from(geom.width), u32::from(geom.height));
        if (w, h) != (self.native_w, self.native_h) {
            self.native_w = w;
            self.native_h = h;
            self.geometry = Geometry::resolve(w, h, &self.config);
        }
        true
    }
}

impl ScreenCaptureBackend for X11Backend {
    fn label(&self) -> &str {
        &self.label
    }

    fn resolution(&self) -> (u32, u32) {
        (self.geometry.out_w, self.geometry.out_h)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        self.layout.format()
    }

    fn next_frame(&mut self) -> Option<CaptureFrame> {
        if self.is_window && !self.refresh_window_size() {
            // The window was closed. The manager keeps the deck alive and
            // unbound rather than dropping it, so `None` is the right answer.
            return None;
        }
        let g = self.geometry;
        let left = self
            .origin_x
            .saturating_add(i16::try_from(g.src_x).unwrap_or(i16::MAX));
        let top = self
            .origin_y
            .saturating_add(i16::try_from(g.src_y).unwrap_or(i16::MAX));
        let width = u16::try_from(g.src_w).unwrap_or(u16::MAX);
        let height = u16::try_from(g.src_h).unwrap_or(u16::MAX);

        let reply = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.drawable,
                left,
                top,
                width,
                height,
                ALL_PLANES,
            )
            .ok()?
            .reply()
            .ok()?;
        if reply.depth < 24 {
            // A paletted or 16-bit drawable would need a colormap lookup we do
            // not implement. Report nothing rather than garbage.
            log::warn!(
                "X11 capture '{}': unsupported drawable depth {}",
                self.label,
                reply.depth
            );
            return None;
        }

        let packed = repack(
            &reply.data,
            u32::from(width),
            u32::from(height),
            self.layout,
        );
        let (data, width, height) = if g.is_identity_scale() {
            (packed, g.src_w, g.src_h)
        } else {
            (
                downscale(&packed, g.src_w, g.src_h, g.out_w, g.out_h),
                g.out_w,
                g.out_h,
            )
        };
        Some(CaptureFrame {
            data,
            width,
            height,
            format: self.layout.format(),
        })
    }

    fn is_self_paced(&self) -> bool {
        false
    }

    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError> {
        let config = config.clone().sanitized();
        if config.show_cursor != self.config.show_cursor && config.show_cursor {
            // Compositing the pointer needs XFixes `GetCursorImage` plus manual
            // alpha blending per frame. Not implemented; see spec/screen-capture.md.
            log::debug!(
                "X11 capture '{}': cursor overlay is not supported",
                self.label
            );
        }
        self.geometry = Geometry::resolve(self.native_w, self.native_h, &config);
        self.config = config;
        Ok(())
    }
}

/// Layout of the screen's root visual.
fn root_visual_layout(conn: &RustConnection, screen_num: usize, msb_first: bool) -> ByteLayout {
    let screen = &conn.setup().roots[screen_num];
    screen
        .allowed_depths
        .iter()
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.visual_id == screen.root_visual)
        .map_or(ByteLayout::Bgrx, |v| {
            resolve_layout(v.red_mask, v.green_mask, v.blue_mask, msb_first)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_endian_truecolor_needs_no_swizzle() {
        // The masks every mainstream X server reports for a 24/32-bit visual.
        assert_eq!(
            resolve_layout(0x00FF_0000, 0x0000_FF00, 0x0000_00FF, false),
            ByteLayout::Bgrx
        );
    }

    #[test]
    fn big_endian_server_is_repacked_rather_than_channel_swapped() {
        let layout = resolve_layout(0x00FF_0000, 0x0000_FF00, 0x0000_00FF, true);
        assert_eq!(layout, ByteLayout::Other { r: 1, g: 2, b: 3 });
        assert_eq!(layout.format(), CapturePixelFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn undecodable_visual_falls_back_to_the_common_layout() {
        // 16-bit 5-6-5: no byte-aligned channels.
        assert_eq!(
            resolve_layout(0xF800, 0x07E0, 0x001F, false),
            ByteLayout::Bgrx
        );
    }

    #[test]
    fn repack_forces_alpha_opaque_on_a_depth_24_drawable() {
        // Depth 24 leaves the fourth byte undefined; here it is zero, which
        // would render the whole capture transparent if trusted.
        let src = vec![10, 20, 30, 0, 40, 50, 60, 0];
        let out = repack(&src, 2, 1, ByteLayout::Bgrx);
        assert_eq!(out, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn repack_swizzles_a_big_endian_pixel_into_rgba() {
        // MSB-first ARGB in memory: X, R, G, B.
        let src = vec![0, 11, 22, 33];
        let out = repack(&src, 1, 1, ByteLayout::Other { r: 1, g: 2, b: 3 });
        assert_eq!(out, vec![11, 22, 33, 255]);
    }

    #[test]
    fn repack_tolerates_a_short_reply_without_panicking() {
        // A truncated GetImage must degrade, not take the render thread down.
        let src = vec![1, 2, 3, 4];
        let out = repack(&src, 4, 4, ByteLayout::Bgrx);
        assert_eq!(out.len(), 4 * 4 * 4);
        let out = repack(&src, 4, 4, ByteLayout::Other { r: 2, g: 1, b: 0 });
        assert_eq!(out.len(), 4 * 4 * 4);
    }

    #[test]
    fn repack_output_is_tightly_packed_at_four_bytes_per_pixel() {
        let src = vec![0u8; 7 * 5 * 4];
        assert_eq!(repack(&src, 7, 5, ByteLayout::Bgrx).len(), 7 * 5 * 4);
    }
}
