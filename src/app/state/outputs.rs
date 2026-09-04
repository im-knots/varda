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
        let ndi_resolution = matches!(&target, OutputTarget::NdiSend { .. }).then(|| {
            let request = self
                .output
                .outputs
                .get(idx)
                .map(UnifiedOutput::presentation_request)
                .unwrap_or_default();
            self.external_io.ndi_manager.resolve_presentation(request)
        });
        {
            let Some(output) = self.output.outputs.get_mut(idx) else {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Output not found".into(),
                };
            };
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
                        if let Err(error) =
                            w.set_presentation_request(&self.context, w.presentation_request)
                        {
                            return CommandResult::Err {
                                code: ErrorCode::InternalError,
                                message: format!(
                                    "Failed to reconfigure output presentation: {error}"
                                ),
                            };
                        }
                    }
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
                        h.set_presentation_request(&self.context.device, h.presentation_request);
                        if let Some(resolved) = ndi_resolution {
                            h.set_resolved_presentation(&self.context.device, resolved);
                        }
                    }
                }
            }
        }
        self.refresh_presentation_notification(idx);
        CommandResult::Ok
    }

    /// Create a new headless output with the given target.
    pub fn cmd_create_headless_output(&mut self, target: OutputTarget) -> CommandResult {
        let idx = self.output.outputs.len() + 1;
        let name = format!("Output {idx}");
        let is_ndi = matches!(&target, OutputTarget::NdiSend { .. });
        let mut headless = HeadlessOutput::new(
            &self.context.device,
            name.clone(),
            OutputSource::Master,
            target,
            self.render_width,
            self.render_height,
        );
        if is_ndi {
            let resolved = self
                .external_io
                .ndi_manager
                .resolve_presentation(headless.presentation_request);
            headless.set_resolved_presentation(&self.context.device, resolved);
        }
        log::info!("Created headless output '{name}'");
        self.output.outputs.push(UnifiedOutput::Headless(headless));
        CommandResult::Ok
    }

    /// Resize every headless output to the render resolution, returning the
    /// names of any that had to be stopped to do it.
    ///
    /// Recording, NDI, Syphon and the ffmpeg stream targets all size their
    /// buffers from the render resolution, but they did so only once, when they
    /// were created. Changing the project resolution afterwards left them at the
    /// old size and the composite was stretch-blitted into it, so a recording
    /// made after switching a project to portrait still wrote a landscape file
    /// with squashed content.
    ///
    /// Anything backed by an ffmpeg subprocess has to stop: the encoder was
    /// spawned with a fixed `-s WxH` and feeding it raw frames of another size
    /// desyncs the stream rather than failing cleanly. Restarting it here would
    /// be worse than stopping — a recording would reopen the same path and
    /// truncate the take already on disk. NDI and Syphon carry their dimensions
    /// per frame and keep running across the change.
    pub(in crate::app) fn resize_headless_outputs(
        &mut self,
        width: u32,
        height: u32,
    ) -> Vec<String> {
        let device = &self.context.device;
        let mut stopped = Vec::new();
        let mut released = Vec::new();
        for output in &mut self.output.outputs {
            let UnifiedOutput::Headless(h) = output else {
                continue;
            };
            if h.width == width && h.height == height {
                continue;
            }
            if let Some(mut sub) = h.subprocess.take() {
                sub.stop();
                h.active = false;
                h.started_at = None;
                released.extend(h.audio_pcm.take());
                stopped.push(h.name.clone());
            }
            h.resize(device, width, height);
        }
        for pass in released {
            self.release_passthrough(Some(*pass));
        }
        stopped
    }

    /// Start a headless output (spawn ffmpeg subprocess or activate NDI/Syphon).
    pub fn cmd_start_output(&mut self, output_uuid: &str) -> CommandResult {
        let idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        // Snapshot what we need so no borrow of `self.output` is held across the
        // audio-subscription and spawn work (which borrow other `self` fields).
        // Also take any stale subscription left by a prior delivery failure.
        //
        // The encoder's frame size is fixed for the life of the subprocess, so
        // pin the output to the render resolution first. `set_render_resolution`
        // already keeps outputs in step; this is belt and braces for the paths
        // that assign `render_width`/`render_height` directly, such as the
        // workspace loader applying a scene's resolution.
        let (render_width, render_height) = (self.render_width, self.render_height);
        let device = &self.context.device;
        let (target, name, width, height, presentation_request, mut readback_format, stale) =
            match self.output.outputs.get_mut(idx) {
                Some(UnifiedOutput::Headless(h)) => {
                    if h.active {
                        return CommandResult::Ok; // already active
                    }
                    h.resize(device, render_width, render_height);
                    (
                        h.target.clone(),
                        h.name.clone(),
                        h.width,
                        h.height,
                        h.presentation_request,
                        h.readback.format(),
                        h.audio_pcm.take(),
                    )
                }
                _ => {
                    return CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: "Output not found or not headless".into(),
                    };
                }
            };
        if let OutputTarget::Recording { codec, .. } = &target {
            let resolved = match crate::renderer::FfmpegSubprocess::probe_recording_presentation(
                codec,
                presentation_request,
                device.features().contains(
                    wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                ),
            ) {
                Ok(resolved) => resolved,
                Err(error) => {
                    self.release_passthrough(stale.map(|subscription| *subscription));
                    return CommandResult::Err {
                        code: ErrorCode::Unavailable,
                        message: error.to_string(),
                    };
                }
            };
            if let Some(UnifiedOutput::Headless(output)) = self.output.outputs.get_mut(idx) {
                output.set_resolved_presentation(device, resolved);
                readback_format = output.readback.format();
            }
        }
        self.release_passthrough(stale.map(|b| *b));

        // Resolve optional audio passthrough (emits a notification + falls back
        // to video-only if the device is missing — Decision 6).
        let (audio_input, passthrough) = resolve_output_audio(
            &mut self.audio_manager,
            &mut self.session.notifications,
            target.audio_device(),
            &name,
        );

        let fps = encoder_fps(self.target_fps);

        let spawn_result = match &target {
            OutputTarget::SrtStream { url, codec, .. } => {
                crate::renderer::FfmpegSubprocess::spawn_srt(
                    url,
                    codec,
                    presentation_request,
                    width,
                    height,
                    fps,
                    audio_input,
                )
            }
            OutputTarget::Recording { path, codec, .. } => {
                crate::renderer::FfmpegSubprocess::spawn_recording_with_presentation(
                    path,
                    codec,
                    width,
                    height,
                    fps,
                    audio_input,
                    presentation_request,
                    readback_format,
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
                presentation_request,
                width,
                height,
                fps,
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
                presentation_request,
                width,
                height,
                fps,
                audio_input,
            ),
            OutputTarget::RtmpStream {
                url,
                codec,
                codec_contract,
                ..
            } => crate::renderer::FfmpegSubprocess::spawn_rtmp(
                url,
                codec,
                *codec_contract,
                presentation_request,
                width,
                height,
                fps,
                audio_input,
            ),
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
                let presentation = sub.presentation().cloned();
                if let Some(UnifiedOutput::Headless(h)) = self.output.outputs.get_mut(idx) {
                    if let Some(resolved) = presentation {
                        h.set_resolved_presentation(&self.context.device, resolved);
                    }
                    h.subprocess = Some(Box::new(sub));
                    h.audio_pcm = passthrough.map(Box::new);
                    h.active = true;
                }
                self.refresh_presentation_notification(idx);
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
    pub fn cmd_stop_output(&mut self, output_uuid: &str) -> CommandResult {
        let idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
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
    pub fn cmd_set_calibration_mode(
        &mut self,
        output_uuid: &str,
        mode: CalibrationMode,
    ) -> CommandResult {
        let idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
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
        output_uuid: &str,
        config: crate::renderer::edge_blend::EdgeBlendConfig,
    ) -> CommandResult {
        let output_idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
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
        output_uuid: &str,
        mode: EdgeBlendMode,
    ) -> CommandResult {
        let output_idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
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
        output_uuid: &str,
        rotation: crate::renderer::context::OutputRotation,
    ) -> CommandResult {
        let idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
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

    /// Set requested SDR precision and dithering, preserving any fallback status.
    pub fn cmd_set_output_presentation(
        &mut self,
        output_uuid: &str,
        request: crate::engine::value::render::PresentationRequest,
    ) -> CommandResult {
        let idx = match self.resolve_output(output_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        let restart = self.output.outputs.get(idx).is_some_and(
            |output| matches!(output, UnifiedOutput::Headless(headless) if headless.active),
        );
        if restart {
            let stopped = self.cmd_stop_output(output_uuid);
            if !matches!(stopped, CommandResult::Ok) {
                return stopped;
            }
        }
        let ndi_resolution = self
            .output
            .outputs
            .get(idx)
            .is_some_and(|output| matches!(output.target(), OutputTarget::NdiSend { .. }))
            .then(|| self.external_io.ndi_manager.resolve_presentation(request));
        if let Some(output) = self.output.outputs.get_mut(idx) {
            if let Err(error) = output.set_presentation_request(&self.context, request) {
                return CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: format!("Failed to configure output presentation: {error}"),
                };
            }
            if let (UnifiedOutput::Headless(headless), Some(resolved)) = (output, ndi_resolution) {
                headless.set_resolved_presentation(&self.context.device, resolved);
            }
            self.refresh_presentation_notification(idx);
            if restart {
                self.cmd_start_output(output_uuid)
            } else {
                CommandResult::Ok
            }
        } else {
            CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Output not found".into(),
            }
        }
    }

    /// Synchronize the one-shot fallback warning with one output's resolution.
    pub(crate) fn refresh_presentation_notification(&mut self, idx: usize) {
        let Some((uuid, name, resolved)) = self.output.outputs.get(idx).map(|output| {
            (
                output.uuid().to_string(),
                output.name().to_string(),
                output.resolved_presentation().clone(),
            )
        }) else {
            return;
        };
        let prefix = format!("presentation_fallback:{uuid}:");
        if let Some(reason) = resolved.fallback_reason {
            self.session.notifications.notify_once(
                format!("{prefix}{reason}"),
                crate::notifications::NotificationLevel::Warning,
                format!(
                    "Output '{name}' requested {} but is delivering {} ({}): {reason}",
                    resolved.requested.label(),
                    resolved.resolved.label(),
                    resolved.pixel_format,
                ),
            );
        } else if self.session.notifications.clear_once_key_prefix(&prefix) {
            self.session.notifications.info(format!(
                "Output '{name}' presentation recovered and is delivering {} ({})",
                resolved.resolved.label(),
                resolved.pixel_format,
            ));
        }
    }
}

/// The frame rate an ffmpeg output is opened at.
///
/// Must be the rate frames are actually produced at. A raw video input is timed
/// by position — frame N sits at N/fps — so if the encoder is told a rate the
/// renderer is not running at, the recording comes out at the wrong speed and
/// drifts against its own audio, which runs on the capture device's clock.
///
/// Every output used to be opened at a hardcoded 30 while the app defaults to
/// 60, so a stock recording was labelled at half the rate it was made: twice as
/// long as the session, in slow motion, with the audio running out halfway. It
/// also disabled the gap padding in `FfmpegSubprocess`, which measures the
/// renderer's shortfall against this same rate and saw a surplus instead.
///
/// Uncapped (`target_fps == 0`) has no rate to report, so it takes 60 — the
/// default cap, and the closest thing to an expected rate. A wildly different
/// actual rate will be corrected by frame padding rather than by mislabelling.
pub(crate) fn encoder_fps(target_fps: u32) -> u32 {
    const UNCAPPED_ASSUMED_FPS: u32 = 60;
    if target_fps == 0 {
        UNCAPPED_ASSUMED_FPS
    } else {
        target_fps
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
            "Audio device '{device_name}' not found for output '{output_name}'; recording/streaming video-only"
        ));
        return (None, None);
    };
    if let Some(sub) = audio_manager.subscribe_pcm(source_id) {
        let input = crate::renderer::AudioInput {
            rx: sub.receiver,
            sample_rate: sub.format.sample_rate,
            channels: sub.format.channels,
            lost_samples: sub.lost_samples,
        };
        let passthrough = AudioPassthrough {
            source_id,
            token: sub.token,
            dropped: sub.dropped,
        };
        (Some(input), Some(passthrough))
    } else {
        notifications.warn(format!(
            "Failed to open audio device '{device_name}' for output '{output_name}'; video-only"
        ));
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::encoder_fps;

    /// An ffmpeg output must be opened at the rate frames are actually made.
    ///
    /// Raw video carries no per-frame timing, so the rate declared at spawn is
    /// the only thing that says how long the recording is. Declaring a rate the
    /// renderer is not running at plays the result back at the wrong speed and
    /// slides it against its own audio, which is timed by the capture device's
    /// sample clock and cannot be talked into agreeing.
    ///
    /// Every spawn site used to pass a literal 30 while the app defaults to 60.
    /// See /spec/av-sync.md.
    #[test]
    fn outputs_are_opened_at_the_rate_frames_are_produced() {
        for target in [24, 25, 30, 50, 60, 120, 144] {
            assert_eq!(
                encoder_fps(target),
                target,
                "an output must be encoded at the rate the renderer runs at"
            );
        }
    }

    /// Uncapped has no rate to declare, so it takes the default cap. Frame
    /// padding absorbs the difference if the real rate turns out lower.
    #[test]
    fn an_uncapped_renderer_falls_back_to_a_declarable_rate() {
        assert_eq!(encoder_fps(0), 60);
    }
}
