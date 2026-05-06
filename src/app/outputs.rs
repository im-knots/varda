//! Output window and surface action processing for VardaApp.

use super::VardaApp;
use crate::usecases::ui;
use crate::renderer::context::{OutputWindow, OutputTarget, SurfaceAssignment};

impl VardaApp {
    /// Apply output-related UI actions.
    pub fn apply_output_actions(&mut self, ui_actions: &ui::UIActions) {
        for action in &ui_actions.output_actions {
            match action {
                ui::OutputAction::Create => {
                    self.pending_output_creates.push(());
                }
                ui::OutputAction::Close { idx } => {
                    if *idx < self.output_windows.len() {
                        let name = self.output_windows[*idx].name.clone();
                        let output = self.output_windows.remove(*idx);
                        output.destroy();
                        log::info!("Closed output window '{}'", name);
                    }
                }
                ui::OutputAction::SetTarget { idx, target } => {
                    if let Some(output) = self.output_windows.get_mut(*idx) {
                        let monitor = match target {
                            OutputTarget::Display { monitor_index, .. } => {
                                self.cached_monitors.get(*monitor_index).map(|(_, h)| h.clone())
                            }
                            _ => None,
                        };
                        log::info!("Output '{}' target: {}", output.name, target);
                        output.set_target(target.clone(), monitor);
                    }
                }
                ui::OutputAction::AssignSurface { output_idx, surface_idx } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if !output.surface_assignments.iter().any(|a| a.surface_idx == *surface_idx) {
                            if let Some(surface) = self.surface_manager.surfaces.get(*surface_idx) {
                                let bb = surface.bounding_box();
                                let assignment = SurfaceAssignment {
                                    surface_idx: *surface_idx,
                                    warp_corners: [
                                        [bb.x, bb.y],
                                        [bb.x + bb.width, bb.y],
                                        [bb.x + bb.width, bb.y + bb.height],
                                        [bb.x, bb.y + bb.height],
                                    ],
                                    enabled: true,
                                };
                                log::info!("Assigned surface '{}' to output '{}'", surface.name, output.name);
                                output.surface_assignments.push(assignment);
                            }
                        }
                    }
                }
                ui::OutputAction::UnassignSurface { output_idx, assignment_idx } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if *assignment_idx < output.surface_assignments.len() {
                            output.surface_assignments.remove(*assignment_idx);
                            log::info!("Removed surface assignment from output '{}'", output.name);
                        }
                    }
                }
                ui::OutputAction::ToggleCalibration { idx } => {
                    if let Some(output) = self.output_windows.get_mut(*idx) {
                        output.calibration_mode = !output.calibration_mode;
                        log::info!("Output '{}' calibration mode: {}", output.name, output.calibration_mode);
                    }
                }
                ui::OutputAction::SetWarpCorner { output_idx, assignment_idx, corner_idx, position } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if let Some(assignment) = output.surface_assignments.get_mut(*assignment_idx) {
                            if *corner_idx < 4 {
                                assignment.warp_corners[*corner_idx] = *position;
                            }
                        }
                    }
                }
                ui::OutputAction::ResetWarp { output_idx, assignment_idx } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if let Some(assignment) = output.surface_assignments.get_mut(*assignment_idx) {
                            if let Some(surface) = self.surface_manager.surfaces.get(assignment.surface_idx) {
                                let bb = surface.bounding_box();
                                assignment.warp_corners = [
                                    [bb.x, bb.y],
                                    [bb.x + bb.width, bb.y],
                                    [bb.x + bb.width, bb.y + bb.height],
                                    [bb.x, bb.y + bb.height],
                                ];
                                log::info!("Reset warp for surface in output '{}'", output.name);
                            }
                        }
                    }
                }
            }
        }
    }


    /// Create pending output windows (deferred from UI actions).
    pub fn create_pending_outputs(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::window::Window;
        let pending: Vec<()> = self.pending_output_creates.drain(..).collect();
        for _ in pending {
            let idx = self.output_windows.len() + 1;
            let name = format!("Output {}", idx);
            let window_attrs = Window::default_attributes()
                .with_title(format!("Varda - {}", name))
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));

            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    let window_static: &'static Window = Box::leak(Box::new(window));
                    match OutputWindow::new(&self.context, window_static, name.clone()) {
                        Ok(output) => {
                            log::info!("Created output window '{}'", name);
                            self.output_windows.push(output);
                        }
                        Err(e) => {
                            log::error!("Failed to create output window: {}", e);
                            self.notifications.error(format!("Failed to create output: {}", e));
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to create output window: {}", e);
                    self.notifications.error(format!("Failed to create window: {}", e));
                }
            }
        }
    }
}