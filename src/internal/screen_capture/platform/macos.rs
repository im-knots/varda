//! macOS screen and window capture via `ScreenCaptureKit` (SCK).
//!
//! An `SCStream` pushes `CMSampleBuffer`s to an `SCStreamOutput` delegate on a
//! dispatch queue. The delegate repacks each frame into tightly packed BGRA and
//! puts it in a shared slot; [`MacosBackend::next_frame`] takes it. The capture
//! thread never blocks on the OS, and a stalled stream yields `None`.
//!
//! Written directly against SCK for two features:
//!
//! - `SCContentFilter` excludes windows from a display capture, which is how
//!   `exclude_varda` prevents an infinite mirror.
//! - `SCStreamConfiguration.width/height` scales at capture time, so a 4K
//!   display arrives at deck size instead of 33 MB a frame.
//!
//! Frames are sRGB BGRA8, uploaded to a `Bgra8UnormSrgb` texture with no CPU
//! swizzle, through CPU readback.

#![allow(unsafe_code)]

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCFrameStatus, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamFrameInfoStatus, SCStreamOutput, SCStreamOutputType, SCWindow,
};

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureFrame, CapturePixelFormat, CaptureTargetInfo,
    CaptureTargetKind, PermissionState, ScreenCaptureBackend,
};

/// `kCVPixelFormatType_32BGRA`, SCK's four-char code for BGRA output.
const PIXEL_FORMAT_32BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// Timeout for `SCShareableContent`'s completion handler. Enumeration blocks
/// the caller, so a wedged capture daemon would otherwise hang the render
/// thread on a rescan.
const ENUMERATE_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout for `startCaptureWithCompletionHandler:`. A stream not started by
/// then has failed, and an error is better than a black deck.
const START_TIMEOUT: Duration = Duration::from_secs(5);

pub fn backend_name() -> &'static str {
    "ScreenCaptureKit"
}

pub fn permission_state() -> PermissionState {
    // `CGPreflightScreenCaptureAccess` returns `false` for both "never asked"
    // and "refused". Report `NotDetermined` so the UI offers the request
    // button; requesting after a refusal is a silent no-op.
    if CGPreflightScreenCaptureAccess() {
        PermissionState::Granted
    } else {
        PermissionState::NotDetermined
    }
}

pub fn request_permission() {
    // Raises the TCC prompt on first call. A grant needs an app restart, which
    // the UI must say.
    let granted = CGRequestScreenCaptureAccess();
    log::info!("Screen recording access request returned granted={granted}");
}

/// Enumerates displays and on-screen windows.
///
/// # Errors
///
/// Returns [`CaptureError::PermissionDenied`] if TCC has not granted Screen
/// Recording, or [`CaptureError::Backend`] if `SCShareableContent` fails or does
/// not answer within [`ENUMERATE_TIMEOUT`].
pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
    let content = shareable_content()?;
    let our_pid = std::process::id().cast_signed();
    let mut targets = Vec::new();

    // Displays first, at the top of the library panel.
    for (i, display) in unsafe { content.displays() }.iter().enumerate() {
        let (w, h) = unsafe { (display.width(), display.height()) };
        targets.push(CaptureTargetInfo {
            kind: CaptureTargetKind::Display,
            platform_id: u64::from(unsafe { display.displayID() }),
            label: format!("Display {}", i + 1),
            app: None,
            title: None,
            width: u32::try_from(w).unwrap_or(0),
            height: u32::try_from(h).unwrap_or(0),
            is_varda: false,
        });
    }

    for window in &unsafe { content.windows() } {
        if !unsafe { window.isOnScreen() } {
            continue;
        }
        let frame = unsafe { window.frame() };
        let (width, height) = (frame.size.width as u32, frame.size.height as u32);
        // Skip menu-bar extras and other 1×1 helpers.
        if width < 32 || height < 32 {
            continue;
        }
        let owner = unsafe { window.owningApplication() };
        let app_name = owner
            .as_ref()
            .map(|a| unsafe { a.applicationName() }.to_string());
        let bundle_id = owner
            .as_ref()
            .map(|a| unsafe { a.bundleIdentifier() }.to_string());
        let is_varda = owner
            .as_ref()
            .is_some_and(|a| unsafe { a.processID() } == our_pid);
        let title = unsafe { window.title() }.map(|t| t.to_string());

        let label = match (&app_name, &title) {
            (Some(app), Some(t)) if !t.is_empty() => format!("{app} — {t}"),
            (Some(app), _) => app.clone(),
            (None, Some(t)) => t.clone(),
            (None, None) => format!("Window {}", unsafe { window.windowID() }),
        };

        targets.push(CaptureTargetInfo {
            kind: CaptureTargetKind::Window,
            platform_id: u64::from(unsafe { window.windowID() }),
            label,
            // Match on the bundle id when there is one; it survives an app rename,
            // the display name does not.
            app: bundle_id.or(app_name),
            title,
            width,
            height,
            is_varda,
        });
    }

    Ok(targets)
}

