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
    /// Start a new shader deck's CPU analyzers and acquire any device its
    /// `PREPROCESSORS` block requires. `deck_loads` calls it on attach; without
    /// it a `depth_sensor` shader silently renders blank textures.
    ///
    /// Returns `Err` when a required preprocessor cannot be satisfied; the
    /// caller must discard the deck.
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
    /// when it does but no sensor is available. Callers abandon the load on `Err`.
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
    /// service. Whether a type is required comes from the registry. Unknown and
    /// optional types degrade to default outputs.
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

    /// Add a deck for `source`, returning its UUID. Slow sources load off the
    /// render thread and attach on a later frame. Config, type and device errors
    /// are reported here, before any deck exists.
    pub(crate) fn add_deck(&mut self, channel_uuid: &str, source: &SourceConfig) -> Result<String> {
        let channel_idx = self.mixer.resolve_channel(channel_uuid)?;
        // Controllers re-subscribe on reconnect; keep one such deck per channel.
        if self
            .sources
            .providers
            .get(source.type_id())
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

    /// Swap a deck's source, keeping its UUID, effects, opacity and modulation.
    /// A slow source builds off the render thread and replaces the current one
    /// on a later frame; the deck draws its current source until then.
    pub(crate) fn replace_deck_source(
        &mut self,
        deck_uuid: &str,
        source: &SourceConfig,
    ) -> Result<()> {
        let (ch, dk) = self.mixer.resolve_deck(deck_uuid)?;
        let labels = self.mixer.channel_labels();
        let query = self.sources.query(&labels);
        if let Some(loader) = self.sources.providers.loader(source, &query) {
            self.spawn_source_swap(deck_uuid, loader?);
            return Ok(());
        }
        self.supersede_source_swaps(deck_uuid);
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

    /// Run a source type's library action and return its fresh entries, so a
    /// rescan and its result are one call.
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

    /// Remove a deck and release its devices and arrangement lane.
    pub(crate) fn remove_deck(&mut self, deck_uuid: &str) -> Result<()> {
        let mut slot = self.mixer.remove_deck(deck_uuid)?;
        self.release_source(&mut slot.deck);
        // A shader deck's depth preprocessor holds the sensor too.
        if let Some(sensor_id) = slot.deck.held_depth_prepro_sensor() {
            self.sources
                .service_mut::<crate::depth::DepthSensorManager>()
                .release(sensor_id);
        }
        // A lane belongs to its deck.
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
        // Checked first, so a refusal leaves the channel intact.
        if self.mixer.channels().len() <= 2 {
            anyhow::bail!("Cannot remove channel (minimum 2 required)")
        }

        // Remove each deck via `remove_deck` so its devices and lane are released.
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
        // Drop the channel's own assignments, which can no longer resolve.
        self.mixer.modulation_mut().remove_assignments_with_prefix(
            &crate::engine::value::param::channel_prefix(channel_uuid),
        );

        if self.mixer.remove_channel(channel_idx) {
            // The UI fixes up selection.
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
                // Cloned to release the registry borrow before acquiring the device.
                let shader = (*shader).clone();
                let metadata = shader.metadata.clone();
                drop(filters);
                // Acquire first so a missing sensor leaves the effect chain untouched.
                let already_attached = self
                    .mixer
                    .channels()
                    .get(channel_idx)
                    .and_then(|c| c.decks.get(deck_idx))
                    .is_some_and(|s| s.deck.depth_prepro.is_some());
                let acquired = if already_attached {
                    // The deck's source shader already holds a session; reuse it.
                    None
                } else {
                    self.acquire_depth_preprocessor(&metadata, shader_name)?
                };

                let effect = Effect::pending(
                    &self.render.context,
                    shader,
                    self.render.context.compositing_format,
                );
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
                let effect = Effect::pending(
                    &self.render.context,
                    (*shader).clone(),
                    self.render.context.compositing_format,
                );
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
                let effect = Effect::pending(
                    &self.render.context,
                    (*shader).clone(),
                    self.render.context.compositing_format,
                );
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
                    .set_transition(&self.render.context, shader.clone());
                Ok(())
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

    /// Load a scene-referred look LUT from `.varda/luts`, the directory
    /// calibration LUTs also use.
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

/// Label for a background load before its deck exists: the file or shader
/// name, else the type.
fn load_name(source: &SourceConfig) -> String {
    ["path", "name", "url"]
        .iter()
        .find_map(|key| source.str(key))
        .map_or_else(
            || source.type_id().to_string(),
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
        // Through the param router, so it behaves like a MIDI/OSC `macro/<uuid>/value`.
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
        // Open the camera temporarily if it is inactive.
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

        // Poll for a frame for up to 500ms.
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
        crate::testing::headless_app()
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
    // No hardware needed: without a Kinect the manager finds nothing, which is
    // the failure path under test. The release test uses the mock backend.

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
        // No deck left behind.
        assert_eq!(
            crate::app::snapshot::build_mixer_snapshot(&app).channels[0]
                .decks
                .len(),
            0
        );
    }

    #[test]
    fn finalize_rejects_a_deck_whose_required_sensor_is_missing() {
        // A sensor unplugged between `add_deck` and attach still blocks the deck.
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
        // A generator with no PREPROCESSORS block is unaffected.
        assert!(app.add_deck(&ch0, &shader("liquid_light")).is_ok());
    }

    #[test]
    fn removing_a_depth_deck_releases_the_sensor() {
        let Some(mut app) = headless_app() else {
            return;
        };
        // Two decks share one mock sensor: the session survives the first
        // removal and closes on the second.
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
        // Minimum is 2 channels.
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
        // An unrouted path is an error; the HTTP API returns 404.
        assert!(matches!(
            app.set_param("ch99/deck99/nonexistent_param", ParamValue::Float(0.5)),
            Err(crate::param_router::ParamRouteError::UnknownPath { .. })
        ));
    }

    // ── Effects build off the render thread ─────────────────────────────────

    /// A headless engine with one solid orange deck on channel 0.
    fn app_with_orange_deck() -> Option<(super::super::VardaApp, String)> {
        let mut app = headless_app()?;
        let channel = channel_uuid(&app, 0);
        let deck = app
            .add_deck(
                &channel,
                &crate::solid_color::SolidColor::config_for([1.0, 0.5, 0.0, 1.0]),
            )
            .expect("deck");
        app.settle_deck_loads();
        Some((app, deck))
    }

    fn add_invert(app: &mut super::super::VardaApp, deck: &str) -> String {
        match app.execute_command(crate::engine::EngineCommand::AddEffect {
            target: crate::engine::EffectTarget::Deck(deck.to_string()),
            shader_name: "invert".into(),
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("effect not added: {other:?}"),
        }
    }

    fn effect_status(
        app: &super::super::VardaApp,
        uuid: &str,
    ) -> Option<crate::engine::value::effect::EffectStatus> {
        crate::app::snapshot::build_mixer_snapshot(app)
            .channels
            .iter()
            .flat_map(|c| c.decks.iter())
            .flat_map(|d| d.effects.iter())
            .find(|e| e.uuid == uuid)
            .map(|e| e.status.clone())
    }

    fn frame(app: &mut super::super::VardaApp) {
        app.begin_frame();
        app.render_frame();
    }

    /// Render frames until the effect is no longer building.
    fn until_built(app: &mut super::super::VardaApp, uuid: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while effect_status(app, uuid) == Some(crate::engine::value::effect::EffectStatus::Building)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the build never finished"
            );
            frame(app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The deck's center pixel.
    fn deck_center(app: &super::super::VardaApp, deck: &str) -> [f32; 4] {
        let (ch, dk) = app.mixer.find_deck_by_uuid(deck).expect("deck");
        let texture = &app.mixer.channels()[ch].decks[dk].deck.texture;
        let (w, h) = (texture.width(), texture.height());
        crate::testing::read_rgba16f(app.gpu_context(), texture, w, h)[(h / 2 * w + w / 2) as usize]
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(&b).take(3).all(|(x, y)| (x - y).abs() < 0.02)
    }

    /// The command returns with the effect in its chain, still building; it
    /// draws once a later frame finds it ready.
    #[test]
    fn adding_an_effect_returns_before_it_is_built() {
        use crate::engine::value::effect::EffectStatus;
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        let effect = add_invert(&mut app, &deck);
        assert_eq!(effect_status(&app, &effect), Some(EffectStatus::Building));
        until_built(&mut app, &effect);
        assert_eq!(effect_status(&app, &effect), Some(EffectStatus::Ready));
    }

    /// While it builds the effect passes the picture through; once ready it
    /// applies.
    #[test]
    fn a_building_effect_passes_the_picture_through() {
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        frame(&mut app);
        let plain = deck_center(&app, &deck);
        let effect = add_invert(&mut app, &deck);
        frame(&mut app);
        assert!(
            close(deck_center(&app, &deck), plain),
            "unchanged while building"
        );
        until_built(&mut app, &effect);
        frame(&mut app);
        assert!(
            !close(deck_center(&app, &deck), plain),
            "inverted once ready"
        );
    }

    /// A toggle made while the effect builds holds once it is ready.
    #[test]
    fn changes_made_while_building_hold_once_ready() {
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        frame(&mut app);
        let plain = deck_center(&app, &deck);
        let effect = add_invert(&mut app, &deck);
        let toggled = app.execute_command(crate::engine::EngineCommand::ToggleEffect {
            effect_uuid: effect.clone(),
        });
        assert!(matches!(toggled, crate::engine::CommandResult::Ok));
        until_built(&mut app, &effect);
        frame(&mut app);
        assert!(close(deck_center(&app, &deck), plain), "still disabled");
    }

    /// Removing an effect while it builds drops its result quietly.
    #[test]
    fn removing_a_building_effect_drops_its_build() {
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        let effect = add_invert(&mut app, &deck);
        let removed = app.execute_command(crate::engine::EngineCommand::RemoveEffect {
            effect_uuid: effect.clone(),
        });
        assert!(matches!(removed, crate::engine::CommandResult::Ok));
        for _ in 0..20 {
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(effect_status(&app, &effect), None);
    }

    /// Undoing an effect's removal brings it back building, then ready.
    #[test]
    fn undoing_a_removal_rebuilds_the_effect_off_the_render_thread() {
        use crate::engine::value::effect::EffectStatus;
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        let effect = add_invert(&mut app, &deck);
        until_built(&mut app, &effect);
        let before = app.history_snapshot();
        app.push_history(before);
        let _ = app.execute_command(crate::engine::EngineCommand::RemoveEffect {
            effect_uuid: effect.clone(),
        });
        let current = app.history_snapshot();
        assert!(app.history_undo(current));
        assert_eq!(effect_status(&app, &effect), Some(EffectStatus::Building));
        until_built(&mut app, &effect);
        assert_eq!(effect_status(&app, &effect), Some(EffectStatus::Ready));
    }

    fn source_pending(app: &super::super::VardaApp, deck: &str) -> bool {
        crate::app::snapshot::build_mixer_snapshot(app)
            .channels
            .iter()
            .flat_map(|c| c.decks.iter())
            .find(|d| d.uuid == deck)
            .is_some_and(|d| d.source_pending)
    }

    fn deck_name(app: &super::super::VardaApp, deck: &str) -> String {
        let (ch, dk) = app.mixer.find_deck_by_uuid(deck).expect("deck");
        app.mixer.channels()[ch].decks[dk]
            .deck
            .source_name()
            .to_string()
    }

    fn swap_source(app: &mut super::super::VardaApp, deck: &str, name: &str) {
        let r = app.execute_command(crate::engine::EngineCommand::ReplaceDeckSource {
            deck_uuid: deck.to_string(),
            source: shader(name),
        });
        assert!(matches!(r, crate::engine::CommandResult::Ok), "{r:?}");
    }

    fn until_swapped(app: &mut super::super::VardaApp, deck: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while source_pending(app, deck) {
            assert!(
                std::time::Instant::now() < deadline,
                "the swap never finished"
            );
            frame(app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The deck keeps drawing its old source until the new one is built.
    #[test]
    fn a_source_swap_keeps_the_old_source_until_the_new_one_is_ready() {
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        frame(&mut app);
        let orange = deck_center(&app, &deck);
        let before = deck_name(&app, &deck);
        swap_source(&mut app, &deck, "bars");
        assert!(source_pending(&app, &deck));
        frame(&mut app);
        assert_eq!(deck_name(&app, &deck), before);
        assert!(
            close(deck_center(&app, &deck), orange),
            "the old source still draws"
        );
        until_swapped(&mut app, &deck);
        assert_ne!(deck_name(&app, &deck), before);
    }

    /// A second swap before the first finishes wins.
    #[test]
    fn the_last_of_two_quick_swaps_wins() {
        let Some((mut app, deck)) = app_with_orange_deck() else {
            return;
        };
        swap_source(&mut app, &deck, "bars");
        swap_source(&mut app, &deck, "plasma");
        until_swapped(&mut app, &deck);
        for _ in 0..20 {
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            deck_name(&app, &deck).to_lowercase().contains("plasma"),
            "{}",
            deck_name(&app, &deck)
        );
    }

    fn active_transition(app: &super::super::VardaApp) -> Option<String> {
        app.mixer.active_transition().map(|t| t.name.clone())
    }

    fn pick_transition(app: &mut super::super::VardaApp, name: &str) {
        let r = app.execute_command(crate::engine::EngineCommand::SetTransition {
            shader_name: Some(name.to_string()),
        });
        assert!(matches!(r, crate::engine::CommandResult::Ok), "{r:?}");
    }

    fn until_transition(app: &mut super::super::VardaApp, name: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while active_transition(app).as_deref() != Some(name) {
            assert!(
                std::time::Instant::now() < deadline,
                "{name} never became active"
            );
            frame(app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The previous transition stays in use until the new one is built.
    #[test]
    fn a_transition_pick_keeps_the_previous_one_until_ready() {
        let Some(mut app) = headless_app() else {
            return;
        };
        pick_transition(&mut app, "transition_dissolve");
        until_transition(&mut app, "transition_dissolve");
        pick_transition(&mut app, "transition_iris");
        assert_eq!(
            active_transition(&app).as_deref(),
            Some("transition_dissolve")
        );
        until_transition(&mut app, "transition_iris");
    }

    /// Starting the engine builds every transition in the background.
    #[test]
    fn transitions_are_warmed_at_startup() {
        let Some(app) = headless_app() else {
            return;
        };
        let sources: Vec<String> = app
            .sources
            .registry
            .transitions()
            .iter()
            .map(|s| s.fragment_source.clone())
            .collect();
        assert_ne!(sources.len(), 0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !sources
            .iter()
            .all(|s| crate::isf::compiler::fragment_is_cached(s))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "transitions were not warmed"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
