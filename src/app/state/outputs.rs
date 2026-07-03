//! Output management state mutations.

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};
use crate::renderer::context::{
    AudioPassthrough, CalibrationMode, HeadlessOutput, OutputSource, OutputTarget, UnifiedOutput,
};
use crate::renderer::edge_blend::EdgeBlendMode;

impl VardaApp {
    /// Set the output target for a windowed or headless output.
    pub fn cmd_set_output_target(&mut self, idx: usize, target: OutputTarget) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(idx) {
            match output {
                UnifiedOutput::Window(w) => {
                    if target.is_windowed() {
                        let monitor = match &target {
                            OutputTarget::Display { monitor_index, .. } => self
                                .output
                                .cached_monitors
                                .get(*monitor_index)
                                .map(|(_, h)| h.clone()),
                            _ => None,
                        };
                        w.set_target(target, monitor);
                    }
                    CommandResult::Ok
                }
                UnifiedOutput::Headless(h) => {
                    if target.is_headless() {
                        if h.active {
                            if let Some(mut sub) = h.subprocess.take() {
                                sub.stop();
                            }
                            let passthrough = h.audio_pcm.take();
                            h.active = false;
                            h.started_at = None;
                            if let Some(pass) = passthrough {
                                self.audio_manager
                                    .unsubscribe_pcm(pass.source_id, pass.token);
                            }
                        }
                        h.target = target;
                    }
                    CommandResult::Ok
                }
            }
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }

    /// Create a new headless output with the given target.
    pub fn cmd_create_headless_output(&mut self, target: OutputTarget) -> CommandResult {
        let idx = self.output.outputs.len() + 1;
        let name = format!("Output {}", idx);
        let headless = HeadlessOutput::new(
            &self.context.device,
            name.clone(),
            OutputSource::Master,
            target,
            self.render_width,
            self.render_height,
        );
        log::info!("Created headless output '{}'", name);
        self.output.outputs.push(UnifiedOutput::Headless(headless));
        CommandResult::Ok
    }