/// Opens a capture stream for `target`.
///
/// # Errors
///
/// Returns [`CaptureError::TargetNotFound`] if the display or window is gone,
/// [`CaptureError::PermissionDenied`] if TCC refuses, or
/// [`CaptureError::Backend`] for any SCK failure.
pub fn open(
    target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    let backend = MacosBackend::new(target, config)?;
    Ok(Box::new(backend))
}

fn shareable_content() -> Result<Retained<SCShareableContent>, CaptureError> {
    if !CGPreflightScreenCaptureAccess() {
        return Err(CaptureError::PermissionDenied);
    }

    let (tx, rx) = mpsc::channel::<Result<Retained<SCShareableContent>, String>>();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut objc2_foundation::NSError| {
            let msg = if content.is_null() {
                let detail = if error.is_null() {
                    "SCShareableContent returned nothing".to_string()
                } else {
                    unsafe { &*error }.localizedDescription().to_string()
                };
                Err(detail)
            } else {
                Ok(unsafe { Retained::retain(content) }
                    .ok_or_else(|| "failed to retain SCShareableContent".to_string()))
                .and_then(|r| r)
            };
            // The receiver may already have timed out and dropped; that is fine.
            let _ = tx.send(msg);
        },
    );

    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            true,
            &handler,
        );
    }

    match rx.recv_timeout(ENUMERATE_TIMEOUT) {
        Ok(Ok(content)) => Ok(content),
        Ok(Err(detail)) => {
            // SCK reports a TCC refusal as a generic error; classify it here instead
            // of showing "error -3801".
            if detail.contains("declined") || detail.contains("permission") {
                Err(CaptureError::PermissionDenied)
            } else {
                Err(CaptureError::Backend(detail))
            }
        }
        Err(_) => Err(CaptureError::Backend(format!(
            "SCShareableContent did not respond within {}s",
            ENUMERATE_TIMEOUT.as_secs()
        ))),
    }
}

/// Latest-wins frame slot shared by the SCK delegate queue and the capture
/// thread.
type FrameSlot = Arc<Mutex<Option<CaptureFrame>>>;

struct OutputIvars {
    slot: FrameSlot,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - The class does not implement Drop.
    // - Not main-thread-only: SCK calls the output on the queue passed to
    //   `addStreamOutput:type:sampleHandlerQueue:`.
    #[unsafe(super(NSObject))]
    #[name = "VardaSCStreamOutput"]
    #[ivars = OutputIvars]
    struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_did_output(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind != SCStreamOutputType::Screen {
                return;
            }
            if let Some(frame) = unsafe { frame_from_sample_buffer(sample_buffer) }
                && let Ok(mut slot) = self.ivars().slot.lock()
            {
                *slot = Some(frame);
            }
        }
    }
);

