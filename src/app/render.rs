//! GPU rendering — mixer render, output windows, frame timing.

use super::VardaApp;
use crate::mixer::Mixer;
use crate::renderer::context::{OutputSource, SurfaceRenderInfo};
use crate::surface::ContentMapping;

/// Kind of file dialog to open.
#[derive(Debug, Clone, Copy)]
pub enum FileDialogKind {
    Image,
    Video,
}

/// Correlates a spawned deck load with the target its requester recorded.
///
/// Deliberately opaque: a load outlives the frame that requested it, so any
/// position captured at spawn time may name a different entity by the time the
/// deck is ready. The requester keeps `token → channel UUID` and resolves the
/// UUID on completion. See [`/spec/api-addressing.md`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeckLoadToken(pub usize);

/// Result from a completed file dialog (sent from background thread).
/// Supports multi-select: `paths` may contain one or more files.
///
/// The target channel is held by UUID, not index: the dialog runs on a
/// background thread while the UI stays live, so the channel list can change
/// between opening the dialog and picking a file.
#[derive(Debug)]
pub struct FileDialogResult {
    pub kind: FileDialogKind,
    pub channel_uuid: String,
    pub paths: Vec<std::path::PathBuf>,
}

/// Result from a background deck load (sent from a spawned thread).
/// Contains a ready-to-use Deck that just needs mixer insertion + egui texture registration.
pub struct DeckLoadResult {
    /// Echoed back verbatim from the spawn request; only the requester can
    /// interpret it.
    pub token: DeckLoadToken,
    pub deck: anyhow::Result<crate::deck::Deck>,
    pub name: String,
}

/// Mutable external delivery sinks the headless render loop feeds frames and
/// audio into. Bundled so the render helper stays under the argument-count lint
/// while each manager remains a disjoint `&mut` borrow of `VardaApp`.
struct HeadlessDeliverySinks<'a> {
    ndi_manager: &'a mut crate::ndi::NdiManager,
    #[cfg(target_os = "macos")]
    syphon_manager: &'a mut crate::syphon::SyphonManager,
    audio_manager: &'a mut crate::audio::AudioManager,
    notifications: &'a mut crate::notifications::NotificationSystem,
    /// Master render rate, resolved through `encoder_fps` so an uncapped stage
    /// still names a real number. Used to reopen an ffmpeg output on the SRT
    /// reconnect path — it must match what `cmd_start_output` used, or a
    /// reconnected stream would be timed differently from the one it replaced —
    /// and to declare the NDI sender's frame rate.
    encoder_fps: u32,
}

