//! Workspace persistence — save/load from `.varda/` directory.

use super::VardaApp;

impl VardaApp {
    /// Save the entire workspace to `.varda/`.
    pub fn save_workspace(&self) {
        if let Err(e) = self.workspace.ensure_dir() {
            log::error!("Failed to create .varda directory: {}", e);
            return;
        }
        if let Some(mixer) = &self.mixer {
            let scene = crate::persistence::snapshot_scene(mixer);
            match scene.save(self.workspace.scene_path()) {
                Ok(()) => log::info!("Saved scene to {}", self.workspace.scene_path().display()),
                Err(e) => log::error!("Failed to save scene: {}", e),
            }
        }
        if let Some(midi) = &self.midi_devices {
            let midi_config = self.midi_mappings.to_config(&midi.devices);
            match midi_config.save(self.workspace.midi_path()) {
                Ok(()) => log::info!("Saved MIDI mappings to {}", self.workspace.midi_path().display()),
                Err(e) => log::error!("Failed to save MIDI config: {}", e),
            }
        }
        let stage = crate::persistence::snapshot_stage(
            &self.surface_manager, &self.output_windows,
            self.stage_editor_grid_size, self.stage_editor_snap,
            self.library_panel_open, self.stage_editor_open,
        );
        match stage.save(self.workspace.stage_path()) {
            Ok(()) => log::info!("Saved stage to {}", self.workspace.stage_path().display()),
            Err(e) => log::error!("Failed to save stage: {}", e),
        }
    }

    /// Load workspace from `.varda/` if it exists.
    pub fn load_workspace(&mut self) {
        let Some(context) = &self.context else { return };
        if !self.workspace.exists() {
            log::info!("No .varda/ directory found, starting fresh");
            return;
        }
        if self.workspace.has_stage() {
            match crate::persistence::StagePrefs::load(self.workspace.stage_path()) {
                Ok(prefs) => {
                    self.stage_editor_grid_size = prefs.grid_size;
                    self.stage_editor_snap = prefs.snap;
                    self.library_panel_open = prefs.library_panel_open;
                    self.stage_editor_open = prefs.stage_editor_open;
                    self.surface_manager = prefs.surfaces;
                    for _output_config in &prefs.outputs {
                        self.pending_output_creates.push(());
                    }
                    log::info!("Loaded stage with {} surfaces, {} outputs",
                        self.surface_manager.surfaces.len(), prefs.outputs.len());
                }
                Err(e) => log::warn!("Failed to load stage: {}", e),
            }
        }
        if self.workspace.has_scene() {
            match crate::scene::SceneConfig::load(self.workspace.scene_path()) {
                Ok(scene_config) => {
                    match crate::persistence::restore_scene(&scene_config, context, &self.registry, &mut self.camera_manager) {
                        Ok(result) => {
                            self.mixer = Some(result.mixer);
                            for warn in &result.warnings {
                                self.notifications.warn(warn.clone());
                            }
                            log::info!("Loaded scene with {} channels", scene_config.channels.len());
                        }
                        Err(e) => {
                            log::error!("Failed to restore scene: {}", e);
                            self.notifications.error(format!("Failed to load scene: {}", e));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load scene file: {}", e);
                    self.notifications.warn(format!("Failed to load scene: {}", e));
                }
            }
        }
        if self.workspace.has_midi() {
            match crate::midi::MidiConfig::load(self.workspace.midi_path()) {
                Ok(midi_config) => {
                    if let Some(midi) = &self.midi_devices {
                        self.midi_mappings.load_from_config(&midi_config, &midi.devices);
                        log::info!("Loaded {} MIDI mappings", midi_config.mappings.len());
                    } else {
                        log::info!("MIDI config found but no MIDI devices connected, mappings deferred");
                    }
                }
                Err(e) => log::warn!("Failed to load MIDI config: {}", e),
            }
        }
    }

    /// Initialize the mixer with a default shader if none loaded from workspace.
    pub fn init_default_mixer(&mut self) {
        let Some(context) = &self.context else { return };
        if self.mixer.is_some() { return; }
        match crate::mixer::Mixer::new(context, crate::app::RENDER_WIDTH, crate::app::RENDER_HEIGHT) {
            Ok(mut mixer) => {
                let generators = self.registry.generators();
                let shader_to_load = generators.iter()
                    .find(|s| s.metadata.passes.as_ref().map(|p| !p.is_empty()).unwrap_or(false))
                    .or_else(|| generators.first());
                if let Some(shader) = shader_to_load {
                    log::info!("Loading shader: {}", shader.name());
                    match crate::Deck::new(context, (*shader).clone(), crate::app::RENDER_WIDTH, crate::app::RENDER_HEIGHT) {
                        Ok(deck) => {
                            if let Some(ch) = mixer.channel_mut(0) {
                                ch.add_deck(deck);
                                log::info!("Created deck in channel A with shader: {}", shader.name());
                            }
                        }
                        Err(e) => log::error!("Failed to create deck: {}", e),
                    }
                }
                self.mixer = Some(mixer);
            }
            Err(e) => log::error!("Failed to create mixer: {}", e),
        }
    }
}
