//! Camera surface-detection mode: opening and releasing the camera as the mode
//! changes, and applying the UI's detection actions.
//!
//! Detection itself runs on the worker in [`super::detect`]; this module owns the
//! mode's lifecycle and its egui texture.

use super::UIRunner;
use super::detect::DetectRequest;
use crate::engine::EngineCommand;
use crate::usecases::ui;

impl UIRunner {
    /// Ask the engine to hold the camera the current mode needs, and register
    /// its preview once the engine reports it open.
    ///
    /// Runs before this frame's command drain, so a request queued here is
    /// answered by the next call.
    pub(super) fn sync_camera_detect_capture(&mut self) {
        let wanted = match &self.layout.camera_detect_mode {
            ui::CameraDetectMode::Live { camera_id, .. }
            | ui::CameraDetectMode::Preview { camera_id, .. } => Some(*camera_id),
            ui::CameraDetectMode::Off => None,
        };
        let Some(varda) = self.varda.as_ref() else {
            return;
        };

        if wanted != self.camera_detect_camera_id {
            if let (Some(tex_id), Some(egui_renderer)) = (
                self.camera_detect_texture.take(),
                self.egui_renderer.as_mut(),
            ) {
                egui_renderer.free_texture(&tex_id);
            }
            self.queued_commands.push(match wanted {
                Some(camera_id) => EngineCommand::AcquireDetectionCamera { camera_id },
                None => EngineCommand::ReleaseDetectionCamera,
            });
            if wanted.is_none() {
                self.camera_detect_contours.clear();
            }
            self.camera_detect_camera_id = wanted;
            return;
        }

        let Some(cam_id) = wanted else {
            return;
        };
        if varda.detection_camera() != Some(cam_id) {
            // The engine refused the camera and has already told the operator.
            log::error!("Camera detection: camera {cam_id} could not be opened");
            self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
            return;
        }
        if self.camera_detect_texture.is_none()
            && let Some(tex_view) = varda.camera_manager().texture_view(cam_id)
            && let Some(egui_renderer) = self.egui_renderer.as_mut()
        {
            let tid = egui_renderer.register_native_texture(
                &varda.gpu_context().device,
                tex_view,
                wgpu::FilterMode::Linear,
            );
            self.camera_detect_texture = Some(tid);
        }
    }

    /// Apply the frame's queued camera-detection actions (enter, capture, accept,
    /// cancel), mutating the runner's mode and texture state.
    pub(super) fn apply_camera_detect_actions(&mut self, ui_actions: &mut ui::UIActions) {
        let actions = std::mem::take(&mut ui_actions.session.camera_detect_actions);
        for action in actions {
            match action {
                ui::CameraDetectAction::Enter { camera_id } => {
                    self.layout.camera_detect_mode = ui::CameraDetectMode::Live {
                        camera_id,
                        params: crate::surface::detect::DetectionParams::default(),
                    };
                }
                ui::CameraDetectAction::Exit => {
                    self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
                    // Camera release handled by lifecycle block on next frame
                }
                ui::CameraDetectAction::UpdateParams(params) => {
                    if let ui::CameraDetectMode::Live {
                        params: ref mut p, ..
                    } = self.layout.camera_detect_mode
                    {
                        *p = params.clone();
                        // Detection runs every frame in the lifecycle block — no need to run here
                    }
                }
                ui::CameraDetectAction::Capture => {
                    // Send a capture request to the background thread — the
                    // response (polled above) will transition to Preview mode.
                    if let ui::CameraDetectMode::Live {
                        camera_id,
                        ref params,
                    } = self.layout.camera_detect_mode
                        && let Some(varda) = &self.varda
                        && let Some(frame) = varda.camera_manager().snapshot_frame(camera_id)
                    {
                        let _ = self.detect_req_tx.send(DetectRequest {
                            rgba: frame.0,
                            w: frame.1,
                            h: frame.2,
                            params: params.clone(),
                            is_capture: true,
                            camera_id,
                        });
                        self.detect_in_flight = true;
                    }
                }
                ui::CameraDetectAction::ToggleContour(idx) => {
                    if let ui::CameraDetectMode::Preview {
                        ref mut selected, ..
                    } = self.layout.camera_detect_mode
                        && let Some(s) = selected.get_mut(idx)
                    {
                        *s = !*s;
                    }
                }
                ui::CameraDetectAction::SelectAll(val) => {
                    if let ui::CameraDetectMode::Preview {
                        ref mut selected, ..
                    } = self.layout.camera_detect_mode
                    {
                        selected.fill(val);
                    }
                }
                ui::CameraDetectAction::Accept => {
                    if let ui::CameraDetectMode::Preview {
                        ref contours,
                        ref selected,
                        ..
                    } = self.layout.camera_detect_mode
                    {
                        let chosen: Vec<_> = contours
                            .iter()
                            .zip(selected.iter())
                            .filter(|&(_, &s)| s)
                            .map(|(c, _)| c.clone())
                            .collect();
                        if !chosen.is_empty() {
                            ui_actions.commands.push(
                                crate::engine::EngineCommand::ConfirmDetectedContours {
                                    contours: chosen,
                                },
                            );
                        }
                    }
                    self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
                }
            }
        }
    }
}
