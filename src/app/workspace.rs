//! Workspace persistence — save/load from `.varda/` directory.

use super::VardaApp;
use crate::usecases::ui::UILayoutState;

impl VardaApp {
    /// Save the entire workspace to `.varda/`.
    /// `layout` is UI-consumer-owned state persisted in stage.json.
    pub fn save_workspace(&self, layout: &UILayoutState) {
        if let Err(e) = self.workspace.ensure_dir() {
            log::error!("Failed to create .varda directory: {}", e);
            return;
        }
        {
            let scene = crate::persistence::snapshot_scene(&self.mixer, self.render_width, self.render_height);
            match scene.save(self.workspace.scene_path()) {
                Ok(()) => log::info!("Saved scene to {}", self.workspace.scene_path().display()),
                Err(e) => log::error!("Failed to save scene: {}", e),
            }
        }
        if let Some(midi) = &self.midi_devices {
            let midi_config = self.midi_mappings.to_config(&midi.devices, &self.auto_map_engine);
            match midi_config.save(self.workspace.midi_path()) {
                Ok(()) => log::info!("Saved MIDI mappings to {}", self.workspace.midi_path().display()),
                Err(e) => log::error!("Failed to save MIDI config: {}", e),
            }
        }
        let stage = crate::persistence::snapshot_stage(
            &self.surface_manager, &self.outputs,
            layout.stage_editor_grid_size, layout.stage_editor_snap,
            layout.library_panel_open, layout.stage_editor_open,
        );
        match stage.save(self.workspace.stage_path()) {
            Ok(()) => log::info!("Saved stage to {}", self.workspace.stage_path().display()),
            Err(e) => log::error!("Failed to save stage: {}", e),
        }
        // Save keyboard shortcuts
        let keymap_config = self.keymap.to_config();
        match keymap_config.save(self.workspace.keymap_path()) {
            Ok(()) => log::info!("Saved keymap to {}", self.workspace.keymap_path().display()),
            Err(e) => log::error!("Failed to save keymap: {}", e),
        }
    }

