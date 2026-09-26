//! Engine operations that span several owners: building and removing decks
//! (GPU context, render size, and the device each source needs), effect chains
//! with their analyzers and depth preprocessors, transitions and LUTs, routed
//! parameter writes, and output windows.

use super::VardaApp;
use super::deck_loads::DeckSource;
use crate::deck::{Deck, Effect};
use crate::depth::preprocess::{AcquiredSensor, DepthPreprocessParams};
use crate::engine::types::{CameraId, EffectTarget, ParamValue};
use crate::mixer::EffectChain;

use anyhow::{Context as _, Result};

impl VardaApp {
    /// Post-construction wiring every new shader deck needs before it joins a
    /// channel: start its CPU analyzers and acquire any device its
    /// `PREPROCESSORS` block requires.
    ///
    /// Every deck built from a shader passes through here: the background
    /// loader (`deck_loads`) calls it when a deck attaches. Skipping it leaves a
    /// `depth_sensor` shader rendering against blank 1x1 textures with no error.
    ///
    /// Returns `Err` when a required preprocessor cannot be satisfied; the
    /// caller must discard the deck and surface the message.
    pub(crate) fn finalize_new_deck(&mut self, deck: &mut Deck) -> Result<()> {
        deck.ensure_preprocessor_analyzers(&self.sources.analyzer_registry);
        let Some(metadata) = deck.shader().map(|s| s.metadata.clone()) else {
            return Ok(());
        };
        let name = deck.source_name().to_string();
        if let Some(sensor) = self.acquire_depth_preprocessor(&metadata, &name)? {
            deck.attach_depth_preprocessor(
                sensor.id,
                sensor.name,
                sensor.pipeline,
                DepthPreprocessParams::default(),
            );
        }
        Ok(())
    }

    /// Acquire the depth sensor a shader's `PREPROCESSORS` block requires.
    ///
    /// `Ok(None)` when the shader declares no `depth_sensor` preprocessor; `Err`
    /// when it does but no sensor is available. `depth_sensor` is a *required*
    /// preprocessor, so callers must propagate the error and abandon the load
    /// rather than degrade to a deck with nothing to draw.
    /// See spec/depth-sensor-preprocessor.md § Device Acquisition.
    fn acquire_depth_preprocessor(
        &mut self,
        metadata: &crate::isf::ISFMetadata,
        shader_name: &str,
    ) -> Result<Option<AcquiredSensor>> {
        self.check_required_preprocessors(metadata, shader_name)?;
        crate::depth::preprocess::acquire_for_shader(
            &mut self.sources.depth_manager,
            &self.render.context.device,
            metadata,
            shader_name,
        )
    }

    /// Reject a shader that declares a required preprocessor the engine cannot
    /// service. Requiredness is a registry property, so this stays correct as
    /// device-backed types are added.
    ///
    /// Unknown and optional types are *not* rejected — they degrade to default
    /// outputs, per /spec/effect-preprocessing.md Decision #2.
    fn check_required_preprocessors(
        &self,
        metadata: &crate::isf::ISFMetadata,
        shader_name: &str,
    ) -> Result<()> {
        for pp in &metadata.preprocessors {
            let ty = pp.preprocessor_type.as_str();
            let Some(category) = self.sources.analyzer_registry.category_for(ty) else {
                log::warn!(
                    "Shader '{shader_name}' declares unknown preprocessor '{ty}'; \
                     its outputs will be blank"
                );
                continue;
            };
            if category.is_required() && ty != crate::depth::preprocess::PREPROCESSOR_TYPE {
                anyhow::bail!(
                    "Shader '{shader_name}' requires preprocessor '{ty}', which this build \
                     cannot provide."
                );
            }
        }
        Ok(())
    }
}

impl VardaApp {
    pub(crate) fn set_crossfader(&mut self, position: f32) {
        self.mixer.snap_crossfader(position);
        if let Some(ref sender) = self.input.osc_feedback {
            sender.send_param("crossfader", self.mixer.crossfader());
        }
    }

    pub(crate) fn add_deck(&mut self, channel_uuid: &str, shader_name: &str) -> Result<String> {
        self.mixer.resolve_channel(channel_uuid)?;
        let shader = self
            .sources
            .registry
            .generators()
            .iter()
            .find(|s| s.name() == shader_name)
            .map(|s| (*s).clone())
            .context("Shader not found")?;
        self.check_required_preprocessors(&shader.metadata, shader_name)?;
        crate::depth::preprocess::preflight_for_shader(
            &self.sources.depth_manager,
            &shader.metadata,
            shader_name,
        )?;
        Ok(self.spawn_deck_load(channel_uuid, DeckSource::Shader(Box::new(shader))))
    }

