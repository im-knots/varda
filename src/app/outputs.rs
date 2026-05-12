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
                ui::OutputAction::SetEdgeBlend { output_idx, config } =>
                    EngineCommand::SetEdgeBlend { output_idx: *output_idx, config: *config },
                ui::OutputAction::SetEdgeBlendMode { output_idx, mode } =>
                    EngineCommand::SetEdgeBlendMode { output_idx: *output_idx, mode: *mode },
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
                                        warp_mode: a.warp_mode.clone(),
                                        enabled: a.enabled,
                                        overlap_zones: Default::default(),
                                    }
                                }).collect();
                                output.edge_blend_mode = config.edge_blend_mode;
                                output.edge_blend = config.edge_blend;
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
                        warp_mode: a.warp_mode.clone(),
                        enabled: a.enabled,
                        overlap_zones: Default::default(),
                    }
                }).collect();
                headless.edge_blend_mode = config.edge_blend_mode;
                headless.edge_blend = config.edge_blend;
                log::info!("Created headless output '{}'", name);
                self.outputs.push(UnifiedOutput::Headless(headless));
            }
        }
    }

    /// Recompute per-surface edge blend for all Auto-mode outputs based on surface topology.
    pub fn recompute_auto_edge_blend(&mut self) {
        use crate::renderer::edge_blend::{EdgeBlendMode, OutputSurfaceInfo, MappedRegion, SurfaceOverlapZones, compute_auto_edge_blend};

        // Check if any output is in Auto mode — early exit if none.
        let auto_count = self.outputs.iter().filter(|o| o.edge_blend_mode() == EdgeBlendMode::Auto).count();
        if auto_count == 0 {
            return;
        }
        log::debug!("[edge-blend] recompute_auto: {} outputs in Auto mode", auto_count);

        // Build OutputSurfaceInfo for each output (include surface_uuid in MappedRegion).
        let infos: Vec<OutputSurfaceInfo> = self.outputs.iter().enumerate().map(|(idx, output)| {
            let mut regions = Vec::new();
            for assignment in output.surface_assignments() {
                if let Some((_, surface)) = self.surface_manager.find_by_uuid(&assignment.surface_uuid) {
                    let bb = surface.bounding_box();
                    regions.push(MappedRegion {
                        source_key: format!("{:?}", surface.source),
                        bbox: [bb.x, bb.y, bb.width, bb.height],
                        surface_uuid: assignment.surface_uuid.clone(),
                        vertices: surface.vertices.clone(),
                        extra_contours: surface.extra_contours.clone(),
                    });
                }
            }
            let default_gamma = output.edge_blend().left.gamma;
            OutputSurfaceInfo {
                output_idx: idx,
                edge_blend_mode: output.edge_blend_mode(),
                default_gamma,
                regions,
            }
        }).collect();

        // Clear overlap zones on all Auto-mode assignments before applying new results.
        for output in self.outputs.iter_mut() {
            if output.edge_blend_mode() == EdgeBlendMode::Auto {
                for assignment in output.surface_assignments_mut() {
                    assignment.overlap_zones = SurfaceOverlapZones::default();
                }
            }
        }

        // Compute per-surface overlap zones and apply to assignments.
        let results = compute_auto_edge_blend(&infos);
        log::debug!("[edge-blend] computed {} results", results.len());
        for result in &results {
            log::debug!(
                "[edge-blend]   output={} surface={} zones={}",
                result.output_idx, result.surface_uuid,
                result.overlap_zones.zones.len(),
            );
        }
        for result in results {
            let output = &mut self.outputs[result.output_idx];
            for assignment in output.surface_assignments_mut() {
                if assignment.surface_uuid == result.surface_uuid {
                    assignment.overlap_zones = result.overlap_zones;
                    break;
                }
            }
        }
    }
}