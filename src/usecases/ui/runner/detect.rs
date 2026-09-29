//! Background surface-detection worker.
//!
//! Contour detection is too slow for the render thread, so it runs on a
//! long-lived worker; the runner reads the result on a later frame.

/// Work item sent to the background detection thread.
pub(super) struct DetectRequest {
    pub(super) rgba: Vec<u8>,
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) params: crate::surface::detect::DetectionParams,
    /// Capture (freeze-frame) request: the response switches to Preview mode
    /// instead of only updating overlays.
    pub(super) is_capture: bool,
    pub(super) camera_id: crate::camera::CameraId,
}

/// Result returned from the background detection thread.
pub(super) struct DetectResponse {
    pub(super) contours: Vec<crate::surface::detect::DetectedContour>,
    pub(super) is_capture: bool,
    pub(super) camera_id: crate::camera::CameraId,
}

/// Spawn the detection worker. It reads requests from `rx` and sends results
/// on the returned receiver. `detect_from_rgba` catches panics.
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
                        // Log the first error, then every 60th.
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
