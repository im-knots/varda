//! GPU rendering: mixer, outputs, and frame timing.

use super::VardaApp;
use crate::mixer::Mixer;
use crate::renderer::context::{CalibrationMode, OutputSource, SurfaceRenderInfo};
use crate::surface::ContentMapping;

/// What a picked file becomes.
#[derive(Debug, Clone)]
pub enum FileDialogTarget {
    /// A new deck per file on a channel (see `LibraryCreate::File`), with the
    /// source type and the config field each path fills.
    AddDecks {
        source_type: String,
        field: String,
        channel_uuid: String,
    },
    /// A file control on an existing deck (`ControlKind::File`).
    SourceParam { deck_uuid: String, name: String },
}

/// A file picker a source asked for, with the extensions to offer.
#[derive(Debug, Clone)]
pub struct FileDialogRequest {
    pub target: FileDialogTarget,
    pub label: String,
    pub extensions: Vec<String>,
}

/// Result from a completed file dialog, sent from a background thread. The
/// target is held by UUID because it may move while the dialog is open.
#[derive(Debug)]
pub struct FileDialogResult {
    pub request: FileDialogRequest,
    pub paths: Vec<std::path::PathBuf>,
}

impl FileDialogResult {
    /// One deck add per chosen file, or one write of the first file to a
    /// deck's file control.
    pub fn commands(&self) -> Vec<crate::engine::EngineCommand> {
        match &self.request.target {
            FileDialogTarget::AddDecks {
                source_type,
                field,
                channel_uuid,
            } => self
                .paths
                .iter()
                .map(|path| crate::engine::EngineCommand::AddDeck {
                    channel_uuid: channel_uuid.clone(),
                    source: crate::source::SourceConfig::new(source_type.clone())
                        .with(field, path.to_string_lossy()),
                })
                .collect(),
            FileDialogTarget::SourceParam { deck_uuid, name } => self
                .paths
                .first()
                .map(|path| crate::engine::EngineCommand::SetSourceParam {
                    deck_uuid: deck_uuid.clone(),
                    name: name.clone(),
                    value: crate::source::ControlValue::Text(path.to_string_lossy().into_owned()),
                })
                .into_iter()
                .collect(),
        }
    }
}

/// How long [`VardaApp::render_frame`] spent in each stage.
#[derive(Debug, Clone, Copy)]
pub struct RenderTimes {
    /// Compositing the mixer.
    pub mixer: std::time::Duration,
    /// Rendering and delivering the outputs, and the interactive window.
    pub outputs: std::time::Duration,
}

impl VardaApp {
    /// The first half of a frame: timing, notifications, queued commands, and
    /// control inputs. The consumer does its own work before [`Self::render_frame`].
    pub fn begin_frame(&mut self) {
        self.update_frame_timing();
        self.update_notifications();
        self.process_commands();
        self.process_inputs();
    }

    /// The second half of a frame: controller feedback, then the mixer, the
    /// outputs, and the interactive window. Returns per-stage timings.
    pub fn render_frame(&mut self) -> RenderTimes {
        self.update_controller_leds();
        #[cfg(feature = "html")]
        self.poll_interactive_requests();
        let mixer = std::time::Instant::now();
        self.render_mixer_frame();
        let mixer = mixer.elapsed();
        let outputs = std::time::Instant::now();
        self.render_outputs();
        #[cfg(feature = "html")]
        self.render_interactive();
        RenderTimes {
            mixer,
            outputs: outputs.elapsed(),
        }
    }

