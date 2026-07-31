//! Camera surface-detection mode: opening and releasing the camera as the mode
//! changes, and applying the UI's detection actions.
//!
//! Detection itself runs on the worker in [`super::detect`]; this module owns the
//! mode's lifecycle and its egui texture.

use super::detect::DetectRequest;
use super::UIRunner;
use crate::usecases::ui;

impl UIRunner {
    /// Open or release the detection camera to match the current mode, and push a
    /// frame to the detection worker when one is due.
    pub(super) fn sync_camera_detect_capture(&mut self) {
        let detect_camera_id = match &self.layout.camera_detect_mode {
            ui::CameraDetectMode::Live { camera_id, .. }
            | ui::CameraDetectMode::Preview { camera_id, .. } => Some(*camera_id),
            ui::CameraDetectMode::Off => None,
        };

        if let (Some(cam_id), Some(varda)) = (detect_camera_id, self.varda.as_mut()) {
            if self.camera_detect_camera_id != Some(cam_id) {
                // Release previous camera if switching
                if let Some(prev_id) = self.camera_detect_camera_id.take() {
                    varda.camera_manager_mut().release_camera(prev_id);
                    if let (Some(tex_id), Some(egui_renderer)) = (
                        self.camera_detect_texture.take(),
                        self.egui_renderer.as_mut(),
                    ) {
                        egui_renderer.free_texture(&tex_id);
                    }
                }
                // Open new camera (uses convenience method to avoid split-borrow)
                match varda.open_camera(cam_id) {
                    Ok(_res) => {
                        if let Some(tex_view) = varda.camera_manager().texture_view(cam_id) {
                            let context = varda.gpu_context();
                            if let Some(egui_renderer) = self.egui_renderer.as_mut() {
                                let tid = egui_renderer.register_native_texture(
                                    &context.device,
                                    tex_view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.camera_detect_texture = Some(tid);
                            }
                        }
                        self.camera_detect_camera_id = Some(cam_id);
                        log::info!("Camera detection: opened camera {cam_id}");
                    }
                    Err(e) => {
                        log::error!("Camera detection: failed to open camera {cam_id}: {e}");
                        self.layout.camera_detect_mode = ui::CameraDetectMode::Off;
                    }
                }
            }
        } else if detect_camera_id.is_none() && self.camera_detect_camera_id.is_some() {
            // Mode is Off — release camera
            if let Some(prev_id) = self.camera_detect_camera_id.take() {
                if let Some(varda) = self.varda.as_mut() {
                    varda.camera_manager_mut().release_camera(prev_id);
                }
                if let (Some(tex_id), Some(egui_renderer)) = (
                    self.camera_detect_texture.take(),
                    self.egui_renderer.as_mut(),
                ) {
                    egui_renderer.free_texture(&tex_id);
                }
            }
            self.camera_detect_contours.clear();
        }
    }

    /// Apply the frame's queued camera-detection actions (enter, capture, accept,
    /// cancel), mutating the runner's mode and texture state.
    pub(super) fn apply_camera_detect_actions(&mut self, ui_actions: &mut ui::UIActions) {
        let actions: Vec<_> = ui_actions.session.camera_detect_actions.drain(..).collect();
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
                    {
                        if let Some(varda) = &self.varda {
                            if let Some(frame) = varda.camera_manager().snapshot_frame(camera_id) {
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
                    }
                }
                ui::CameraDetectAction::ToggleContour(idx) => {
                    if let ui::CameraDetectMode::Preview {
                        ref mut selected, ..
                    } = self.layout.camera_detect_mode
                    {
                        if let Some(s) = selected.get_mut(idx) {
                            *s = !*s;
                        }
                    }
                }
                ui::CameraDetectAction::SelectAll(val) => {
                    if let ui::CameraDetectMode::Preview {
                        ref mut selected, ..
                    } = self.layout.camera_detect_mode
                    {
                        for s in &mut *selected {
                            *s = val;
                        }
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
                            .filter(|(_, &s)| s)
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
