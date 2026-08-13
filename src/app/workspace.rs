//! Workspace persistence — save/load from `.varda/` directory.

use super::VardaApp;
use crate::usecases::ui::UILayoutState;

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
    /// Save the entire workspace to `.varda/`.
    /// `layout` is UI-consumer-owned state persisted in stage.json.
    pub fn save_workspace(&mut self, layout: &UILayoutState) {
        self.session.last_layout = layout.clone();
        if let Err(e) = self.session.workspace.ensure_dir() {
            log::error!("Failed to create .varda directory: {e}");
            return;
        }
        {
            let scene = crate::persistence::snapshot_scene(
                &self.mixer,
                Some(&self.transport_config()),
                self.render_width,
                self.render_height,
            );
            match scene.save(self.session.workspace.scene_path()) {
                Ok(()) => log::info!(
                    "Saved scene to {}",
                    self.session.workspace.scene_path().display()
                ),
                Err(e) => log::error!("Failed to save scene: {e}"),
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
                Err(e) => log::error!("Failed to save MIDI config: {e}"),
            }
        }
        let mut stage = crate::persistence::snapshot_stage(
            &self.output.surface_manager,
            &self.output.outputs,
            layout.stage_editor_grid_size,
            layout.stage_editor_snap,
            layout.library_panel_open,
            layout.right_panel_open,
            layout.stage_editor_open,
            layout.dome_preview_open,
            layout.dome_mode_active,
            layout.dome_preset,
            layout.dome_geometry,
            self.output.domemaster_resolution,
        );
        stage.timecode = self.timecode_config();
        match stage.save(self.session.workspace.stage_path()) {
            Ok(()) => log::info!(
                "Saved stage to {}",
                self.session.workspace.stage_path().display()
            ),
            Err(e) => log::error!("Failed to save stage: {e}"),
        }
        // Save keyboard shortcuts
        let keymap_config = self.input.keymap.to_config();
        match keymap_config.save(self.session.workspace.keymap_path()) {
            Ok(()) => log::info!(
                "Saved keymap to {}",
                self.session.workspace.keymap_path().display()
            ),
            Err(e) => log::error!("Failed to save keymap: {e}"),
        }
        // Save OSC config
        match self
            .input
            .osc_config
            .save(self.session.workspace.osc_path())
        {
            Ok(()) => log::info!(
                "Saved OSC config to {}",
                self.session.workspace.osc_path().display()
            ),
            Err(e) => log::error!("Failed to save OSC config: {e}"),
        }
    }

    /// Load workspace from `.varda/` if it exists.
    /// If a scene is found, replaces the default mixer with the restored one.
    /// Returns layout preferences loaded from stage.json (if any).
    pub fn load_workspace(&mut self) -> Option<UILayoutState> {
        if !self.session.workspace.exists() {
            log::info!("No .varda/ directory found, starting fresh");
            return None;
        }
        // Loading a workspace replaces the live scene/stage, so the undo/redo
        // timeline (which references the previous state) must be cleared.
        self.session.history.clear();
        let mut loaded_layout: Option<UILayoutState> = None;
        if self.session.workspace.has_stage() {
            match crate::persistence::StagePrefs::load(self.session.workspace.stage_path()) {
                Ok(prefs) => {
                    loaded_layout = Some(UILayoutState {
                        stage_editor_grid_size: prefs.grid_size,
                        stage_editor_snap: prefs.snap,
                        library_panel_open: prefs.library_panel_open,
                        right_panel_open: prefs.right_panel_open,
                        stage_editor_open: prefs.stage_editor_open,
                        dome_preview_open: prefs.dome_preview_open,
                        dome_mode_active: prefs.dome_mode_active,
                        dome_preset: prefs.dome_preset,
                        dome_geometry: prefs.dome_geometry,
                        ..UILayoutState::default()
                    });
                    self.apply_timecode_config(&prefs.timecode);
                    self.output.surface_manager = prefs.surfaces;
                    // Set before `ensure_domemaster` below, which builds at
                    // whatever this says.
                    self.output.domemaster_resolution = prefs.domemaster_resolution;
                    for output_config in &prefs.outputs {
                        // Migrate legacy target_display field to new target config
                        let mut config = output_config.clone();
                        if matches!(config.target, crate::scene::OutputTargetConfig::Windowed) {
                            if let Some(ref display_name) = config.target_display {
                                config.target = crate::scene::OutputTargetConfig::Display {
                                    name: display_name.clone(),
                                };
                            }
                        }
                        self.output.pending_output_creates.push(config);
                    }
                    log::info!(
                        "Loaded stage with {} surfaces, {} outputs",
                        self.output.surface_manager.surfaces.len(),
                        prefs.outputs.len()
                    );

                    // If any surface uses Domemaster source, ensure the renderer exists
                    let has_dome_surfaces = self.output.surface_manager.surfaces.iter().any(|s| {
                        matches!(s.source, crate::renderer::context::OutputSource::Domemaster)
                    });
                    if has_dome_surfaces {
                        self.ensure_domemaster();
                    }
                }
                Err(e) => log::warn!("Failed to load stage: {e}"),
            }
        }
        if self.session.workspace.has_scene() {
            match crate::scene::SceneConfig::load(self.session.workspace.scene_path()) {
                Ok(scene_config) => {
                    // Apply render resolution from scene if present
                    if let (Some(w), Some(h)) =
                        (scene_config.render_width, scene_config.render_height)
                    {
                        if w > 0 && h > 0 {
                            self.render_width = w;
                            self.render_height = h;
                            log::info!("Scene render resolution: {w}×{h}");
                        }
                    }
                    match crate::persistence::restore_scene(
                        &scene_config,
                        &self.context,
                        &self.registry,
                        &mut self.camera_manager,
                        &mut self.screen_capture_manager,
                        &mut self.depth_manager,
                        &mut self.external_io.ndi_manager,
                        &mut self.external_io.stream_manager,
                        &mut self.external_io.html_manager,
                        self.render_width,
                        self.render_height,
                    ) {
                        Ok(result) => {
                            self.mixer = result.mixer;
                            // How the show counts frames and where it loops are
                            // authored; where it was stopped is not, so the
                            // position stays at zero and the arrangement stays
                            // inert until someone starts it.
                            self.transport
                                .set_timecode_rate(scene_config.transport.timecode_rate);
                            self.transport
                                .set_loop_region(scene_config.transport.loop_region);
                            for warn in &result.warnings {
                                self.session.notifications.warn(warn.clone());
                            }
                            // Syphon decks that could not resolve at restore time
                            // (producer not publishing yet) — the render thread
                            // auto-binds them as their servers appear.
                            #[cfg(target_os = "macos")]
                            {
                                if !result.pending_syphon.is_empty() {
                                    log::info!(
                                        "{} Syphon deck(s) deferred to late-bind",
                                        result.pending_syphon.len()
                                    );
                                }
                                self.external_io.pending_syphon = result.pending_syphon;
                            }

                            // Start preprocessor analyzers for active decks restored from save.
                            // A deck is "active" when it is not muted and has non-zero opacity.
                            for ch in self.mixer.channels_mut() {
                                let any_solo = ch.decks.iter().any(|s| s.solo);
                                for slot in &mut ch.decks {
                                    let active = !slot.mute
                                        && (!any_solo || slot.solo)
                                        && slot.opacity > 0.0;
                                    if active {
                                        slot.deck
                                            .ensure_preprocessor_analyzers(&self.analyzer_registry);
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
                            self.session
                                .notifications
                                .error(format!("Failed to load scene: {e}"));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load scene file: {e}");
                    self.session
                        .notifications
                        .warn(format!("Failed to load scene: {e}"));
                }
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
                Err(e) => log::warn!("Failed to load MIDI config: {e}"),
            }
        }
        // Load keyboard shortcuts
        if self.session.workspace.has_keymap() {
            match crate::keymap::KeymapConfig::load(self.session.workspace.keymap_path()) {
                Ok(keymap_config) => {
                    self.input.keymap.load_config(&keymap_config);
                    log::info!("Loaded {} keyboard shortcuts", keymap_config.bindings.len());
                }
                Err(e) => log::warn!("Failed to load keymap config: {e}"),
            }
        }
        // Load OSC config (already loaded in new(), but refresh feedback targets on workspace load)
        if self.session.workspace.has_osc() {
            match crate::osc::OscConfig::load(self.session.workspace.osc_path()) {
                Ok(config) => {
                    // Update feedback targets
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
                Err(e) => log::warn!("Failed to load OSC config: {e}"),
            }
        }
        if let Some(layout) = &loaded_layout {
            self.session.last_layout = layout.clone();
        }
        loaded_layout
    }

    /// Apply a scene diff: compare current mixer state to a target `SceneConfig`
    /// and patch only what changed. Returns (warnings, `structural_changed`).
    /// `structural_changed` is true when channels/decks were added/removed/rebuilt
    /// (requiring texture re-registration).
    pub fn apply_scene_diff(
        &mut self,
        target: &crate::scene::SceneConfig,
        rw: u32,
        rh: u32,
    ) -> (Vec<String>, bool) {
        let mut warnings = Vec::new();
        let mut structural = false;

        // (a) Crossfader — always cheap
        self.mixer.set_crossfader(target.crossfader);

        // (b) Modulation — cheap clone
        self.mixer.set_modulation(target.modulation.clone());

        // (b2) Macros — cheap clone (config changes are undoable; live turns are not)
        self.mixer.set_macros(target.macros.clone());

        // (b3) Arrangement — cheap clone. Regions and lanes are edited through
        // the undo stack like any other scene data; the transport's *position*
        // is not, since undoing an edit must not also rewind the show.
        self.mixer.set_arrangement(target.arrangement.clone());
        self.transport
            .set_timecode_rate(target.transport.timecode_rate);
        self.transport.set_loop_region(target.transport.loop_region);

        // (c) Transition shader — compare names, only recreate if changed
        {
            let current_name = self.mixer.active_transition().map(|t| t.name.clone());
            let target_name = target.active_transition.as_deref();
            if current_name.as_deref() != target_name {
                match target_name {
                    Some(name) => {
                        if let Some(shader) = self
                            .registry
                            .transitions()
                            .iter()
                            .find(|s| s.name() == name)
                        {
                            if let Err(e) =
                                self.mixer.set_transition(&self.context, (*shader).clone())
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

        // (d) Channels — diff each paired channel
        let current_ch_count = self.mixer.channels().len();
        let target_ch_count = target.channels.len();

        // Patch existing channels that have a target counterpart
        let paired_count = current_ch_count.min(target_ch_count);
        for ch_idx in 0..paired_count {
            let ch_config = &target.channels[ch_idx];
            let ch = &mut self.mixer.channels_mut()[ch_idx];

            // Patch channel properties (zero cost)
            ch.name.clone_from(&ch_config.name);
            ch.opacity = ch_config.opacity;
            ch.blend_mode = ch_config.blend_mode.into();

            // Diff decks within this channel
            let current_deck_count = ch.decks.len();
            let target_deck_count = ch_config.decks.len();
            let paired_decks = current_deck_count.min(target_deck_count);

            for d_idx in 0..paired_decks {
                let deck_config = &ch_config.decks[d_idx];
                if crate::persistence::source_configs_match(
                    &ch.decks[d_idx].deck,
                    &deck_config.source,
                ) {
                    // Same source — patch properties in place (zero GPU cost)
                    Self::patch_deck_slot(
                        &mut ch.decks[d_idx],
                        deck_config,
                        &self.context,
                        &self.registry,
                    );
                } else {
                    // Different source — rebuild just this deck
                    match crate::persistence::restore_deck(
                        deck_config,
                        &self.context,
                        &self.registry,
                        &mut self.camera_manager,
                        &mut self.screen_capture_manager,
                        &mut self.depth_manager,
                        &mut self.external_io.ndi_manager,
                        &mut self.external_io.stream_manager,
                        &mut self.external_io.html_manager,
                        rw,
                        rh,
                    ) {
                        Ok(deck) => {
                            let mut slot = crate::channel::DeckSlot::new(deck);
                            Self::patch_deck_slot(
                                &mut slot,
                                deck_config,
                                &self.context,
                                &self.registry,
                            );
                            ch.decks[d_idx] = slot;
                            structural = true;
                        }
                        Err(e) => {
                            warnings.push(format!(
                                "Failed to restore deck '{}': {}",
                                deck_config.name, e
                            ));
                        }
                    }
                }
            }

            // Remove excess decks
            if current_deck_count > target_deck_count {
                ch.decks.truncate(target_deck_count);
                structural = true;
            }

            // Add missing decks
            for d_idx in paired_decks..target_deck_count {
                let deck_config = &ch_config.decks[d_idx];
                match crate::persistence::restore_deck(
                    deck_config,
                    &self.context,
                    &self.registry,
                    &mut self.camera_manager,
                    &mut self.screen_capture_manager,
                    &mut self.depth_manager,
                    &mut self.external_io.ndi_manager,
                    &mut self.external_io.stream_manager,
                    &mut self.external_io.html_manager,
                    rw,
                    rh,
                ) {
                    Ok(deck) => {
                        let mut slot = crate::channel::DeckSlot::new(deck);
                        Self::patch_deck_slot(
                            &mut slot,
                            deck_config,
                            &self.context,
                            &self.registry,
                        );
                        ch.decks.push(slot);
                        structural = true;
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "Failed to restore deck '{}': {}",
                            deck_config.name, e
                        ));
                    }
                }
            }

            // Diff channel effects
            Self::diff_effects(
                &mut ch.effects,
                &ch_config.effects,
                &self.context,
                self.context.compositing_format,
                &mut warnings,
            );
        }

        // Remove excess channels
        if current_ch_count > target_ch_count {
            self.mixer.channels_mut().truncate(target_ch_count);
            structural = true;
        }

        // Add missing channels
        for ch_idx in paired_count..target_ch_count {
            let ch_config = &target.channels[ch_idx];
            match crate::channel::Channel::new(ch_config.name.clone(), &self.context, rw, rh) {
                Ok(mut channel) => {
                    channel.opacity = ch_config.opacity;
                    channel.blend_mode = ch_config.blend_mode.into();
                    for deck_config in &ch_config.decks {
                        match crate::persistence::restore_deck(
                            deck_config,
                            &self.context,
                            &self.registry,
                            &mut self.camera_manager,
                            &mut self.screen_capture_manager,
                            &mut self.depth_manager,
                            &mut self.external_io.ndi_manager,
                            &mut self.external_io.stream_manager,
                            &mut self.external_io.html_manager,
                            rw,
                            rh,
                        ) {
                            Ok(deck) => {
                                let mut slot = crate::channel::DeckSlot::new(deck);
                                Self::patch_deck_slot(
                                    &mut slot,
                                    deck_config,
                                    &self.context,
                                    &self.registry,
                                );
                                channel.add_deck_slot(slot);
                            }
                            Err(e) => {
                                warnings.push(format!(
                                    "Failed to restore deck '{}': {}",
                                    deck_config.name, e
                                ));
                            }
                        }
                    }
                    for eff_config in &ch_config.effects {
                        match crate::persistence::restore_effect(
                            eff_config,
                            &self.context,
                            self.context.texture_format,
                        ) {
                            Ok(eff) => channel.add_effect(eff),
                            Err(e) => {
                                warnings.push(format!("Failed to restore channel effect: {e}"));
                            }
                        }
                    }
                    self.mixer.channels_mut().push(channel);
                    structural = true;
                }
                Err(e) => {
                    warnings.push(format!(
                        "Failed to create channel '{}': {}",
                        ch_config.name, e
                    ));
                }
            }
        }

        // Update next_channel_index
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

        // (e) Master effects — diff
        Self::diff_effects(
            self.mixer.master_effects_mut(),
            &target.master_effects,
            &self.context,
            self.context.compositing_format,
            &mut warnings,
        );

        // (f) Transition sequences — cheap clone
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

        (warnings, structural)
    }

    /// The persisted half of the transport: how this show counts frames and
    /// where it loops, without its position or run state.
    pub fn transport_config(&self) -> crate::scene::TransportConfig {
        crate::scene::TransportConfig {
            timecode_rate: self.transport.timecode_rate(),
            loop_region: self.transport.loop_region(),
        }
    }

    /// The timecode patch with its devices named, ready to persist.
    pub fn timecode_config(&self) -> crate::timecode::TimecodeConfig {
        self.input.timecode.to_config(
            |id| {
                self.audio_manager
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

    /// Restore a saved timecode patch against the devices present now,
    /// reporting anything the rig no longer has.
    pub fn apply_timecode_config(&mut self, config: &crate::timecode::TimecodeConfig) {
        let audio: Vec<(crate::audio::AudioSourceId, String)> = self
            .audio_manager
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

    /// Build a combined history snapshot (scene + stage) of current engine
    /// state using neutral/default editor prefs.
    ///
    /// Used by the headless/API undo path (`EngineCommand::Undo`/`Redo`), which
    /// has no UI layout to source cosmetic editor prefs or dome layout flags
    /// from. `apply_stage_diff` ignores those cosmetic fields anyway, so the
    /// defaults are inconsequential to what undo actually restores. The windowed
    /// runner builds its own snapshot with real layout prefs.
    pub fn history_snapshot_default(&self) -> super::history::HistorySnapshot {
        let scene = crate::persistence::snapshot_scene(
            &self.mixer,
            Some(&self.transport_config()),
            self.render_width,
            self.render_height,
        );
        let d = crate::persistence::StagePrefs::default();
        let stage = crate::persistence::snapshot_stage(
            &self.output.surface_manager,
            &self.output.outputs,
            d.grid_size,
            d.snap,
            d.library_panel_open,
            d.right_panel_open,
            d.stage_editor_open,
            d.dome_preview_open,
            d.dome_mode_active,
            d.dome_preset,
            d.dome_geometry,
            self.output.domemaster_resolution,
        );
        super::history::HistorySnapshot { scene, stage }
    }

    /// Build a combined history snapshot (scene + stage) of current engine
    /// state, sourcing cosmetic editor prefs and dome layout flags from the UI
    /// `layout`. Used by the windowed runner's undo/redo push and restore.
    pub fn history_snapshot(&self, layout: &UILayoutState) -> super::history::HistorySnapshot {
        let scene = crate::persistence::snapshot_scene(
            &self.mixer,
            Some(&self.transport_config()),
            self.render_width,
            self.render_height,
        );
        let stage = crate::persistence::snapshot_stage(
            &self.output.surface_manager,
            &self.output.outputs,
            layout.stage_editor_grid_size,
            layout.stage_editor_snap,
            layout.library_panel_open,
            layout.right_panel_open,
            layout.stage_editor_open,
            layout.dome_preview_open,
            layout.dome_mode_active,
            layout.dome_preset,
            layout.dome_geometry,
            self.output.domemaster_resolution,
        );
        super::history::HistorySnapshot { scene, stage }
    }

    // ── Unified undo/redo timeline ─────────────────────────────────────────
    //
    // The engine owns the single `HistoryManager` (on `SessionState`). Both
    // consumers push into and restore from it through these methods, so the
    // windowed UI and the HTTP/headless command bus share one timeline. See
    // [undo-redo.md](/spec/undo-redo.md) → "Recording Points".

    /// True if there is an undoable action on the shared timeline.
    pub fn history_can_undo(&self) -> bool {
        self.session.history.can_undo()
    }

    /// True if there is a redoable action on the shared timeline.
    pub fn history_can_redo(&self) -> bool {
        self.session.history.can_redo()
    }

    /// Record `snapshot` as the pre-mutation state for one undoable step.
    pub fn push_history(&mut self, snapshot: super::history::HistorySnapshot) {
        self.session.history.push(snapshot);
    }

    /// Clear the undo/redo timeline (e.g. on workspace load).
    pub fn clear_history(&mut self) {
        self.session.history.clear();
    }

    /// Restore a history snapshot onto live state (scene + stage), returning
    /// what changed so a windowed caller can refresh GPU preview textures.
    fn restore_history_snapshot(
        &mut self,
        snapshot: super::history::HistorySnapshot,
    ) -> super::history::HistoryRestore {
        let rw = self.render_width;
        let rh = self.render_height;
        // Scene half — diff-apply (patches only what changed).
        let (warnings, structural_changed) = self.apply_scene_diff(&snapshot.scene, rw, rh);
        // Stage half — restore surfaces + assignments (no window lifecycle).
        // Cosmetic editor prefs are intentionally left untouched; dome layout
        // flags live in UI layout and are restored by the windowed caller.
        self.apply_stage_diff(&snapshot.stage);
        self.mixer.clear_sub_mix_cache();
        for w in &warnings {
            log::warn!("History restore warning: {w}");
        }
        super::history::HistoryRestore {
            snapshot,
            structural_changed,
        }
    }

    /// Undo the most recent undoable action. `current` is the live state to
    /// place on the redo stack. Returns `None` when the undo stack is empty.
    pub fn history_undo(
        &mut self,
        current: super::history::HistorySnapshot,
    ) -> Option<super::history::HistoryRestore> {
        let snapshot = self.session.history.undo(current)?;
        Some(self.restore_history_snapshot(snapshot))
    }

    /// Redo the most recently undone action. `current` is the live state to
    /// place on the undo stack. Returns `None` when the redo stack is empty.
    pub fn history_redo(
        &mut self,
        current: super::history::HistorySnapshot,
    ) -> Option<super::history::HistoryRestore> {
        let snapshot = self.session.history.redo(current)?;
        Some(self.restore_history_snapshot(snapshot))
    }

    /// Apply a stage diff: restore the authored stage state from a `StagePrefs`
    /// snapshot onto live state for undo/redo.
    ///
    /// Restores surfaces (geometry, warp, holes, combine, stacking, `dome_setup`)
    /// and per-output surface assignments. Deliberately does NOT recreate,
    /// remove, move, or resize output windows/monitors, and does NOT touch
    /// cosmetic editor prefs (grid size, snap, panel-open flags) — those are not
    /// authored content. Dome layout flags (mode/preset/geometry) live in UI
    /// layout state and are restored by the caller (the runner).
    ///
    /// GPU-derived caches rebuild automatically from the restored surface data:
    /// hole masks are content-hash keyed and warp meshes are tessellated per
    /// frame, so no explicit cache invalidation is needed here.
    pub fn apply_stage_diff(&mut self, target: &crate::persistence::StagePrefs) {
        // (a) Surfaces — plain data, swap wholesale.
        self.output.surface_manager = target.surfaces.clone();

        // (b) Per-output surface assignments — patch by matching uuid to the
        //     snapshot's OutputConfig. Never create/destroy/reposition windows;
        //     outputs present in the snapshot but not live (or vice versa) are
        //     ignored for lifecycle.
        for output in &mut self.output.outputs {
            let uuid = output.uuid().to_string();
            if let Some(cfg) = target.outputs.iter().find(|c| c.uuid == uuid) {
                *output.surface_assignments_mut() = cfg
                    .surface_assignments
                    .iter()
                    .map(|a| crate::renderer::context::SurfaceAssignment {
                        surface_uuid: a.surface_uuid.clone(),
                        enabled: a.enabled,
                        overlap_zones: crate::renderer::edge_blend::SurfaceOverlapZones::default(),
                    })
                    .collect();
            }
        }

        // (c) Recompute Auto-mode edge-blend overlap zones for the restored
        //     surface topology.
        self.recompute_auto_edge_blend();
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

        // Patch generator params for shader sources
        if let crate::scene::SourceConfig::Shader { params, .. } = &config.source {
            slot.deck.generator_params.values.clone_from(params);
        }

        // Patch solid color
        if let crate::scene::SourceConfig::SolidColor { color } = &config.source {
            slot.deck.set_solid_color(*color);
        }

        // Patch deck effects
        Self::diff_effects(
            &mut slot.deck.effects,
            &config.effects,
            context,
            context.compositing_format,
            &mut Vec::new(),
        );

        // Patch auto-transition config
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

            // Compile transition shader if specified
            if let Some(shader_name) = &at_config.transition_shader {
                // Only recompile if the shader name changed
                let needs_compile = slot
                    .transition_effect
                    .as_ref()
                    .is_none_or(|te| te.shader.name() != *shader_name);
                if needs_compile {
                    if let Some(shader) = registry
                        .transitions()
                        .iter()
                        .find(|s| s.name() == *shader_name)
                    {
                        if let Err(e) = slot.set_transition_shader(context, (*shader).clone()) {
                            log::warn!(
                                "Failed to restore deck transition shader '{shader_name}': {e}"
                            );
                        }
                    }
                }
            } else {
                slot.transition_effect = None;
            }
        } else {
            slot.auto_transition = None;
            slot.transition_effect = None;
        }
    }

    /// Diff an effect chain: patch params for matching effects, rebuild for mismatches.
    fn diff_effects(
        effects: &mut Vec<crate::deck::Effect>,
        target: &[crate::scene::EffectConfig],
        context: &crate::renderer::GpuContext,
        target_format: wgpu::TextureFormat,
        warnings: &mut Vec<String>,
    ) {
        let current_count = effects.len();
        let target_count = target.len();
        let paired = current_count.min(target_count);

        for i in 0..paired {
            let eff = &mut effects[i];
            let cfg = &target[i];
            let current_path = eff.shader.file_path.as_deref().unwrap_or("");
            if current_path == cfg.path {
                // Same shader — patch params + enabled (zero cost)
                eff.enabled = cfg.enabled;
                eff.params.values.clone_from(&cfg.params);
            } else {
                // Different shader — rebuild this effect
                match crate::persistence::restore_effect(cfg, context, target_format) {
                    Ok(new_eff) => effects[i] = new_eff,
                    Err(e) => {
                        warnings.push(format!("Failed to restore effect '{}': {}", cfg.path, e));
                    }
                }
            }
        }

        // Remove excess effects
        effects.truncate(target_count);

        // Add missing effects
        for cfg in target.iter().skip(paired) {
            match crate::persistence::restore_effect(cfg, context, target_format) {
                Ok(eff) => effects.push(eff),
                Err(e) => warnings.push(format!("Failed to restore effect '{}': {}", cfg.path, e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::traits::*;
    use clap::Parser;
    use tempfile::TempDir;

    fn parse_args(args: &[&str]) -> super::super::AppConfig {
        super::super::AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
    }

    fn headless_app_in(workspace: &std::path::Path) -> Option<super::super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let ws = workspace.to_str().unwrap();
        let config = parse_args(&[
            "--headless",
            "--no-osc",
            "--no-ndi",
            "--no-syphon",
            "--workspace",
            ws,
        ]);
        super::super::VardaApp::new(gpu, &config).ok()
    }

    #[test]
    fn load_workspace_no_varda_dir() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        // No .varda/ exists → load_workspace returns None
        let result = app.load_workspace();
        assert!(result.is_none());
    }

    #[test]
    fn save_load_roundtrip_scene() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let ch = app.mixer_snapshot().channels[0].uuid.clone();
        app.add_solid_color_deck(&ch, [1.0, 0.0, 0.0, 1.0]).unwrap();
        app.set_crossfader(0.6);
        app.save_workspace(&UILayoutState::default());

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let _ = app2.load_workspace();
        let snap = app2.mixer_snapshot();
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
        let _uuid = app.add_surface(
            "Test Surface",
            crate::renderer::context::OutputSource::Master,
        );
        app.save_workspace(&UILayoutState::default());

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let _ = app2.load_workspace();
        let surfaces = app2.surface_snapshot();
        assert!(
            surfaces.iter().any(|s| s.name == "Test Surface"),
            "surface should survive roundtrip"
        );
    }

    #[test]
    fn load_corrupt_scene_json() {
        let tmp = TempDir::new().unwrap();
        let varda_dir = tmp.path().join(".varda");
        std::fs::create_dir_all(&varda_dir).unwrap();
        // Write corrupt JSON
        std::fs::write(varda_dir.join("scene.json"), "not valid json {{{").unwrap();

        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        // Should not crash, should return None or gracefully handle
        let _result = app.load_workspace();
        // App should still function with default state
        let snap = app.mixer_snapshot();
        assert_eq!(snap.channels.len(), 2);
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
        // Should skip scene loading gracefully
        let snap = app.mixer_snapshot();
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
        app.save_workspace(&UILayoutState::default());
        assert!(varda_dir.exists());
    }

    #[test]
    fn load_workspace_returns_layout() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let layout = UILayoutState {
            stage_editor_open: true,
            library_panel_open: true,
            ..Default::default()
        };
        app.save_workspace(&layout);

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        let loaded = app2.load_workspace();
        assert!(loaded.is_some(), "should return layout");
        let loaded = loaded.unwrap();
        assert!(loaded.stage_editor_open);
        assert!(loaded.library_panel_open);
    }

    /// `EngineCommand::SaveWorkspace` carries no layout, so it must reuse the one
    /// loaded from disk instead of writing defaults over the user's panels.
    #[test]
    fn command_save_preserves_loaded_layout() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        app.save_workspace(&UILayoutState {
            stage_editor_open: true,
            library_panel_open: true,
            ..Default::default()
        });

        let Some(mut app2) = headless_app_in(tmp.path()) else {
            return;
        };
        app2.load_workspace();
        app2.execute_command(crate::engine::EngineCommand::SaveWorkspace);

        let Some(mut app3) = headless_app_in(tmp.path()) else {
            return;
        };
        let reloaded = app3.load_workspace().expect("layout should survive");
        assert!(reloaded.stage_editor_open);
        assert!(reloaded.library_panel_open);
    }

    #[test]
    fn apply_scene_diff_crossfader() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        // Build a minimal SceneConfig with different crossfader
        let scene = crate::scene::SceneConfig {
            version: 3,
            channels: vec![
                crate::scene::ChannelConfig {
                    uuid: crate::deck::generate_short_uuid(),
                    name: "Ch 0".into(),
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendModeConfig::Normal,
                    decks: vec![],
                    effects: vec![],
                    modulation: vec![],
                },
                crate::scene::ChannelConfig {
                    uuid: crate::deck::generate_short_uuid(),
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
        let (warnings, _structural) = app.apply_scene_diff(&scene, 1920, 1080);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let snap = app.mixer_snapshot();
        assert!(
            (snap.crossfader - 0.42).abs() < 1e-4,
            "crossfader should be applied via diff"
        );
    }

    // ── The timecode patch in stage.json ────────────────────────

    /// Warnings the load raised, as the performer would read them.
    fn warnings(app: &super::super::VardaApp) -> Vec<String> {
        app.session
            .notifications
            .visible()
            .iter()
            .filter(|n| n.level == crate::notifications::NotificationLevel::Warning)
            .map(|n| n.message.clone())
            .collect()
    }

    /// The patch is written down as the name of a box, not the slot it happened
    /// to enumerate in. Ids are handed out at enumeration and shift whenever the
    /// rig changes between load-ins, so a saved id would point at whatever
    /// interface came up in that slot the next night.
    #[test]
    fn a_saved_ltc_patch_names_the_interface_rather_than_its_slot() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let Some(device) = app.audio_manager.devices().first().cloned() else {
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

    /// The same patch coming back finds the interface by name, whatever id it
    /// holds tonight.
    #[test]
    fn a_saved_ltc_patch_is_restored_against_the_interface_of_that_name() {
        let tmp = TempDir::new().unwrap();
        let Some(mut app) = headless_app_in(tmp.path()) else {
            return;
        };
        let Some(device) = app.audio_manager.devices().last().cloned() else {
            return;
        };
        // Written by a previous load-in, when nobody recorded an id at all.
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

    /// A rig one interface short has to say so. Pointing the patch at whatever
    /// box now holds that slot would read timecode off the wrong cable, and
    /// saying nothing looks identical to a show that has not started.
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

    /// Same for the MIDI port a forced MTC patch names: it falls back to
    /// following whatever arrives, and says that it did.
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

    /// A MIDI port that is present is found by name and forced by id, which is
    /// the whole reason `midi.json` and the timecode patch both store names.
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