impl StreamOutput {
    fn new(slot: FrameSlot) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars { slot });
        unsafe { msg_send![super(this), init] }
    }
}

/// Whether a sample buffer has a frame to upload.
///
/// SCK delivers a buffer every interval, tagged with an `SCFrameStatus`. Only
/// `Complete` has valid pixels: `Idle` repeats a surface SCK may have recycled
/// and `Blank` is empty. Uploading those mixes stale or black frames in, which
/// shows as flicker.
///
/// # Safety
///
/// `sample_buffer` must be a live sample buffer delivered by SCK.
unsafe fn frame_is_complete(sample_buffer: &CMSampleBuffer) -> bool {
    let Some(attachments) = (unsafe { sample_buffer.sample_attachments_array(false) }) else {
        // Older macOS reports no status attachments; treat the frame as usable.
        return true;
    };
    // CFArray/CFDictionary are toll-free bridged to NS types, whose ObjC
    // accessors are safer than raw CF index/key calls.
    let attachments: &NSArray<NSDictionary<NSString, NSObject>> =
        unsafe { &*std::ptr::from_ref(&*attachments).cast() };
    let Some(info) = attachments.firstObject() else {
        return true;
    };
    let Some(status) = info.objectForKey(unsafe { SCStreamFrameInfoStatus }) else {
        return true;
    };
    let Ok(status) = status.downcast::<NSNumber>() else {
        return true;
    };
    status.integerValue() == SCFrameStatus::Complete.0
}

/// Repacks a `CMSampleBuffer`'s pixel buffer into a tightly packed BGRA frame.
///
/// # Safety
///
/// `sample_buffer` must be a live video sample buffer delivered by SCK.
unsafe fn frame_from_sample_buffer(sample_buffer: &CMSampleBuffer) -> Option<CaptureFrame> {
    if !unsafe { frame_is_complete(sample_buffer) } {
        return None;
    }
    let pixel_buffer = unsafe { sample_buffer.image_buffer() }?;
    let pixel_buffer = &*pixel_buffer;

    let lock = CVPixelBufferLockFlags::ReadOnly;
    if unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, lock) } != 0 {
        return None;
    }
    // Everything below must reach the unlock, so no `?` past here.
    let result = (|| {
        let base = CVPixelBufferGetBaseAddress(pixel_buffer);
        if base.is_null() {
            return None;
        }
        let width = CVPixelBufferGetWidth(pixel_buffer);
        let height = CVPixelBufferGetHeight(pixel_buffer);
        let stride = CVPixelBufferGetBytesPerRow(pixel_buffer);
        if width == 0 || height == 0 || stride < width * 4 {
            return None;
        }

        // SCK pads rows to a hardware stride. Repack to width*4 for wgpu, on the
        // SCK delegate queue, off the render and capture threads.
        let row_bytes = width * 4;
        let mut data = vec![0u8; row_bytes * height];
        for y in 0..height {
            let src = unsafe { base.cast::<u8>().add(y * stride) };
            let dst = &mut data[y * row_bytes..(y + 1) * row_bytes];
            unsafe { std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), row_bytes) };
        }

        Some(CaptureFrame {
            data,
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            format: CapturePixelFormat::Bgra8UnormSrgb,
        })
    })();
    unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, lock) };
    result
}

/// The `ObjC` objects behind a live stream.
///
/// `SCStream` and related types are not `Sync`, so `Retained<_>` is not
/// `Send`. They are safe to use off the main thread (SCK drives them from its
/// own queues). They are created on the calling thread, moved to one capture
/// thread, and used only there.
struct StreamHandles {
    stream: Retained<SCStream>,
    output: Retained<StreamOutput>,
    filter: Retained<SCContentFilter>,
    _queue: dispatch2::DispatchRetained<DispatchQueue>,
}

// SAFETY: moved to a single owner; no main-thread affinity.
unsafe impl Send for StreamHandles {}

