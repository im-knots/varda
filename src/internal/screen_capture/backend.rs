//! Device-agnostic screen/window capture backend abstraction.
//!
//! A `ScreenCaptureBackend` produces RGBA/BGRA frames from an OS display or an
//! application window. The manager owns one backend per open capture and polls
//! it on a dedicated capture thread (see [`super::ScreenCaptureManager`]),
//! mirroring the `DepthBackend` / `CameraManager` split.
//!
//! Concrete backends live in [`super::platform`] and are selected by target OS
//! behind the default-on `screen-capture` cargo feature. A [`MockBackend`] is
//! always compiled so the manager, deck integration, persistence, API, and UI
//! can be built and tested with no display server and no permissions.
//!
//! See spec/screen-capture.md.

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

/// A capturable display or window, as reported by a platform enumeration.
///
/// `platform_id` is the OS handle (CoreGraphics display id, window number, …).
/// It is deliberately **not** persisted — handles are ephemeral across restarts,
/// so scenes match on `label` / `(app, title)` instead. See spec/screen-capture.md
/// § Configuration and Persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTargetInfo {
    pub kind: CaptureTargetKind,
    pub platform_id: u64,
    /// Human-readable name for UI and for display-target persistence matching.
    pub label: String,
    /// Owning application (bundle id or process name). Windows only.
    pub app: Option<String>,
    /// Window title at enumeration time. Windows only.
    pub title: Option<String>,
    pub width: u32,
    pub height: u32,
    /// This target belongs to Varda's own process.
    pub is_varda: bool,
}

impl CaptureTargetInfo {
    /// Stable-ish identity used to detect "this is the same target" across a
    /// rescan, and to match a persisted scene back onto a live target.
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

/// The persisted, handle-free identity of a capture target.
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
    /// Clamp into a valid sub-rectangle: origin in 0..1, extent positive, and
    /// `x + w <= 1` / `y + h <= 1`. A zero-or-negative extent collapses to the
    /// full frame rather than producing an empty capture.
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

/// Lowest and highest capture rates accepted from the parameter router.
pub const MIN_CAPTURE_RATE: f32 = 1.0;
pub const MAX_CAPTURE_RATE: f32 = 120.0;
/// Default capture rate. Deliberately below the render rate — see
/// spec/screen-capture.md § Self-Capture and Feedback Safety.
pub const DEFAULT_CAPTURE_RATE: f32 = 30.0;

/// Per-capture tunables. Shared with the capture thread and re-read each tick,
/// so router-driven changes take effect without restarting the session.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureConfig {
    pub rate: f32,
    pub crop: CropRect,
    pub show_cursor: bool,
    /// Exclude Varda's own windows from the capture. Display targets only.
    pub exclude_varda: bool,
    /// Ask the OS to scale at capture time. Set to the deck render resolution so
    /// a 4K display does not move 33 MB per frame.
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
    /// Normalize user- or router-supplied values into the accepted ranges.
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

    /// Frame interval implied by `rate`.
    pub fn frame_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.rate.max(MIN_CAPTURE_RATE))
    }
}

/// Pixel layout of a delivered frame. Backends report their native layout so
/// the manager can pick a matching texture format instead of swizzling on CPU.
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

/// Whether the platform will let us capture at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    /// Capture is permitted.
    Granted,
    /// The user explicitly refused. Requires a trip to system settings.
    Denied,
    /// Never asked. A capture attempt will raise the OS prompt.
    NotDetermined,
    /// This platform has no capture permission gate.
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
    /// The `screen-capture` feature is off, the manager is disabled, or this
    /// platform has no backend.
    Unavailable(String),
    /// The OS refused; the user must grant Screen Recording access.
    PermissionDenied,
    /// The requested display or window no longer exists.
    TargetNotFound(String),
    /// The platform backend failed for some other reason.
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

/// A live capture session. Implementations run on the capture thread and are
/// never touched from the render thread.
pub trait ScreenCaptureBackend: Send {
    /// Human-readable label of the captured target (logs / UI).
    fn label(&self) -> &str;
    /// Current output resolution `(width, height)`.
    fn resolution(&self) -> (u32, u32);
    /// Poll the next frame. `None` means no new frame is ready yet — for a
    /// static desktop that is the common case and is not an error.
    fn next_frame(&mut self) -> Option<CaptureFrame>;
    /// Native pixel layout of the frames this backend delivers.
    ///
    /// Declared rather than probed: a push-based backend has nothing ready at
    /// `open`, so probing there always guesses, allocates the shared texture in
    /// the wrong format, and throws it away on the first real frame. The
    /// manager still reallocates if a frame disagrees, so a wrong answer here
    /// costs a texture, not correctness.
    fn pixel_format(&self) -> CapturePixelFormat {
        CapturePixelFormat::Rgba8UnormSrgb
    }
    /// Whether the OS already paces delivery at [`CaptureConfig::rate`].
    ///
    /// Push-based backends (`ScreenCaptureKit`, Windows Graphics Capture,
    /// `PipeWire`) are handed the rate and deliver on the compositor's clock.
    /// The capture loop must then poll *faster* than the rate rather than
    /// matching it: two independent clocks of the same nominal frequency drift
    /// against each other, so ticks periodically find nothing while a delivered
    /// frame is overwritten before it is taken. The delivered cadence stutters
    /// even though the average rate looks correct, and in a self-capture
    /// feedback loop that irregularity is exactly what reads as flicker.
    ///
    /// Polled backends (X11, the mock) produce a frame only when asked, so the
    /// loop owns their pacing and this stays `false`.
    fn is_self_paced(&self) -> bool {
        false
    }
    /// Apply a live config change (rate, crop, cursor).
    ///
    /// # Errors
    ///
    /// Returns an error if the platform rejects the new configuration.
    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError>;
}

/// A synthetic capture backend, always compiled.
///
/// Emits a moving diagonal-bar pattern with a distinct solid marker in the
/// top-left texel so tests can assert *which* target they are looking at after a
/// scale or crop, not merely that some pixels arrived.
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
        // Derive a stable per-target marker colour from the label so two mock
        // captures in one test are distinguishable in the readback.
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

    /// Output size after `scale_to`, which is what a real backend would deliver.
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
                // Map into the cropped sub-rectangle so a crop visibly changes
                // the content rather than only the reported size.
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
        // Identity marker, top-left texel.
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

/// Synthetic targets reported when the mock provider is in use.
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
        // It synthesizes a frame per call, so the capture loop owns its rate.
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
        // Skip the marker texel (identical by construction) and compare content.
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
