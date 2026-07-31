//! Background surface-detection worker.
//!
//! Contour detection on a camera frame is far too slow for the render thread, so
//! it runs on a long-lived worker driven by a request/response channel pair. The
//! runner fires a request and picks the result up on a later frame.

/// Work item sent to the background detection thread.
pub(super) struct DetectRequest {
    pub(super) rgba: Vec<u8>,
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) params: crate::surface::detect::DetectionParams,
    /// When true, this is a capture (freeze-frame) request — the response
    /// triggers a transition to Preview mode rather than just updating overlays.
    pub(super) is_capture: bool,
    pub(super) camera_id: crate::camera::CameraId,
}

/// Result returned from the background detection thread.
pub(super) struct DetectResponse {
    pub(super) contours: Vec<crate::surface::detect::DetectedContour>,
    pub(super) is_capture: bool,
    pub(super) camera_id: crate::camera::CameraId,
}

/// Spawn a long-lived detection worker thread. It reads requests from `rx`,
/// runs detection (which is wrapped in `catch_unwind` inside `detect_from_rgba`),
/// and sends results back on the returned receiver.
pub(super) fn spawn_detect_thread(
    rx: std::sync::mpsc::Receiver<DetectRequest>,
) -> std::sync::mpsc::Receiver<DetectResponse> {
    let (tx, result_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("varda-detect".into())
        .spawn(move || {
            let mut consecutive_errors: u32 = 0;
            while let Ok(req) = rx.recv() {
                let contours = match crate::surface::import::detect_from_rgba(
                    &req.rgba,
                    req.w,
                    req.h,
                    &req.params,
                ) {
                    Ok(result) => {
                        consecutive_errors = 0;
                        result.contours
                    }
                    Err(e) => {
                        // Rate-limit error logging: log first, then every 60th
                        if !matches!(e, crate::surface::import::ImportError::NoContours) {
                            consecutive_errors += 1;
                            if consecutive_errors == 1 || consecutive_errors.is_multiple_of(60) {
                                log::warn!("Detection error (count={consecutive_errors}): {e}");
                            }
                        }
                        Vec::new()
                    }
                };
                if tx
                    .send(DetectResponse {
                        contours,
                        is_capture: req.is_capture,
                        camera_id: req.camera_id,
                    })
                    .is_err()
                {
                    break; // main thread dropped the receiver — exit
                }
            }
            log::info!("Detection worker thread exiting");
        })
        .expect("Failed to spawn detection thread");
    result_rx
}