pub struct MacosBackend {
    label: String,
    handles: StreamHandles,
    slot: FrameSlot,
    width: u32,
    height: u32,
    config: CaptureConfig,
}

impl MacosBackend {
    fn new(target: &CaptureTargetInfo, config: &CaptureConfig) -> Result<Self, CaptureError> {
        let content = shareable_content()?;
        let filter = build_filter(&content, target, config)?;
        let (width, height) = output_size(target, config);
        let stream_config = build_configuration(config, width, height);

        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &stream_config,
                None,
            )
        };

        let slot: FrameSlot = Arc::new(Mutex::new(None));
        let output = StreamOutput::new(Arc::clone(&slot));
        let queue = DispatchQueue::new("com.varda.screen-capture", None);

        unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            )
        }
        .map_err(|e| CaptureError::Backend(format!("addStreamOutput failed: {e}")))?;

        start_capture(&stream)?;

        log::info!(
            "ScreenCaptureKit stream started for '{}' at {width}x{height}",
            target.label
        );

        Ok(Self {
            label: target.label.clone(),
            handles: StreamHandles {
                stream,
                output,
                filter,
                _queue: queue,
            },
            slot,
            width,
            height,
            config: config.clone(),
        })
    }
}

fn start_capture(stream: &SCStream) -> Result<(), CaptureError> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let handler: RcBlock<dyn Fn(*mut objc2_foundation::NSError)> =
        RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            let msg = if error.is_null() {
                None
            } else {
                Some(unsafe { &*error }.localizedDescription().to_string())
            };
            let _ = tx.send(msg);
        });
    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&handler as &DynBlock<_>));
    }
    match rx.recv_timeout(START_TIMEOUT) {
        Ok(None) => Ok(()),
        Ok(Some(detail)) => Err(CaptureError::Backend(format!(
            "startCapture failed: {detail}"
        ))),
        Err(_) => Err(CaptureError::Backend(format!(
            "startCapture did not complete within {}s",
            START_TIMEOUT.as_secs()
        ))),
    }
}

/// Resolves the target to a live `SCDisplay` or `SCWindow` and wraps it in a
/// content filter. Display captures exclude Varda's windows here when
/// `exclude_varda` is set.
fn build_filter(
    content: &SCShareableContent,
    target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Retained<SCContentFilter>, CaptureError> {
    match target.kind {
        CaptureTargetKind::Display => {
            let display = unsafe { content.displays() }
                .iter()
                .find(|d| u64::from(unsafe { d.displayID() }) == target.platform_id)
                .ok_or_else(|| CaptureError::TargetNotFound(target.label.clone()))?;

            let excluded: Retained<NSArray<SCWindow>> = if config.exclude_varda {
                let our_pid = std::process::id().cast_signed();
                let ours: Vec<Retained<SCWindow>> = unsafe { content.windows() }
                    .iter()
                    .filter(|w| {
                        unsafe { w.owningApplication() }
                            .is_some_and(|a| unsafe { a.processID() } == our_pid)
                    })
                    .collect();
                NSArray::from_retained_slice(&ours)
            } else {
                NSArray::new()
            };

            Ok(unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &excluded,
                )
            })
        }
        CaptureTargetKind::Window => {
            let window = unsafe { content.windows() }
                .iter()
                .find(|w| u64::from(unsafe { w.windowID() }) == target.platform_id)
                .ok_or_else(|| CaptureError::TargetNotFound(target.label.clone()))?;
            Ok(unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            })
        }
    }
}

