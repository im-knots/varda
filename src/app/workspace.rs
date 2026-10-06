//! Saving and loading the `.varda/` workspace.

use super::VardaApp;
use crate::engine::value::editor::EditorPrefs;

/// Outcome of loading `.varda/`. Hard failures are listed so a partial load
/// isn't reported as `Ok`. Editor prefs are returned if stage.json loaded.
pub struct WorkspaceLoad {
    pub editor_prefs: Option<EditorPrefs>,
    errors: Vec<String>,
}

impl WorkspaceLoad {
    /// Whether every existing workspace file loaded.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Joined hard-failure messages, if any.
    #[must_use]
    pub fn error_message(&self) -> Option<String> {
        if self.errors.is_empty() {
            None
        } else {
            Some(format!(
                "Failed to load workspace: {}",
                self.errors.join("; ")
            ))
        }
    }
}
/// The restore report line for a deck kept as a placeholder, if it was.
fn placeholder_warning(
    config: &crate::scene::DeckConfig,
    reason: Option<String>,
) -> Option<String> {
    reason.map(|r| format!("Deck '{}' kept as a placeholder: {r}", config.name))
}

fn duration_config_to_spec(
    config: &crate::scene::DurationSpecConfig,
) -> crate::channel::DurationSpec {
    use crate::channel::DurationSpec;
    use crate::scene::DurationSpecConfig;
    match config {
        DurationSpecConfig::Beats(v) => DurationSpec::Beats(*v),
        DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(*v),
        DurationSpecConfig::Minutes(v) => DurationSpec::Minutes(*v),
        DurationSpecConfig::Hours(v) => DurationSpec::Hours(*v),
    }
}

