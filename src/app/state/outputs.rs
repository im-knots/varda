//! Output management state mutations.

use crate::engine::{CommandResult, ErrorCode};
use crate::renderer::context::{HeadlessOutput, OutputSource, OutputTarget, UnifiedOutput};
use crate::renderer::edge_blend::EdgeBlendMode;
use crate::renderer::warp::WarpMode;
use super::super::VardaApp;

impl VardaApp {
    /// Set the output target for a windowed or headless output.
    pub fn cmd_set_output_target(&mut self, idx: usize, target: OutputTarget) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(idx) {
            match output {
                UnifiedOutput::Window(w) => {
                    if target.is_windowed() {
                        let monitor = match &target {
                            OutputTarget::Display { monitor_index, .. } => {
                                self.output.cached_monitors.get(*monitor_index).map(|(_, h)| h.clone())
                            }
                            _ => None,
                        };
                        w.set_target(target, monitor);
                    }
                    CommandResult::Ok
                }
                UnifiedOutput::Headless(h) => {
                    if target.is_headless() {
                        if h.active {
                            if let Some(mut sub) = h.subprocess.take() { sub.stop(); }
                            h.active = false;
                            h.started_at = None;
                        }
                        h.target = target;
                    }
                    CommandResult::Ok
                }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
        }
    }

    /// Create a new headless output with the given target.
    pub fn cmd_create_headless_output(&mut self, target: OutputTarget) -> CommandResult {
        let idx = self.output.outputs.len() + 1;
        let name = format!("Output {}", idx);
        let headless = HeadlessOutput::new(
            &self.context.device, name.clone(), OutputSource::Master,
            target, self.render_width, self.render_height,
        );
        log::info!("Created headless output '{}'", name);
        self.output.outputs.push(UnifiedOutput::Headless(headless));
        CommandResult::Ok
    }

    /// Start a headless output (spawn ffmpeg subprocess or activate NDI/Syphon).
    pub fn cmd_start_output(&mut self, idx: usize) -> CommandResult {
        if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
            if !h.active {
                match &h.target {
                    OutputTarget::SrtStream { url, codec } => {
                        match crate::renderer::FfmpegSubprocess::spawn_srt(url, codec, h.width, h.height, 30) {
                            Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                            Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    OutputTarget::Recording { path, codec } => {
                        match crate::renderer::FfmpegSubprocess::spawn_recording(path, codec, h.width, h.height, 30) {
                            Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                            Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    OutputTarget::HlsStream { name, codec, low_latency } => {
                        match crate::renderer::FfmpegSubprocess::spawn_hls(name, codec, h.width, h.height, 30, *low_latency) {
                            Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                            Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    OutputTarget::DashStream { name, codec } => {
                        match crate::renderer::FfmpegSubprocess::spawn_dash(name, codec, h.width, h.height, 30) {
                            Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                            Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    OutputTarget::RtmpStream { url, codec } => {
                        match crate::renderer::FfmpegSubprocess::spawn_rtmp(url, codec, h.width, h.height, 30) {
                            Ok(sub) => { h.subprocess = Some(sub); h.active = true; }
                            Err(e) => return CommandResult::Err { code: ErrorCode::InternalError, message: e.to_string() },
                        }
                    }
                    OutputTarget::NdiSend { .. } | OutputTarget::SyphonServer { .. } => {
                        h.active = true;
                        h.started_at = Some(std::time::Instant::now());
                    }
                    _ => return CommandResult::Err { code: ErrorCode::InvalidInput, message: "Cannot start windowed target".into() },
                }
                CommandResult::Ok
            } else {
                CommandResult::Ok // already active
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found or not headless".into() }
        }
    }

    /// Stop a headless output (kill subprocess and deactivate).
    pub fn cmd_stop_output(&mut self, idx: usize) -> CommandResult {
        if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
            if h.active {
                if let Some(mut sub) = h.subprocess.take() { sub.stop(); }
                h.active = false;
                h.started_at = None;
            }
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found or not headless".into() }
        }
    }

    /// Toggle calibration mode on a windowed output.
    pub fn cmd_toggle_calibration(&mut self, idx: usize) -> CommandResult {
        if let Some(UnifiedOutput::Window(w)) = self.output.outputs.get_mut(idx) {
            w.calibration_mode = !w.calibration_mode;
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found or not windowed".into() }
        }
    }

    /// Set a warp corner position for a surface assignment on a windowed output.
    pub fn cmd_set_warp_corner(
        &mut self,
        output_idx: usize,
        assignment_idx: usize,
        corner_idx: usize,
        position: [f32; 2],
    ) -> CommandResult {
        if let Some(UnifiedOutput::Window(w)) = self.output.outputs.get_mut(output_idx) {
            if let Some(a) = w.surface_assignments.get_mut(assignment_idx) {
                if let Some(corners) = a.warp_mode.corners_mut() {
                    if corner_idx < 4 { corners[corner_idx] = position; }
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Assignment not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
        }
    }

    /// Reset warp to identity corners based on the surface bounding box.
    pub fn cmd_reset_warp(&mut self, output_idx: usize, assignment_idx: usize) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(output_idx) {
            let assignments = output.surface_assignments_mut();
            if let Some(a) = assignments.get_mut(assignment_idx) {
                if let Some((_, surface)) = self.output.surface_manager.find_by_uuid(&a.surface_uuid) {
                    let bb = surface.bounding_box();
                    a.warp_mode = WarpMode::identity_corners(
                        [bb.x, bb.y, bb.width, bb.height]
                    );
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Assignment not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
        }
    }

    /// Set edge blend configuration for an output.
    pub fn cmd_set_edge_blend(
        &mut self,
        output_idx: usize,
        config: crate::renderer::edge_blend::EdgeBlendConfig,
    ) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(output_idx) {
            match output {
                UnifiedOutput::Window(w) => { w.edge_blend = config; }
                UnifiedOutput::Headless(h) => { h.edge_blend = config; }
            }
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
        }
    }

    /// Set edge blend mode for an output; triggers auto-recompute if mode is Auto.
    pub fn cmd_set_edge_blend_mode(&mut self, output_idx: usize, mode: EdgeBlendMode) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(output_idx) {
            match output {
                UnifiedOutput::Window(w) => { w.edge_blend_mode = mode; }
                UnifiedOutput::Headless(h) => { h.edge_blend_mode = mode; }
            }
            if mode == EdgeBlendMode::Auto {
                self.recompute_auto_edge_blend();
            }
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
        }
    }

    /// Set output rotation and rebuild intermediate textures.
    pub fn cmd_set_output_rotation(&mut self, idx: usize, rotation: crate::renderer::context::OutputRotation) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(idx) {
            match output {
                UnifiedOutput::Window(w) => { w.set_rotation(&self.context.device, rotation); }
                UnifiedOutput::Headless(h) => { h.set_rotation(rotation); }
            }
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Output not found".into() }
        }
    }
}