    pub(crate) fn add_image_deck(
        &mut self,
        channel_uuid: &str,
        path: &std::path::Path,
    ) -> Result<String> {
        self.mixer.resolve_channel(channel_uuid)?;
        anyhow::ensure!(path.is_file(), "Image file not found: {}", path.display());
        Ok(self.spawn_deck_load(channel_uuid, DeckSource::Image(path.to_path_buf())))
    }

    pub(crate) fn add_video_deck(
        &mut self,
        channel_uuid: &str,
        path: &std::path::Path,
    ) -> Result<String> {
        self.mixer.resolve_channel(channel_uuid)?;
        anyhow::ensure!(path.is_file(), "Video file not found: {}", path.display());
        Ok(self.spawn_deck_load(channel_uuid, DeckSource::Video(path.to_path_buf())))
    }

    pub(crate) fn add_solid_color_deck(
        &mut self,
        channel_uuid: &str,
        color: [f32; 4],
    ) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        let deck = Deck::new_solid_color(
            &self.render.context,
            color,
            self.render.width,
            self.render.height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let name = deck.source_name().to_string();
        let idx = ch.add_deck(deck);
        log::info!("Added solid color deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    pub(crate) fn add_camera_deck(
        &mut self,
        channel_uuid: &str,
        camera_id: CameraId,
    ) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        let cam_name = self
            .sources
            .camera_manager
            .devices()
            .iter()
            .find(|d| d.id == camera_id)
            .map_or_else(|| format!("Camera {camera_id}"), |d| d.name.clone());
        let (src_w, src_h) = self
            .sources
            .camera_manager
            .open_camera(camera_id, &self.render.context.device)?;
        let deck = Deck::new_from_camera(
            &self.render.context,
            camera_id,
            &cam_name,
            src_w,
            src_h,
            self.render.width,
            self.render.height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added camera deck {idx} to channel {channel_idx}: {cam_name}");
        Ok(uuid)
    }

    pub(crate) fn add_screen_capture_deck(
        &mut self,
        channel_uuid: &str,
        target: &crate::scene::CaptureTargetConfig,
        options: &crate::screen_capture::backend::CaptureConfig,
    ) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        let identity = crate::screen_capture::backend::TargetIdentity::from(target);
        let info = self
            .sources
            .screen_capture_manager
            .find_target(&identity)
            .cloned()
            .with_context(|| {
                format!(
                    "No capture target matches '{}' — rescan and try again",
                    target.label()
                )
            })?;

        // Default the capture-time downscale to the largest deck-sized frame
        // that keeps the target's own shape. Without a cap a 4K display would
        // move 33 MB per frame only to be scaled down immediately after; without
        // the shape, a window narrower than the stage would arrive pre-squashed
        // and the deck's scaling mode would have nothing left to do.
        // See spec/screen-capture.md § Performance.
        let config = crate::screen_capture::backend::CaptureConfig {
            scale_to: options.scale_to.or_else(|| {
                Some(crate::screen_capture::resample::fit_within(
                    info.width,
                    info.height,
                    self.render.width,
                    self.render.height,
                ))
            }),
            ..options.clone()
        };
        let (capture_id, src_w, src_h) = self
            .sources
            .screen_capture_manager
            .open(&info, config.clone(), &self.render.context.device)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let deck = Deck::new_from_screen_capture(
            &self.render.context,
            crate::deck::ScreenCaptureState {
                capture_id,
                identity,
                config,
                config_dirty: false,
            },
            &info.label,
            src_w,
            src_h,
            self.render.width,
            self.render.height,
        )
        .inspect_err(|_| self.sources.screen_capture_manager.release(capture_id))?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!(
            "Added screen capture deck {idx} to channel {channel_idx}: {}",
            info.label
        );
        Ok(uuid)
    }

    pub(crate) fn add_tap_deck(
        &mut self,
        channel_uuid: &str,
        source: &crate::scene::TapSourceConfig,
    ) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        let tap_source = crate::deck::TapSource::from(source);
        let label = tap_source.label(&self.mixer.channel_labels());
        let deck = Deck::new_from_tap(
            &self.render.context,
            tap_source,
            &label,
            self.render.width,
            self.render.height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added tap deck {idx} to channel {channel_idx}: {label}");
        Ok(uuid)
    }

