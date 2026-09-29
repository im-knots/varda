//! Screen and window capture backend trait.
//!
//! A `ScreenCaptureBackend` produces RGBA/BGRA frames from a display or a
//! window. [`super::ScreenCaptureManager`] holds one backend per open capture
//! and polls it on a capture thread.
//!
//! Real backends live in [`super::platform`], chosen by target OS behind the
//! default-on `screen-capture` feature. [`MockBackend`] is always compiled so
//! the manager, decks, persistence, API and UI are testable without a display
//! server or permissions.

use std::fmt;

/// Whether a capture target is a whole display or a single window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTargetKind {
    Display,
    Window,
}

impl CaptureTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Window => "window",
        }
    }
}

/// A display or window from a platform enumeration.
///
/// `platform_id` is the OS handle (CoreGraphics display id, window number, …).
/// It is not persisted because handles change across restarts; scenes match on
/// `label` or `(app, title)` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTargetInfo {
    pub kind: CaptureTargetKind,
    pub platform_id: u64,
    /// Name for the UI and for matching saved display targets.
    pub label: String,
    /// Owning application (bundle id or process name). Windows only.
    pub app: Option<String>,
    /// Window title at enumeration time. Windows only.
    pub title: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Belongs to Varda's own process.
    pub is_varda: bool,
}

impl CaptureTargetInfo {
    /// Approximate identity for recognizing the same target across a rescan and
    /// matching a saved scene to a live target.
    pub fn identity(&self) -> TargetIdentity {
        match self.kind {
            CaptureTargetKind::Display => TargetIdentity::Display {
                label: self.label.clone(),
            },
            CaptureTargetKind::Window => TargetIdentity::Window {
                app: self.app.clone().unwrap_or_default(),
                title: self.title.clone().unwrap_or_default(),
            },
        }
    }
}

/// Handle-free identity of a capture target, as saved in scenes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetIdentity {
    Display { label: String },
    Window { app: String, title: String },
}

/// A normalized crop rectangle within the captured target (0.0–1.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for CropRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

impl CropRect {
    /// Clamps into a valid sub-rectangle: origin in 0..1, positive extent,
    /// `x + w <= 1` and `y + h <= 1`. A zero or negative extent becomes the full
    /// frame.
    #[must_use]
    pub fn clamped(self) -> Self {
        let x = self.x.clamp(0.0, 1.0);
        let y = self.y.clamp(0.0, 1.0);
        let w = if self.w <= 0.0 {
            1.0 - x
        } else {
            self.w.min(1.0 - x)
        };
        let h = if self.h <= 0.0 {
            1.0 - y
        } else {
            self.h.min(1.0 - y)
        };
        Self {
            x,
            y,
            w: w.max(f32::EPSILON),
            h: h.max(f32::EPSILON),
        }
    }

    pub fn is_full_frame(self) -> bool {
        self.x <= 0.0 && self.y <= 0.0 && self.w >= 1.0 && self.h >= 1.0
    }
}

/// Capture rate bounds accepted from the parameter router.
pub const MIN_CAPTURE_RATE: f32 = 1.0;
pub const MAX_CAPTURE_RATE: f32 = 120.0;
/// Default capture rate, kept below the render rate to limit self-capture
/// feedback.
pub const DEFAULT_CAPTURE_RATE: f32 = 30.0;

/// Per-capture settings. The capture thread re-reads them each tick, so
/// router changes apply without restarting the session.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureConfig {
    pub rate: f32,
    pub crop: CropRect,
    pub show_cursor: bool,
    /// Exclude Varda's own windows. Display targets only.
    pub exclude_varda: bool,
    /// Size the OS scales to at capture time. Set to the deck resolution so a 4K
    /// display does not move 33 MB per frame.
    pub scale_to: Option<(u32, u32)>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            rate: DEFAULT_CAPTURE_RATE,
            crop: CropRect::default(),
            show_cursor: false,
            exclude_varda: true,
            scale_to: None,
        }
    }
}

impl CaptureConfig {
    /// Clamps user or router values into the accepted ranges.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.rate = if self.rate.is_finite() {
            self.rate.clamp(MIN_CAPTURE_RATE, MAX_CAPTURE_RATE)
        } else {
            DEFAULT_CAPTURE_RATE
        };
        self.crop = self.crop.clamped();
        self
    }

    pub fn frame_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.rate.max(MIN_CAPTURE_RATE))
    }
}