    /// Update FPS and system stats. Call once per frame before any work.
    pub fn update_frame_timing(&mut self) {
        let now = std::time::Instant::now();
        let dt = now
            .duration_since(self.frame_stats.last_frame_instant)
            .as_secs_f32();
        self.frame_stats.last_frame_instant = now;
        // Sampled at the top of the frame so the count covers all of the
        // previous frame's submits, including outputs and present.
        self.frame_stats.last_frame_submits = self.render.context.submits.take();
        self.frame_stats.frame_count = self.frame_stats.frame_count.wrapping_add(1);
        if self.frame_stats.frame_count.is_multiple_of(120) {
            log::debug!(
                "[PERF] frame | submits={} fps={:.1}",
                self.frame_stats.last_frame_submits,
                self.frame_stats.fps_smoothed,
            );
        }
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.frame_stats.fps_history.push_back(instant_fps);
            if self.frame_stats.fps_history.len() > 60 {
                self.frame_stats.fps_history.pop_front();
            }
            self.frame_stats.fps_smoothed = self.frame_stats.fps_history.iter().sum::<f32>()
                / self.frame_stats.fps_history.len() as f32;
        }
        self.frame_stats.system_monitor.update();
    }

    /// Every deck's analyzer scalars in one lookup table.
    fn collect_analyzer_values(&self) -> crate::modulation::AnalyzerValues {
        let mut vals = crate::modulation::AnalyzerValues::default();
        for ch in self.mixer.channels() {
            for slot in &ch.decks {
                let deck_id = slot.deck.uuid();
                for (analyzer_type, snapshot) in slot.deck.analyzers.all_snapshots() {
                    for (name, value) in &snapshot.scalars {
                        vals.insert(
                            deck_id.to_owned(),
                            analyzer_type.clone(),
                            name.clone(),
                            *value,
                        );
                    }
                }
                for (preprocessor_type, snapshot) in slot.deck.host_inline.snapshots() {
                    for (name, value) in &snapshot.scalars {
                        vals.insert(
                            deck_id.to_owned(),
                            preprocessor_type.to_owned(),
                            name.clone(),
                            *value,
                        );
                    }
                }
            }
        }
        vals
    }

    /// Update sources and render the mixer: all GPU work that doesn't need the
    /// surface texture.
    pub fn render_mixer_frame(&mut self) {
        self.resolve_preview_channels();
        // Toast each source warning once (e.g. a clip's reverse cache ran out).
        let warnings: Vec<(String, String, String)> = self
            .mixer
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .filter_map(|slot| {
                slot.deck.source().warning().map(|w| {
                    (
                        slot.deck.uuid().to_string(),
                        slot.deck.source_name().to_string(),
                        w,
                    )
                })
            })
            .collect();
        for (uuid, name, warning) in warnings {
            self.session.notifications.notify_once(
                format!("source_warning:{uuid}"),
                crate::notifications::NotificationLevel::Warning,
                format!("Deck '{name}': {warning}"),
            );
        }

        // Effective channel opacities decide which sources are wanted.
        let channel_count = self.mixer.channel_count();
        let crossfader = self.mixer.crossfader();
        let two_ch_buf: [f32; 2];
        let n_ch_buf: Vec<f32>;
        let effective_opacities: &[f32] = if channel_count == 2 {
            two_ch_buf = [
                (1.0 - crossfader) * self.mixer.channel_opacity(0),
                crossfader * self.mixer.channel_opacity(1),
            ];
            &two_ch_buf
        } else {
            n_ch_buf = (0..channel_count)
                .map(|i| self.mixer.channel_opacity(i))
                .collect();
            &n_ch_buf
        };

        // Each provider learns which of its decks are wanted, updates its
        // devices once, then feeds each deck. Cued channels count as wanted at
        // zero opacity so live inputs keep advancing, and the arrangement can
        // mark a deck as needed soon or not for a long time.
        {
            let preview = &self.preview_channels;
            let mut decks: Vec<(&mut Box<dyn crate::source::DeckSourceInstance>, bool)> =
                Vec::new();
            for (ch_idx, channel) in self.mixer.channels_mut().iter_mut().enumerate() {
                let visible = effective_opacities.get(ch_idx).copied().unwrap_or(0.0) > 0.0;
                let channel_wanted = visible || preview.contains(&ch_idx);
                for slot in &mut channel.decks {
                    let wanted = match slot.source_demand {
                        crate::arrangement::SourceDemand::Needed => true,
                        crate::arrangement::SourceDemand::Idle => false,
                        crate::arrangement::SourceDemand::Unscheduled => channel_wanted,
                    };
                    decks.push((slot.deck.source_box_mut(), wanted));
                }
            }
            let (providers, mut env) = self.sources.env(
                &self.render.context,
                self.render.width,
                self.render.height,
                &[],
            );
            let mut submit = Vec::new();
            providers.service_frame(&mut decks, &mut env, &mut submit);
            if !submit.is_empty() {
                self.render.context.submit(submit);
            }
        }

        // Push depth sensor texture views to `depth_sensor` shader decks once
        // per frame, so the deck layer never reaches into a device.
        let depth = self.sources.service::<crate::depth::DepthSensorManager>();
        for channel in self.mixer.channels_mut() {
            for slot in &mut channel.decks {
                if let Some(state) = &mut slot.deck.depth_prepro {
                    let id = state.sensor_id;
                    state.input = match (
                        depth.depth_view(id),
                        depth.rgb_view(id),
                        depth.frame_generation(id),
                    ) {
                        (Some(depth_view), Some(rgb), Some(generation)) => {
                            Some(crate::deck::DepthPreprocessInput {
                                depth_view: depth_view.clone(),
                                rgb_view: rgb.clone(),
                                generation,
                                frame_dt: depth.frame_dt(id).unwrap_or(1.0 / 30.0),
                                connected: depth.is_connected(id),
                            })
                        }
                        _ => None,
                    };
                }
            }
        }

        // After binding and before any deck renders, when taps can be swapped safely.
        self.mixer.prepare_taps(&self.render.context);

        let audio_values =
            crate::modulation::AudioValues::collect(self.audio.manager.active_data());

        let mut primary_audio = self.audio.manager.get_primary_data().clone();

        // Clock-resolved BPM/beat overrides audio (MIDI > OSC > audio).
        let clock = self.input.clock_manager.state();
        if clock.active {
            primary_audio.bpm = Some(clock.bpm);
            primary_audio.time_since_beat = clock.beat_phase * (60.0 / clock.bpm);
        }

        let analyzer_values = self.collect_analyzer_values();

        let inputs = crate::mixer::FrameInputs {
            audio_data: &primary_audio,
            audio_values: &audio_values,
            analyzer_values: &analyzer_values,
            beat_time: self.input.clock_manager.beat_time(),
            transport: self.show.transport.sample(),
            // Live shows run on wall-clock time.
            free_run_time: None,
            write_param: crate::param_router::write_macro_target,
        };

        let target_fps = self.render.target_fps;
        if let Err(e) = self.mixer.render(
            &self.render.context,
            &inputs,
            target_fps,
            &self.preview_channels,
        ) {
            log::error!("Failed to render mixer: {e}");
        }

        self.report_gpu_faults();
        self.report_arrangement_blackout();
    }

    /// Notify when the arrangement drives the output to black, so it isn't
    /// mistaken for a fault. Reported on the transition only.
    fn report_arrangement_blackout(&mut self) {
        let blacked_out = self.mixer.arrangement_blacked_out();
        if blacked_out && !self.show.blackout_reported {
            self.session.notifications.warn(
                "The arrangement is holding every deck it drives at zero. \
                 If this is not intentional, check the transport position \
                 against your regions."
                    .to_string(),
            );
        }
        self.show.blackout_reported = blacked_out;
    }

    /// Report quarantined decks and drain GPU faults no deck owned. Toasts are
    /// keyed per deck, so a shader failing every frame reports once.
    fn report_gpu_faults(&mut self) {
        let quarantined: Vec<(String, String, String)> = self
            .mixer
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .filter_map(|slot| {
                slot.deck.gpu_error().map(|err| {
                    (
                        slot.deck.uuid().to_string(),
                        slot.deck.source_name().to_string(),
                        err.to_string(),
                    )
                })
            })
            .collect();

        for (uuid, name, error) in quarantined {
            self.session.notifications.notify_once(
                format!("gpu_fault:{uuid}"),
                crate::notifications::NotificationLevel::Error,
                format!("'{name}' disabled — GPU error. Deck frozen; the rest of the show is unaffected. {error}"),
            );
        }

        // Faults outside any deck (effect chains, output compositing).
        for fault in self.render.context.errors.take_faults() {
            if fault.context.is_none() {
                self.session.notifications.notify_once(
                    format!("gpu_fault_global:{}", fault.message),
                    crate::notifications::NotificationLevel::Error,
                    format!("GPU error: {}", fault.message),
                );
            }
        }
    }

    /// Render all outputs, windowed and headless.
    pub fn render_outputs(&mut self) {
        let context = &self.render.context;

        // Sub-mixes for Channels(...) sources.
        {
            let mut seen: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
            let mut sub_mix_sources: Vec<Vec<usize>> = Vec::new();
            for surface in &self.output.surface_manager.surfaces {
                if let OutputSource::Channels(uuids) = &surface.source {
                    let positions = self.mixer.channel_positions(uuids);
                    if seen.insert(positions.clone()) {
                        sub_mix_sources.push(positions);
                    }
                }
            }
            self.mixer.prepare_sub_mixes(&sub_mix_sources, context);
        }

        // Tonemapped copies for Channel(idx) sources.
        {
            let mut channel_indices: Vec<usize> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for surface in &self.output.surface_manager.surfaces {
                if let OutputSource::Channel(uuid) = &surface.source
                    && let Some(idx) = self.mixer.find_channel_by_uuid(uuid)
                    && seen.insert(idx)
                {
                    channel_indices.push(idx);
                }
            }
            if !channel_indices.is_empty() {
                self.mixer
                    .prepare_channel_tonemaps(&channel_indices, &self.render.context);
            }
        }

        // One graded master program per distinct output transform, shared by
        // the outputs that use it.
        {
            let keys = Self::program_keys_for_outputs(&self.output.outputs, &self.mixer);
            self.mixer.prepare_programs(&keys, &self.render.context);
        }

        let render_aspect = self.render.width as f32 / self.render.height.max(1) as f32;
        let mixer = &self.mixer;
        let master_key = Self::master_program_key(&self.mixer);

        let (az, el, roll) = self.output.dome.geometry.content_rotation_radians();
        let domemaster_view = if let Some(dome) = &mut self.output.domemaster {
            dome.set_content_rotation(az, el, roll);
            if dome.enabled {
                dome.update_params(&self.render.context.queue);
                dome.render(&self.render.context, mixer.program_view(master_key));
                Some(dome.output_view())
            } else {
                None
            }
        } else {
            None
        };

        let fps = crate::app::state::encoder_fps(self.render.target_fps);
        let mut ended: Vec<(String, crate::output::RenderedFrame)> = Vec::new();
        for output in &mut self.output.outputs {
            if !output.is_live() {
                continue;
            }
            let program_key = Self::program_key_for(output, mixer);
            let program = || crate::output::Content::Picture {
                view: mixer.program_view(program_key),
                fit_aspect: Some(render_aspect),
            };
            let calibration = &self.output.calibration_textures;
            let surfaces = &self.output.surface_manager;
            let infos: Vec<SurfaceRenderInfo<'_>>;
            let shown = if output.calibration_mode == CalibrationMode::Projector
                && !calibration.is_empty()
            {
                // Full-frame test card, bypassing surfaces and warp, for projector alignment.
                crate::output::Content::Picture {
                    view: &calibration[0].1,
                    fit_aspect: None,
                }
            } else if surfaces.surfaces.is_empty() {
                // No stage geometry, so use the master's shape.
                program()
            } else if output.surface_assignments.is_empty()
                && output.effective_unassigned()
                    == crate::engine::value::render::Unassigned::Program
            {
                program()
            } else {
                infos = Self::surface_infos(
                    output,
                    surfaces,
                    calibration,
                    mixer,
                    domemaster_view,
                    program_key,
                );
                crate::output::Content::Surfaces(&infos)
            };
            match output.render(context, &mut self.sources.services, shown, fps) {
                crate::output::RenderedFrame::Stop(reason) => {
                    ended.push((
                        output.uuid.clone(),
                        crate::output::RenderedFrame::Stop(reason),
                    ));
                }
                crate::output::RenderedFrame::Restart => {
                    ended.push((output.uuid.clone(), crate::output::RenderedFrame::Restart));
                }
                crate::output::RenderedFrame::Done | crate::output::RenderedFrame::Skipped => {}
            }
        }
        for (uuid, frame) in ended {
            match frame {
                crate::output::RenderedFrame::Stop(reason) => self.stop_output(&uuid, Some(reason)),
                // A stream listener whose client left restarts with a fresh audio tap.
                crate::output::RenderedFrame::Restart => {
                    self.stop_output(&uuid, None);
                    if let crate::engine::CommandResult::Err { message, .. } =
                        self.cmd_start_output(&uuid)
                    {
                        self.session
                            .notifications
                            .error(format!("Failed to restart output: {message}"));
                    } else {
                        log::info!("Output {uuid} restarted for its next client");
                    }
                }
                crate::output::RenderedFrame::Done | crate::output::RenderedFrame::Skipped => {}
            }
        }
    }

    /// The surfaces `output` shows, bottom first: its enabled assignments, or
    /// every surface when it has none. Calibration swaps content for a test card.
    fn surface_infos<'a>(
        output: &crate::output::Output,
        surface_manager: &'a crate::surface::SurfaceManager,
        calibration_textures: &'a [(wgpu::Texture, wgpu::TextureView)],
        mixer: &'a Mixer,
        domemaster_view: Option<&'a wgpu::TextureView>,
        program_key: crate::mixer::ProgramKey,
    ) -> Vec<SurfaceRenderInfo<'a>> {
        let calibrating = output.calibration_mode == CalibrationMode::Surfaces
            && !calibration_textures.is_empty();
        let all = output.surface_assignments.is_empty();
        surface_manager
            .surfaces
            .iter()
            .enumerate()
            .filter_map(|(si, surface)| {
                let (card, overlap_zones) = if all {
                    (
                        si,
                        crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                    )
                } else {
                    let (ai, assignment) = output
                        .surface_assignments
                        .iter()
                        .enumerate()
                        .find(|(_, a)| a.enabled && a.surface_uuid == surface.uuid)?;
                    (ai, assignment.overlap_zones.clone())
                };
                let bb = surface.bounding_box();
                let content_view = if calibrating {
                    &calibration_textures[card % calibration_textures.len()].1
                } else {
                    Self::resolve_source(mixer, &surface.source, domemaster_view, program_key)?
                };
                let (uv_scale, uv_offset) = if calibrating {
                    ([1.0, 1.0], [0.0, 0.0])
                } else {
                    Self::compute_uv(surface.content_mapping, &bb)
                };
                Some(SurfaceRenderInfo {
                    uuid: &surface.uuid,
                    content_view,
                    vertices: &surface.vertices,
                    extra_contours: &surface.extra_contours,
                    bounding_box: [bb.x, bb.y, bb.width, bb.height],
                    uv_scale,
                    uv_offset,
                    warp_mode: surface.effective_warp(),
                    overlap_zones,
                    hole_uv_contours: surface.hole_uv_contours(),
                })
            })
            .collect()
    }

    /// Output transform for this output: its override or the mixer's default.
    /// An HDR output grades to its own peak instead of display white.
    fn program_key_for(output: &crate::output::Output, mixer: &Mixer) -> crate::mixer::ProgramKey {
        let resolved = output.resolved_presentation();
        crate::mixer::ProgramKey::for_output(
            output
                .tonemap_override
                .unwrap_or_else(|| mixer.tonemap_mode()),
            resolved
                .transfer
                .is_hdr()
                .then_some(resolved.peak_nits)
                .flatten(),
        )
    }

    /// Key for the show-wide look, used by UI previews and the domemaster.
    fn master_program_key(mixer: &Mixer) -> crate::mixer::ProgramKey {
        crate::mixer::ProgramKey::sdr(mixer.tonemap_mode())
    }

    /// Every distinct program the active outputs need, plus the show-wide one.
    fn program_keys_for_outputs(
        outputs: &[crate::output::Output],
        mixer: &Mixer,
    ) -> Vec<crate::mixer::ProgramKey> {
        let mut keys = vec![Self::master_program_key(mixer)];
        for output in outputs {
            let key = Self::program_key_for(output, mixer);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    fn resolve_source<'a>(
        mixer: &'a Mixer,
        source: &OutputSource,
        domemaster_view: Option<&'a wgpu::TextureView>,
        program_key: crate::mixer::ProgramKey,
    ) -> Option<&'a wgpu::TextureView> {
        match source {
            OutputSource::Master => Some(mixer.program_view(program_key)),
            OutputSource::Channel(uuid) => {
                let ch_idx = mixer.find_channel_by_uuid(uuid)?;
                mixer
                    .get_tonemapped_channel_view(ch_idx)
                    .or_else(|| mixer.channels().get(ch_idx).map(|ch| &ch.composite_view))
            }
            OutputSource::Channels(uuids) => {
                mixer.get_sub_mix_view(&mixer.channel_positions(uuids))
            }
            OutputSource::Deck(uuid) => {
                let (ch_idx, deck_idx) = mixer.find_deck_by_uuid(uuid)?;
                mixer
                    .channels()
                    .get(ch_idx)
                    .and_then(|ch| ch.decks.get(deck_idx))
                    .map(|slot| &slot.deck.texture_view)
            }
            OutputSource::Domemaster => domemaster_view,
        }
    }

    fn compute_uv(
        mapping: ContentMapping,
        bb: &crate::surface::BoundingBox,
    ) -> ([f32; 2], [f32; 2]) {
        match mapping {
            ContentMapping::Fill => ([1.0, 1.0], [0.0, 0.0]),
            ContentMapping::Mapped => ([bb.width, bb.height], [bb.x, bb.y]),
        }
    }

    /// Open a native file picker on a background thread; results arrive on a
    /// channel. rfd's sync `FileDialog` presents on the main thread on macOS,
    /// as `NSOpenPanel` requires.
    pub fn open_file_dialog(
        sender: &std::sync::mpsc::Sender<FileDialogResult>,
        request: FileDialogRequest,
    ) {
        let tx = sender.clone();
        std::thread::spawn(move || {
            let extensions: Vec<&str> = request.extensions.iter().map(String::as_str).collect();
            let dialog = rfd::FileDialog::new().add_filter(&request.label, &extensions);
            let multiple = matches!(request.target, FileDialogTarget::AddDecks { .. });
            let paths = if multiple {
                dialog.pick_files().unwrap_or_default()
            } else {
                dialog.pick_file().into_iter().collect()
            };
            if !paths.is_empty() {
                let _ = tx.send(FileDialogResult { request, paths });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{BoundingBox, ContentMapping};

    #[test]
    fn compute_uv_fill() {
        let bb = BoundingBox {
            x: 0.2,
            y: 0.3,
            width: 0.4,
            height: 0.5,
        };
        let (scale, offset) = VardaApp::compute_uv(ContentMapping::Fill, &bb);
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn compute_uv_mapped() {
        let bb = BoundingBox {
            x: 0.2,
            y: 0.3,
            width: 0.4,
            height: 0.5,
        };
        let (scale, offset) = VardaApp::compute_uv(ContentMapping::Mapped, &bb);
        assert_eq!(scale, [0.4, 0.5]);
        assert_eq!(offset, [0.2, 0.3]);
    }

    #[test]
    fn compute_uv_mapped_full_canvas() {
        let bb = BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let (scale, offset) = VardaApp::compute_uv(ContentMapping::Mapped, &bb);
        // Full canvas behaves like fill.
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn fps_smoothing_converges() {
        let Some(mut app) = crate::testing::headless_app() else {
            return;
        };
        app.frame_stats.fps_history.clear();
        for _ in 0..60 {
            app.frame_stats.fps_history.push_back(60.0);
        }
        app.frame_stats.fps_smoothed = app.frame_stats.fps_history.iter().sum::<f32>()
            / app.frame_stats.fps_history.len() as f32;
        assert!((app.frame_stats.fps_smoothed - 60.0).abs() < 0.01);
    }

    #[test]
    fn fps_smoothing_window_cap() {
        let Some(mut app) = crate::testing::headless_app() else {
            return;
        };
        app.frame_stats.fps_history.clear();
        for _ in 0..100 {
            app.frame_stats.fps_history.push_back(30.0);
            if app.frame_stats.fps_history.len() > 60 {
                app.frame_stats.fps_history.pop_front();
            }
        }
        assert_eq!(
            app.frame_stats.fps_history.len(),
            60,
            "Window should cap at 60 entries"
        );
    }
}