    /// Start a headless output (spawn ffmpeg subprocess or activate NDI/Syphon).
    pub fn cmd_start_output(&mut self, idx: usize) -> CommandResult {
        // Snapshot what we need so no borrow of `self.output` is held across the
        // audio-subscription and spawn work (which borrow other `self` fields).
        // Also take any stale subscription left by a prior delivery failure.
        let (target, name, width, height, stale) = match self.output.outputs.get_mut(idx) {
            Some(UnifiedOutput::Headless(h)) => {
                if h.active {
                    return CommandResult::Ok; // already active
                }
                (
                    h.target.clone(),
                    h.name.clone(),
                    h.width,
                    h.height,
                    h.audio_pcm.take(),
                )
            }
            _ => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Output not found or not headless".into(),
                }
            }
        };
        self.release_passthrough(stale.map(|b| *b));

        // Resolve optional audio passthrough (emits a notification + falls back
        // to video-only if the device is missing — Decision 6).
        let (audio_input, passthrough) = resolve_output_audio(
            &mut self.audio_manager,
            &mut self.session.notifications,
            target.audio_device(),
            &name,
        );

        let spawn_result = match &target {
            OutputTarget::SrtStream { url, codec, .. } => {
                crate::renderer::FfmpegSubprocess::spawn_srt(
                    url,
                    codec,
                    width,
                    height,
                    30,
                    audio_input,
                )
            }
            OutputTarget::Recording { path, codec, .. } => {
                crate::renderer::FfmpegSubprocess::spawn_recording(
                    path,
                    codec,
                    width,
                    height,
                    30,
                    audio_input,
                )
            }
            OutputTarget::HlsStream {
                name: target_name,
                codec,
                low_latency,
                ..
            } => crate::renderer::FfmpegSubprocess::spawn_hls(
                target_name,
                codec,
                width,
                height,
                30,
                *low_latency,
                audio_input,
            ),
            OutputTarget::DashStream {
                name: target_name,
                codec,
                ..
            } => crate::renderer::FfmpegSubprocess::spawn_dash(
                target_name,
                codec,
                width,
                height,
                30,
                audio_input,
            ),
            OutputTarget::RtmpStream { url, codec, .. } => {
                crate::renderer::FfmpegSubprocess::spawn_rtmp(
                    url,
                    codec,
                    width,
                    height,
                    30,
                    audio_input,
                )
            }
            OutputTarget::NdiSend { .. } => {
                // No ffmpeg subprocess; NDI doesn't carry passthrough audio.
                self.release_passthrough(passthrough);
                if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
                    h.active = true;
                    h.started_at = Some(std::time::Instant::now());
                }
                return CommandResult::Ok;
            }
            OutputTarget::SyphonServer { .. } => {
                // No ffmpeg subprocess; Syphon doesn't carry passthrough audio.
                self.release_passthrough(passthrough);
                #[cfg(target_os = "macos")]
                {
                    if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
                        h.active = true;
                        h.started_at = Some(std::time::Instant::now());
                    }
                    return CommandResult::Ok;
                }
                // Parity with the Syphon receive path (cmd_add_syphon_deck): reject
                // explicitly on non-macOS so an API client gets clear feedback rather
                // than a silently inert output.
                #[cfg(not(target_os = "macos"))]
                {
                    return CommandResult::Err {
                        code: ErrorCode::Unavailable,
                        message: "Syphon is only available on macOS".into(),
                    };
                }
            }
            _ => {
                self.release_passthrough(passthrough);
                return CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: "Cannot start windowed target".into(),
                };
            }
        };

        match spawn_result {
            Ok(sub) => {
                if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
                    h.subprocess = Some(sub);
                    h.audio_pcm = passthrough.map(Box::new);
                    h.active = true;
                }
                CommandResult::Ok
            }
            Err(e) => {
                // Spawn failed — release the PCM subscription we reserved.
                self.release_passthrough(passthrough);
                CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        }
    }

    /// Release a reserved PCM subscription (used when an output fails to start or
    /// is a non-ffmpeg target that can't carry passthrough audio).
    fn release_passthrough(&mut self, passthrough: Option<AudioPassthrough>) {
        if let Some(pass) = passthrough {
            self.audio_manager
                .unsubscribe_pcm(pass.source_id, pass.token);
        }
    }

    /// Stop a headless output (kill subprocess and deactivate).
    pub fn cmd_stop_output(&mut self, idx: usize) -> CommandResult {
        if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
            if h.active {
                if let Some(mut sub) = h.subprocess.take() {
                    sub.stop();
                }
                let passthrough = h.audio_pcm.take();
                h.active = false;
                h.started_at = None;
                // Disjoint field borrow (audio_manager vs. output): release the
                // PCM subscription so the cpal callback stops fanning to it.
                if let Some(pass) = passthrough {
                    self.audio_manager
                        .unsubscribe_pcm(pass.source_id, pass.token);
                }
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found or not headless".into(),
            }
        }
    }

    /// Set the calibration display mode on a windowed output.
    pub fn cmd_set_calibration_mode(&mut self, idx: usize, mode: CalibrationMode) -> CommandResult {
        if let Some(UnifiedOutput::Window(w)) = self.output.outputs.get_mut(idx) {
            w.calibration_mode = mode;
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found or not windowed".into(),
            }
        }
    }

    /// Move one corner-pin corner of a surface's warp (per-surface).
    pub fn cmd_set_warp_corner(
        &mut self,
        surface_uuid: &str,
        corner_idx: usize,
        position: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_warp_corner(corner_idx, position);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Clear a surface's warp (back to no-warp / native position).
    pub fn cmd_reset_warp(&mut self, surface_uuid: &str) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.reset_warp();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Set the warp grid resolution for a surface, converting its warp into a
    /// `cols` × `rows` mesh while preserving the current deformation. Dimensions
    /// are clamped to `[2, MAX_WARP_SUBDIVISIONS]` in the domain method.
    pub fn cmd_set_warp_subdivisions(
        &mut self,
        surface_uuid: &str,
        cols: u32,
        rows: u32,
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_warp_subdivisions(cols, rows);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Move a single mesh grid point (row-major) of a surface's mesh warp.
    /// No-op on the geometry if the surface's warp is not a mesh; still returns
    /// `Ok` so callers can treat it uniformly.
    pub fn cmd_set_warp_mesh_point(
        &mut self,
        surface_uuid: &str,
        row: usize,
        col: usize,
        position: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_warp_mesh_point(row, col, position);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Bind/unbind a surface's warp from its shape (auto-warp). Unbinding
    /// materialises the conforming warp for manual editing.
    pub fn cmd_set_warp_bound(&mut self, surface_uuid: &str, bound: bool) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_warp_bound(bound);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Convert a surface's warp into a smooth bezier patch grid (8i.6).
    pub fn cmd_convert_warp_to_bezier(&mut self, surface_uuid: &str) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.convert_warp_to_bezier();
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Move a bezier-warp control anchor. No-op on the geometry if the warp is
    /// not bezier; still returns `Ok` so callers can treat it uniformly.
    pub fn cmd_move_warp_anchor(
        &mut self,
        surface_uuid: &str,
        row: usize,
        col: usize,
        position: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_warp_bezier_anchor(row, col, position);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Move a bezier-warp tangent handle. No-op on the geometry if the warp is
    /// not bezier; still returns `Ok`.
    pub fn cmd_move_warp_handle(
        &mut self,
        surface_uuid: &str,
        horizontal: bool,
        row: usize,
        col: usize,
        which: usize,
        position: [f32; 2],
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_warp_bezier_handle(horizontal, row, col, which, position);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
        }
    }

    /// Set the bezier-warp control-cage resolution. No-op on the geometry if the
    /// warp is not bezier; still returns `Ok`.
    pub fn cmd_set_bezier_cage_subdivisions(
        &mut self,
        surface_uuid: &str,
        cols: u32,
        rows: u32,
    ) -> CommandResult {
        if let Some((_, surface)) = self.output.surface_manager.find_by_uuid_mut(surface_uuid) {
            surface.set_bezier_cage_subdivisions(cols, rows);
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Surface not found".into(),
            }
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
                UnifiedOutput::Window(w) => {
                    w.edge_blend = config;
                }
                UnifiedOutput::Headless(h) => {
                    h.edge_blend = config;
                }
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }

    /// Set edge blend mode for an output; triggers auto-recompute if mode is Auto.
    pub fn cmd_set_edge_blend_mode(
        &mut self,
        output_idx: usize,
        mode: EdgeBlendMode,
    ) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(output_idx) {
            match output {
                UnifiedOutput::Window(w) => {
                    w.edge_blend_mode = mode;
                }
                UnifiedOutput::Headless(h) => {
                    h.edge_blend_mode = mode;
                }
            }
            if mode == EdgeBlendMode::Auto {
                self.recompute_auto_edge_blend();
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }

    /// Set output rotation and rebuild intermediate textures.
    pub fn cmd_set_output_rotation(
        &mut self,
        idx: usize,
        rotation: crate::renderer::context::OutputRotation,
    ) -> CommandResult {
        if let Some(output) = self.output.outputs.get_mut(idx) {
            match output {
                UnifiedOutput::Window(w) => {
                    w.set_rotation(&self.context.device, rotation);
                }
                UnifiedOutput::Headless(h) => {
                    h.set_rotation(rotation);
                }
            }
            CommandResult::Ok
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }
}

/// Resolve a persisted audio device name to a live PCM subscription for output
/// passthrough. Returns `(AudioInput for ffmpeg, AudioPassthrough to retain for
/// teardown)`. On a missing/unopenable device, emits a warning and returns
/// `(None, None)` → video-only (Decision 6). Shared by `cmd_start_output` and
/// the SRT auto-restart path in the render loop, which both need a fresh tap
/// off disjoint field borrows rather than `&mut self`.
pub(crate) fn resolve_output_audio(
    audio_manager: &mut crate::audio::AudioManager,
    notifications: &mut crate::notifications::NotificationSystem,
    device_name: Option<&str>,
    output_name: &str,
) -> (
    Option<crate::renderer::AudioInput>,
    Option<AudioPassthrough>,
) {
    let Some(device_name) = device_name else {
        return (None, None);
    };
    let source_id = audio_manager
        .devices()
        .iter()
        .find(|d| d.name == device_name)
        .map(|d| d.id);
    let Some(source_id) = source_id else {
        notifications.warn(format!(
            "Audio device '{}' not found for output '{}'; recording/streaming video-only",
            device_name, output_name
        ));
        return (None, None);
    };
    match audio_manager.subscribe_pcm(source_id) {
        Some(sub) => {
            let input = crate::renderer::AudioInput {
                rx: sub.receiver,
                sample_rate: sub.format.sample_rate,
                channels: sub.format.channels,
            };
            let passthrough = AudioPassthrough {
                source_id,
                token: sub.token,
                dropped: sub.dropped,
            };
            (Some(input), Some(passthrough))
        }
        None => {
            notifications.warn(format!(
                "Failed to open audio device '{}' for output '{}'; video-only",
                device_name, output_name
            ));
            (None, None)
        }
    }
}
