//! Xbox Kinect v1 depth backend via `libfreenect` (the `freenectrs` crate).
//!
//! Gated behind the default-off `depth` cargo feature — this module only
//! compiles when `--features depth` is set and native `libfreenect` is present.
//! The rest of the depth subsystem builds without it via the mock backend.
//!
//! See spec/depth-sensors.md.

use super::backend::{DepthBackend, DepthFrame, DepthIntrinsics};
use anyhow::{Context, Result};
use freenectrs::freenect;

/// Kinect v1 medium (VGA) depth/colour resolution.
const KINECT_W: u32 = 640;
const KINECT_H: u32 = 480;

/// Wrapper asserting `Send` for the libfreenect context.
///
/// `FreenectContext` holds a raw `*mut freenect_context`, so it is not `Send`
/// by default. It is safe to move to and use from a single dedicated capture
/// thread because: (a) the `DepthSensorManager` creates exactly one
/// `FreenectBackend` per device and hands sole ownership to that device's
/// capture thread, (b) the context is never shared or accessed concurrently
/// from any other thread, and (c) libfreenect's own USB process thread is
/// spawned and managed internally by the context. See spec/depth-sensors.md.
struct SendContext(freenect::FreenectContext);
// SAFETY: sole-owner, single-thread access invariant documented above.
unsafe impl Send for SendContext {}

/// Kinect v1 backend. Owns a `FreenectContext` and the depth/video streams.
///
/// The context spawns libfreenect's own USB process thread; we `try_recv` the
/// latest depth (and optional RGB) frame each poll. Non-blocking by design so
/// the manager's capture loop never stalls.
pub struct FreenectBackend {
    ctx: SendContext,
    device_index: u32,
    name: String,
}

impl FreenectBackend {
    /// Open Kinect device `index` with depth (mm) + RGB video at VGA.
    pub fn open(index: u32) -> Result<Self> {
        let ctx = freenect::FreenectContext::init_with_video()
            .map_err(|e| anyhow::anyhow!("libfreenect init failed: {:?}", e))?;
        {
            let device = ctx
                .open_device(index)
                .map_err(|e| anyhow::anyhow!("open Kinect {} failed: {:?}", index, e))?;
            device
                .set_depth_mode(
                    freenect::FreenectResolution::Medium,
                    freenect::FreenectDepthFormat::MM,
                )
                .map_err(|e| anyhow::anyhow!("set_depth_mode failed: {:?}", e))?;
            device
                .set_video_mode(
                    freenect::FreenectResolution::Medium,
                    freenect::FreenectVideoFormat::Rgb,
                )
                .map_err(|e| anyhow::anyhow!("set_video_mode failed: {:?}", e))?;
            device
                .depth_stream()
                .map_err(|e| anyhow::anyhow!("depth_stream failed: {:?}", e))?;
            device
                .video_stream()
                .map_err(|e| anyhow::anyhow!("video_stream failed: {:?}", e))?;
        }
        ctx.spawn_process_thread()
            .map_err(|e| anyhow::anyhow!("spawn_process_thread failed: {:?}", e))?;
        Ok(Self {
            ctx: SendContext(ctx),
            device_index: index,
            name: format!("Kinect v1 (#{index})"),
        })
    }

    /// Enumerate connected Kinect devices without holding them open.
    pub fn enumerate() -> Result<u32> {
        let ctx = freenect::FreenectContext::init_with_video()
            .context("libfreenect init failed during enumeration")?;
        let n = ctx.num_devices().unwrap_or(0);
        Ok(n)
    }
}

impl DepthBackend for FreenectBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn intrinsics(&self) -> DepthIntrinsics {
        DepthIntrinsics::kinect_v1()
    }

    fn resolution(&self) -> (u32, u32) {
        (KINECT_W, KINECT_H)
    }

    fn next_frame(&mut self) -> Option<DepthFrame> {
        let device = self.ctx.0.open_device(self.device_index).ok()?;
        let dstream = device.depth_stream().ok()?;
        let (depth_raw, _ts) = dstream.receiver.try_recv().ok()?;

        // libfreenect MM depth is u16 mm packed as bytes; interpret as u16.
        let depth: Vec<u16> = depth_raw.to_vec();

        // RGB is optional — take it if a fresh frame is queued.
        let rgb = device
            .video_stream()
            .ok()
            .and_then(|vstream| vstream.receiver.try_recv().ok())
            .map(|(rgb_raw, _ts)| {
                let px = (KINECT_W * KINECT_H) as usize;
                let mut rgba = vec![0u8; px * 4];
                for i in 0..px {
                    let s = i * 3;
                    if s + 2 < rgb_raw.len() {
                        rgba[i * 4] = rgb_raw[s];
                        rgba[i * 4 + 1] = rgb_raw[s + 1];
                        rgba[i * 4 + 2] = rgb_raw[s + 2];
                        rgba[i * 4 + 3] = 255;
                    }
                }
                rgba
            });

        Some(DepthFrame {
            depth,
            rgb,
            width: KINECT_W,
            height: KINECT_H,
        })
    }
}