    pub(crate) fn set_tap_source(
        &mut self,
        deck_uuid: &str,
        source: &crate::scene::TapSourceConfig,
    ) -> Result<()> {
        self.mixer
            .set_tap_source(deck_uuid, crate::deck::TapSource::from(source))
    }

    pub(crate) fn add_depth_sensor_deck(
        &mut self,
        channel_uuid: &str,
        depth_sensor_id: crate::depth::DepthSensorId,
    ) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        let name = self
            .sources
            .depth_manager
            .devices()
            .iter()
            .find(|d| d.id == depth_sensor_id)
            .map_or_else(
                || format!("Depth Sensor {depth_sensor_id}"),
                |d| d.name.clone(),
            );
        let (src_w, src_h) = crate::depth::open_depth_sensor(
            &mut self.sources.depth_manager,
            depth_sensor_id,
            &self.render.context.device,
        )?;
        let deck = Deck::new_from_depth_sensor(
            &self.render.context,
            depth_sensor_id,
            &name,
            src_w,
            src_h,
            self.render.width,
            self.render.height,
        )?;
        let uuid = deck.uuid().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added depth sensor deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    /// Remove a deck, then release every device it held and its arrangement
    /// lane.
    pub(crate) fn remove_deck(&mut self, deck_uuid: &str) -> Result<()> {
        let slot = self.mixer.remove_deck(deck_uuid)?;
        if let Some(cam_id) = slot.deck.camera_id() {
            self.sources.camera_manager.release_camera(cam_id);
        }
        // Covers both point-cloud sources and shader preprocessors.
        for sensor_id in slot.deck.held_depth_sensors() {
            self.sources.depth_manager.release(sensor_id);
        }
        if let Some(capture_id) = slot.deck.screen_capture_id() {
            self.sources.screen_capture_manager.release(capture_id);
        }
        if let Some(idx) = slot.deck.srt_receiver_idx() {
            self.sources.io.stream_manager.stop_receive(idx);
        }
        if let Some(idx) = slot.deck.ndi_receiver_idx() {
            self.sources.io.ndi_manager.stop_receive(idx);
        }
        #[cfg(target_os = "macos")]
        if let Some(idx) = slot.deck.syphon_client_idx() {
            self.sources.io.syphon_manager.stop_receive(idx);
        }
        // A lane is where this deck sits in show time, so it leaves with the
        // deck. See /spec/arrangement.md § A lane is a deck.
        self.mixer.drop_lane(deck_uuid);
        Ok(())
    }

    pub(crate) fn add_channel(&mut self) -> Result<String> {
        let idx =
            self.mixer
                .add_channel(&self.render.context, self.render.width, self.render.height)?;
        Ok(self.mixer.channels()[idx].uuid().to_string())
    }

    pub(crate) fn remove_channel(&mut self, channel_uuid: &str) -> Result<()> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        // Asked before anything is torn down, because a refusal has to leave the
        // channel exactly as it was rather than empty.
        if self.mixer.channels().len() <= 2 {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }

        // Through the deck path rather than dropping the column wholesale, so a
        // camera, a depth sensor, a stream receiver, and an arrangement lane all
        // leave with the deck that held them, exactly as they do when the deck is
        // removed on its own.
        let channel = &self.mixer.channels()[channel_idx];
        let decks: Vec<String> = channel
            .decks
            .iter()
            .map(|slot| slot.deck.uuid().to_string())
            .collect();
        let effects: Vec<String> = channel
            .effects
            .iter()
            .map(|effect| effect.uuid().to_string())
            .collect();
        for uuid in decks {
            if let Err(e) = self.remove_deck(&uuid) {
                log::warn!("Removing deck {uuid} with its channel: {e}");
            }
        }
        for uuid in effects {
            self.mixer
                .modulation_mut()
                .remove_assignments_with_prefix(&crate::engine::value::param::effect_prefix(&uuid));
        }
        // The fader's own curves. A key that can never resolve again would be
        // persisted and reloaded as dead weight.
        self.mixer.modulation_mut().remove_assignments_with_prefix(
            &crate::engine::value::param::channel_prefix(channel_uuid),
        );

