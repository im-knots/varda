//! Engine operations that span several owners: building and removing decks
//! (GPU context, render size, and the device each source needs), effect chains
//! with their analyzers and depth preprocessors, transitions and LUTs, routed
//! parameter writes, and output windows.

use super::VardaApp;
use crate::deck::{Deck, Effect};
use crate::depth::preprocess::{AcquiredSensor, DepthPreprocessParams};
use crate::engine::types::{CameraId, EffectTarget, ParamValue};
use crate::mixer::EffectChain;
use crate::source::SourceConfig;

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
            self.sources
                .service_mut::<crate::depth::DepthSensorManager>(),
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

    /// Add a deck whose source `source` describes, returning the UUID it has
    /// (or will have, for a source that loads in the background).
    ///
    /// A provider that builds slowly hands out a loader that runs off the
    /// render thread; the deck attaches, finalized, on a later frame. Anything
    /// wrong with the config, the type, or a device is reported here, before
    /// any deck exists.
    pub(crate) fn add_deck(&mut self, channel_uuid: &str, source: &SourceConfig) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        // A source a controller re-subscribes to on every reconnect converges
        // on one deck per channel rather than stacking duplicates.
        if self
            .sources
            .providers
            .get(source.source_type())
            .is_some_and(crate::source::DeckSourceProvider::one_per_channel)
            && let Some(existing) = self.mixer.channels()[channel_idx]
                .decks
                .iter()
                .find(|slot| {
                    self.sources
                        .providers
                        .same_source(&slot.deck.source().config(), source)
                })
        {
            return Ok(existing.deck.uuid().to_string());
        }

        let labels = self.mixer.channel_labels();
        let query = self.sources.query(&labels);
        if let Some(loader) = self.sources.providers.loader(source, &query) {
            let loader = loader?;
            return Ok(self.spawn_deck_load(channel_uuid, load_name(source), loader));
        }

        let (width, height) = (self.render.width, self.render.height);
        let (providers, mut env) = self
            .sources
            .env(&self.render.context, width, height, &labels);
        let instance = providers.create(source, &mut env)?;
        let mut deck = Deck::from_source(&self.render.context, instance, width, height);
        if let Err(e) = self.finalize_new_deck(&mut deck) {
            self.release_source(&mut deck);
            return Err(e);
        }
        let uuid = deck.uuid().to_string();
        let name = deck.source_name().to_string();
        let ch = self
            .mixer
            .channel_mut(channel_idx)
            .context("Invalid channel")?;
        let idx = ch.add_deck(deck);
        log::info!("Added deck {idx} to channel {channel_idx}: {name}");
        Ok(uuid)
    }

    /// Swap a deck's source for another, keeping its identity, effects,
    /// opacity and modulation. The new source is built on the render thread.
    pub(crate) fn replace_deck_source(
        &mut self,
        deck_uuid: &str,
        source: &SourceConfig,
    ) -> Result<()> {
        let (ch, dk) = self.mixer.resolve_deck(deck_uuid)?;
        let labels = self.mixer.channel_labels();
        let (width, height) = (self.render.width, self.render.height);
        let (providers, mut env) = self
            .sources
            .env(&self.render.context, width, height, &labels);
        let instance = providers.create(source, &mut env)?;
        let deck = &mut self.mixer.channels_mut()[ch].decks[dk].deck;
        let mut old = deck.replace_source(instance);
        let (providers, mut env) = self
            .sources
            .env(&self.render.context, width, height, &labels);
        providers.release(old.as_mut(), &mut env);
        Ok(())
    }

    /// Run `f` on the provider of `source_type` (add or remove a user's
    /// library entry).
    pub(crate) fn source_library(
        &mut self,
        source_type: &str,
        f: impl FnOnce(&mut dyn crate::source::DeckSourceProvider) -> Result<()>,
    ) -> Result<()> {
        let provider = self
            .sources
            .providers
            .get_mut(source_type)
            .with_context(|| format!("unknown source type '{source_type}'"))?;
        f(provider)
    }

    /// Run a library action a source type offers, answering with its fresh
    /// entries so a probe that rescans is one non-racy call rather than a
    /// rescan and a separate read that may land before discovery finished.
    pub(crate) fn source_library_action(
        &mut self,
        source_type: &str,
        action: &str,
    ) -> crate::engine::CommandResult {
        use crate::engine::{CommandResult, ErrorCode};
        let labels = self.mixer.channel_labels();
        let (width, height) = (self.render.width, self.render.height);
        let (providers, mut env) = self
            .sources
            .env(&self.render.context, width, height, &labels);
        let Some(provider) = providers.get_mut(source_type) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("unknown source type '{source_type}'"),
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

    /// Release what a deck's source holds on a device manager.
    fn release_source(&mut self, deck: &mut Deck) {
        let (width, height) = (self.render.width, self.render.height);
        let (providers, mut env) = self.sources.env(&self.render.context, width, height, &[]);
        providers.release(deck.source_mut(), &mut env);
    }

    /// Remove a deck, then release every device it held and its arrangement
    /// lane.
    pub(crate) fn remove_deck(&mut self, deck_uuid: &str) -> Result<()> {
        let mut slot = self.mixer.remove_deck(deck_uuid)?;
        self.release_source(&mut slot.deck);
        // A shader deck's depth preprocessor holds the sensor too.
        if let Some(sensor_id) = slot.deck.held_depth_prepro_sensor() {
            self.sources
                .service_mut::<crate::depth::DepthSensorManager>()
                .release(sensor_id);
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
            self.sources
                .service_mut::<crate::depth::DepthSensorManager>()
                .release(sensor_id);
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

/// What a background load is reported as before its deck exists: the file or
/// shader it names, else its type.
fn load_name(source: &SourceConfig) -> String {
    ["path", "name", "url"]
        .iter()
        .find_map(|key| source.str(key))
        .map_or_else(
            || source.source_type().to_string(),
            |value| {
                std::path::Path::new(value)
                    .file_name()
                    .map_or_else(|| value.to_string(), |f| f.to_string_lossy().into_owned())
            },
        )
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
        let was_inactive = !self
            .sources
            .service::<crate::camera::CameraManager>()
            .is_active(camera_id);
        if was_inactive {
            self.sources
                .service_mut::<crate::camera::CameraManager>()
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
            if let Some(f) = self
                .sources
                .service::<crate::camera::CameraManager>()
                .snapshot_frame(camera_id)
            {
                frame = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Release the camera if we opened it just for this snapshot.
        if was_inactive {
            self.sources
                .service_mut::<crate::camera::CameraManager>()
                .release_camera(camera_id);
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

    fn shader(name: &str) -> SourceConfig {
        SourceConfig::new("Shader").with("name", name)
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
        if !app.depth_manager().devices().is_empty() {
            // A real sensor is attached; this test asserts the absent case.
            return;
        }
        let ch0 = channel_uuid(&app, 0);
        let err = app
            .add_deck(&ch0, &shader("liquid_light_depth"))
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
        if !app.depth_manager().devices().is_empty() {
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
        let mut deck = Deck::from_shader(&app.render.context, shader, 64, 64).expect("deck builds");
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
        assert!(app.add_deck(&ch0, &shader("liquid_light")).is_ok());
    }

    #[test]
    fn removing_a_depth_deck_releases_the_sensor() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Two decks sharing one mock sensor: the session must survive the first
        // removal and be torn down by the second.
        let device = app.render.context.device.clone();
        let depth = app
            .sources
            .service_mut::<crate::depth::DepthSensorManager>();
        depth.open_mock(0, 32, 24, &device).expect("open mock");
        depth.open_mock(0, 32, 24, &device).expect("share mock");
        assert_eq!(app.depth_manager().ref_count(0), 2);

        let ch0 = channel_uuid(&app, 0);
        let ch_idx = app.mixer.resolve_channel(&ch0).expect("channel");
        let mut uuids = Vec::new();
        for _ in 0..2 {
            let source = crate::depth::provider::DepthSensor::for_open_sensor(0, "Mock Depth (#0)");
            let deck = Deck::from_source(&app.render.context, Box::new(source), 64, 64);
            uuids.push(deck.uuid().to_string());
            app.mixer
                .channel_mut(ch_idx)
                .expect("channel")
                .add_deck(deck);
        }
        let (first, second) = (uuids[0].clone(), uuids[1].clone());

        app.remove_deck(&first).expect("remove first");
        assert_eq!(
            app.depth_manager().ref_count(0),
            1,
            "one consumer left, session must stay open"
        );

        app.remove_deck(&second).expect("remove second");
        assert!(
            !app.depth_manager().is_active(0),
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