/// Pixel layout of a delivered frame. Backends report their native layout so
/// the manager picks a matching texture format instead of swizzling on the CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturePixelFormat {
    Rgba8UnormSrgb,
    Bgra8UnormSrgb,
}

impl CapturePixelFormat {
    pub fn wgpu_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
        }
    }
}

/// One captured frame, tightly packed at `width * 4` bytes per row.
pub struct CaptureFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: CapturePixelFormat,
}

/// Whether the platform allows capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    /// The user refused. Needs a change in system settings.
    Denied,
    /// Never asked. A capture attempt raises the OS prompt.
    NotDetermined,
    /// The platform has no capture permission.
    NotRequired,
}

impl PermissionState {
    pub fn allows_capture(self) -> bool {
        matches!(self, Self::Granted | Self::NotRequired)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::NotDetermined => "not_determined",
            Self::NotRequired => "not_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// The `screen-capture` feature is off, the manager is disabled, or the
    /// platform has no backend.
    Unavailable(String),
    /// The OS refused; the user must grant Screen Recording access.
    PermissionDenied,
    /// The requested display or window no longer exists.
    TargetNotFound(String),
    /// Any other platform backend failure.
    Backend(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "screen capture unavailable: {why}"),
            Self::PermissionDenied => write!(
                f,
                "screen recording permission denied — grant access in System Settings and restart Varda"
            ),
            Self::TargetNotFound(what) => write!(f, "capture target not found: {what}"),
            Self::Backend(why) => write!(f, "screen capture backend error: {why}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// A live capture session. Runs on the capture thread only.
pub trait ScreenCaptureBackend: Send {
    /// Label of the captured target (logs, UI).
    fn label(&self) -> &str;
    /// Output resolution `(width, height)`.
    fn resolution(&self) -> (u32, u32);
    /// Polls the next frame. `None` means none is ready yet, which is normal for
    /// a static desktop.
    fn next_frame(&mut self) -> Option<CaptureFrame>;
    /// Native pixel layout of the delivered frames.
    ///
    /// Declared, not probed: a push-based backend has no frame at `open`. If a
    /// frame disagrees the manager reallocates the texture, so a wrong answer
    /// costs an allocation, not correctness.
    fn pixel_format(&self) -> CapturePixelFormat {
        CapturePixelFormat::Rgba8UnormSrgb
    }
    /// Whether the OS already paces delivery at [`CaptureConfig::rate`].
    ///
    /// Push-based backends (`ScreenCaptureKit`, Windows Graphics Capture,
    /// `PipeWire`) deliver on the compositor's clock. The capture loop must then
    /// poll faster than the rate: two clocks at the same nominal rate drift, so
    /// some ticks find nothing while other frames are overwritten before being
    /// taken. The result stutters, which shows as flicker in self-capture feedback.
    ///
    /// Polled backends (X11, the mock) produce a frame on request, so the loop
    /// paces them and this returns `false`.
    fn is_self_paced(&self) -> bool {
        false
    }
    /// Applies a live config change (rate, crop, cursor).
    ///
    /// # Errors
    ///
    /// Returns an error if the platform rejects the new configuration.
    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError>;
}

/// Synthetic capture backend, always compiled.
///
/// Draws moving diagonal bars plus a solid per-target marker in the top-left
/// texel, so tests can tell which target they see after a scale or crop.
pub struct MockBackend {
    label: String,
    width: u32,
    height: u32,
    config: CaptureConfig,
    frame: u64,
    marker: [u8; 3],
}

impl MockBackend {
    pub fn new(label: impl Into<String>, width: u32, height: u32, config: CaptureConfig) -> Self {
        let label = label.into();
        // Marker color derived from the label so two mock captures differ.
        let mut h: u32 = 2_166_136_261;
        for b in label.as_bytes() {
            h = (h ^ u32::from(*b)).wrapping_mul(16_777_619);
        }
        let marker = [
            (h & 0xFF) as u8 | 0x40,
            ((h >> 8) & 0xFF) as u8 | 0x40,
            ((h >> 16) & 0xFF) as u8 | 0x40,
        ];
        Self {
            label,
            width,
            height,
            config: config.sanitized(),
            frame: 0,
            marker,
        }
    }

    /// Output size after `scale_to`, as a real backend would deliver.
    fn output_size(&self) -> (u32, u32) {
        self.config
            .scale_to
            .map_or((self.width, self.height), |(w, h)| (w.max(1), h.max(1)))
    }

    pub fn marker(&self) -> [u8; 3] {
        self.marker
    }
}

impl ScreenCaptureBackend for MockBackend {
    fn label(&self) -> &str {
        &self.label
    }

    fn resolution(&self) -> (u32, u32) {
        self.output_size()
    }

    fn next_frame(&mut self) -> Option<CaptureFrame> {
        self.frame = self.frame.wrapping_add(1);
        let (width, height) = self.output_size();
        let crop = self.config.crop.clamped();
        let phase = (self.frame % 64) as f32 / 64.0;
        let mut data = vec![0u8; (width as usize) * (height as usize) * 4];
        for y in 0..height {
            for x in 0..width {
                // Map into the crop rectangle so a crop changes the content, not only the
                // size.
                let u = crop.x + (x as f32 / width as f32) * crop.w;
                let v = crop.y + (y as f32 / height as f32) * crop.h;
                let bar = ((u + v + phase) * 8.0).fract();
                let texel = ((y * width + x) * 4) as usize;
                data[texel] = (bar * 255.0) as u8;
                data[texel + 1] = (u * 255.0) as u8;
                data[texel + 2] = (v * 255.0) as u8;
                data[texel + 3] = 255;
            }
        }
        // Identity marker in the top-left texel.
        data[0] = self.marker[0];
        data[1] = self.marker[1];
        data[2] = self.marker[2];
        data[3] = 255;
        Some(CaptureFrame {
            data,
            width,
            height,
            format: CapturePixelFormat::Rgba8UnormSrgb,
        })
    }

    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError> {
        self.config = config.clone().sanitized();
        Ok(())
    }
}

/// Targets reported by the mock provider.
pub fn mock_targets() -> Vec<CaptureTargetInfo> {
    vec![
        CaptureTargetInfo {
            kind: CaptureTargetKind::Display,
            platform_id: 1,
            label: "Mock Display 1".into(),
            app: None,
            title: None,
            width: 1920,
            height: 1080,
            is_varda: false,
        },
        CaptureTargetInfo {
            kind: CaptureTargetKind::Display,
            platform_id: 2,
            label: "Mock Display 2".into(),
            app: None,
            title: None,
            width: 1280,
            height: 720,
            is_varda: false,
        },
        CaptureTargetInfo {
            kind: CaptureTargetKind::Window,
            platform_id: 100,
            label: "Mock App — Untitled".into(),
            app: Some("com.example.mock".into()),
            title: Some("Untitled".into()),
            width: 800,
            height: 600,
            is_varda: false,
        },
        CaptureTargetInfo {
            kind: CaptureTargetKind::Window,
            platform_id: 101,
            label: "Varda — Main".into(),
            app: Some("com.varda.app".into()),
            title: Some("Main".into()),
            width: 1920,
            height: 1080,
            is_varda: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_clamps_into_the_unit_square() {
        let c = CropRect {
            x: -0.5,
            y: 0.2,
            w: 5.0,
            h: 0.5,
        }
        .clamped();
        assert!((c.x - 0.0).abs() < f32::EPSILON);
        assert!((c.y - 0.2).abs() < f32::EPSILON);
        // w is capped so x + w never exceeds 1.0.
        assert!(c.x + c.w <= 1.0 + f32::EPSILON);
        assert!(c.y + c.h <= 1.0 + f32::EPSILON);
    }

    #[test]
    fn zero_extent_crop_collapses_to_full_frame_not_empty() {
        let c = CropRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: -1.0,
        }
        .clamped();
        assert!(c.w > 0.9, "zero width should expand, got {}", c.w);
        assert!(c.h > 0.9, "negative height should expand, got {}", c.h);
    }

    #[test]
    fn config_sanitize_clamps_rate_and_rejects_nan() {
        assert!(
            (CaptureConfig {
                rate: 1000.0,
                ..Default::default()
            }
            .sanitized()
            .rate
                - MAX_CAPTURE_RATE)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (CaptureConfig {
                rate: 0.0,
                ..Default::default()
            }
            .sanitized()
            .rate
                - MIN_CAPTURE_RATE)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (CaptureConfig {
                rate: f32::NAN,
                ..Default::default()
            }
            .sanitized()
            .rate
                - DEFAULT_CAPTURE_RATE)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn default_config_excludes_varda_and_hides_cursor() {
        let c = CaptureConfig::default();
        assert!(
            c.exclude_varda,
            "display captures must exclude Varda by default"
        );
        assert!(!c.show_cursor);
    }

    #[test]
    fn mock_backend_frame_matches_reported_resolution() {
        let mut b = MockBackend::new("Mock Display 1", 1920, 1080, CaptureConfig::default());
        assert_eq!(b.resolution(), (1920, 1080));
        let f = b.next_frame().expect("frame");
        assert_eq!((f.width, f.height), (1920, 1080));
        assert_eq!(f.data.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn mock_backend_honours_scale_to() {
        let cfg = CaptureConfig {
            scale_to: Some((320, 180)),
            ..Default::default()
        };
        let mut b = MockBackend::new("Mock", 1920, 1080, cfg);
        assert_eq!(b.resolution(), (320, 180));
        let f = b.next_frame().expect("frame");
        assert_eq!((f.width, f.height), (320, 180));
        assert_eq!(f.data.len(), 320 * 180 * 4);
    }

    #[test]
    fn mock_backend_is_polled_not_self_paced() {
        // It makes a frame per call, so the capture loop sets its rate.
        let b = MockBackend::new("Mock", 8, 8, CaptureConfig::default());
        assert!(!b.is_self_paced());
    }

    #[test]
    fn mock_backend_marker_is_stable_per_label_and_distinct_across_labels() {
        let a = MockBackend::new("Display A", 8, 8, CaptureConfig::default());
        let a2 = MockBackend::new("Display A", 8, 8, CaptureConfig::default());
        let b = MockBackend::new("Display B", 8, 8, CaptureConfig::default());
        assert_eq!(a.marker(), a2.marker());
        assert_ne!(a.marker(), b.marker());
    }

    #[test]
    fn crop_changes_frame_content_not_just_size() {
        let full = MockBackend::new("M", 64, 64, CaptureConfig::default())
            .next_frame()
            .expect("frame");
        let cropped = MockBackend::new(
            "M",
            64,
            64,
            CaptureConfig {
                crop: CropRect {
                    x: 0.5,
                    y: 0.5,
                    w: 0.5,
                    h: 0.5,
                },
                ..Default::default()
            },
        )
        .next_frame()
        .expect("frame");
        assert_eq!(full.data.len(), cropped.data.len());
        // Skip the marker texel (identical by design) and compare content.
        assert_ne!(&full.data[4..], &cropped.data[4..]);
    }

    #[test]
    fn permission_state_gates_capture() {
        assert!(PermissionState::Granted.allows_capture());
        assert!(PermissionState::NotRequired.allows_capture());
        assert!(!PermissionState::Denied.allows_capture());
        assert!(!PermissionState::NotDetermined.allows_capture());
    }

    #[test]
    fn target_identity_ignores_ephemeral_platform_handles() {
        let a = CaptureTargetInfo {
            kind: CaptureTargetKind::Window,
            platform_id: 7,
            label: "X".into(),
            app: Some("com.a".into()),
            title: Some("T".into()),
            width: 1,
            height: 1,
            is_varda: false,
        };
        let b = CaptureTargetInfo {
            platform_id: 999_999,
            ..a.clone()
        };
        assert_eq!(a.identity(), b.identity());
    }

    #[test]
    fn mock_targets_include_a_varda_owned_window() {
        let t = mock_targets();
        assert!(t.iter().any(|t| t.is_varda));
        assert!(t.iter().any(|t| t.kind == CaptureTargetKind::Display));
        assert!(t.iter().any(|t| t.kind == CaptureTargetKind::Window));
    }
}