        if self.mixer.remove_channel(channel_idx) {
            // Selection fixup is handled by the UI consumer (UIRunner)
            Ok(())
        } else {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }
    }

    pub(crate) fn add_effect(
        &mut self,
        target: &EffectTarget,
        shader_name: &str,
    ) -> Result<String> {
        let chain = self.mixer.resolve_effect_target(target)?;
        let filters = self.sources.registry.filters();
        let shader = filters
            .iter()
            .find(|s| s.name() == shader_name)
            .context("Filter shader not found")?;
        match chain {
            EffectChain::Deck {
                channel_idx,
                deck_idx,
            } => {
                // Clone off the registry borrow so the device manager can be
                // borrowed mutably for acquisition.
                let shader = (*shader).clone();
                let metadata = shader.metadata.clone();
                drop(filters);
                // Acquire before mutating the deck so a missing sensor leaves the
                // effect chain untouched rather than half-added.
                let already_attached = self
                    .mixer
                    .channels()
                    .get(channel_idx)
                    .and_then(|c| c.decks.get(deck_idx))
                    .is_some_and(|s| s.deck.depth_prepro.is_some());
                let acquired = if already_attached {
                    // The deck's source shader already holds a session; reuse it
                    // rather than opening the device a second time.
                    None
                } else {
                    self.acquire_depth_preprocessor(&metadata, shader_name)?
                };

                let effect = Effect::new(&self.render.context, shader)?;
                let uuid = effect.uuid().to_owned();
                let ch = self
                    .mixer
                    .channel_mut(channel_idx)
                    .context("Invalid channel")?;
                let deck = &mut ch.decks[deck_idx].deck;
                deck.add_effect(effect);
                deck.ensure_preprocessor_analyzers(&self.sources.analyzer_registry);
                if let Some(sensor) = acquired {
                    deck.attach_depth_preprocessor(
                        sensor.id,
                        sensor.name,
                        sensor.pipeline,
                        DepthPreprocessParams::default(),
                    );
                } else if already_attached {
                    deck.rebind_depth_preprocessor_slots();
                }
                log::info!("Added effect {shader_name} to deck chain ({uuid})");
                Ok(uuid)
            }
            EffectChain::Channel { channel_idx } => {
                if crate::depth::preprocess::requested_device(&shader.metadata).is_some() {
                    anyhow::bail!(
                        "Effect '{shader_name}' requires a depth sensor, which is only \
                         available on deck effect chains — not channel or master chains."
                    );
                }
                let effect = Effect::new_with_format(
                    &self.render.context,
                    (*shader).clone(),
                    self.render.context.compositing_format,
                )?;
                let uuid = effect.uuid().to_owned();
                let ch = self
                    .mixer
                    .channel_mut(channel_idx)
                    .context("Invalid channel")?;
                ch.add_effect(effect);
                log::info!("Added channel effect {shader_name} ({uuid})");
                Ok(uuid)
            }
            EffectChain::Master => {
                if crate::depth::preprocess::requested_device(&shader.metadata).is_some() {
                    anyhow::bail!(
                        "Effect '{shader_name}' requires a depth sensor, which is only \
                         available on deck effect chains — not channel or master chains."
                    );
                }
                let effect = Effect::new_with_format(
                    &self.render.context,
                    (*shader).clone(),
                    self.render.context.compositing_format,
                )?;
                let uuid = effect.uuid().to_owned();
                self.mixer.add_master_effect(effect);
                log::info!("Added master effect {shader_name} ({uuid})");
                Ok(uuid)
            }
        }
    }

    /// Remove an effect, releasing the depth sensor its deck no longer needs.
    pub(crate) fn remove_effect(&mut self, effect_uuid: &str) -> Result<()> {
        if let Some(sensor_id) = self.mixer.remove_effect(effect_uuid)? {
            self.sources.depth_manager.release(sensor_id);
        }
        Ok(())
    }

    pub(crate) fn set_transition(&mut self, shader_name: Option<&str>) -> Result<()> {
        match shader_name {
            None => {
                self.mixer.clear_transition();
                Ok(())
            }
            Some(name) => {
                let shader = self
                    .sources
                    .registry
                    .get(name)
                    .context("Transition shader not found")?;
                self.mixer
                    .set_transition(&self.render.context, shader.clone())
            }
        }
    }

    pub(crate) fn load_lut(&mut self, filename: &str) -> Result<()> {
        let lut_dir = self.session.workspace.varda_dir().join("luts");
        let path = lut_dir.join(filename);
        let parsed = crate::renderer::lut::parse_lut_file(&path)?;
        self.mixer.load_lut(
            &self.render.context.device,
            &self.render.context.queue,
            &parsed,
            filename.to_string(),
        );
        Ok(())
    }

    /// Load a scene-referred look LUT from the same `.varda/luts` directory.
    ///
    /// Shares the directory with calibration LUTs deliberately: a `.cube` is a
    /// `.cube`, and which slot it occupies is the user's choice rather than a
    /// property of the file.
    pub(crate) fn load_look_lut(&mut self, filename: &str) -> Result<()> {
        let lut_dir = self.session.workspace.varda_dir().join("luts");
        let path = lut_dir.join(filename);
        let parsed = crate::renderer::lut::parse_lut_file(&path)?;
        self.mixer.set_look_lut(
            &self.render.context.device,
            &self.render.context.queue,
            &parsed,
            filename.to_string(),
        );
        Ok(())
    }

    pub(crate) fn set_param(
        &mut self,
        path: &str,
        value: ParamValue,
    ) -> std::result::Result<(), crate::param_router::ParamRouteError> {
        // Typed routing preserves Color/Point2D for shader/effect params; scalar
        // paths flatten internally. OSC feedback stays a scalar f32.
        let feedback_value = crate::param_router::param_value_to_norm_f32(&value);
        match crate::param_router::apply_typed_param_by_path(&mut self.mixer, path, value) {
            Ok(()) => {
                // Broadcast to OSC feedback targets
                if let Some(ref sender) = self.input.osc_feedback
                    && sender.has_targets()
                {
                    sender.send_param(path, feedback_value);
                }
                Ok(())
            }
            Err(e) => {
                log::warn!("set_param route failed ({path}): {e}");
                Err(e)
            }
        }
    }
}

