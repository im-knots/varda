//! Device-agnostic depth-sensor backend abstraction.
//!
//! A `DepthBackend` produces depth frames (16-bit mm) and optional aligned RGBA
//! from a physical sensor. The manager owns one backend per open device and
//! polls it on a dedicated capture thread (see [`super::DepthSensorManager`]).
//!
//! The first concrete backend is `FreenectBackend` (Xbox Kinect v1 via
//! `libfreenect`), gated behind the default-off `depth` cargo feature. A
//! `MockBackend` is always compiled so the manager, deck integration, API, and
//! UI can be built and tested without the native library.
//!
//! See spec/depth-sensors.md.

/// Camera intrinsics used to deproject `(u, v, depth)` into camera-space XYZ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthIntrinsics {
    /// Focal length in pixels (x).
    pub fx: f32,
    /// Focal length in pixels (y).
    pub fy: f32,
    /// Principal point (x).
    pub cx: f32,
    /// Principal point (y).
    pub cy: f32,
    /// Multiply raw depth units by this to get metres.
    pub depth_scale_m: f32,
}

impl DepthIntrinsics {
    /// Default Kinect v1 intrinsics (VGA depth, values in mm → metres).
    pub fn kinect_v1() -> Self {
        Self {
            fx: 594.21,
            fy: 591.04,
            cx: 339.5,
            cy: 242.7,
            depth_scale_m: 0.001,
        }
    }
}

/// A single depth frame plus optional colour, as delivered by a backend.
pub struct DepthFrame {
    /// Depth in raw sensor units (typically mm), `width * height` values.
    pub depth: Vec<u16>,
    /// Optional RGBA colour aligned to the depth image, `width * height * 4`.
    pub rgb: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

/// Information about a detected depth device.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthDeviceInfo {
    pub id: super::DepthSensorId,
    pub name: String,
}

/// Device-agnostic depth sensor. Implementations run on the capture thread.
pub trait DepthBackend: Send {
    /// Human-readable device name (for logs / UI / persistence matching).
    fn name(&self) -> &str;
    /// Intrinsics for deprojecting this device's depth image.
    fn intrinsics(&self) -> DepthIntrinsics;
    /// Native depth resolution `(width, height)`.
    fn resolution(&self) -> (u32, u32);
    /// Poll the next frame. Returns `None` if no new frame is ready yet.
    fn next_frame(&mut self) -> Option<DepthFrame>;
}

/// A synthetic depth backend used for tests and for building/running without
/// the `depth` feature. Emits a moving radial depth gradient so the point-cloud
/// pass has something to render.
pub struct MockBackend {
    name: String,
    width: u32,
    height: u32,
    frame: u64,
}

impl MockBackend {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            width,
            height,
            frame: 0,
        }
    }
}

impl DepthBackend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn intrinsics(&self) -> DepthIntrinsics {
        DepthIntrinsics::kinect_v1()
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self) -> Option<DepthFrame> {
        self.frame = self.frame.wrapping_add(1);
        let (w, h) = (self.width, self.height);
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        let phase = (self.frame as f32) * 0.05;
        let mut depth = vec![0u16; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let r = (dx * dx + dy * dy).sqrt();
                // 800mm..3000mm ring that breathes with `phase`.
                let d = 800.0 + 1000.0 * (0.5 + 0.5 * (r * 0.02 - phase).sin());
                depth[(y * w + x) as usize] = d as u16;
            }
        }
        Some(DepthFrame {
            depth,
            rgb: None,
            width: w,
            height: h,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_backend_produces_frames_of_correct_size() {
        let mut b = MockBackend::new("Mock", 64, 48);
        assert_eq!(b.resolution(), (64, 48));
        let f = b.next_frame().expect("frame");
        assert_eq!(f.width, 64);
        assert_eq!(f.height, 48);
        assert_eq!(f.depth.len(), 64 * 48);
        assert!(f.rgb.is_none());
    }

    #[test]
    fn mock_backend_intrinsics_are_kinect_v1() {
        let b = MockBackend::new("Mock", 8, 8);
        assert_eq!(b.intrinsics(), DepthIntrinsics::kinect_v1());
    }
}
