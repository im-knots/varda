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
use freenectrs::freenect::{
    FreenectContext, FreenectDepthStream, FreenectDevice, FreenectVideoStream,
};

/// Kinect v1 medium (VGA) depth/colour resolution.
const KINECT_W: u32 = 640;
const KINECT_H: u32 = 480;

/// Kinect v1 backend. Owns a `FreenectContext`, device, and depth/video streams.
///
/// libfreenect's stream objects are self-referential: the device borrows the
/// context, and each stream borrows the device (see the `freenectrs` source).
/// A stream may only be opened *once* per device — `depth_stream()` errors on a
/// second call — and dropping a stream stops capture and clears its callback
/// sender. So the streams must be created exactly once and held for the whole
/// session, not re-opened per poll.
///
/// To store the borrowed streams in a struct we leak the context and device to
/// `'static` (via `Box::leak`), mirroring the crate's own `kinect_live`
/// example. The leak is bounded — one context + device per physical sensor ever
/// opened — and reclaimed by the OS on exit. On `Drop` we explicitly stop the
/// process thread so the capture thread's USB polling ends cleanly.
///
/// The context spawns libfreenect's own USB process thread; we `try_recv` the
/// latest depth (and optional RGB) frame each poll. Non-blocking by design so
/// the manager's capture loop never stalls.
pub struct FreenectBackend {
    /// Leaked context — kept so `Drop` can stop the process thread.
    ctx: &'static FreenectContext,
    /// Leaked device — kept alive for the streams that borrow it.
    #[allow(dead_code)]
    device: &'static FreenectDevice<'static, 'static>,
    dstream: FreenectDepthStream<'static, 'static>,
    vstream: FreenectVideoStream<'static, 'static>,
    name: String,
}

// SAFETY: `FreenectContext`/`FreenectDevice`/streams hold raw libfreenect
// pointers and are not `Send` by default. The `DepthSensorManager` creates
// exactly one `FreenectBackend` per device and hands sole ownership to that
// device's dedicated capture thread; the handles are never shared or accessed
// concurrently from any other thread. libfreenect's USB process thread is
// spawned and managed internally by the context. See spec/depth-sensors.md.
unsafe impl Send for FreenectBackend {}

impl FreenectBackend {
    /// Open Kinect device `index` with depth (mm) + RGB video at VGA.
    ///
    /// The device + both streams are opened once and stored; the process thread
    /// then feeds the stored streams' receivers for the session's lifetime.
    ///
    /// # Errors
    ///
    /// Returns an error if libfreenect fails to initialise, if no device with
    /// `index` is present, or if the depth/video streams cannot be started.
    pub fn open(index: u32) -> Result<Self> {
        // Leak the context to `'static` so the device (and thus the streams)
        // can be stored. Reclaimed on process exit; the process thread is
        // stopped via `Drop`.
        let ctx: &'static FreenectContext = Box::leak(Box::new(
            freenect::FreenectContext::init_with_video()
                .map_err(|e| anyhow::anyhow!("libfreenect init failed: {e:?}"))?,
        ));

        // Leak the device to `'static` so its borrowed streams can be stored.
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

        // Open each stream exactly once; hold them for the whole session.
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

    /// Enumerate connected Kinect devices without holding them open.
    ///
    /// # Errors
    ///
    /// Returns an error if libfreenect fails to initialise.
    pub fn enumerate() -> Result<u32> {
        let ctx = freenect::FreenectContext::init_with_video()
            .context("libfreenect init failed during enumeration")?;
        let n = ctx.num_devices().unwrap_or(0);
        Ok(n)
    }
}

impl Drop for FreenectBackend {
    fn drop(&mut self) {
        // Stop libfreenect's USB process thread. The context/device were leaked
        // to `'static`, so their `Drop` never runs — stop the thread explicitly
        // here so the capture thread's USB polling ends cleanly.
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
        // Poll the stored streams' receivers — the process thread feeds them.
        let (depth_raw, _ts) = self.dstream.receiver.try_recv().ok()?;

        // libfreenect MM depth is u16 mm packed as bytes; interpret as u16.
        let depth: Vec<u16> = depth_raw.to_vec();

        // RGB is optional — take it if a fresh frame is queued.
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