    /// Load workspace from `.varda/` if it exists.
    /// If a scene is found, replaces the default mixer with the restored one.
    /// Returns layout preferences loaded from stage.json (if any).
    pub fn load_workspace(&mut self) -> Option<UILayoutState> {
        if !self.workspace.exists() {
            log::info!("No .varda/ directory found, starting fresh");
            return None;
        }
        let mut loaded_layout: Option<UILayoutState> = None;
        if self.workspace.has_stage() {
            match crate::persistence::StagePrefs::load(self.workspace.stage_path()) {
                Ok(prefs) => {
                    loaded_layout = Some(UILayoutState {
                        stage_editor_grid_size: prefs.grid_size,
                        stage_editor_snap: prefs.snap,
                        library_panel_open: prefs.library_panel_open,
                        stage_editor_open: prefs.stage_editor_open,
                        ..UILayoutState::default()
                    });
                    self.surface_manager = prefs.surfaces;
                    for output_config in &prefs.outputs {
                        // Migrate legacy target_display field to new target config
                        let mut config = output_config.clone();
                        if matches!(config.target, crate::scene::OutputTargetConfig::Windowed) {
                            if let Some(ref display_name) = config.target_display {
                                config.target = crate::scene::OutputTargetConfig::Display { name: display_name.clone() };
                            }
                        }
                        self.pending_output_creates.push(config);
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
                    // Apply render resolution from scene if present
                    if let (Some(w), Some(h)) = (scene_config.render_width, scene_config.render_height) {
                        if w > 0 && h > 0 {
                            self.render_width = w;
                            self.render_height = h;
                            log::info!("Scene render resolution: {}×{}", w, h);
                        }
                    }
                    match crate::persistence::restore_scene(&scene_config, &self.context, &self.registry, &mut self.camera_manager, &mut self.ndi_manager, &mut self.srt_manager, self.render_width, self.render_height) {
                        Ok(result) => {
                            self.mixer = result.mixer;
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
        // Load keyboard shortcuts
        if self.workspace.has_keymap() {
            match crate::keymap::KeymapConfig::load(self.workspace.keymap_path()) {
                Ok(keymap_config) => {
                    self.keymap.load_config(&keymap_config);
                    log::info!("Loaded {} keyboard shortcuts", keymap_config.bindings.len());
                }
                Err(e) => log::warn!("Failed to load keymap config: {}", e),
            }
        }
        loaded_layout
    }

    /// Apply a scene diff: compare current mixer state to a target SceneConfig
    /// and patch only what changed. Returns (warnings, structural_changed).
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

        // (c) Transition shader — compare names, only recreate if changed
        {
            let current_name = self.mixer.active_transition().map(|t| t.name.clone());
            let target_name = target.active_transition.as_deref();
            if current_name.as_deref() != target_name {
                match target_name {
                    Some(name) => {
                        if let Some(shader) = self.registry.transitions().iter()
                            .find(|s| s.name() == name)
                        {
                            if let Err(e) = self.mixer.set_transition(&self.context, (*shader).clone()) {
                                warnings.push(format!("Failed to restore transition '{}': {}", name, e));
                            }
                        } else {
                            warnings.push(format!("Transition '{}' not found in registry", name));
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
            ch.name = ch_config.name.clone();
            ch.opacity = ch_config.opacity;
            ch.blend_mode = ch_config.blend_mode.into();

            // Diff decks within this channel
            let current_deck_count = ch.decks.len();
            let target_deck_count = ch_config.decks.len();
            let paired_decks = current_deck_count.min(target_deck_count);

            for d_idx in 0..paired_decks {
                let deck_config = &ch_config.decks[d_idx];
                if crate::persistence::source_configs_match(&ch.decks[d_idx].deck, &deck_config.source) {
                    // Same source — patch properties in place (zero GPU cost)
                    Self::patch_deck_slot(&mut ch.decks[d_idx], deck_config, &self.context, &self.registry);
                } else {
                    // Different source — rebuild just this deck
                    match crate::persistence::restore_deck(
                        deck_config, &self.context, &self.registry,
                        &mut self.camera_manager, &mut self.ndi_manager, &mut self.srt_manager, rw, rh,
                    ) {
                        Ok(deck) => {
                            let mut slot = crate::channel::DeckSlot::new(deck);
                            Self::patch_deck_slot(&mut slot, deck_config, &self.context, &self.registry);
                            ch.decks[d_idx] = slot;
                            structural = true;
                        }
                        Err(e) => {
                            warnings.push(format!("Failed to restore deck '{}': {}", deck_config.name, e));
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
                    deck_config, &self.context, &self.registry,
                    &mut self.camera_manager, &mut self.ndi_manager, &mut self.srt_manager, rw, rh,
                ) {
                    Ok(deck) => {
                        let mut slot = crate::channel::DeckSlot::new(deck);
                        Self::patch_deck_slot(&mut slot, deck_config, &self.context, &self.registry);
                        ch.decks.push(slot);
                        structural = true;
                    }
                    Err(e) => {
                        warnings.push(format!("Failed to restore deck '{}': {}", deck_config.name, e));
                    }
                }
            }

            // Diff channel effects
            Self::diff_effects(&mut ch.effects, &ch_config.effects, &self.context,
                self.context.texture_format, &mut warnings);
        }

        // Remove excess channels
        if current_ch_count > target_ch_count {
            self.mixer.channels_mut().truncate(target_ch_count);
            structural = true;
        }

        // Add missing channels
        for ch_idx in paired_count..target_ch_count {
            let ch_config = &target.channels[ch_idx];
            match crate::channel::Channel::new(
                ch_config.name.clone(), &self.context, rw, rh,
            ) {
                Ok(mut channel) => {
                    channel.opacity = ch_config.opacity;
                    channel.blend_mode = ch_config.blend_mode.into();
                    for deck_config in &ch_config.decks {
                        match crate::persistence::restore_deck(
                            deck_config, &self.context, &self.registry,
                            &mut self.camera_manager, &mut self.ndi_manager, &mut self.srt_manager, rw, rh,
                        ) {
                            Ok(deck) => {
                                let mut slot = crate::channel::DeckSlot::new(deck);
                                Self::patch_deck_slot(&mut slot, deck_config, &self.context, &self.registry);
                                channel.add_deck_slot(slot);
                            }
                            Err(e) => {
                                warnings.push(format!("Failed to restore deck '{}': {}", deck_config.name, e));
                            }
                        }
                    }
                    for eff_config in &ch_config.effects {
                        match crate::persistence::restore_effect(eff_config, &self.context, self.context.texture_format) {
                            Ok(eff) => channel.add_effect(eff),
                            Err(e) => warnings.push(format!("Failed to restore channel effect: {}", e)),
                        }
                    }
                    self.mixer.channels_mut().push(channel);
                    structural = true;
                }
                Err(e) => {
                    warnings.push(format!("Failed to create channel '{}': {}", ch_config.name, e));
                }
            }
        }

        // Update next_channel_index
        let max_idx = self.mixer.channels().iter()
            .filter_map(|ch| ch.name.strip_prefix("Ch ").and_then(|s| s.parse::<usize>().ok()))
            .max()
            .map(|n| n + 1)
            .unwrap_or(self.mixer.channels().len());
        self.mixer.set_next_channel_index(max_idx);

        // (e) Master effects — diff
        Self::diff_effects(self.mixer.master_effects_mut(), &target.master_effects,
            &self.context, self.context.texture_format, &mut warnings);

        // (f) Transition sequences — cheap clone
        self.mixer.transition_sequences_mut().clear();
        for seq_config in &target.transition_sequences {
            use crate::channel::DurationSpec;
            use crate::mixer::{TransitionSequence, TransitionStep, StepKind, SequencerState};
            use crate::scene::{TransitionStepConfig, DurationSpecConfig};
            let steps = seq_config.steps.iter().map(|step| {
                let kind = match step {
                    TransitionStepConfig::Fade { from_ch, to_ch, duration, easing, transition_shader } => {
                        StepKind::Fade {
                            from_ch: *from_ch, to_ch: *to_ch,
                            duration: match duration {
                                DurationSpecConfig::Beats(v) => DurationSpec::Beats(*v),
                                DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(*v),
                            },
                            easing: (*easing).into(),
                            transition_shader: transition_shader.clone(),
                        }
                    }
                    TransitionStepConfig::Wait { duration } => StepKind::Wait {
                        duration: match duration {
                            DurationSpecConfig::Beats(v) => DurationSpec::Beats(*v),
                            DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(*v),
                        },
                    },
                    TransitionStepConfig::GoTo { step_index } => StepKind::GoTo { step_index: *step_index },
                };
                TransitionStep { kind }
            }).collect();
            self.mixer.transition_sequences_mut().push(TransitionSequence {
                name: seq_config.name.clone(), steps, enabled: seq_config.enabled,
                state: SequencerState::new(),
            });
        }

        (warnings, structural)
    }

    /// Patch a DeckSlot's properties from config without rebuilding the source.
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
            slot.deck.generator_params.values = params.clone();
        }

        // Patch solid color
        if let crate::scene::SourceConfig::SolidColor { color } = &config.source {
            slot.deck.set_solid_color(*color);
        }

        // Patch deck effects
        Self::diff_effects(&mut slot.deck.effects, &config.effects, context,
            wgpu::TextureFormat::Rgba8Unorm, &mut Vec::new());

        // Patch auto-transition config
        if let Some(at_config) = &config.auto_transition {
            use crate::channel::{DeckAutoTransition, DurationSpec, TransitionTrigger};
            let mut at = slot.auto_transition.take().unwrap_or_else(DeckAutoTransition::new);
            at.enabled = at_config.enabled;
            at.trigger = match at_config.trigger {
                crate::scene::TriggerConfig::Timer => TransitionTrigger::Timer,
                crate::scene::TriggerConfig::ClipEnd => TransitionTrigger::ClipEnd,
            };
            at.play_duration = match at_config.play_duration {
                crate::scene::DurationSpecConfig::Beats(v) => DurationSpec::Beats(v),
                crate::scene::DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(v),
            };
            at.transition_duration = match at_config.transition_duration {
                crate::scene::DurationSpecConfig::Beats(v) => DurationSpec::Beats(v),
                crate::scene::DurationSpecConfig::Seconds(v) => DurationSpec::Seconds(v),
            };
            at.transition_shader_name = at_config.transition_shader.clone();
            slot.auto_transition = Some(at);

            // Compile transition shader if specified
            if let Some(shader_name) = &at_config.transition_shader {
                // Only recompile if the shader name changed
                let needs_compile = slot.transition_effect.as_ref()
                    .map(|te| te.shader.name() != *shader_name)
                    .unwrap_or(true);
                if needs_compile {
                    if let Some(shader) = registry.transitions().iter()
                        .find(|s| s.name() == *shader_name)
                    {
                        if let Err(e) = slot.set_transition_shader(context, (*shader).clone()) {
                            log::warn!("Failed to restore deck transition shader '{}': {}", shader_name, e);
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
                eff.params.values = cfg.params.clone();
            } else {
                // Different shader — rebuild this effect
                match crate::persistence::restore_effect(cfg, context, target_format) {
                    Ok(new_eff) => effects[i] = new_eff,
                    Err(e) => warnings.push(format!("Failed to restore effect '{}': {}", cfg.path, e)),
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
