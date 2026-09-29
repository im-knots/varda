//! Kinect v1 depth backend via `libfreenect` (the `freenectrs` crate).
//! Compiled only with the `depth` feature and native `libfreenect`.

use super::backend::{DepthBackend, DepthFrame, DepthIntrinsics};
use anyhow::{Context, Result};
use freenectrs::freenect;
use freenectrs::freenect::{
    FreenectContext, FreenectDepthStream, FreenectDevice, FreenectVideoStream,
};

/// Kinect v1 VGA depth and color resolution.
const KINECT_W: u32 = 640;
const KINECT_H: u32 = 480;

/// Kinect v1 backend. Holds a `FreenectContext`, device, and depth/video
/// streams.
///
/// The device borrows the context and each stream borrows the device. A stream
/// can be opened only once per device, and dropping it stops capture, so the
/// streams are opened once and held for the session. To store them, the
/// context and device are leaked to `'static` (as in the crate's `kinect_live`
/// example): one leak per sensor opened, reclaimed at exit. `Drop` stops the
/// USB process thread.
///
/// `next_frame` uses `try_recv`, so the capture loop never blocks.
pub struct FreenectBackend {
    /// Leaked; kept so `Drop` can stop the process thread.
    ctx: &'static FreenectContext,
    /// Leaked; kept alive for the streams that borrow it.
    #[allow(dead_code)]
    device: &'static FreenectDevice<'static, 'static>,
    dstream: FreenectDepthStream<'static, 'static>,
    vstream: FreenectVideoStream<'static, 'static>,
    name: String,
}

// SAFETY: the libfreenect handles hold raw pointers and are not `Send`. The
// `DepthSensorManager` moves each `FreenectBackend` to its device's capture
// thread, and nothing else accesses the handles. The context manages its own
// USB process thread.
unsafe impl Send for FreenectBackend {}

impl FreenectBackend {
    /// Opens Kinect `index` with depth (mm) and VGA RGB video. The device and
    /// both streams are opened once and kept for the session.
    ///
    /// # Errors
    ///
    /// Returns an error if libfreenect fails to initialize, no device `index`
    /// exists, or a stream cannot start.
    pub fn open(index: u32) -> Result<Self> {
        // Leaked so the device and streams can be stored; `Drop` stops the
        // process thread.
        let ctx: &'static FreenectContext = Box::leak(Box::new(
            freenect::FreenectContext::init_with_video()
                .map_err(|e| anyhow::anyhow!("libfreenect init failed: {e:?}"))?,
        ));

        let device: &'static FreenectDevice<'static, 'static> =
            Box::leak(Box::new(ctx.open_device(index).map_err(|e| {
                anyhow::anyhow!("open Kinect {index} failed: {e:?}")
            })?));

        device
            .set_depth_mode(
                freenect::FreenectResolution::Medium,
                freenect::FreenectDepthFormat::MM,
            )
            .map_err(|e| anyhow::anyhow!("set_depth_mode failed: {e:?}"))?;
        device
            .set_video_mode(
                freenect::FreenectResolution::Medium,
                freenect::FreenectVideoFormat::Rgb,
            )
            .map_err(|e| anyhow::anyhow!("set_video_mode failed: {e:?}"))?;

        // Open each stream once and hold it for the session.
        let dstream = device
            .depth_stream()
            .map_err(|e| anyhow::anyhow!("depth_stream failed: {e:?}"))?;
        let vstream = device
            .video_stream()
            .map_err(|e| anyhow::anyhow!("video_stream failed: {e:?}"))?;

        ctx.spawn_process_thread()
            .map_err(|e| anyhow::anyhow!("spawn_process_thread failed: {e:?}"))?;

        Ok(Self {
            ctx,
            device,
            dstream,
            vstream,
            name: format!("Kinect v1 (#{index})"),
        })
    }

    /// Counts connected Kinect devices without opening them.
    ///
    /// # Errors
    ///
    /// Returns an error if libfreenect fails to initialize.
    pub fn enumerate() -> Result<u32> {
        let ctx = freenect::FreenectContext::init_with_video()
            .context("libfreenect init failed during enumeration")?;
        let n = ctx.num_devices().unwrap_or(0);
        Ok(n)
    }
}

impl Drop for FreenectBackend {
    fn drop(&mut self) {
        // The leaked context never drops, so stop its USB process thread here.
        if let Err(e) = self.ctx.stop_process_thread() {
            log::warn!("freenect: stop_process_thread failed on drop: {e:?}");
        }
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
        let (depth_raw, _ts) = self.dstream.receiver.try_recv().ok()?;

        // libfreenect MM depth is u16 millimeters.
        let depth: Vec<u16> = depth_raw.to_vec();

        // RGB is optional; take a fresh frame if one is queued.
        let rgb = self.vstream.receiver.try_recv().ok().map(|(rgb_raw, _ts)| {
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
