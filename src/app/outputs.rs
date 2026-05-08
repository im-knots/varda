//! Output action processing for VardaApp (unified windowed + headless).

use super::VardaApp;
use crate::usecases::ui;
use crate::renderer::context::{OutputWindow, OutputTarget, HeadlessOutput, UnifiedOutput, SurfaceAssignment, OutputSource};

impl VardaApp {
    /// Apply output-related UI actions.
    pub fn apply_output_actions(&mut self, ui_actions: &ui::UIActions) {
        for action in &ui_actions.output_actions {
            match action {
                ui::OutputAction::Create => {
                    self.pending_output_creates.push(crate::scene::OutputConfig::default_windowed());
                }
                ui::OutputAction::CreateHeadless { target } => {
                    let idx = self.outputs.len() + 1;
                    let name = format!("Output {}", idx);
                    let headless = HeadlessOutput::new(
                        &self.context.device,
                        name.clone(),
                        OutputSource::Master,
                        target.clone(),
                        self.render_width,
                        self.render_height,
                    );
                    log::info!("Created headless output '{}' → {}", name, target);
                    self.outputs.push(UnifiedOutput::Headless(headless));
                }
                ui::OutputAction::Close { idx } => {
                    if *idx < self.outputs.len() {
                        let name = self.outputs[*idx].name().to_string();
                        // Stop active subprocess before removing to release ports/resources
                        if let UnifiedOutput::Headless(h) = &mut self.outputs[*idx] {
                            if let Some(mut sub) = h.subprocess.take() {
                                sub.stop();
                            }
                        }
                        let removed = self.outputs.remove(*idx);
                        if let UnifiedOutput::Window(w) = removed {
                            w.destroy();
                        }
                        log::info!("Closed output '{}'", name);
                    }
                }
                ui::OutputAction::SetTarget { idx, target } => {
                    if let Some(output) = self.outputs.get_mut(*idx) {
                        match output {
                            UnifiedOutput::Window(w) => {
                                // Windowed targets stay windowed
                                if target.is_windowed() {
                                    let monitor = match target {
                                        OutputTarget::Display { monitor_index, .. } => {
                                            self.cached_monitors.get(*monitor_index).map(|(_, h)| h.clone())
                                        }
                                        _ => None,
                                    };
                                    log::info!("Output '{}' target: {}", w.name, target);
                                    w.set_target(target.clone(), monitor);
                                }
                                // TODO: windowed→headless swap (destroy window, create headless)
                            }
                            UnifiedOutput::Headless(h) => {
                                if target.is_headless() {
                                    // Stop active subprocess before changing target to release ports
                                    if h.active {
                                        if let Some(mut sub) = h.subprocess.take() {
                                            sub.stop();
                                        }
                                        h.active = false;
                                        h.started_at = None;
                                    }
                                    log::info!("Output '{}' target: {} → {}", h.name, h.target, target);
                                    h.target = target.clone();
                                }
                                // TODO: headless→windowed swap
                            }
                        }
                    }
                }
                ui::OutputAction::Start { idx } => {
                    if let Some(UnifiedOutput::Headless(h)) = self.outputs.get_mut(*idx) {
                        if !h.active {
                            match &h.target {
                                OutputTarget::Recording { path, codec } => {
                                    match crate::renderer::FfmpegSubprocess::spawn_recording(
                                        path, codec, h.width, h.height, 30,
                                    ) {
                                        Ok(sub) => {
                                            h.subprocess = Some(sub);
                                            h.active = true;
                                            log::info!("Started recording on output '{}'", h.name);
                                        }
                                        Err(e) => {
                                            log::error!("Failed to start recording: {}", e);
                                            self.notifications.error(format!("Recording failed: {}", e));
                                        }
                                    }
                                }
                                OutputTarget::SrtStream { url } => {
                                    match crate::renderer::FfmpegSubprocess::spawn_srt(
                                        url, h.width, h.height, 30,
                                    ) {
                                        Ok(sub) => {
                                            h.subprocess = Some(sub);
                                            h.active = true;
                                            log::info!("Started SRT stream on output '{}'", h.name);
                                        }
                                        Err(e) => {
                                            log::error!("Failed to start SRT: {}", e);
                                            self.notifications.error(format!("SRT failed: {}", e));
                                        }
                                    }
                                }
                                OutputTarget::NdiSend { .. } => {
                                    h.active = true;
                                    h.started_at = Some(std::time::Instant::now());
                                    log::info!("Started NDI send on output '{}'", h.name);
                                }
                                OutputTarget::SyphonServer { .. } => {
                                    h.active = true;
                                    h.started_at = Some(std::time::Instant::now());
                                    log::info!("Started Syphon server on output '{}'", h.name);
                                }
                                _ => {
                                    log::warn!("Cannot start windowed target as headless");
                                }
                            }
                        }
                    }
                }
                ui::OutputAction::Stop { idx } => {
                    if let Some(UnifiedOutput::Headless(h)) = self.outputs.get_mut(*idx) {
                        if h.active {
                            if let Some(mut sub) = h.subprocess.take() {
                                sub.stop();
                            }
                            h.active = false;
                            h.started_at = None;
                            log::info!("Stopped output '{}'", h.name);
                        }
                    }
                }
                ui::OutputAction::AssignSurface { output_idx, surface_idx } => {
                    if let Some(output) = self.outputs.get_mut(*output_idx) {
                        let name = output.name().to_string();
                        let assignments = output.surface_assignments_mut();
                        if !assignments.iter().any(|a| a.surface_idx == *surface_idx) {
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
                                log::info!("Assigned surface '{}' to output '{}'", surface.name, name);
                                assignments.push(assignment);
                            }
                        }
                    }
                }
                ui::OutputAction::UnassignSurface { output_idx, assignment_idx } => {
                    if let Some(output) = self.outputs.get_mut(*output_idx) {
                        let assignments = output.surface_assignments_mut();
                        if *assignment_idx < assignments.len() {
                            assignments.remove(*assignment_idx);
                            log::info!("Removed surface assignment from output '{}'", output.name());
                        }
                    }
                }
                ui::OutputAction::ToggleCalibration { idx } => {
                    if let Some(UnifiedOutput::Window(output)) = self.outputs.get_mut(*idx) {
                        output.calibration_mode = !output.calibration_mode;
                        log::info!("Output '{}' calibration mode: {}", output.name, output.calibration_mode);
                    }
                }
                ui::OutputAction::SetWarpCorner { output_idx, assignment_idx, corner_idx, position } => {
                    if let Some(UnifiedOutput::Window(output)) = self.outputs.get_mut(*output_idx) {
                        if let Some(assignment) = output.surface_assignments.get_mut(*assignment_idx) {
                            if *corner_idx < 4 {
                                assignment.warp_corners[*corner_idx] = *position;
                            }
                        }
                    }
                }
                ui::OutputAction::ResetWarp { output_idx, assignment_idx } => {
                    if let Some(output) = self.outputs.get_mut(*output_idx) {
                        let name = output.name().to_string();
                        let assignments = output.surface_assignments_mut();
                        if let Some(assignment) = assignments.get_mut(*assignment_idx) {
                            if let Some(surface) = self.surface_manager.surfaces.get(assignment.surface_idx) {
                                let bb = surface.bounding_box();
                                assignment.warp_corners = [
                                    [bb.x, bb.y],
                                    [bb.x + bb.width, bb.y],
                                    [bb.x + bb.width, bb.y + bb.height],
                                    [bb.x, bb.y + bb.height],
                                ];
                                log::info!("Reset warp for surface in output '{}'", name);
                            }
                        }
                    }
                }
            }
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
                                        surface_idx: a.surface_idx,
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
                // Restore surface assignments from config
                headless.surface_assignments = config.surface_assignments.iter().map(|a| {
                    SurfaceAssignment {
                        surface_idx: a.surface_idx,
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