impl VardaApp {
    /// Save the workspace to `.varda/`, including the GUI's last editor prefs.
    ///
    /// # Errors
    ///
    /// Returns an error listing every file that failed to write, or if `.varda/`
    /// cannot be created. Successful writes are kept.
    pub fn save_workspace(&mut self) -> anyhow::Result<()> {
        if let Err(e) = self.session.workspace.ensure_dir() {
            let msg = format!("Failed to create .varda directory: {e}");
            log::error!("{msg}");
            self.session.notifications.error(msg.clone());
            return Err(anyhow::anyhow!("{msg}"));
        }
        let mut errors: Vec<String> = Vec::new();
        {
            let scene = crate::persistence::snapshot_scene(
                &self.mixer,
                Some(&self.transport_config()),
                self.render.width,
                self.render.height,
            );
            match scene.save(self.session.workspace.scene_path()) {
                Ok(()) => log::info!(
                    "Saved scene to {}",
                    self.session.workspace.scene_path().display()
                ),
                Err(e) => {
                    log::error!("Failed to save scene: {e}");
                    errors.push(format!("scene: {e}"));
                }
            }
        }
        if let Some(midi) = &self.input.midi_devices {
            let midi_config = self
                .input
                .midi_mappings
                .to_config(&midi.devices, &self.input.auto_map_engine);
            match midi_config.save(self.session.workspace.midi_path()) {
                Ok(()) => log::info!(
                    "Saved MIDI mappings to {}",
                    self.session.workspace.midi_path().display()
                ),
                Err(e) => {
                    log::error!("Failed to save MIDI config: {e}");
                    errors.push(format!("midi: {e}"));
                }
            }
        }
        let mut stage = crate::persistence::snapshot_stage(
            &self.output.surface_manager,
            &self.output.outputs,
            &self.session.editor_prefs,
            &self.output.dome,
            self.output.domemaster_resolution,
        );
        stage.timecode = self.timecode_config();
        match stage.save(self.session.workspace.stage_path()) {
            Ok(()) => log::info!(
                "Saved stage to {}",
                self.session.workspace.stage_path().display()
            ),
            Err(e) => {
                log::error!("Failed to save stage: {e}");
                errors.push(format!("stage: {e}"));
            }
        }
        let keymap_config = self.input.keymap.to_config();
        match keymap_config.save(self.session.workspace.keymap_path()) {
            Ok(()) => log::info!(
                "Saved keymap to {}",
                self.session.workspace.keymap_path().display()
            ),
            Err(e) => {
                log::error!("Failed to save keymap: {e}");
                errors.push(format!("keymap: {e}"));
            }
        }
        match self
            .input
            .osc_config
            .save(self.session.workspace.osc_path())
        {
            Ok(()) => log::info!(
                "Saved OSC config to {}",
                self.session.workspace.osc_path().display()
            ),
            Err(e) => {
                log::error!("Failed to save OSC config: {e}");
                errors.push(format!("osc: {e}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            let msg = format!("Failed to save workspace: {}", errors.join("; "));
            self.session.notifications.error(msg.clone());
            Err(anyhow::anyhow!("{msg}"))
        }
    }

    /// Load the workspace from `.varda/` if it exists, replacing the mixer when
    /// a scene is found. Returns stage.json editor prefs and any hard failures.
    pub fn load_workspace(&mut self) -> WorkspaceLoad {
        if !self.session.workspace.exists() {
            log::info!("No .varda/ directory found, starting fresh");
            return WorkspaceLoad {
                editor_prefs: None,
                errors: Vec::new(),
            };
        }
        // History refers to the replaced state.
        self.session.history.clear();
        let mut loaded_prefs: Option<EditorPrefs> = None;
        let mut errors: Vec<String> = Vec::new();
        if self.session.workspace.has_stage() {
            match crate::persistence::StagePrefs::load(self.session.workspace.stage_path()) {
                Ok(prefs) => {
                    loaded_prefs = Some(prefs.editor_prefs());
                    self.output.dome = prefs.dome_config();
                    self.apply_timecode_config(&prefs.timecode);
                    self.output.surface_manager = prefs.surfaces;
                    // Set before `ensure_domemaster`, which reads it.
                    self.output.domemaster_resolution = prefs.domemaster_resolution;
                    errors.extend(self.reconcile_outputs(&prefs.outputs));
                    log::info!(
                        "Loaded stage with {} surfaces, {} outputs",
                        self.output.surface_manager.surfaces.len(),
                        prefs.outputs.len()
                    );

                    let has_dome_surfaces = self.output.surface_manager.surfaces.iter().any(|s| {
                        matches!(s.source, crate::renderer::context::OutputSource::Domemaster)
                    });
                    if has_dome_surfaces {
                        self.ensure_domemaster();
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load stage: {e}");
                    errors.push(format!("stage: {e}"));
                }
            }
        }
        if self.session.workspace.has_scene() {
            match crate::scene::SceneConfig::load(self.session.workspace.scene_path()) {
                Ok(scene_config) => {
                    if let (Some(w), Some(h)) =
                        (scene_config.render_width, scene_config.render_height)
                        && w > 0
                        && h > 0
                    {
                        self.render.width = w;
                        self.render.height = h;
                        log::info!("Scene render resolution: {w}×{h}");
                    }
                    let (width, height) = (self.render.width, self.render.height);
                    let restored = {
                        let (providers, mut env) =
                            self.sources.env(&self.render.context, width, height, &[]);
                        crate::persistence::restore_scene(&scene_config, providers, &mut env)
                    };
                    match restored {
                        Ok(result) => {
                            self.mixer = result.mixer;
                            // Only frame rate and loop are saved; position starts at zero.
                            self.show
                                .transport
                                .set_timecode_rate(scene_config.transport.timecode_rate);
                            self.show
                                .transport
                                .set_loop_region(scene_config.transport.loop_region);
                            for warn in &result.warnings {
                                self.session.notifications.warn(warn.clone());
                            }
                            // Start analyzers for active (unmuted, nonzero opacity) decks.
                            for ch in self.mixer.channels_mut() {
                                let any_solo = ch.decks.iter().any(|s| s.solo);
                                for slot in &mut ch.decks {
                                    let active = !slot.mute
                                        && (!any_solo || slot.solo)
                                        && slot.opacity > 0.0;
                                    if active {
                                        slot.deck.ensure_preprocessor_analyzers(
                                            &self.sources.analyzer_registry,
                                        );
                                    }
                                }
                            }

                            log::info!(
                                "Loaded scene with {} channels",
                                scene_config.channels.len()
                            );
                        }
                        Err(e) => {
                            log::error!("Failed to restore scene: {e}");
                            errors.push(format!("scene: {e}"));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load scene file: {e}");
                    errors.push(format!("scene: {e}"));
                }
            }
        }
        // Older stages name channels by index; resolve them against the loaded scene.
        {
            let channels = self.mixer.channels();
            let channel_uuids: Vec<String> =
                channels.iter().map(|c| c.uuid().to_string()).collect();
            let deck_uuids: Vec<Vec<String>> = channels
                .iter()
                .map(|c| c.decks.iter().map(|s| s.deck.uuid().to_string()).collect())
                .collect();
            for warning in self
                .output
                .surface_manager
                .resolve_legacy_sources(&channel_uuids, &deck_uuids)
            {
                log::warn!("{warning}");
                self.session.notifications.warn(warning);
            }
        }
        if self.session.workspace.has_midi() {
            match crate::midi::MidiConfig::load(self.session.workspace.midi_path()) {
                Ok(midi_config) => {
                    if let Some(midi) = &self.input.midi_devices {
                        self.input
                            .midi_mappings
                            .load_from_config(&midi_config, &midi.devices);
                        log::info!("Loaded {} MIDI mappings", midi_config.mappings.len());
                    } else {
                        log::info!(
                            "MIDI config found but no MIDI devices connected, mappings deferred"
                        );
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load MIDI config: {e}");
                    errors.push(format!("midi: {e}"));
                }
            }
        }
        if self.session.workspace.has_keymap() {
            match crate::keymap::KeymapConfig::load(self.session.workspace.keymap_path()) {
                Ok(keymap_config) => {
                    self.input.keymap.load_config(&keymap_config);
                    log::info!("Loaded {} keyboard shortcuts", keymap_config.bindings.len());
                }
                Err(e) => {
                    log::warn!("Failed to load keymap config: {e}");
                    errors.push(format!("keymap: {e}"));
                }
            }
        }
        // OSC config loads in new(); this refreshes feedback targets.
        if self.session.workspace.has_osc() {
            match crate::osc::OscConfig::load(self.session.workspace.osc_path()) {
                Ok(config) => {
                    if let Some(ref mut sender) = self.input.osc_feedback {
                        for target in &config.feedback_targets {
                            if let Err(e) = sender.add_target(target) {
                                log::warn!("Failed to add OSC feedback target '{target}': {e}");
                            }
                        }
                    }
                    self.input.osc_config = config;
                    log::info!(
                        "Loaded OSC config: port={}, enabled={}, {} feedback target(s)",
                        self.input.osc_config.in_port,
                        self.input.osc_config.enabled,
                        self.input.osc_config.feedback_targets.len()
                    );
                }
                Err(e) => {
                    log::warn!("Failed to load OSC config: {e}");
                    errors.push(format!("osc: {e}"));
                }
            }
        }
        if let Some(prefs) = loaded_prefs {
            self.session.editor_prefs = prefs;
        }
        if !errors.is_empty() {
            let msg = format!("Failed to load workspace: {}", errors.join("; "));
            self.session.notifications.error(msg);
        }
        WorkspaceLoad {
            editor_prefs: loaded_prefs,
            errors,
        }
    }

    /// Patch the mixer to match `target`, changing only what differs. Returns
    /// restore warnings.
    pub fn apply_scene_diff(
        &mut self,
        target: &crate::scene::SceneConfig,
        rw: u32,
        rh: u32,
    ) -> Vec<String> {
        let mut warnings = Vec::new();

        // (a) Crossfader
        self.mixer.set_crossfader(target.crossfader);

        // (b) Modulation
        self.mixer.set_modulation(target.modulation.clone());

        // (b2) Macros
        self.mixer.set_macros(target.macros.clone());

        // (b3) Arrangement. Transport position is not part of undo.
        self.mixer.set_arrangement(target.arrangement.clone());
        self.show
            .transport
            .set_timecode_rate(target.transport.timecode_rate);
        self.show
            .transport
            .set_loop_region(target.transport.loop_region);

        // (c) Transition shader, recreated only if the name changed
        {
            let current_name = self.mixer.active_transition().map(|t| t.name.clone());
            let target_name = target.active_transition.as_deref();
            if current_name.as_deref() != target_name {
                match target_name {
                    Some(name) => {
                        if let Some(shader) = self
                            .sources
                            .registry
                            .transitions()
                            .iter()
                            .find(|s| s.name() == name)
                        {
                            if let Err(e) = self
                                .mixer
                                .set_transition(&self.render.context, (*shader).clone())
                            {
                                warnings
                                    .push(format!("Failed to restore transition '{name}': {e}"));
                            }
                        } else {
                            warnings.push(format!("Transition '{name}' not found in registry"));
                        }
                    }
                    None => self.mixer.clear_transition(),
                }
            }
        }

        // (d) Channels and decks, matched by UUID so references survive undo.
        // All are taken out, then the target is rebuilt in order, reusing UUID
        // matches and building the rest with their saved UUIDs.
        let labels = self.mixer.channel_labels();
        let (providers, mut env) = self.sources.env(&self.render.context, rw, rh, &labels);
        // Released through their providers once channels are settled.
        let mut dropped: Vec<crate::deck::Deck> = Vec::new();
        let mut spare_channels = std::mem::take(self.mixer.channels_mut());
        let mut spare_decks: Vec<crate::channel::DeckSlot> = spare_channels
            .iter_mut()
            .flat_map(|ch| ch.decks.drain(..))
            .collect();
        let mut channels = Vec::with_capacity(target.channels.len());

        for ch_config in &target.channels {
            let reused = take_by_uuid(&mut spare_channels, &ch_config.uuid, |ch| ch.uuid());
            let mut channel = if let Some(channel) = reused {
                channel
            } else {
                match crate::channel::Channel::new(
                    ch_config.name.clone(),
                    &self.render.context,
                    rw,
                    rh,
                ) {
                    Ok(mut channel) => {
                        if !ch_config.uuid.is_empty() {
                            channel.set_uuid(ch_config.uuid.clone());
                        }
                        channel
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "Failed to create channel '{}': {}",
                            ch_config.name, e
                        ));
                        continue;
                    }
                }
            };
            channel.name.clone_from(&ch_config.name);
            channel.opacity = ch_config.opacity;
            channel.blend_mode = ch_config.blend_mode.into();

            for deck_config in &ch_config.decks {
                let reused =
                    take_by_uuid(&mut spare_decks, &deck_config.uuid, |slot| slot.deck.uuid());
                let mut slot = match reused {
                    // Same deck and source: patch in place.
                    Some(slot)
                        if crate::persistence::source_configs_match(
                            &slot.deck,
                            &deck_config.source,
                            providers,
                        ) =>
                    {
                        slot
                    }
                    other => {
                        dropped.extend(other.map(|slot| slot.deck));
                        let (deck, warning) =
                            crate::persistence::restore_deck(deck_config, providers, &mut env);
                        warnings.extend(placeholder_warning(deck_config, warning));
                        crate::channel::DeckSlot::new(deck)
                    }
                };
                Self::patch_deck_slot(&mut slot, deck_config, env.gpu, env.shaders);
                channel.add_deck_slot(slot);
            }

            Self::diff_effects(
                &mut channel.effects,
                &ch_config.effects,
                &self.render.context,
                self.render.context.compositing_format,
                &mut warnings,
            );
            channels.push(channel);
        }
        dropped.extend(spare_decks.into_iter().map(|slot| slot.deck));
        *self.mixer.channels_mut() = channels;

        // Release devices held by dropped decks.
        for mut deck in dropped {
            providers.release(deck.source_mut(), &mut env);
            if let (Some(sensor), Some(depth)) = (
                deck.held_depth_prepro_sensor(),
                env.services.get_mut::<crate::depth::DepthSensorManager>(),
            ) {
                depth.release(sensor);
            }
        }

        let max_idx = self
            .mixer
            .channels()
            .iter()
            .filter_map(|ch| {
                ch.name
                    .strip_prefix("Ch ")
                    .and_then(|s| s.parse::<usize>().ok())
            })
            .max()
            .map_or(self.mixer.channels().len(), |n| n + 1);
        self.mixer.set_next_channel_index(max_idx);

        // (e) Master effects
        Self::diff_effects(
            self.mixer.master_effects_mut(),
            &target.master_effects,
            &self.render.context,
            self.render.context.compositing_format,
            &mut warnings,
        );

        // (f) Transition sequences
        let channel_uuids: Vec<String> = self
            .mixer
            .channels()
            .iter()
            .map(|ch| ch.uuid().to_string())
            .collect();
        self.mixer.transition_sequences_mut().clear();
        for seq_config in &target.transition_sequences {
            let steps = crate::persistence::restore_sequence_steps(
                &seq_config.steps,
                &channel_uuids,
                &mut warnings,
            );
            self.mixer.transition_sequences_mut().push(
                crate::mixer::TransitionSequence::with_uuid(
                    seq_config.uuid.clone(),
                    seq_config.name.clone(),
                    steps,
                    seq_config.enabled,
                ),
            );
        }

        // Last, once every deck and effect a key can name exists.
        self.mixer.rekey_legacy_modulation();
        warnings
    }

    /// The saved part of the transport: frame rate and loop, not position or run state.
    pub fn transport_config(&self) -> crate::scene::TransportConfig {
        crate::scene::TransportConfig {
            timecode_rate: self.show.transport.timecode_rate(),
            loop_region: self.show.transport.loop_region(),
        }
    }

    /// The timecode patch with its devices named, ready to persist.
    pub fn timecode_config(&self) -> crate::timecode::TimecodeConfig {
        self.input.timecode.to_config(
            |id| {
                self.audio
                    .manager
                    .devices()
                    .iter()
                    .find(|d| d.id == id)
                    .map(|d| d.name.clone())
            },
            |id| {
                self.input
                    .midi_devices
                    .as_ref()
                    .and_then(|midi| midi.devices.get(&id))
                    .map(|d| d.name.clone())
            },
        )
    }

    /// Restore a saved timecode patch against present devices, warning about
    /// missing ones.
    pub fn apply_timecode_config(&mut self, config: &crate::timecode::TimecodeConfig) {
        let audio: Vec<(crate::audio::AudioSourceId, String)> = self
            .audio
            .manager
            .devices()
            .iter()
            .map(|d| (d.id, d.name.clone()))
            .collect();
        let midi: Vec<(crate::midi::DeviceId, String)> = self
            .input
            .midi_devices
            .as_ref()
            .map(|m| m.devices.values().map(|d| (d.id, d.name.clone())).collect())
            .unwrap_or_default();

        let warnings = self.input.timecode.apply_config(
            config,
            |name| {
                audio
                    .iter()
                    .find(|(_, device)| device == name)
                    .map(|(id, _)| *id)
            },
            |name| {
                midi.iter()
                    .find(|(_, device)| device == name)
                    .map(|(id, _)| *id)
            },
        );
        for warning in warnings {
            self.session.notifications.warn(warning);
        }
    }

    /// Snapshot scene and stage for history. Editor prefs are captured but
    /// not restored.
    pub fn history_snapshot(&self) -> super::history::HistorySnapshot {
        let scene = crate::persistence::snapshot_scene(
            &self.mixer,
            Some(&self.transport_config()),
            self.render.width,
            self.render.height,
        );
        let stage = crate::persistence::snapshot_stage(
            &self.output.surface_manager,
            &self.output.outputs,
            &self.session.editor_prefs,
            &self.output.dome,
            self.output.domemaster_resolution,
        );
        super::history::HistorySnapshot { scene, stage }
    }

    // ── Undo/redo ──────────────────────────────────────────────────────────
    //
    // One `HistoryManager` (on `SessionState`) shared by the GUI and the
    // command bus.

    pub fn history_can_undo(&self) -> bool {
        self.session.history.can_undo()
    }

    pub fn history_can_redo(&self) -> bool {
        self.session.history.can_redo()
    }

    fn restore_history_snapshot(&mut self, snapshot: &super::history::HistorySnapshot) {
        let rw = self.render.width;
        let rh = self.render.height;
        let warnings = self.apply_scene_diff(&snapshot.scene, rw, rh);
        self.apply_stage_diff(&snapshot.stage);
        self.mixer.clear_sub_mix_cache();
        for w in &warnings {
            log::warn!("History restore warning: {w}");
        }
    }

    /// Undo, pushing `current` onto the redo stack. Returns false if nothing to undo.
    pub fn history_undo(&mut self, current: super::history::HistorySnapshot) -> bool {
        let Some(snapshot) = self.session.history.undo(current) else {
            return false;
        };
        self.restore_history_snapshot(&snapshot);
        true
    }

    /// Redo, pushing `current` onto the undo stack. Returns false if nothing to redo.
    pub fn history_redo(&mut self, current: super::history::HistorySnapshot) -> bool {
        let Some(snapshot) = self.session.history.redo(current) else {
            return false;
        };
        self.restore_history_snapshot(&snapshot);
        true
    }

    /// Restore authored stage state for undo/redo: surfaces, per-output surface
    /// assignments, and the dome config. Output windows and editor prefs are
    /// left alone.
    ///
    /// GPU caches need no invalidation: hole masks are keyed by content hash
    /// and warp meshes are tessellated per frame.
    pub fn apply_stage_diff(&mut self, target: &crate::persistence::StagePrefs) {
        // (a) Surfaces and dome config, replaced wholesale.
        self.output.surface_manager = target.surfaces.clone();
        self.output.dome = target.dome_config();

        // (b) Surface assignments, patched on outputs matched by UUID. Outputs
        //     are never created or removed here.
        let mut restored_output_indices = Vec::new();
        for (idx, output) in self.output.outputs.iter_mut().enumerate() {
            let Some(cfg) = target.outputs.iter().find(|c| c.uuid == output.uuid) else {
                continue;
            };
            output.surface_assignments = cfg
                .surface_assignments
                .iter()
                .map(|a| crate::renderer::context::SurfaceAssignment {
                    surface_uuid: a.surface_uuid.clone(),
                    enabled: a.enabled,
                    overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                })
                .collect();
            output.tonemap_override = cfg.tonemap_override;
            match output.set_presentation_request(
                &self.render.context,
                &self.sources.services,
                cfg.presentation,
            ) {
                Ok(()) => restored_output_indices.push(idx),
                Err(error) => log::warn!(
                    "Could not restore presentation precision for output '{}': {error}",
                    output.uuid
                ),
            }
        }
        for idx in restored_output_indices {
            self.refresh_presentation_notification(idx);
        }

        // (c) Auto-mode edge blend.
        self.output.recompute_auto_edge_blend();
    }

    /// Patch a `DeckSlot`'s properties from config without rebuilding the source.
    fn patch_deck_slot(
        slot: &mut crate::channel::DeckSlot,
        config: &crate::scene::DeckConfig,
        context: &crate::renderer::GpuContext,
        registry: &crate::registry::ShaderRegistry,
    ) {
        slot.opacity = config.opacity;
        slot.blend_mode = config.blend_mode.into();
        slot.mute = config.mute;
        slot.solo = config.solo;
        slot.z_index = config.z_index;

        // The deck takes the ISF values stored under `params`.
        slot.deck.source_mut().patch(&config.source);
        if let Some(params) = config.source.get("params").and_then(|v| {
            serde_json::from_value::<std::collections::HashMap<String, crate::params::ParamValue>>(
                v.clone(),
            )
            .ok()
        }) {
            slot.deck.generator_params.restore(&params);
        }

        Self::diff_effects(
            &mut slot.deck.effects,
            &config.effects,
            context,
            context.compositing_format,
            &mut Vec::new(),
        );

        if let Some(at_config) = &config.auto_transition {
            use crate::channel::TransitionTrigger;
            let mut at = slot.auto_transition.take().unwrap_or_default();
            at.enabled = at_config.enabled;
            at.trigger = match at_config.trigger {
                crate::scene::TriggerConfig::Timer => TransitionTrigger::Timer,
                crate::scene::TriggerConfig::ClipEnd => TransitionTrigger::ClipEnd,
            };
            at.play_duration = duration_config_to_spec(&at_config.play_duration);
            at.transition_duration = duration_config_to_spec(&at_config.transition_duration);
            at.transition_shader_name
                .clone_from(&at_config.transition_shader);
            slot.auto_transition = Some(at);

            if let Some(shader_name) = &at_config.transition_shader {
                let needs_compile = slot
                    .transition_effect
                    .as_ref()
                    .is_none_or(|te| te.shader.name() != *shader_name);
                if needs_compile
                    && let Some(shader) = registry
                        .transitions()
                        .iter()
                        .find(|s| s.name() == *shader_name)
                    && let Err(e) = slot.set_transition_shader(context, (*shader).clone())
                {
                    log::warn!("Failed to restore deck transition shader '{shader_name}': {e}");
                }
            } else {
                slot.transition_effect = None;
            }
        } else {
            slot.auto_transition = None;
            slot.transition_effect = None;
        }
    }

    /// Diff an effect chain: patch matching effects, rebuild the rest.
    fn diff_effects(
        effects: &mut Vec<crate::deck::Effect>,
        target: &[crate::scene::EffectConfig],
        context: &crate::renderer::GpuContext,
        target_format: wgpu::TextureFormat,
        warnings: &mut Vec<String>,
    ) {
        // Matched by UUID, since modulation keys use it. Same shader patches
        // in place; anything else is rebuilt with its saved UUID.
        let mut spare = std::mem::take(effects);
        for cfg in target {
            let reused = take_by_uuid(&mut spare, &cfg.uuid, |eff| eff.uuid())
                .filter(|eff| eff.shader.file_path.as_deref().unwrap_or("") == cfg.path);
            if let Some(mut eff) = reused {
                eff.enabled = cfg.enabled;
                eff.params.restore(&cfg.params);
                effects.push(eff);
                continue;
            }
            match crate::persistence::restore_effect(cfg, context, target_format) {
                Ok(eff) => effects.push(eff),
                Err(e) => warnings.push(format!("Failed to restore effect '{}': {}", cfg.path, e)),
            }
        }
    }
}

/// Remove and return the item with `uuid`. An empty UUID (older files) matches nothing.
fn take_by_uuid<T>(items: &mut Vec<T>, uuid: &str, uuid_of: impl Fn(&T) -> &str) -> Option<T> {
    if uuid.is_empty() {
        return None;
    }
    let idx = items.iter().position(|item| uuid_of(item) == uuid)?;
    Some(items.swap_remove(idx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use tempfile::TempDir;

    fn parse_args(args: &[&str]) -> super::super::AppConfig {
        super::super::AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
    }

    fn headless_app_in(workspace: &std::path::Path) -> Option<super::super::VardaApp> {
        let ws = workspace.to_str().unwrap();
        let config = parse_args(&[
            "--headless",
            "--no-osc",
            "--no-ndi",
            "--no-syphon",
            "--workspace",
            ws,
        ]);
        crate::testing::headless_app_with(&config)
    }

    #[test]
    fn load_workspace_no_varda_dir() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let result = app.load_workspace();
        assert!(result.editor_prefs.is_none());
        assert!(result.is_ok());
    }

    #[test]
    fn save_load_roundtrip_scene() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let ch = crate::app::snapshot::build_mixer_snapshot(&app).channels[0]
            .uuid
            .clone();
        app.add_deck(
            &ch,
            &crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
        )
        .unwrap();
        app.set_crossfader(0.6);
        app.save_workspace().expect("save workspace");

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let _ = app2.load_workspace();
        let snap = crate::app::snapshot::build_mixer_snapshot(&app2);
        assert!(
            !snap.channels[0].decks.is_empty(),
            "deck should survive roundtrip"
        );
        assert!((snap.crossfader - 0.6).abs() < 1e-4);
    }

    #[test]
    fn save_load_roundtrip_stage() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let _uuid = app.output.surface_manager.add_surface(
            "Test Surface".to_string(),
            crate::renderer::context::OutputSource::Master,
        );
        app.save_workspace().expect("save workspace");

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let _ = app2.load_workspace();
        let surfaces = app2.build_engine_state().outputs.surfaces;
        assert!(
            surfaces.iter().any(|s| s.name == "Test Surface"),
            "surface should survive roundtrip"
        );
    }

    #[test]
    fn save_load_roundtrip_keeps_requested_presentation() {
        use crate::engine::value::render::{PresentationDepth, PresentationRequest};
        use crate::engine::{CommandResult, EngineCommand};
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        assert!(matches!(
            app.execute_command(EngineCommand::CreateOutput {
                sink: crate::output::SinkConfig::new("recording").with("path", "precision.mov"),
            }),
            CommandResult::OkWithId { .. }
        ));
        let output_uuid = app.build_engine_state().outputs.windows[0].uuid.clone();
        assert!(matches!(
            app.execute_command(EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.clone(),
                request: PresentationRequest {
                    depth: PresentationDepth::Sdr10,
                    dither: false,
                    ..PresentationRequest::default()
                },
            }),
            CommandResult::Ok
        ));
        app.save_workspace().expect("save workspace");

        let stage: crate::persistence::StagePrefs = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join(".varda").join("stage.json")).unwrap(),
        )
        .unwrap();
        let output = stage.outputs.first().expect("saved headless output");
        assert_eq!(output.uuid, output_uuid);
        assert_eq!(output.presentation.depth, PresentationDepth::Sdr10);
        assert!(!output.presentation.dither);
    }

    #[test]
    fn load_corrupt_scene_json() {
        let tmp = TempDir::new().unwrap();
        let varda_dir = tmp.path().join(".varda");
        std::fs::create_dir_all(&varda_dir).unwrap();
        std::fs::write(varda_dir.join("scene.json"), "not valid json {{{").unwrap();

        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let result = app.load_workspace();
        assert!(!result.is_ok(), "corrupt scene.json must be a load failure");
        assert!(result.error_message().expect("error").contains("scene"));
        let snap = crate::app::snapshot::build_mixer_snapshot(&app);
        assert_eq!(snap.channels.len(), 2);
        assert!(
            app.session.notifications.visible().iter().any(|n| n.level
                == crate::notifications::NotificationLevel::Error
                && n.message.contains("scene")),
            "operator must see the load failure"
        );
    }

    #[test]
    fn load_missing_scene_file() {
        let tmp = TempDir::new().unwrap();
        let varda_dir = tmp.path().join(".varda");
        std::fs::create_dir_all(&varda_dir).unwrap();
        // .varda/ exists but no scene.json

        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let _ = app.load_workspace();
        let snap = crate::app::snapshot::build_mixer_snapshot(&app);
        assert_eq!(snap.channels.len(), 2);
    }

    #[test]
    fn save_creates_varda_dir() {
        let tmp = TempDir::new().unwrap();
        let varda_dir = tmp.path().join(".varda");
        assert!(!varda_dir.exists());
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        app.save_workspace().expect("save workspace");
        assert!(varda_dir.exists());
    }

    #[test]
    fn save_failure_toasts_and_returns_err() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        app.save_workspace().expect("save workspace");
        let scene = tmp.path().join(".varda").join("scene.json");
        std::fs::remove_file(&scene).unwrap();
        std::fs::create_dir(&scene).unwrap();
        let err = app
            .save_workspace()
            .expect_err("a directory named scene.json must fail the save");
        assert!(err.to_string().contains("scene"));
        assert!(
            app.session.notifications.visible().iter().any(|n| n.level
                == crate::notifications::NotificationLevel::Error
                && n.message.contains("scene")),
            "operator must see the save failure"
        );
    }