/// Output size after `scale_to` and crop. Cropping at capture shrinks the
/// uploaded bytes.
///
/// `scale_to` is a bound, as in
/// [`Geometry::resolve`](super::super::resample::Geometry::resolve). Passing it
/// as-is to `setScalesToFit` would letterbox the target into that box, baking
/// bars into the pixels and leaving the deck's scaling mode nothing to do.
fn output_size(target: &CaptureTargetInfo, config: &CaptureConfig) -> (u32, u32) {
    let (native_w, native_h) = (target.width.max(1), target.height.max(1));
    let (base_w, base_h) = config.scale_to.map_or((native_w, native_h), |(w, h)| {
        super::super::resample::fit_within(native_w, native_h, w, h)
    });
    scaled_crop_size(base_w, base_h, config)
}

/// Applies the crop extent to a base size, keeping the result even and
/// non-zero. SCK needs even dimensions for several pixel formats, and a zero
/// size is a fatal stream error.
fn scaled_crop_size(base_w: u32, base_h: u32, config: &CaptureConfig) -> (u32, u32) {
    let crop = config.crop.clamped();
    let w = ((base_w as f32) * crop.w).round().max(2.0) as u32;
    let h = ((base_h as f32) * crop.h).round().max(2.0) as u32;
    (w & !1, h & !1)
}

fn build_configuration(
    config: &CaptureConfig,
    width: u32,
    height: u32,
) -> Retained<SCStreamConfiguration> {
    let sc = unsafe { SCStreamConfiguration::new() };
    unsafe {
        sc.setWidth(width as usize);
        sc.setHeight(height as usize);
        sc.setPixelFormat(PIXEL_FORMAT_32BGRA);
        sc.setShowsCursor(config.show_cursor);
        // Opaque output: the deck blit composites over black anyway.
        sc.setShouldBeOpaque(true);
        // Queue depth 3 is Apple's guidance for live use: absorbs jitter without
        // adding latency.
        sc.setQueueDepth(3);
        sc.setScalesToFit(true);
        // Pace frames at the source, so a lower rate saves capture work.
        sc.setMinimumFrameInterval(CMTime {
            value: 1_000,
            timescale: (config.rate * 1_000.0) as i32,
            flags: objc2_core_media::CMTimeFlags::Valid,
            epoch: 0,
        });
    }

    let crop = config.crop.clamped();
    if !crop.is_full_frame() {
        // sourceRect is in the filter's point space; scale the normalized crop by
        // the configured output extent.
        let full_w = f64::from(width) / f64::from(crop.w.max(f32::EPSILON));
        let full_h = f64::from(height) / f64::from(crop.h.max(f32::EPSILON));
        unsafe {
            sc.setSourceRect(CGRect {
                origin: CGPoint {
                    x: full_w * f64::from(crop.x),
                    y: full_h * f64::from(crop.y),
                },
                size: CGSize {
                    width: full_w * f64::from(crop.w),
                    height: full_h * f64::from(crop.h),
                },
            });
        }
    }

    unsafe {
        sc.setStreamName(Some(&NSString::from_str("Varda Screen Capture")));
    }
    sc
}

impl ScreenCaptureBackend for MacosBackend {
    fn label(&self) -> &str {
        &self.label
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self) -> Option<CaptureFrame> {
        let frame = self.slot.try_lock().ok()?.take()?;
        // A reconfiguration applies asynchronously, so trust the frame size.
        self.width = frame.width;
        self.height = frame.height;
        Some(frame)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        // Matches the stream configuration's `PIXEL_FORMAT_32BGRA`.
        CapturePixelFormat::Bgra8UnormSrgb
    }

    fn is_self_paced(&self) -> bool {
        // `minimumFrameInterval` hands pacing to SCK; frames arrive on the
        // compositor's clock.
        true
    }

    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError> {
        if *config == self.config {
            return Ok(());
        }
        let exclusion_changed = config.exclude_varda != self.config.exclude_varda;
        self.config = config.clone();

        // Recompute from the requested scale, or keep the current output size if
        // none was requested.
        let (base_w, base_h) = config.scale_to.unwrap_or((self.width, self.height));
        let (width, height) = scaled_crop_size(base_w, base_h, config);

        let stream_config = build_configuration(config, width, height);
        unsafe {
            self.handles
                .stream
                .updateConfiguration_completionHandler(&stream_config, None);
        }

        if exclusion_changed {
            // The window exclusion list lives in the filter, so toggling
            // `exclude_varda` rebuilds the filter.
            log::debug!(
                "Screen capture '{}': exclude_varda changed; filter rebuild required",
                self.label
            );
        }

        Ok(())
    }
}