// ── Audio trait implementations ─────────────────────────────────────

// ── Modulation trait implementations ────────────────────────────────

impl VardaApp {
    pub(crate) fn set_macro_value(&mut self, uuid: &str, value: f32) {
        // Route through the shared param router so the fan-out (and any global
        // trigger actions, drained in process_inputs) behave identically to a
        // MIDI/OSC-driven `macro/<uuid>/value`.
        let path = crate::engine::value::param::ParamAddress::macro_value(uuid).to_string();
        if let Err(e) = crate::param_router::apply_param_by_path(&mut self.mixer, &path, value) {
            log::debug!("set_macro_value {uuid}: {e}");
        }
    }
}

// ── Output trait implementations ────────────────────────────────────

// ── MixerQueries ────────────────────────────────────────────────────

// ── SurfaceCommands / SurfaceQueries ────────────────────────────────

impl VardaApp {
    pub(crate) fn detect_from_camera(
        &mut self,
        camera_id: CameraId,
        params: &crate::surface::detect::DetectionParams,
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError> {
        // If camera isn't active yet, open it temporarily for the snapshot.
        let was_inactive = !self.sources.camera_manager.is_active(camera_id);
        if was_inactive {
            self.sources
                .camera_manager
                .open_camera(camera_id, &self.render.context.device)
                .map_err(|e| {
                    crate::surface::import::ImportError::ImageLoad(format!(
                        "Failed to open camera {camera_id}: {e}"
                    ))
                })?;
        }

        // Spin-wait for a frame (capture thread needs time to produce one).
        // Budget: up to 500ms in 10ms increments.
        let mut frame = None;
        for _ in 0..50 {
            if let Some(f) = self.sources.camera_manager.snapshot_frame(camera_id) {
                frame = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Release the camera if we opened it just for this snapshot.
        if was_inactive {
            self.sources.camera_manager.release_camera(camera_id);
        }

        let (rgba, w, h) = frame.ok_or_else(|| {
            crate::surface::import::ImportError::ImageLoad(format!(
                "No frame received from camera {camera_id} within timeout"
            ))
        })?;
        crate::surface::import::detect_from_rgba(&rgba, w, h, params)
    }
}

// ── Analyzer trait implementations ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> Option<super::super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let config = crate::testing::headless_config();
        super::super::VardaApp::new(gpu, &config).ok()
    }

    fn channel_uuid(app: &super::super::VardaApp, idx: usize) -> String {
        crate::app::snapshot::build_mixer_snapshot(app).channels[idx]
            .uuid
            .clone()
    }

    // ── Depth-sensor preprocessor ────────────────────────────────────────────
    //
    // These need no hardware: without a Kinect attached (and on builds with the
    // `depth` feature compiled out) the manager enumerates nothing, which is
    // exactly the failure path being asserted. The release regression uses the
    // mock backend.

    #[test]
    fn depth_sensor_shader_fails_to_load_without_a_sensor() {
        let Some(mut app) = headless_app() else {
            return;
        };
        if !app.sources.depth_manager.devices().is_empty() {
            // A real sensor is attached; this test asserts the absent case.
            return;
        }
        let ch0 = channel_uuid(&app, 0);
        let err = app
            .add_deck(&ch0, "liquid_light_depth")
            .expect_err("must not load without a depth sensor");
        let msg = err.to_string();
        assert!(
            msg.contains("depth sensor") && msg.contains("none detected"),
            "unhelpful message: {msg}"
        );
        // The load aborted, so no deck was left behind.
        assert_eq!(
            crate::app::snapshot::build_mixer_snapshot(&app).channels[0]
                .decks
                .len(),
            0
        );
    }

    #[test]
    fn finalize_rejects_a_deck_whose_required_sensor_is_missing() {
        // The background loader finalizes decks built off-thread, after the
        // pre-flight in `add_deck` has passed. A sensor unplugged in between
        // must still stop the deck from attaching.
        let Some(mut app) = headless_app() else {
            return;
        };
        if !app.sources.depth_manager.devices().is_empty() {
            return;
        }
        let shader = app
            .sources
            .registry
            .generators()
            .iter()
            .find(|s| s.name() == "liquid_light_depth")
            .map(|s| (*s).clone())
            .expect("showcase shader is registered");
        let mut deck = Deck::new(&app.render.context, shader, 64, 64).expect("deck builds");
        let err = app
            .finalize_new_deck(&mut deck)
            .expect_err("must reject without a depth sensor");
        assert!(err.to_string().contains("none detected"), "{err}");
        assert!(deck.depth_prepro.is_none());
    }

    #[test]
    fn optional_preprocessor_shaders_still_load() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch0 = channel_uuid(&app, 0);
        // A plain generator with no PREPROCESSORS block must be unaffected by
        // the new required-preprocessor pre-flight.
        assert!(app.add_deck(&ch0, "liquid_light").is_ok());
    }

