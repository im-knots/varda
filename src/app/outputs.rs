//! Output action processing for VardaApp (unified windowed + headless).

use super::VardaApp;
use crate::usecases::ui;
use crate::renderer::context::{OutputWindow, OutputTarget, HeadlessOutput, UnifiedOutput, SurfaceAssignment, OutputSource};

impl VardaApp {
    /// Apply output-related UI actions.
    pub fn apply_output_actions(&mut self, ui_actions: &ui::UIActions) {
        use crate::engine::EngineCommand;
        for action in &ui_actions.output_actions {
            let cmd = match action {
                ui::OutputAction::Create => EngineCommand::CreateOutput,
                ui::OutputAction::CreateHeadless { target } =>
                    EngineCommand::CreateHeadlessOutput { target: target.clone() },
                ui::OutputAction::Close { idx } => EngineCommand::CloseOutput { idx: *idx },
                ui::OutputAction::SetTarget { idx, target } =>
                    EngineCommand::SetOutputTarget { idx: *idx, target: target.clone() },
                ui::OutputAction::Start { idx } => EngineCommand::StartOutput { idx: *idx },
                ui::OutputAction::Stop { idx } => EngineCommand::StopOutput { idx: *idx },
                ui::OutputAction::AssignSurface { output_idx, surface_uuid } =>
                    EngineCommand::AssignSurfaceToOutputByIdx { output_idx: *output_idx, surface_uuid: surface_uuid.clone() },
                ui::OutputAction::UnassignSurface { output_idx, assignment_idx } =>
                    EngineCommand::UnassignSurfaceFromOutputByIdx { output_idx: *output_idx, assignment_idx: *assignment_idx },
                ui::OutputAction::ToggleCalibration { idx } =>
                    EngineCommand::ToggleCalibration { idx: *idx },
                ui::OutputAction::SetWarpCorner { output_idx, assignment_idx, corner_idx, position } =>
                    EngineCommand::SetWarpCorner { output_idx: *output_idx, assignment_idx: *assignment_idx, corner_idx: *corner_idx, position: *position },
                ui::OutputAction::ResetWarp { output_idx, assignment_idx } =>
                    EngineCommand::ResetWarp { output_idx: *output_idx, assignment_idx: *assignment_idx },
            };
            self.execute_command(cmd);
        }
    }

    /// Create pending outputs (deferred from UI actions).
    /// Windowed/Display outputs need the event loop; headless outputs are created directly.
    pub fn create_pending_outputs(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::window::Window;

        let pending: Vec<crate::scene::OutputConfig> = self.pending_output_creates.drain(..).collect();
        for config in pending {
            let idx = self.outputs.len() + 1;
            let name = if config.name.is_empty() { format!("Output {}", idx) } else { config.name.clone() };
            let target = crate::persistence::config_to_target_pub(&config.target);

            if target.is_windowed() {
                // Windowed/Display: needs an OS window
                let mut window_attrs = Window::default_attributes()
                    .with_title(format!("Varda - {}", name));

                // Restore saved window size, or default to 1280x720
                if let Some([w, h]) = config.window_size {
                    window_attrs = window_attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
                } else {
                    window_attrs = window_attrs.with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
                }

                // Set position hint in attributes (works on some platforms)
                if let Some([x, y]) = config.window_position {
                    window_attrs = window_attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
                }

                match event_loop.create_window(window_attrs) {
                    Ok(window) => {
                        let window_static: &'static Window = Box::leak(Box::new(window));
                        match OutputWindow::new(&self.context, window_static, name.clone()) {
                            Ok(mut output) => {
                                output.uuid = config.uuid.clone();
                                // Force position after full initialization — macOS
                                // ignores with_position() in attrs and surface.configure()
                                // can reset position, so we set it last.
                                if let Some([x, y]) = config.window_position {
                                    output.window.set_outer_position(
                                        winit::dpi::PhysicalPosition::new(x, y),
                                    );
                                    log::info!("Restored output '{}' position to ({}, {})", output.name, x, y);
                                }
                                // Restore surface assignments from config
                                output.surface_assignments = config.surface_assignments.iter().map(|a| {
                                    SurfaceAssignment {
                                        surface_uuid: a.surface_uuid.clone(),
                                        warp_corners: a.warp_corners,
                                        enabled: a.enabled,
                                    }
                                }).collect();
                                // If Display target, set fullscreen — or fall back to
                                // Windowed if the target monitor is no longer connected.
                                if let OutputTarget::Display { ref name, .. } = target {
                                    if let Some((_, handle)) = self.cached_monitors.iter()
                                        .find(|(n, _)| n == name)
                                    {
                                        output.set_target(target.clone(), Some(handle.clone()));
                                    } else {
                                        log::warn!(
                                            "Monitor '{}' not available for output '{}' — falling back to windowed",
                                            name, output.name,
                                        );
                                        self.notifications.warn(format!(
                                            "Monitor '{}' not connected — output '{}' opened as window",
                                            name, output.name,
                                        ));
                                        output.set_target(OutputTarget::Windowed, None);
                                    }
                                }
                                log::info!("Created output window '{}'", output.name);
                                self.outputs.push(UnifiedOutput::Window(output));
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
            } else {
                // Headless output (Recording, SRT, NDI, Syphon)
                let mut headless = HeadlessOutput::new(
                    &self.context.device,
                    name.clone(),
                    OutputSource::Master,
                    target,
                    self.render_width,
                    self.render_height,
                );
                headless.uuid = config.uuid.clone();
                // Restore surface assignments from config
                headless.surface_assignments = config.surface_assignments.iter().map(|a| {
                    SurfaceAssignment {
                        surface_uuid: a.surface_uuid.clone(),
                        warp_corners: a.warp_corners,
                        enabled: a.enabled,
                    }
                }).collect();
                log::info!("Created headless output '{}'", name);
                self.outputs.push(UnifiedOutput::Headless(headless));
            }
        }
    }
}