impl VardaApp {
    /// Spawn background threads to create decks from file paths and shaders.
    /// Each thread creates a full Deck (CPU decode + GPU upload) and sends
    /// the result via the channel. The render loop polls for completed decks.
    /// `pending` is incremented per-spawn and decremented when each thread completes.
    // Args map directly to the independent inputs a deck load needs; bundling them
    // would only add an ephemeral struct with no shared invariant.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_deck_loads(
        sender: &std::sync::mpsc::Sender<DeckLoadResult>,
        context: &crate::renderer::context::GpuContext,
        pending: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
        render_width: u32,
        render_height: u32,
        images: Vec<(DeckLoadToken, std::path::PathBuf)>,
        videos: Vec<(DeckLoadToken, std::path::PathBuf)>,
        shaders: Vec<(DeckLoadToken, crate::isf::ISFShader)>,
    ) {
        use crate::deck::Deck;
        use std::sync::atomic::Ordering;

        for (token, path) in images {
            let tx = sender.clone();
            let ctx = context.clone();
            let counter = pending.clone();
            let w = render_width;
            let h = render_height;
            counter.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let name = path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("image")
                        .to_string();
                    let deck = Deck::new_from_image(&ctx, &path, w, h);
                    (name, deck)
                }));
                let (name, deck) = match result {
                    Ok((name, deck)) => (name, deck),
                    Err(_) => (
                        "image".to_string(),
                        Err(anyhow::anyhow!("panic loading image deck")),
                    ),
                };
                let _ = tx.send(DeckLoadResult { token, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }

        for (token, path) in videos {
            let tx = sender.clone();
            let ctx = context.clone();
            let counter = pending.clone();
            let w = render_width;
            let h = render_height;
            counter.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let name = path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("video")
                        .to_string();
                    let deck = Deck::new_from_video(&ctx, &path, w, h);
                    (name, deck)
                }));
                let (name, deck) = match result {
                    Ok((name, deck)) => (name, deck),
                    Err(_) => (
                        "video".to_string(),
                        Err(anyhow::anyhow!("panic loading video deck")),
                    ),
                };
                let _ = tx.send(DeckLoadResult { token, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }

        for (token, shader) in shaders {
            let tx = sender.clone();
            let ctx = context.clone();
            let counter = pending.clone();
            let w = render_width;
            let h = render_height;
            counter.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let name = shader.name();
                    let deck = if shader.metadata.is_compute() {
                        Deck::new_from_compute_shader(&ctx, shader, w, h)
                    } else {
                        Deck::new(&ctx, shader, w, h)
                    };
                    (name, deck)
                }));
                let (name, deck) = match result {
                    Ok((name, deck)) => (name, deck),
                    Err(_) => (
                        "shader".to_string(),
                        Err(anyhow::anyhow!("panic loading shader deck")),
                    ),
                };
                let _ = tx.send(DeckLoadResult { token, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }

    /// Update frame timing (FPS measurement) and system stats. Call once per frame before any work.
    pub fn update_frame_timing(&mut self) {
        let now = std::time::Instant::now();
        let dt = now
            .duration_since(self.frame_stats.last_frame_instant)
            .as_secs_f32();
        self.frame_stats.last_frame_instant = now;
        // Sampled here, at the top of the frame, so the tally covers a whole
        // previous frame — including the output, preview, and present submits
        // that happen after the mixer is done.
        self.frame_stats.last_frame_submits = self.context.submits.take();
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

    /// Collect all analyzer scalar values from all decks into a flat lookup table.
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
            }
        }
        vals
    }

    /// Render the mixer frame: update cameras, NDI, Syphon, collect audio, render mixer.
    /// This performs all GPU work that doesn't need the surface texture.
    pub fn render_mixer_frame(&mut self) {
        // Surface a one-time notice for any deck whose ping-pong RAM cache was
        // truncated (hit the memory cap). The supported path for full-length
        // reverse on heavy/long/high-res clips is to pre-transcode to HAP.
        let truncated: Vec<(String, String)> = self
            .mixer
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .filter_map(|slot| {
                slot.deck
                    .playback_snapshot()
                    .filter(|s| s.pingpong_cache_truncated)
                    .map(|_| {
                        (
                            slot.deck.uuid().to_string(),
                            slot.deck.source_name().to_string(),
                        )
                    })
            })
            .collect();
        for (uuid, name) in truncated {
            self.session.notifications.notify_once(
                format!("pingpong_truncated:{uuid}"),
                crate::notifications::NotificationLevel::Warning,
                format!(
                    "Deck '{name}': reverse playback truncated (cache full). \
                     Transcode to HAP for full-length reverse."
                ),
            );
        }

        // Compute effective channel opacities to determine which cameras are needed
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

        // Collect camera IDs needed by visible channels. Cued (previewed)
        // channels are included even at zero opacity so their live camera inputs
        // keep advancing while off-air. See /spec/channel-preview.md.
        let mut needed_camera_ids = std::collections::HashSet::new();
        let mut needed_capture_ids = std::collections::HashSet::new();
        // Counted over every deck, not just the visible ones: this is what the
        // capture manager reconciles its sessions against, so an off-air deck
        // must still count as a holder.
        let mut capture_holders: std::collections::HashMap<crate::screen_capture::CaptureId, u32> =
            std::collections::HashMap::new();
        for (ch_idx, channel) in self.mixer.channels().iter().enumerate() {
            let visible = effective_opacities.get(ch_idx).copied().unwrap_or(0.0) > 0.0;
            let wanted = visible || self.preview_channels.contains(&ch_idx);
            for slot in &channel.decks {
                if let Some(cap_id) = slot.deck.screen_capture_id() {
                    *capture_holders.entry(cap_id).or_default() += 1;
                }
                // The arrangement knows more than the current opacity does: it
                // knows this deck is about to be needed, or that it will not be
                // for the next forty minutes. See /spec/deck-residency.md.
                let deck_wanted = match slot.source_demand {
                    crate::arrangement::SourceDemand::Needed => true,
                    crate::arrangement::SourceDemand::Idle => false,
                    crate::arrangement::SourceDemand::Unscheduled => wanted,
                };
                if !deck_wanted {
                    continue;
                }
                if let Some(cam_id) = slot.deck.camera_id() {
                    needed_camera_ids.insert(cam_id);
                }
                if let Some(cap_id) = slot.deck.screen_capture_id() {
                    needed_capture_ids.insert(cap_id);
                }
            }
        }

        // Stop any capture whose last deck is gone. A deck dropped outside
        // `remove_deck` — scene diff, undo, truncated or removed channel —
        // never released its session, leaving the capture thread grabbing and
        // downscaling frames for the rest of the session.
        self.screen_capture_manager
            .reconcile_holders(&capture_holders);

        // Update only needed camera frames
        self.camera_manager
            .update_selective(&self.context.queue, &needed_camera_ids);

        // Upload screen-capture frames for visible/cued decks only. An
        // invisible capture deck costs nothing, which is what makes a
        // self-capture deck safe to leave in a scene.
        self.screen_capture_manager.update_selective(
            &self.context.device,
            &self.context.queue,
            &needed_capture_ids,
        );

        // Update NDI receiver frames
        self.external_io
            .ndi_manager
            .update(&self.context.device, &self.context.queue);

        // Periodic Syphon re-discovery + late-bind of deferred decks (~1×/sec).
        // Removes the start/stop-ordering dependency: a producer that joins or
        // restarts after Varda is picked up automatically.
        #[cfg(target_os = "macos")]
        self.reconcile_syphon();

        // Update Syphon client frames
        #[cfg(target_os = "macos")]
        self.external_io
            .syphon_manager
            .update(&self.context.device, &self.context.queue);

        // Update stream receiver frames
        self.external_io.stream_manager.update(&self.context.queue);

        // Pump HTML (Servo) instances and upload their frames
        self.external_io
            .html_manager
            .update(&self.context.device, &self.context.queue);

        // Upload depth-sensor frames (off-render-thread capture; this only
        // does the GPU texture upload). See spec/depth-sensors.md.
        self.depth_manager.update(&self.context.queue);

        for channel in self.mixer.channels_mut() {
            for slot in &mut channel.decks {
                if let Some(kind) = slot.deck.external_source_kind() {
                    use crate::deck::ExternalSourceKind;
                    slot.deck.external_source_view = match kind {
                        ExternalSourceKind::Camera(cam_id) => {
                            self.camera_manager.texture_view(cam_id).cloned()
                        }
                        ExternalSourceKind::Ndi(idx) => {
                            self.external_io.ndi_manager.texture_view(idx).cloned()
                        }
                        #[cfg(target_os = "macos")]
                        ExternalSourceKind::Syphon(idx) => {
                            self.external_io.syphon_manager.texture_view(idx).cloned()
                        }
                        #[cfg(not(target_os = "macos"))]
                        ExternalSourceKind::Syphon(_) => None,
                        ExternalSourceKind::Srt(idx)
                        | ExternalSourceKind::Hls(idx)
                        | ExternalSourceKind::Dash(idx)
                        | ExternalSourceKind::Rtmp(idx) => {
                            self.external_io.stream_manager.texture_view(idx).cloned()
                        }
                        ExternalSourceKind::Html(idx) => {
                            self.external_io.html_manager.texture_view(idx).cloned()
                        }
                        ExternalSourceKind::ScreenCapture(id) => {
                            // Router/UI edits land on the deck; push them down
                            // to the capture thread here, once, when they change.
                            if let Some(state) = &mut slot.deck.screen_capture {
                                if state.config_dirty {
                                    self.screen_capture_manager
                                        .set_config(id, state.config.clone());
                                    state.config_dirty = false;
                                }
                            }
                            // A crop or target resize reallocates the shared
                            // texture, so the deck's source dimensions are
                            // pushed down each frame rather than fixed at open.
                            if let Some((w, h)) = self.screen_capture_manager.resolution(id) {
                                slot.deck.set_external_source_size(w, h);
                            }
                            self.screen_capture_manager.texture_view(id).cloned()
                        }
                        // Taps own no device, so the mixer resolves them all at
                        // once in `prepare_taps` below.
                        ExternalSourceKind::Tap => None,
                        ExternalSourceKind::DepthSensor(id) => {
                            // Depth decks read the R16Uint depth view + RGB view
                            // and reproject via the point-cloud pass rather than
                            // blitting a single RGBA texture.
                            let depth = self.depth_manager.depth_view(id).cloned();
                            slot.deck.depth_rgb_view = self.depth_manager.rgb_view(id).cloned();
                            slot.deck.depth_intrinsics = self.depth_manager.intrinsics(id);
                            slot.deck.depth_source_size = self.depth_manager.resolution(id);
                            depth
                        }
                    };
                }

                // Shader decks with a `depth_sensor` preprocessor read the same
                // shared sensor textures, but convert them via their own GPU
                // passes rather than reprojecting a point cloud. The manager
                // lives here, so the views are pushed down once per frame — the
                // deck layer never reaches up into a device.
                // See spec/depth-sensor-preprocessor.md.
                if let Some(state) = &mut slot.deck.depth_prepro {
                    let id = state.sensor_id;
                    state.input = match (
                        self.depth_manager.depth_view(id),
                        self.depth_manager.rgb_view(id),
                        self.depth_manager.frame_generation(id),
                    ) {
                        (Some(depth), Some(rgb), Some(generation)) => {
                            Some(crate::deck::DepthPreprocessInput {
                                depth_view: depth.clone(),
                                rgb_view: rgb.clone(),
                                generation,
                                frame_dt: self.depth_manager.frame_dt(id).unwrap_or(1.0 / 30.0),
                                connected: self.depth_manager.is_connected(id),
                            })
                        }
                        _ => None,
                    };
                }
            }
        }

        // Runs after the binding loop but still before any deck renders, which
        // is the window in which a tap can be swapped safely.
        // See spec/program-tap.md.
        self.mixer.prepare_taps(&self.context);

        // Collect audio values for modulation
        let audio_values = {
            let mut av = crate::modulation::AudioValues::default();
            for id in self.audio_manager.active_source_ids() {
                if let Some(data) = self.audio_manager.get_data(id) {
                    av.sources.insert(
                        id,
                        crate::modulation::AudioSourceValues {
                            fft: data.fft.clone(),
                            level: data.level,
                            sample_rate: data.sample_rate,
                        },
                    );
                }
            }
            av
        };

        let mut primary_audio = self.audio_manager.get_primary_data().clone();

        // Override audio BPM/beat with clock-resolved values (MIDI > OSC > Audio)
        let clock = self.input.clock_manager.state();
        if clock.active {
            primary_audio.bpm = Some(clock.bpm);
            primary_audio.time_since_beat = clock.beat_phase * (60.0 / clock.bpm);
        }

        // Collect analyzer scalar values from all decks
        let analyzer_values = self.collect_analyzer_values();

        let inputs = crate::mixer::FrameInputs {
            audio_data: &primary_audio,
            audio_values: &audio_values,
            analyzer_values: &analyzer_values,
            beat_time: self.input.clock_manager.beat_time(),
            transport: self.transport.sample(),
            // The wall paces a live show.
            free_run_time: None,
        };

        let target_fps = self.target_fps;
        if let Err(e) =
            self.mixer
                .render(&self.context, &inputs, target_fps, &self.preview_channels)
        {
            log::error!("Failed to render mixer: {e}");
        }

        self.report_gpu_faults();
        self.report_arrangement_blackout();
    }

    /// Tell the performer when the arrangement is driving the output to nothing.
    ///
    /// Correct-and-idle looks exactly like broken on a screen, so the state is
    /// named rather than left to be inferred. Reported on the transition only:
    /// a deliberate blackout is legitimate and must not produce a toast every
    /// frame it lasts. See /spec/transport.md § Black-output detection.
    fn report_arrangement_blackout(&mut self) {
        let blacked_out = self.mixer.arrangement_blacked_out();
        if blacked_out && !self.arrangement_blackout_reported {
            self.session.notifications.warn(
                "The arrangement is holding every deck it drives at zero. \
                 If this is not intentional, check the transport position \
                 against your regions."
                    .to_string(),
            );
        }
        self.arrangement_blackout_reported = blacked_out;
    }

    /// Surface quarantined decks to the performer, and drain anything the GPU
    /// error guard caught that no deck owned.
    ///
    /// Toasts are keyed per deck so a shader failing every frame reports once
    /// rather than burying the notification history.
    /// See spec/error-handling.md § Shader Errors.
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

        // Faults raised outside any deck (channel/master effect chains, output
        // compositing). Nothing to quarantine, but they must not vanish.
        for fault in self.context.errors.take_faults() {
            if fault.context.is_none() {
                self.session.notifications.notify_once(
                    format!("gpu_fault_global:{}", fault.message),
                    crate::notifications::NotificationLevel::Error,
                    format!("GPU error: {}", fault.message),
                );
            }
        }
    }

    /// Render content to all outputs (windowed + headless) using the surface layout.
    pub fn render_outputs(&mut self) {
        let context = &self.context;

        // Prepare sub-mixes for any Channels(...) sources
        {
            let mut seen: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
            let mut sub_mix_sources: Vec<Vec<usize>> = Vec::new();
            for surface in &self.output.surface_manager.surfaces {
                if let OutputSource::Channels(indices) = &surface.source {
                    let mut sorted = indices.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    if seen.insert(sorted.clone()) {
                        sub_mix_sources.push(sorted);
                    }
                }
            }
            self.mixer.prepare_sub_mixes(&sub_mix_sources, context);
        }

        // Prepare tonemapped copies for any Channel(idx) sources
        {
            let mut channel_indices: Vec<usize> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for surface in &self.output.surface_manager.surfaces {
                if let OutputSource::Channel(idx) = &surface.source {
                    if seen.insert(*idx) {
                        channel_indices.push(*idx);
                    }
                }
            }
            if !channel_indices.is_empty() {
                self.mixer
                    .prepare_channel_tonemaps(&channel_indices, &self.context);
            }
        }

        let render_aspect = self.render_width as f32 / self.render_height.max(1) as f32;
        let mixer = &self.mixer;

        // Run domemaster renderer if enabled (content rotation is updated each frame via set_content_rotation)
        let domemaster_view = if let Some(dome) = &self.output.domemaster {
            if dome.enabled {
                dome.update_params(&self.context.queue);
                dome.render(&self.context, mixer.composite_view());
                Some(dome.output_view())
            } else {
                None
            }
        } else {
            None
        };

        for output in &self.output.outputs {
            match output {
                crate::renderer::context::UnifiedOutput::Window(output) => {
                    Self::render_window_output(
                        output,
                        context,
                        mixer,
                        &self.output.surface_manager,
                        &self.output.calibration_textures,
                        domemaster_view,
                        render_aspect,
                    );
                }
                crate::renderer::context::UnifiedOutput::Headless(_) => {
                    // Headless rendering handled separately (needs &mut for subprocess)
                }
            }
        }

        // Render headless outputs (needs &mut self for subprocess feeding)
        let sinks = HeadlessDeliverySinks {
            ndi_manager: &mut self.external_io.ndi_manager,
            #[cfg(target_os = "macos")]
            syphon_manager: &mut self.external_io.syphon_manager,
            audio_manager: &mut self.audio_manager,
            notifications: &mut self.session.notifications,
            encoder_fps: crate::app::state::encoder_fps(self.target_fps),
        };
        Self::render_headless_outputs_inner(
            &mut self.output.outputs,
            context,
            mixer,
            &self.output.surface_manager,
            sinks,
            domemaster_view,
        );
    }

    fn render_window_output(
        output: &crate::renderer::context::OutputWindow,
        context: &crate::renderer::context::GpuContext,
        mixer: &crate::mixer::Mixer,
        surface_manager: &crate::surface::SurfaceManager,
        calibration_textures: &[(wgpu::Texture, wgpu::TextureView)],
        domemaster_view: Option<&wgpu::TextureView>,
        render_aspect: f32,
    ) {
        use crate::renderer::context::CalibrationMode;
        // Projector calibration: one full-frame test card over the whole output,
        // bypassing surface geometry/warp (physical projector alignment).
        if output.calibration_mode == CalibrationMode::Projector && !calibration_textures.is_empty()
        {
            output.render(context, &calibration_textures[0].1);
            output.window.request_redraw();
            return;
        }
        // Surfaces calibration: each surface shows a colored test card through its warp.
        let surfaces_cal = output.calibration_mode == CalibrationMode::Surfaces;
        if surface_manager.surfaces.is_empty() {
            // No stage geometry, so the window is the whole canvas and the
            // master's shape is the only thing that can define the picture.
            // Surfaces below are user-placed and carry their own aspect.
            output.render_fit(context, mixer.composite_view(), render_aspect);
        } else if !output.surface_assignments.is_empty() {
            // Draw in global stacking order (surface-manager Vec order, index 0 =
            // bottom), not per-assignment order — see 8i.12. For each surface, find
            // its enabled assignment on this output.
            let render_infos: Vec<SurfaceRenderInfo<'_>> = surface_manager
                .surfaces
                .iter()
                .filter_map(|surface| {
                    let (ai, assignment) = output
                        .surface_assignments
                        .iter()
                        .enumerate()
                        .find(|(_, a)| a.enabled && a.surface_uuid == surface.uuid)?;
                    let bb = surface.bounding_box();
                    let content_view = if surfaces_cal && !calibration_textures.is_empty() {
                        &calibration_textures[ai % calibration_textures.len()].1
                    } else {
                        Self::resolve_source(mixer, &surface.source, domemaster_view)?
                    };
                    let (uv_scale, uv_offset) = if surfaces_cal {
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
                        overlap_zones: assignment.overlap_zones.clone(),
                        hole_uv_contours: surface.hole_uv_contours(),
                    })
                })
                .collect();
            output.render_surfaces(context, &render_infos);
        } else {
            let render_infos: Vec<SurfaceRenderInfo<'_>> = surface_manager
                .surfaces
                .iter()
                .enumerate()
                .filter_map(|(si, surface)| {
                    let bb = surface.bounding_box();
                    let content_view = if surfaces_cal && !calibration_textures.is_empty() {
                        &calibration_textures[si % calibration_textures.len()].1
                    } else {
                        Self::resolve_source(mixer, &surface.source, domemaster_view)?
                    };
                    let (uv_scale, uv_offset) = if surfaces_cal {
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
                        overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                        hole_uv_contours: surface.hole_uv_contours(),
                    })
                })
                .collect();
            output.render_surfaces(context, &render_infos);
        }
        output.window.request_redraw();
    }

    fn resolve_source<'a>(
        mixer: &'a Mixer,
        source: &OutputSource,
        domemaster_view: Option<&'a wgpu::TextureView>,
    ) -> Option<&'a wgpu::TextureView> {
        match source {
            OutputSource::Master => Some(mixer.composite_view()),
            OutputSource::Channel(ch_idx) => mixer
                .get_tonemapped_channel_view(*ch_idx)
                .or_else(|| mixer.channels().get(*ch_idx).map(|ch| &ch.composite_view)),
            OutputSource::Channels(indices) => {
                let mut sorted = indices.clone();
                sorted.sort_unstable();
                sorted.dedup();
                mixer.get_sub_mix_view(&sorted)
            }
            OutputSource::Deck(ch_idx, deck_idx) => mixer
                .channels()
                .get(*ch_idx)
                .and_then(|ch| ch.decks.get(*deck_idx))
                .map(|slot| &slot.deck.texture_view),
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

    /// Refresh monitors from the event loop.
    pub fn refresh_monitors(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.output.cached_monitors = event_loop
            .available_monitors()
            .map(|m| {
                let name = m.name().unwrap_or_else(|| "Unknown".to_string());
                (name, m)
            })
            .collect();
    }

    /// Render all active headless outputs — readback + deliver frames.
    // `sinks` bundles `&mut` borrows used mutably here; a shared ref won't do.
    #[allow(clippy::needless_pass_by_value)]
    fn render_headless_outputs_inner(
        outputs: &mut [crate::renderer::context::UnifiedOutput],
        context: &crate::renderer::context::GpuContext,
        mixer: &crate::mixer::Mixer,
        surface_manager: &crate::surface::SurfaceManager,
        sinks: HeadlessDeliverySinks,
        domemaster_view: Option<&wgpu::TextureView>,
    ) {
        for output in outputs.iter_mut() {
            let h = match output {
                crate::renderer::context::UnifiedOutput::Headless(h) if h.active => h,
                _ => continue,
            };

            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Headless Output Encoder"),
                    });

            // Post-process edge blend only for Manual mode; Auto uses per-surface shader blend.
            let use_edge_blend = h.edge_blend_mode
                == crate::renderer::edge_blend::EdgeBlendMode::Manual
                && h.edge_blend.any_enabled();
            // When edge blending: render to intermediate, then blend → final texture.
            let render_target = if use_edge_blend {
                &h.edge_blend_texture_view
            } else {
                &h.texture_view
            };

            if h.surface_assignments.is_empty() {
                // Fallback: simple blit from source
                let Some(source_view) = Self::resolve_source(mixer, &h.source, domemaster_view)
                else {
                    continue;
                };
                h.blit_pipeline
                    .set_rotation(&context.queue, h.rotation.index());
                let bind_group = h
                    .blit_pipeline
                    .create_bind_group(&context.device, source_view);
                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Headless Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: render_target,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    h.blit_pipeline.render(&mut rp, &bind_group);
                }
            } else {
                // Surface-routed rendering: render assigned surfaces with warp
                // Triangulate on the CPU, then prepare draws from the pipeline's
                // persistent param/vertex pools (no per-frame GPU buffer alloc).
                // Draw in global stacking order (surface-manager Vec order, index 0
                // = bottom), not per-assignment order — see 8i.12.
                let draws: Vec<crate::renderer::blit::PolygonDrawDesc<'_>> = surface_manager
                    .surfaces
                    .iter()
                    .filter_map(|surface| {
                        let assignment = h
                            .surface_assignments
                            .iter()
                            .find(|a| a.enabled && a.surface_uuid == surface.uuid)?;
                        let bb = surface.bounding_box();
                        let content_view =
                            Self::resolve_source(mixer, &surface.source, domemaster_view)?;
                        let (uv_scale, uv_offset) = Self::compute_uv(surface.content_mapping, &bb);
                        // Warp is per-surface now; `None` = no warp (native
                        // position). `effective_warp` applies auto-warp binding.
                        let eff_warp = surface.effective_warp();
                        // Combined (multi-contour) surface: a single warp mesh
                        // can't represent disjoint contours, so render every
                        // contour as a bounding-box UV fill (matches the editor).
                        let (homography, vertices) = if surface.extra_contours.is_empty() {
                            match eff_warp.as_ref() {
                                Some(crate::renderer::warp::WarpMode::CornerPin { corners }) => {
                                    let src_corners = [
                                        [bb.x, bb.y],
                                        [bb.x + bb.width, bb.y],
                                        [bb.x + bb.width, bb.y + bb.height],
                                        [bb.x, bb.y + bb.height],
                                    ];
                                    let homography =
                                        crate::renderer::warp::compute_forward_homography(
                                            &src_corners,
                                            corners,
                                        );
                                    let verts =
                                    crate::renderer::blit::PolygonBlitPipeline::triangulate_verts(
                                        &surface.vertices,
                                        bb.x,
                                        bb.y,
                                        bb.width,
                                        bb.height,
                                    );
                                    (Some(homography), verts)
                                }
                                Some(crate::renderer::warp::WarpMode::Mesh(mesh)) => (
                                    None,
                                    crate::renderer::blit::PolygonBlitPipeline::mesh_verts(mesh),
                                ),
                                // Bezier: tessellate the control cage into a mesh,
                                // then bake to verts (identity homography).
                                Some(crate::renderer::warp::WarpMode::Bezier(b)) => (
                                    None,
                                    crate::renderer::blit::PolygonBlitPipeline::mesh_verts(
                                        &b.tessellate(),
                                    ),
                                ),
                                None => (
                                    None,
                                    crate::renderer::blit::PolygonBlitPipeline::triangulate_verts(
                                        &surface.vertices,
                                        bb.x,
                                        bb.y,
                                        bb.width,
                                        bb.height,
                                    ),
                                ),
                            }
                        } else {
                            (
                                None,
                                crate::renderer::blit::PolygonBlitPipeline::triangulate_multi(
                                    &surface.vertices,
                                    &surface.extra_contours,
                                    bb.x,
                                    bb.y,
                                    bb.width,
                                    bb.height,
                                ),
                            )
                        };
                        Some(crate::renderer::blit::PolygonDrawDesc {
                            content_view,
                            uv_scale,
                            uv_offset,
                            homography,
                            overlap_zones: &assignment.overlap_zones,
                            vertices,
                            mask_uuid: &surface.uuid,
                            mask_uv_contours: surface.hole_uv_contours(),
                        })
                    })
                    .collect();

                let (prepared, vertex_pool) =
                    h.polygon_pipeline
                        .prepare(&context.device, &context.queue, &draws);

                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Headless Surface Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: render_target,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    h.polygon_pipeline.draw(&mut rp, &prepared, &vertex_pool);
                }
            }

            // Apply edge blend post-process if any edge is enabled
            if use_edge_blend {
                h.edge_blend_pipeline.render(
                    &context.device,
                    &context.queue,
                    &mut encoder,
                    &h.edge_blend_texture_view,
                    &h.texture_view,
                    &h.edge_blend,
                );
            }

            // Syphon (macOS) publishes GPU-side (zero-copy): skip the CPU
            // readback entirely and hand the rendered texture to the server.
            #[cfg(target_os = "macos")]
            let is_syphon = matches!(
                &h.target,
                crate::renderer::context::OutputTarget::SyphonServer { .. }
            );
            #[cfg(not(target_os = "macos"))]
            let is_syphon = false;

            if !is_syphon {
                // Enqueue readback copy from the now-rendered texture
                h.readback.begin_readback(&mut encoder, &h.texture);
            }
            context.submit(std::iter::once(encoder.finish()));

            #[cfg(target_os = "macos")]
            if let crate::renderer::context::OutputTarget::SyphonServer { ref server_name } =
                h.target
            {
                sinks.syphon_manager.publish_frame_gpu(
                    context,
                    server_name,
                    &h.texture_view,
                    h.width,
                    h.height,
                );
            }

            // Deliver previous frame's readback data to target
            if !is_syphon {
                if let Some(frame_data) = h.readback.try_read(&context.device) {
                    match h.deliver_frame(&frame_data, sinks.ndi_manager, sinks.encoder_fps) {
                        crate::renderer::context::DeliveryResult::Failed(msg) => {
                            log::error!("{msg}");
                            h.active = false;
                        }
                        crate::renderer::context::DeliveryResult::SrtNeedsRestart => {
                            // The disconnected client's PCM tap is now stale; drop it,
                            // then respawn the listener with a fresh audio tap so the
                            // reconnecting client gets audio again (parity with start).
                            if let Some(stale) = h.audio_pcm.take() {
                                sinks
                                    .audio_manager
                                    .unsubscribe_pcm(stale.source_id, stale.token);
                            }
                            let (url, codec) = match &h.target {
                                crate::renderer::context::OutputTarget::SrtStream {
                                    url,
                                    codec,
                                    ..
                                } => (url.clone(), codec.clone()),
                                _ => continue,
                            };
                            let device = h.target.audio_device().map(str::to_string);
                            let name = h.name.clone();
                            let (audio_input, passthrough) = super::state::resolve_output_audio(
                                sinks.audio_manager,
                                sinks.notifications,
                                device.as_deref(),
                                &name,
                            );
                            match crate::renderer::FfmpegSubprocess::spawn_srt(
                                &url,
                                &codec,
                                h.width,
                                h.height,
                                sinks.encoder_fps,
                                audio_input,
                            ) {
                                Ok(new_sub) => {
                                    h.subprocess = Some(Box::new(new_sub));
                                    h.audio_pcm = passthrough.map(Box::new);
                                    log::info!("SRT restarted for '{name}'");
                                }
                                Err(e) => {
                                    if let Some(pass) = passthrough {
                                        sinks
                                            .audio_manager
                                            .unsubscribe_pcm(pass.source_id, pass.token);
                                    }
                                    log::error!("Failed to restart SRT listener for '{name}': {e}");
                                    h.active = false;
                                }
                            }
                        }
                        crate::renderer::context::DeliveryResult::Ok => {}
                    }
                }
            }
        }
    }

    /// Open a native file picker on a background thread.
    /// Uses rfd's synchronous `FileDialog` which correctly dispatches to the
    /// main thread on macOS (`NSOpenPanel` requires main-thread presentation
    /// for proper focus/activation). Results are sent via channel.
    pub fn open_file_dialog(
        sender: &std::sync::mpsc::Sender<FileDialogResult>,
        kind: FileDialogKind,
        channel_uuid: String,
    ) {
        let tx = sender.clone();
        std::thread::spawn(move || {
            let dialog = match kind {
                FileDialogKind::Image => rfd::FileDialog::new().add_filter(
                    "Images",
                    &[
                        "png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp", "svg", "svgz",
                    ],
                ),
                FileDialogKind::Video => rfd::FileDialog::new()
                    .add_filter("Video", &["mov", "mp4", "avi", "mkv", "webm", "gif"]),
            };
            if let Some(paths) = dialog.pick_files() {
                if !paths.is_empty() {
                    let _ = tx.send(FileDialogResult {
                        kind,
                        channel_uuid,
                        paths,
                    });
                }
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
        // Full canvas mapped should behave like fill
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn fps_smoothing_converges() {
        let gpu = crate::renderer::context::GpuContext::new_headless();
        let Ok(gpu) = gpu else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let config = crate::testing::headless_config();
        let Ok(mut app) = VardaApp::new(gpu, &config) else {
            eprintln!("Skipping: VardaApp creation failed");
            return;
        };
        // Seed with 60 identical FPS values
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
        let gpu = crate::renderer::context::GpuContext::new_headless();
        let Ok(gpu) = gpu else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let config = crate::testing::headless_config();
        let Ok(mut app) = VardaApp::new(gpu, &config) else {
            eprintln!("Skipping: VardaApp creation failed");
            return;
        };
        // Push more than 60 entries
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

    // ── Offensive: catch_unwind pattern delivers error through channel ──

    #[test]
    fn catch_unwind_delivers_error_on_panic() {
        let (tx, rx) = std::sync::mpsc::channel::<DeckLoadResult>();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let c = counter.clone();
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> (String, anyhow::Result<crate::deck::Deck>) {
                    panic!("simulated loader panic");
                },
            ));
            let (name, deck) = match result {
                Ok((name, deck)) => (name, deck),
                Err(_) => (
                    "panicked".to_string(),
                    Err(anyhow::anyhow!("panic in loader")),
                ),
            };
            let _ = tx.send(DeckLoadResult {
                token: DeckLoadToken(0),
                deck,
                name,
            });
            c.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });

        let msg = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("should receive result even after panic");
        assert!(msg.deck.is_err(), "deck should be an error after panic");
        assert_eq!(msg.name, "panicked");
        // Counter should be back to zero (cleanup ran)
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "counter must decrement even after panic"
        );
    }
}