    #[test]
    fn removing_a_depth_deck_releases_the_sensor() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Two decks sharing one mock sensor: the session must survive the first
        // removal and be torn down by the second.
        app.sources
            .depth_manager
            .open_mock(0, 32, 24, &app.render.context.device)
            .expect("open mock");
        app.sources
            .depth_manager
            .open_mock(0, 32, 24, &app.render.context.device)
            .expect("share mock");
        assert_eq!(app.sources.depth_manager.ref_count(0), 2);

        let ch0 = channel_uuid(&app, 0);
        let mut uuids = Vec::new();
        for _ in 0..2 {
            let deck = Deck::new_from_depth_sensor(
                &app.render.context,
                0,
                "Mock Depth (#0)",
                32,
                24,
                64,
                64,
            )
            .expect("build depth deck");
            uuids.push(deck.uuid().to_string());
            let ch_idx = app.mixer.resolve_channel(&ch0).expect("channel");
            app.mixer
                .channel_mut(ch_idx)
                .expect("channel")
                .add_deck(deck);
        }

        app.remove_deck(&uuids[0]).expect("remove first");
        assert_eq!(
            app.sources.depth_manager.ref_count(0),
            1,
            "one consumer left, session must stay open"
        );

        app.remove_deck(&uuids[1]).expect("remove second");
        assert!(
            !app.sources.depth_manager.is_active(0),
            "last consumer removed — the capture session must be torn down"
        );
    }

    #[test]
    fn add_channel_increases_count() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let before = crate::app::snapshot::build_mixer_snapshot(&app)
            .channels
            .len();
        assert_eq!(before, 2);
        app.add_channel().unwrap();
        let after = crate::app::snapshot::build_mixer_snapshot(&app)
            .channels
            .len();
        assert_eq!(after, 3);
    }

    #[test]
    fn remove_channel_enforces_minimum() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert_eq!(
            crate::app::snapshot::build_mixer_snapshot(&app)
                .channels
                .len(),
            2
        );
        // Trying to remove should fail (minimum 2)
        let ch0 = channel_uuid(&app, 0);
        let result = app.remove_channel(&ch0);
        assert!(result.is_err());
        assert_eq!(
            crate::app::snapshot::build_mixer_snapshot(&app)
                .channels
                .len(),
            2
        );
    }

    #[test]
    fn set_param_invalid_path_is_reported() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // A path that matches no route must surface, not fail silently: the HTTP
        // API turns this into a 404 rather than reporting a write that never
        // landed as `{"status": "ok"}`.
        assert!(matches!(
            app.set_param("ch99/deck99/nonexistent_param", ParamValue::Float(0.5)),
            Err(crate::param_router::ParamRouteError::UnknownPath { .. })
        ));
    }
}