    #[test]
    fn load_workspace_command_returns_err_on_corrupt_scene() {
        let tmp = TempDir::new().unwrap();
        let varda_dir = tmp.path().join(".varda");
        std::fs::create_dir_all(&varda_dir).unwrap();
        std::fs::write(varda_dir.join("scene.json"), "not valid json {{{").unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        match app.execute_command(crate::engine::EngineCommand::LoadWorkspace) {
            crate::engine::CommandResult::Err {
                code: crate::engine::ErrorCode::InternalError,
                message,
            } => assert!(message.contains("scene")),
            other => panic!("expected InternalError, got {other:?}"),
        }
    }

    #[test]
    fn load_workspace_returns_editor_prefs() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        app.session.editor_prefs = EditorPrefs {
            stage_editor_open: true,
            library_panel_open: true,
            ..Default::default()
        };
        app.save_workspace().expect("save workspace");

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let loaded = app2.load_workspace();
        assert!(loaded.is_ok(), "should load without hard errors");
        let loaded = loaded.editor_prefs.expect("should return editor prefs");
        assert!(loaded.stage_editor_open);
        assert!(loaded.library_panel_open);
    }

    /// Saving without GUI editor prefs keeps the ones loaded from disk.
    #[test]
    fn command_save_preserves_loaded_editor_prefs() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        app.session.editor_prefs = EditorPrefs {
            stage_editor_open: true,
            library_panel_open: true,
            ..Default::default()
        };
        app.save_workspace().expect("save workspace");

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let _ = app2.load_workspace();
        app2.execute_command(crate::engine::EngineCommand::SaveWorkspace);

        let Some(mut app3) = headless_app_in(tmp.path()) else {
            return;
        };
        let reloaded = app3
            .load_workspace()
            .editor_prefs
            .expect("editor prefs should survive");
        assert!(reloaded.stage_editor_open);
        assert!(reloaded.library_panel_open);
    }

    fn output_uuids(app: &super::super::VardaApp) -> Vec<String> {
        app.output.outputs.iter().map(|o| o.uuid.clone()).collect()
    }

    fn add_recording(app: &mut super::super::VardaApp, path: &str) -> String {
        let sink =
            crate::engine::value::provider::ProviderConfig::new("recording").with("path", path);
        match app.cmd_create_output(sink) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("output not created: {other:?}"),
        }
    }

    /// Loading the same workspace twice leaves one of each saved output, not
    /// two with the same UUID, and drops outputs the file does not have.
    #[test]
    fn reloading_a_workspace_matches_the_saved_outputs() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let saved = add_recording(&mut app, "a.mp4");
        app.save_workspace().expect("save workspace");
        let extra = add_recording(&mut app, "b.mp4");
        assert_eq!(output_uuids(&app), vec![saved.clone(), extra]);

        let _ = app.load_workspace();
        assert_eq!(output_uuids(&app), vec![saved.clone()]);
        let _ = app.load_workspace();
        assert_eq!(output_uuids(&app), vec![saved]);
    }

    /// A running output with matching saved settings keeps running through a
    /// reload; one with changed settings is stopped and rebuilt.
    #[test]
    fn reloading_keeps_running_outputs_whose_settings_match() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let same = add_recording(&mut app, "same.mp4");
        let changed = add_recording(&mut app, "before.mp4");
        app.save_workspace().expect("save workspace");
        for output in &mut app.output.outputs {
            output.active = true;
        }
        let idx = app.output.resolve_output(&changed).unwrap();
        app.output.outputs[idx].name = "Renamed".into();
        let sink = app.output.outputs[idx]
            .sink()
            .config()
            .with("path", "after.mp4");
        let _ = app.cmd_set_output_target(&changed, &sink);
        app.output.outputs[idx].active = true;

        let _ = app.load_workspace();

        let find = |uuid: &str| app.output.outputs.iter().find(|o| o.uuid == uuid).unwrap();
        assert!(find(&same).active, "an unchanged output keeps running");
        assert!(!find(&changed).active, "a changed output is stopped");
        assert_eq!(
            find(&changed).sink().config().str("path"),
            Some("before.mp4")
        );
        assert_ne!(find(&changed).name, "Renamed");
    }

    #[test]
    fn apply_scene_diff_crossfader() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let scene = crate::scene::SceneConfig {
            version: 3,
            channels: vec![
                crate::scene::ChannelConfig {
                    uuid: crate::ids::generate_short_uuid(),
                    name: "Ch 0".into(),
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendModeConfig::Normal,
                    decks: vec![],
                    effects: vec![],
                    modulation: vec![],
                },
                crate::scene::ChannelConfig {
                    uuid: crate::ids::generate_short_uuid(),
                    name: "Ch 1".into(),
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendModeConfig::Normal,
                    decks: vec![],
                    effects: vec![],
                    modulation: vec![],
                },
            ],
            crossfader: 0.42,
            active_transition: None,
            master_effects: vec![],
            modulation: crate::modulation::ModulationEngine::default(),
            macros: crate::macros::MacroBank::default(),
            transition_sequences: vec![],
            render_width: Some(1920),
            render_height: Some(1080),
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        };
        let warnings = app.apply_scene_diff(&scene, 1920, 1080);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let snap = crate::app::snapshot::build_mixer_snapshot(&app);
        assert!(
            (snap.crossfader - 0.42).abs() < 1e-4,
            "crossfader should be applied via diff"
        );
    }

    // ── The timecode patch in stage.json ────────────────────────

    /// Warnings the load raised.
    fn warnings(app: &super::super::VardaApp) -> Vec<String> {
        app.session
            .notifications
            .visible()
            .iter()
            .filter(|n| n.level == crate::notifications::NotificationLevel::Warning)
            .map(|n| n.message.clone())
            .collect()
    }

    /// The patch saves device names, since ids shift when the rig changes.
    #[test]
    fn a_saved_ltc_patch_names_the_interface_rather_than_its_slot() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let Some(device) = app.audio.manager.devices().first().cloned() else {
            return;
        };
        app.input
            .timecode
            .set_ltc_input(Some(crate::timecode::LtcInput {
                source_id: device.id,
                channel: 1,
                rate: Some(crate::transport::TimecodeRate::Fps2997),
            }));

        let saved = app.timecode_config().ltc_input.expect("a patched input");

        assert_eq!(saved.device, device.name);
        assert_eq!(
            saved.channel, 1,
            "which channel carries timecode is the patch"
        );
        assert_eq!(
            saved.rate,
            Some(crate::transport::TimecodeRate::Fps2997),
            "29.97 non-drop has to be named, so the naming must survive"
        );
    }

    /// A restored patch finds the interface by name.
    #[test]
    fn a_saved_ltc_patch_is_restored_against_the_interface_of_that_name() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let Some(device) = app.audio.manager.devices().last().cloned() else {
            return;
        };
        // Saved with no id.
        let config = crate::timecode::TimecodeConfig {
            preference: crate::timecode::PreferenceConfig::Auto,
            ltc_input: Some(crate::timecode::LtcInputConfig {
                device: device.name.clone(),
                channel: 1,
                rate: None,
            }),
        };

        app.apply_timecode_config(&config);

        assert_eq!(
            app.input.timecode.ltc_input(),
            Some(crate::timecode::LtcInput {
                source_id: device.id,
                channel: 1,
                rate: None,
            })
        );
        assert!(warnings(&app).is_empty(), "the interface is right there");
    }

    /// A missing interface is reported, not replaced by whatever holds its slot.
    #[test]
    fn an_ltc_patch_naming_an_absent_interface_is_reported_and_left_unset() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let config = crate::timecode::TimecodeConfig {
            preference: crate::timecode::PreferenceConfig::Auto,
            ltc_input: Some(crate::timecode::LtcInputConfig {
                device: "Scarlett 2i2 That Stayed Home".to_string(),
                channel: 1,
                rate: None,
            }),
        };

        app.apply_timecode_config(&config);

        assert_eq!(
            app.input.timecode.ltc_input(),
            None,
            "unset beats pointing at the wrong box"
        );
        assert!(
            warnings(&app)
                .iter()
                .any(|w| w.contains("Scarlett 2i2 That Stayed Home")),
            "the performer is told which interface is missing: {:?}",
            warnings(&app)
        );
    }

    /// A missing forced MTC port falls back to any input, with a warning.
    #[test]
    fn a_forced_mtc_port_that_is_absent_is_reported_and_falls_back() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let config = crate::timecode::TimecodeConfig {
            preference: crate::timecode::PreferenceConfig::ForceMtc {
                device: "Tascam DA-6400 On The Other Truck".to_string(),
            },
            ltc_input: None,
        };

        app.apply_timecode_config(&config);

        assert_eq!(
            app.input.timecode.preference(),
            crate::timecode::TimecodePreference::Auto
        );
        assert!(
            warnings(&app)
                .iter()
                .any(|w| w.contains("Tascam DA-6400 On The Other Truck")),
            "the performer is told which port is missing: {:?}",
            warnings(&app)
        );
    }

    /// A present MIDI port is found by name and forced by id.
    #[test]
    fn a_forced_mtc_port_that_is_present_is_restored_by_name() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let Some((id, name)) = app
            .input
            .midi_devices
            .as_ref()
            .and_then(|midi| midi.devices.values().next())
            .map(|device| (device.id, device.name.clone()))
        else {
            return;
        };
        let config = crate::timecode::TimecodeConfig {
            preference: crate::timecode::PreferenceConfig::ForceMtc { device: name },
            ltc_input: None,
        };

        app.apply_timecode_config(&config);

        assert_eq!(
            app.input.timecode.preference(),
            crate::timecode::TimecodePreference::ForceMtc { device_id: id }
        );
        assert!(warnings(&app).is_empty());
    }
}