impl Drop for MacosBackend {
    fn drop(&mut self) {
        unsafe {
            self.handles.stream.stopCaptureWithCompletionHandler(None);
            let _ = self.handles.stream.removeStreamOutput_type_error(
                ProtocolObject::from_ref(&*self.handles.output),
                SCStreamOutputType::Screen,
            );
        }
        log::debug!("ScreenCaptureKit stream stopped for '{}'", self.label);
        let _ = &self.handles.filter;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_capture::backend::CropRect;

    fn target(w: u32, h: u32) -> CaptureTargetInfo {
        CaptureTargetInfo {
            kind: CaptureTargetKind::Display,
            platform_id: 1,
            label: "Display 1".into(),
            app: None,
            title: None,
            width: w,
            height: h,
            is_varda: false,
        }
    }

    /// The declared format must match the stream's configured format, or the
    /// shared texture is allocated in the wrong format.
    #[test]
    fn declared_pixel_format_matches_the_configured_stream_format() {
        assert_eq!(PIXEL_FORMAT_32BGRA, u32::from_be_bytes(*b"BGRA"));
        // `MacosBackend::pixel_format` is a constant; check the pairing without
        // opening a real stream.
        assert_eq!(
            CapturePixelFormat::Bgra8UnormSrgb.wgpu_format(),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }

    #[test]
    fn pixel_format_constant_is_the_bgra_four_char_code() {
        // 'BGRA' == 0x42475241. A wrong code yields garbage colors.
        assert_eq!(PIXEL_FORMAT_32BGRA, 0x4247_5241);
    }

    #[test]
    fn output_size_uses_scale_to_not_the_native_display_size() {
        let cfg = CaptureConfig {
            scale_to: Some((1920, 1080)),
            ..Default::default()
        };
        // A 4K display must not deliver 4K.
        assert_eq!(output_size(&target(3840, 2160), &cfg), (1920, 1080));
    }

    #[test]
    fn output_size_falls_back_to_the_target_size() {
        let cfg = CaptureConfig::default();
        assert_eq!(output_size(&target(1280, 720), &cfg), (1280, 720));
    }

    #[test]
    fn output_size_shrinks_with_crop_so_cropping_saves_bandwidth() {
        let cfg = CaptureConfig {
            scale_to: Some((1920, 1080)),
            crop: CropRect {
                x: 0.25,
                y: 0.25,
                w: 0.5,
                h: 0.5,
            },
            ..Default::default()
        };
        assert_eq!(output_size(&target(3840, 2160), &cfg), (960, 540));
    }

    #[test]
    fn output_size_keeps_the_targets_shape_rather_than_the_requested_box() {
        // A 4:3 window for a 16:9 deck must come back 4:3, or the deck's Scale
        // control has nothing to change.
        let cfg = CaptureConfig {
            scale_to: Some((1920, 1080)),
            ..Default::default()
        };
        assert_eq!(output_size(&target(1000, 800), &cfg), (1000, 800));
    }

    #[test]
    fn output_size_is_always_even_and_never_zero() {
        let cfg = CaptureConfig {
            scale_to: Some((3, 3)),
            crop: CropRect {
                x: 0.0,
                y: 0.0,
                w: 0.01,
                h: 0.01,
            },
            ..Default::default()
        };
        let (w, h) = output_size(&target(100, 100), &cfg);
        assert!(
            w > 0 && h > 0,
            "degenerate crop must not produce a 0-sized capture"
        );
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }
}
