//! Output management state mutations.

use super::super::VardaApp;
use crate::delivery::AudioPassthrough;
use crate::engine::value::provider::ControlValue;
use crate::engine::value::render::Unassigned;
use crate::engine::{CommandResult, ErrorCode};
use crate::output::{Output, ParamEffect, SinkConfig, SinkEnv, SinkStart};
use crate::renderer::context::CalibrationMode;
use crate::renderer::edge_blend::EdgeBlendMode;

fn not_found(uuid: &str) -> CommandResult {
    CommandResult::Err {
        code: ErrorCode::NotFound,
        message: format!("Output '{uuid}' not found"),
    }
}

impl VardaApp {
    /// Point an output at another sink, keeping its surfaces, warp, edge blend
    /// and presentation. A window moving between monitors keeps its window; any
    /// other change rebuilds the sink, stopping it first.
    pub fn cmd_set_output_target(&mut self, output_uuid: &str, sink: &SinkConfig) -> CommandResult {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        match self.apply_sink_config(idx, sink) {
            Ok(()) => CommandResult::Ok,
            Err(e) => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("{e:#}"),
            },
        }
    }

    /// Write one of an output's sink settings.
    pub fn cmd_set_sink_param(
        &mut self,
        output_uuid: &str,
        name: &str,
        value: &ControlValue,
    ) -> CommandResult {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        let effect = match self.output.outputs[idx].sink_mut().set_param(name, value) {
            Ok(effect) => effect,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                };
            }
        };
        if effect == ParamEffect::Rebuild {
            let config = self.output.outputs[idx].sink().config();
            if let Err(e) = self.apply_sink_config(idx, &config) {
                return CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: format!("{e:#}"),
                };
            }
        }
        CommandResult::Ok
    }

    /// Take `config` in place when the sink can, otherwise rebuild the sink
    /// from it. A running output is stopped before a rebuild.
    pub(crate) fn apply_sink_config(
        &mut self,
        idx: usize,
        config: &SinkConfig,
    ) -> anyhow::Result<()> {
        let render = (self.render.width, self.render.height);
        let output = &mut self.output.outputs[idx];
        if output.sink_mut().patch(config, &self.sources.services) {
            output.set_presentation_request(
                &self.render.context,
                &self.sources.services,
                output.presentation_request(),
            )?;
        } else {
            let sink = {
                let mut env = SinkEnv {
                    gpu: &self.render.context,
                    services: &mut self.sources.services,
                    width: render.0,
                    height: render.1,
                };
                self.output.sinks.create(config, &mut env)?
            };
            let uuid = self.output.outputs[idx].uuid.clone();
            self.stop_output(&uuid, None);
            let output = &mut self.output.outputs[idx];
            output.replace_sink(&self.render.context, &self.sources.services, sink, render)?;
        }
        self.refresh_presentation_notification(idx);
        Ok(())
    }

    /// Write text to an address that takes it: a surface's source, or an
    /// output's text setting by its route.
    pub(crate) fn set_path_text(&mut self, path: &str, value: String) -> CommandResult {
        use crate::engine::value::param::{OutputControl, ParamAddress};
        let invalid = |message: String| CommandResult::Err {
            code: ErrorCode::InvalidInput,
            message,
        };
        match path.parse::<ParamAddress>() {
            Ok(ParamAddress::SurfaceSource { surface }) => {
                match value.parse::<crate::renderer::context::OutputSource>() {
                    Ok(source) => {
                        self.execute_command(crate::engine::EngineCommand::SetSurfaceSource {
                            uuid: surface,
                            source,
                        })
                    }
                    Err(reason) => invalid(reason),
                }
            }
            Ok(ParamAddress::Output {
                output,
                target: OutputControl::Sink(route),
            }) => match self.sink_setting_for_route(&output, &route) {
                Some(name) => self.cmd_set_sink_param(&output, &name, &ControlValue::Text(value)),
                None => invalid(format!("output '{output}' has no setting at '{route}'")),
            },
            _ => invalid(format!("'{path}' takes no text")),
        }
    }

    /// Create an output delivering through `sink`. Returns its UUID.
    pub fn cmd_create_output(&mut self, sink: SinkConfig) -> CommandResult {
        let config = crate::scene::OutputConfig::for_sink(sink);
        match self.create_output(&config) {
            Ok(uuid) => CommandResult::OkWithId { uuid },
            Err(e) => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("{e:#}"),
            },
        }
    }

    /// Run a library action an output type offers. Answers with the type's
    /// fresh entries.
    pub fn cmd_sink_library_action(&mut self, sink_type: &str, action: &str) -> CommandResult {
        let (width, height) = (self.render.width, self.render.height);
        let mut env = SinkEnv {
            gpu: &self.render.context,
            services: &mut self.sources.services,
            width,
            height,
        };
        let Some(provider) = self.output.sinks.get_mut(sink_type) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("unknown output type '{sink_type}'"),
            };
        };
        if let Err(e) = provider.library_action(action, &mut env) {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("{e:#}"),
            };
        }
        let entries = provider.library(&env.query()).entries;
        CommandResult::OkWithData {
            data: serde_json::to_value(entries).unwrap_or_default(),
        }
    }

    /// Resize every output that renders at the render resolution, returning
    /// the names of any that had to be stopped to do it.
    ///
    /// An encoder is opened with a fixed `-s WxH` and feeding it raw frames of
    /// another size desyncs the stream rather than failing cleanly. Restarting
    /// it here would be worse than stopping: a recording would reopen the same
    /// path and truncate the take already on disk. Senders that carry their
    /// size per frame keep running.
    pub(in crate::app) fn resize_headless_outputs(
        &mut self,
        width: u32,
        height: u32,
    ) -> Vec<String> {
        let mut stopped = Vec::new();
        for idx in 0..self.output.outputs.len() {
            let output = &self.output.outputs[idx];
            if output.sink().size((width, height)) == output.size() {
                continue;
            }
            if output.active && output.sink().restart_on_resize() {
                let uuid = output.uuid.clone();
                stopped.push(output.name.clone());
                self.stop_output(&uuid, None);
            }
            if let Err(e) = self.output.outputs[idx].resize(&self.render.context, (width, height)) {
                log::error!("Output could not follow the render resolution: {e:#}");
            }
        }
        stopped
    }

    /// Start an output's sink: settle its presentation, open its audio, begin
    /// delivering.
    pub fn cmd_start_output(&mut self, output_uuid: &str) -> CommandResult {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        let render = (self.render.width, self.render.height);
        let gpu = &self.render.context;
        let output = &mut self.output.outputs[idx];
        if output.active {
            return CommandResult::Ok;
        }
        if !output.sink().startable() {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Output '{}' is always showing", output.name),
            };
        }
        // The encoder's frame size is fixed for the life of the session, so
        // the output is pinned to the render resolution first.
        if let Err(e) = output.resize(gpu, render) {
            return CommandResult::Err {
                code: ErrorCode::InternalError,
                message: format!("{e:#}"),
            };
        }
        let request = output.presentation_request();
        match output.sink_mut().prepare_start(gpu, request) {
            Ok(Some(resolved)) => {
                if let Err(e) = output.apply_resolved(gpu, resolved) {
                    return CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: format!("{e:#}"),
                    };
                }
            }
            Ok(None) => {}
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::Unavailable,
                    message: e.to_string(),
                };
            }
        }
        let device = output.sink().audio_device().map(str::to_string);
        let name = output.name.clone();
        let (audio, passthrough) = resolve_output_audio(
            &mut self.audio.manager,
            &mut self.session.notifications,
            device.as_deref(),
            &name,
        );
        let output = &mut self.output.outputs[idx];
        let (width, height) = output.size();
        let start = SinkStart {
            width,
            height,
            fps: encoder_fps(self.render.target_fps),
            request: output.presentation_request(),
            readback_format: output
                .readback_format()
                .unwrap_or(crate::renderer::ReadbackFormat::Rgba8),
            audio,
        };
        match output.sink_mut().start(start) {
            Ok(settled) => {
                if let Some(resolved) = settled
                    && let Err(e) = output.apply_resolved(&self.render.context, resolved)
                {
                    log::error!("Output '{name}' could not take its encoder's format: {e:#}");
                }
                output.active = true;
                output.started_at = Some(std::time::Instant::now());
                output.audio = passthrough;
                self.refresh_presentation_notification(idx);
                CommandResult::Ok
            }
            Err(e) => {
                self.release_passthrough(passthrough);
                CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        }
    }

    /// Release a PCM subscription an output held.
    pub(crate) fn release_passthrough(&mut self, passthrough: Option<AudioPassthrough>) {
        if let Some(pass) = passthrough {
            self.audio
                .manager
                .unsubscribe_pcm(pass.source_id, pass.token);
        }
    }

    /// Stop an output's sink and release what its session held, telling the
    /// operator `reason` when it stopped on its own.
    pub(crate) fn stop_output(&mut self, output_uuid: &str, reason: Option<String>) {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return;
        };
        let output = &mut self.output.outputs[idx];
        if !output.active {
            return;
        }
        output.sink_mut().stop();
        // Report what the content actually reached. The file's MaxCLL and
        // MaxFALL are whatever the encoder wrote at start and cannot be
        // rewritten, so these are for choosing the peak next time.
        // See /spec/hdr-recording-output.md § As built.
        let levels = output.take_light_levels();
        if let Some((max_cll, max_fall)) = levels.measured() {
            log::info!(
                "Output '{}': measured content light over {} frames: \
                 MaxCLL {max_cll} cd/m², MaxFALL {max_fall} cd/m². \
                 The file declares its configured peak; set the output's \
                 peak near MaxCLL for a tighter declaration.",
                output.name,
                levels.frames(),
            );
        }
        output.active = false;
        output.started_at = None;
        let passthrough = output.audio.take();
        let name = output.name.clone();
        self.release_passthrough(passthrough);
        if let Some(reason) = reason {
            log::error!("{reason}");
            self.session
                .notifications
                .error(format!("Output '{name}' stopped: {reason}"));
        }
    }

    /// Stop an output's sink.
    pub fn cmd_stop_output(&mut self, output_uuid: &str) -> CommandResult {
        if self.output.resolve_output(output_uuid).is_err() {
            return not_found(output_uuid);
        }
        self.stop_output(output_uuid, None);
        CommandResult::Ok
    }

    /// Set output rotation and rebuild what depends on it.
    pub fn cmd_set_output_rotation(
        &mut self,
        output_uuid: &str,
        rotation: crate::renderer::context::OutputRotation,
    ) -> CommandResult {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        match self.output.outputs[idx].set_rotation(&self.render.context, rotation) {
            Ok(()) => CommandResult::Ok,
            Err(e) => CommandResult::Err {
                code: ErrorCode::InternalError,
                message: format!("{e:#}"),
            },
        }
    }

    /// Set what an output shows with nothing assigned.
    pub fn cmd_set_output_unassigned(
        &mut self,
        output_uuid: &str,
        unassigned: Option<Unassigned>,
    ) -> CommandResult {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        self.output.outputs[idx].unassigned = unassigned;
        CommandResult::Ok
    }

    pub fn cmd_set_output_presentation(
        &mut self,
        output_uuid: &str,
        request: crate::engine::value::render::PresentationRequest,
    ) -> CommandResult {
        let Ok(idx) = self.output.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        let restart = self.output.outputs[idx].active;
        if restart {
            self.stop_output(output_uuid, None);
        }
        let output = &mut self.output.outputs[idx];
        if let Err(error) =
            output.set_presentation_request(&self.render.context, &self.sources.services, request)
        {
            return CommandResult::Err {
                code: ErrorCode::InternalError,
                message: format!("Failed to configure output presentation: {error}"),
            };
        }
        self.refresh_presentation_notification(idx);
        if restart {
            self.cmd_start_output(output_uuid)
        } else {
            CommandResult::Ok
        }
    }

    /// Synchronize the one-shot fallback warning with one output's resolution.
    pub(crate) fn refresh_presentation_notification(&mut self, idx: usize) {
        let Some((uuid, name, resolved)) = self.output.outputs.get(idx).map(|output| {
            (
                output.uuid.clone(),
                output.name.clone(),
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
/// `(None, None)` → video-only (Decision 6).
fn resolve_output_audio(
    audio_manager: &mut crate::audio::AudioManager,
    notifications: &mut crate::notifications::NotificationSystem,
    device_name: Option<&str>,
    output_name: &str,
) -> (
    Option<crate::delivery::AudioInput>,
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
        let input = crate::delivery::AudioInput {
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

impl super::super::Outputs {
    /// How long an output has been sending: the sink's own clock when it keeps
    /// one, otherwise the time since it started. Zero for an idle output.
    pub(crate) fn active_duration(output: &Output) -> std::time::Duration {
        if !output.active {
            return std::time::Duration::ZERO;
        }
        output
            .sink()
            .duration()
            .or_else(|| output.started_at.map(|t| t.elapsed()))
            .unwrap_or_default()
    }

    /// Close an output, stopping whatever it was sending through. Returns its
    /// audio passthrough, for the caller to unsubscribe from the audio manager.
    pub(crate) fn close_output(
        &mut self,
        output_uuid: &str,
    ) -> anyhow::Result<Option<AudioPassthrough>> {
        let idx = self.resolve_output(output_uuid)?;
        let mut output = self.outputs.remove(idx);
        // Stop the encoder before dropping, to release ports and files.
        output.sink_mut().stop();
        log::info!("Closed output '{}'", output.name);
        Ok(output.audio.take())
    }

    pub(crate) fn assign_surface_to_output(&mut self, output_uuid: &str, surface_uuid: &str) {
        if let Some(output) = self.outputs.iter_mut().find(|o| o.uuid == output_uuid) {
            let assignments = &mut output.surface_assignments;
            // Warp lives on the surface now — the assignment is membership only.
            if !assignments.iter().any(|a| a.surface_uuid == surface_uuid)
                && self.surface_manager.find_by_uuid(surface_uuid).is_some()
            {
                assignments.push(crate::renderer::context::SurfaceAssignment {
                    surface_uuid: surface_uuid.to_string(),
                    enabled: true,
                    overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                });
            }
        }
    }

    pub(crate) fn unassign_surface_from_output(&mut self, output_uuid: &str, surface_uuid: &str) {
        if let Some(output) = self.outputs.iter_mut().find(|o| o.uuid == output_uuid) {
            output
                .surface_assignments
                .retain(|a| a.surface_uuid != surface_uuid);
        }
    }

    /// Enable or disable one surface assignment, the `output/<uuid>/surface/
    /// <surface_uuid>` control. Assigns the surface when enabling one that is
    /// not assigned yet.
    pub(crate) fn set_surface_assignment_enabled(
        &mut self,
        output_uuid: &str,
        surface_uuid: &str,
        enabled: bool,
    ) -> anyhow::Result<()> {
        let idx = self.resolve_output(output_uuid)?;
        if self.surface_manager.find_by_uuid(surface_uuid).is_none() {
            return Err(
                crate::engine::value::entity::UnknownEntity::new("surface", surface_uuid).into(),
            );
        }
        let assignments = &mut self.outputs[idx].surface_assignments;
        match assignments
            .iter_mut()
            .find(|a| a.surface_uuid == surface_uuid)
        {
            Some(assignment) => assignment.enabled = enabled,
            None if enabled => assignments.push(crate::renderer::context::SurfaceAssignment {
                surface_uuid: surface_uuid.to_string(),
                enabled: true,
                overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
            }),
            None => {}
        }
        Ok(())
    }

    /// Recompute per-surface edge blend for all Auto-mode outputs based on surface topology.
    pub fn recompute_auto_edge_blend(&mut self) {
        use crate::renderer::edge_blend::{
            MappedRegion, OutputSurfaceInfo, SurfaceOverlapZones, compute_auto_edge_blend,
        };

        // Check if any output is in Auto mode — early exit if none.
        let auto_count = self
            .outputs
            .iter()
            .filter(|o| o.edge_blend_mode == EdgeBlendMode::Auto)
            .count();
        if auto_count == 0 {
            return;
        }
        log::debug!("[edge-blend] recompute_auto: {auto_count} outputs in Auto mode");

        let infos: Vec<OutputSurfaceInfo> = self
            .outputs
            .iter()
            .enumerate()
            .map(|(idx, output)| {
                let mut regions = Vec::new();
                for assignment in &output.surface_assignments {
                    if let Some((_, surface)) =
                        self.surface_manager.find_by_uuid(&assignment.surface_uuid)
                    {
                        let bb = surface.bounding_box();
                        regions.push(MappedRegion {
                            source_key: surface.source.to_path_value(),
                            bbox: [bb.x, bb.y, bb.width, bb.height],
                            surface_uuid: assignment.surface_uuid.clone(),
                            vertices: surface.vertices.clone(),
                            extra_contours: surface.extra_contours.clone(),
                            holes: surface.hole_contours.clone(),
                        });
                    }
                }
                OutputSurfaceInfo {
                    output_idx: idx,
                    edge_blend_mode: output.edge_blend_mode,
                    default_gamma: output.edge_blend.left.gamma,
                    regions,
                }
            })
            .collect();

        // Clear overlap zones on all Auto-mode assignments before applying new results.
        for output in &mut self.outputs {
            if output.edge_blend_mode == EdgeBlendMode::Auto {
                for assignment in &mut output.surface_assignments {
                    assignment.overlap_zones = SurfaceOverlapZones::default();
                }
            }
        }

        let results = compute_auto_edge_blend(&infos);
        log::debug!("[edge-blend] computed {} results", results.len());
        for result in results {
            let output = &mut self.outputs[result.output_idx];
            for assignment in &mut output.surface_assignments {
                if assignment.surface_uuid == result.surface_uuid {
                    assignment.overlap_zones = result.overlap_zones;
                    break;
                }
            }
        }
    }

    /// Resolve an output UUID to its current index.
    pub(crate) fn resolve_output(
        &self,
        uuid: &str,
    ) -> crate::engine::value::entity::Resolved<usize> {
        self.outputs
            .iter()
            .position(|o| o.uuid == uuid)
            .ok_or_else(|| crate::engine::value::entity::UnknownEntity::new("output", uuid))
    }

    /// Set the calibration display mode on any output.
    pub fn cmd_set_calibration_mode(
        &mut self,
        output_uuid: &str,
        mode: CalibrationMode,
    ) -> CommandResult {
        let Ok(idx) = self.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        self.outputs[idx].calibration_mode = mode;
        CommandResult::Ok
    }

    /// Set edge blend configuration for an output.
    pub fn cmd_set_edge_blend(
        &mut self,
        output_uuid: &str,
        config: crate::renderer::edge_blend::EdgeBlendConfig,
    ) -> CommandResult {
        let Ok(idx) = self.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        self.outputs[idx].edge_blend = config;
        CommandResult::Ok
    }

    /// Set edge blend mode for an output; triggers auto-recompute if mode is Auto.
    pub fn cmd_set_edge_blend_mode(
        &mut self,
        output_uuid: &str,
        mode: EdgeBlendMode,
    ) -> CommandResult {
        let Ok(idx) = self.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        self.outputs[idx].edge_blend_mode = mode;
        if mode == EdgeBlendMode::Auto {
            self.recompute_auto_edge_blend();
        }
        CommandResult::Ok
    }

    /// Set or clear one output's tonemap override.
    ///
    /// Needs no restart and no GPU reconfiguration: the curve only selects which
    /// graded program the output reads, and the mixer materializes that on the
    /// next frame.
    pub fn cmd_set_output_tonemap(
        &mut self,
        output_uuid: &str,
        tonemap: Option<crate::engine::value::render::TonemapMode>,
    ) -> CommandResult {
        let Ok(idx) = self.resolve_output(output_uuid) else {
            return not_found(output_uuid);
        };
        self.outputs[idx].tonemap_override = tonemap;
        CommandResult::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::encoder_fps;
    use crate::engine::value::provider::ControlValue;
    use crate::engine::{CommandResult, EngineCommand as C};

    fn headless_app() -> Option<crate::app::VardaApp> {
        crate::testing::headless_app()
    }

    fn create(app: &mut crate::app::VardaApp, sink: crate::output::SinkConfig) -> String {
        match app.execute_command(C::CreateOutput { sink }) {
            CommandResult::OkWithId { uuid } => uuid,
            other => panic!("creating an output failed: {other:?}"),
        }
    }

    /// The default RTMP output (`rtmp://`) names no server. Starting it is
    /// refused with what to fill in, before ffmpeg runs, and it stays stopped.
    #[test]
    fn starting_an_rtmp_output_without_a_server_says_so() {
        let Some(mut app) = crate::testing::headless_app() else {
            return;
        };
        let uuid = create(&mut app, crate::output::SinkConfig::new("rtmp_stream"));
        let result = app.execute_command(C::StartOutput {
            output_uuid: uuid.clone(),
        });
        let CommandResult::Err { message, .. } = result else {
            panic!("starting rtmp:// should fail, got {result:?}");
        };
        assert!(message.contains("RTMP URL"), "{message}");
        let idx = app.output.resolve_output(&uuid).unwrap();
        assert!(!app.output.outputs[idx].active);
    }

    /// A setting written by name lands in the sink's saved config, and a bare
    /// type is filled with the type's defaults.
    #[test]
    fn a_sink_setting_rebuilds_the_sink_with_it() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let uuid = create(&mut app, crate::output::SinkConfig::new("recording"));
        let config = app.output.outputs[0].sink().config();
        assert!(
            config.str("path").is_some_and(|p| !p.is_empty()),
            "defaults filled in"
        );
        let result = app.execute_command(C::SetSinkParam {
            output_uuid: uuid,
            name: "path".into(),
            value: ControlValue::Text("take.mov".into()),
        });
        assert!(matches!(result, CommandResult::Ok), "{result:?}");
        assert_eq!(
            app.output.outputs[0].sink().config().str("path"),
            Some("take.mov")
        );

        // The same setting by its address, as OSC sends it.
        let result = app.execute_command(C::SetPathText {
            path: format!("output/{}/path", app.output.outputs[0].uuid),
            value: "encore.mov".into(),
        });
        assert!(matches!(result, CommandResult::Ok), "{result:?}");
        assert_eq!(
            app.output.outputs[0].sink().config().str("path"),
            Some("encore.mov")
        );
    }

    /// `output/<uuid>/surface/<surface_uuid>` shows or hides a surface, and
    /// `surface/<uuid>/source` re-routes one, through the same addresses a
    /// controller uses. See /spec/output-sink-providers.md Decisions 11 and 12.
    #[test]
    fn output_and_surface_addresses_reach_the_stage() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let output = create(&mut app, crate::output::SinkConfig::new("recording"));
        app.execute_command(C::AddSurface {
            name: "Wall".into(),
            source: crate::renderer::context::OutputSource::Master,
        });
        let surface = app.output.surface_manager.surfaces[0].uuid.clone();
        let path = format!("output/{output}/surface/{surface}");
        let toggle = app.surface_command(&path, 1.0).expect("a stage command");
        assert!(matches!(app.execute_command(toggle), CommandResult::Ok));
        assert!(app.output.outputs[0].surface_assignments[0].enabled);
        let off = app.surface_command(&path, 0.0).expect("a stage command");
        app.execute_command(off);
        assert!(!app.output.outputs[0].surface_assignments[0].enabled);

        let channel = app.mixer_ref().channels()[1].uuid().to_string();
        let result = app.execute_command(C::SetPathText {
            path: format!("surface/{surface}/source"),
            value: format!("ch/{channel}"),
        });
        assert!(matches!(result, CommandResult::Ok), "{result:?}");
        assert_eq!(
            app.output.surface_manager.surfaces[0].source,
            crate::renderer::context::OutputSource::Channel(channel)
        );
        let bad = app.execute_command(C::SetPathText {
            path: format!("surface/{surface}/source"),
            value: "nonsense".into(),
        });
        assert!(matches!(bad, CommandResult::Err { .. }));
    }

    /// What an unassigned output shows is the sink's default until chosen.
    #[test]
    fn the_unassigned_choice_overrides_the_sink_default() {
        use crate::engine::value::render::Unassigned;
        let Some(mut app) = headless_app() else {
            return;
        };
        let uuid = create(&mut app, crate::output::SinkConfig::new("recording"));
        assert_eq!(
            app.output.outputs[0].effective_unassigned(),
            Unassigned::Program
        );
        app.execute_command(C::SetOutputUnassigned {
            output_uuid: uuid,
            unassigned: Some(Unassigned::Stage),
        });
        assert_eq!(
            app.output.outputs[0].effective_unassigned(),
            Unassigned::Stage
        );
    }

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
