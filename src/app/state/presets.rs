//! Deck/channel preset load + save commands.
//!
//! Preset operations are pure engine mutations (build decks, create channels,
//! read/write preset files) with no egui coupling: preset *loads* only ever
//! append decks or channels, so the GUI drain reacts to the returned
//! `CommandOutcome::DecksReindexed` (or the per-frame `refresh_textures` pass)
//! to register the new previews — the handlers themselves never touch a texture.

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};

impl VardaApp {
    /// Load a deck preset (by library index) as a new deck appended to `channel_idx`.
    pub(crate) fn cmd_load_deck_preset(
        &mut self,
        channel_idx: usize,
        preset_idx: usize,
    ) -> CommandResult {
        if preset_idx >= self.session.preset_library.deck_presets.len() {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Deck preset index {} out of range", preset_idx),
            };
        }
        let preset = self.session.preset_library.deck_presets[preset_idx].clone();
        match Self::restore_deck_into_channel(
            &preset.config,
            channel_idx,
            &self.context,
            &self.registry,
            &mut self.camera_manager,
            &mut self.external_io.ndi_manager,
            &mut self.external_io.stream_manager,
            &mut self.external_io.html_manager,
            self.render_width,
            self.render_height,
            &mut self.mixer,
        ) {
            Ok(()) => {
                self.session
                    .notifications
                    .info(format!("💾 Loaded deck preset '{}'", preset.name));
                CommandResult::Ok
            }
            Err(e) => {
                log::warn!("Failed to load deck preset '{}': {}", preset.name, e);
                self.session
                    .notifications
                    .warn(format!("Failed to load preset '{}': {}", preset.name, e));
                CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        }
    }

    /// Load a channel preset (by library index). Fills `target_channel` when it
    /// is supplied and empty; otherwise appends a new channel.
    pub(crate) fn cmd_load_channel_preset(
        &mut self,
        target_channel: Option<usize>,
        preset_idx: usize,
    ) -> CommandResult {
        if preset_idx >= self.session.preset_library.channel_presets.len() {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Channel preset index {} out of range", preset_idx),
            };
        }
        let preset = self.session.preset_library.channel_presets[preset_idx].clone();

        let context = &self.context;
        let mixer = &mut self.mixer;

        // Only fill into the target channel if it's empty (no decks); otherwise
        // create a new channel to avoid clobbering existing content.
        let use_existing = target_channel.and_then(|idx| {
            mixer
                .channel_mut(idx)
                .filter(|ch| ch.decks.is_empty())
                .map(|_| idx)
        });

        let resolved = if let Some(ch_idx) = use_existing {
            if let Some(channel) = mixer.channel_mut(ch_idx) {
                channel.opacity = preset.config.opacity;
                channel.blend_mode = preset.config.blend_mode.into();
            }
            Some((ch_idx, false))
        } else {
            let ch_name = mixer.take_next_channel_name();
            match crate::channel::Channel::new(
                ch_name,
                context,
                self.render_width,
                self.render_height,
            ) {
                Ok(mut channel) => {
                    channel.opacity = preset.config.opacity;
                    channel.blend_mode = preset.config.blend_mode.into();
                    let idx = mixer.channels().len();
                    mixer.channels_mut().push(channel);
                    Some((idx, true))
                }
                Err(e) => {
                    log::error!("Failed to create channel for preset: {}", e);
                    self.session
                        .notifications
                        .error(format!("Failed to load channel preset: {}", e));
                    return CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    };
                }
            }
        };

        let Some((ch_idx, created_new)) = resolved else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "No target channel for preset".into(),
            };
        };

        let mut had_errors = false;

        // Restore channel effects (only for new channels to avoid duplicating effects).
        if created_new {
            for eff_config in &preset.config.effects {
                match crate::persistence::restore_effect(
                    eff_config,
                    context,
                    context.compositing_format,
                ) {
                    Ok(eff) => {
                        if let Some(ch) = mixer.channel_mut(ch_idx) {
                            ch.add_effect(eff);
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to restore channel effect '{}': {}",
                            eff_config.path,
                            e
                        );
                        had_errors = true;
                    }
                }
            }
        }

        // Bulk-load decks into the channel.
        for deck_config in &preset.config.decks {
            if let Err(e) = Self::restore_deck_into_channel(
                deck_config,
                ch_idx,
                context,
                &self.registry,
                &mut self.camera_manager,
                &mut self.external_io.ndi_manager,
                &mut self.external_io.stream_manager,
                &mut self.external_io.html_manager,
                self.render_width,
                self.render_height,
                mixer,
            ) {
                log::warn!(
                    "Failed to restore deck '{}' in channel preset: {}",
                    deck_config.name,
                    e
                );
                had_errors = true;
            }
        }

        let target_desc = if created_new {
            "new channel".to_string()
        } else {
            format!("ch{}", ch_idx)
        };
        let msg = if had_errors {
            format!(
                "💾 Loaded channel preset '{}' into {} (with warnings)",
                preset.name, target_desc
            )
        } else {
            format!(
                "💾 Loaded channel preset '{}' into {}",
                preset.name, target_desc
            )
        };
        self.session.notifications.info(msg);
        CommandResult::Ok
    }

    /// Save a deck's current config as a named deck preset (writes to disk).
    pub(crate) fn cmd_save_deck_preset(
        &mut self,
        channel_idx: usize,
        deck_idx: usize,
        name: &str,
    ) -> CommandResult {
        let mixer = &mut self.mixer;
        let scene =
            crate::persistence::snapshot_scene(mixer, self.render_width, self.render_height);
        let Some(ch_config) = scene.channels.get(channel_idx) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Channel {} not found", channel_idx),
            };
        };
        let Some(deck_config) = ch_config.decks.get(deck_idx) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Deck {} not found in channel {}", deck_idx, channel_idx),
            };
        };
        let mut preset_config = deck_config.clone();
        preset_config.name = name.to_string();
        let deck_uuid = mixer
            .channel(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .map(|slot| slot.deck.uuid().to_string())
            .unwrap_or_default();
        let effect_uuids: Vec<String> = mixer
            .channel(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .map(|slot| slot.deck.effects.iter().map(|e| e.uuid.clone()).collect())
            .unwrap_or_default();
        let prefix = format!("deck_{}", deck_uuid);
        preset_config.modulation =
            extract_modulation_recipes(mixer.modulation(), &prefix, &effect_uuids);
        match crate::persistence::presets::PresetLibrary::save_deck_preset(
            &self.session.workspace,
            name,
            &preset_config,
        ) {
            Ok(()) => {
                // Update the deck's display name to match the saved preset name.
                if let Some(ch) = mixer.channel_mut(channel_idx) {
                    if let Some(slot) = ch.decks.get_mut(deck_idx) {
                        slot.deck.set_source_name(name.to_string());
                    }
                }
                self.session.preset_library.refresh(&self.session.workspace);
                self.session
                    .notifications
                    .info(format!("💾 Saved deck preset '{}'", name));
                CommandResult::Ok
            }
            Err(e) => {
                log::error!("Failed to save deck preset: {}", e);
                self.session
                    .notifications
                    .error(format!("Failed to save preset: {}", e));
                CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        }
    }

    /// Save a channel's current config as a named channel preset (writes to disk).
    pub(crate) fn cmd_save_channel_preset(
        &mut self,
        channel_idx: usize,
        name: &str,
    ) -> CommandResult {
        let mixer = &mut self.mixer;
        let scene =
            crate::persistence::snapshot_scene(mixer, self.render_width, self.render_height);
        let Some(ch_config) = scene.channels.get(channel_idx) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Channel {} not found", channel_idx),
            };
        };
        let mut preset_ch_config = ch_config.clone();
        for (deck_idx, deck_config) in preset_ch_config.decks.iter_mut().enumerate() {
            let deck_uuid = mixer
                .channel(channel_idx)
                .and_then(|ch| ch.decks.get(deck_idx))
                .map(|slot| slot.deck.uuid().to_string())
                .unwrap_or_default();
            let effect_uuids: Vec<String> = mixer
                .channel(channel_idx)
                .and_then(|ch| ch.decks.get(deck_idx))
                .map(|slot| slot.deck.effects.iter().map(|e| e.uuid.clone()).collect())
                .unwrap_or_default();
            let prefix = format!("deck_{}", deck_uuid);
            deck_config.modulation =
                extract_modulation_recipes(mixer.modulation(), &prefix, &effect_uuids);
        }
        match crate::persistence::presets::PresetLibrary::save_channel_preset(
            &self.session.workspace,
            name,
            &preset_ch_config,
        ) {
            Ok(()) => {
                self.session.preset_library.refresh(&self.session.workspace);
                self.session
                    .notifications
                    .info(format!("💾 Saved channel preset '{}'", name));
                CommandResult::Ok
            }
            Err(e) => {
                log::error!("Failed to save channel preset: {}", e);
                self.session
                    .notifications
                    .error(format!("Failed to save preset: {}", e));
                CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        }
    }

    /// Restore a single `DeckConfig` into an existing channel (append). Shared by
    /// deck-preset loading and channel-preset bulk-loading. Pure engine: no egui
    /// texture registration (the GUI drain handles previews via the command
    /// outcome / the per-frame refresh).
    #[allow(clippy::too_many_arguments)]
    fn restore_deck_into_channel(
        config: &crate::scene::DeckConfig,
        ch_idx: usize,
        context: &crate::renderer::GpuContext,
        registry: &crate::registry::ShaderRegistry,
        camera_manager: &mut crate::camera::CameraManager,
        ndi_manager: &mut crate::ndi::NdiManager,
        stream_manager: &mut crate::stream::StreamManager,
        html_manager: &mut crate::html::HtmlManager,
        render_width: u32,
        render_height: u32,
        mixer: &mut crate::mixer::Mixer,
    ) -> anyhow::Result<()> {
        let mut deck = crate::persistence::restore_deck(
            config,
            context,
            registry,
            camera_manager,
            ndi_manager,
            stream_manager,
            html_manager,
            render_width,
            render_height,
        )?;
        // Apply the preset's display name (overrides the generator/source name).
        if !config.name.is_empty() {
            deck.set_source_name(config.name.clone());
        }
        let dk_idx = {
            let ch = mixer
                .channel_mut(ch_idx)
                .ok_or_else(|| anyhow::anyhow!("Channel {} not found", ch_idx))?;
            let mut slot = crate::channel::DeckSlot::new(deck);
            slot.opacity = config.opacity;
            slot.blend_mode = config.blend_mode.into();
            slot.mute = config.mute;
            slot.solo = config.solo;
            slot.z_index = config.z_index;
            let dk_idx = ch.decks.len();
            ch.add_deck_slot(slot);
            dk_idx
        };
        // Apply modulation recipes with deduplication.
        if !config.modulation.is_empty() {
            let deck_uuid = mixer
                .channel(ch_idx)
                .and_then(|ch| ch.decks.get(dk_idx))
                .map(|slot| slot.deck.uuid().to_string())
                .unwrap_or_default();
            let new_prefix = format!("deck_{}", deck_uuid);
            apply_modulation_recipes(&config.modulation, &new_prefix, mixer.modulation_mut());
        }
        Ok(())
    }
}

/// Extract modulation recipes for a specific deck from the global engine.
/// Scans all assignments matching the deck's prefix and effect UUIDs,
/// groups by source, and strips prefixes to make them portable.
fn extract_modulation_recipes(
    engine: &crate::modulation::ModulationEngine,
    prefix: &str,
    effect_uuids: &[String],
) -> Vec<crate::scene::ModulationRecipe> {
    let prefix_colon = format!("{}:", prefix);
    let mut source_map: std::collections::HashMap<
        String,
        Vec<crate::scene::ModulationRecipeAssignment>,
    > = std::collections::HashMap::new();

    // Build a set of effect key prefixes for this deck's effects.
    let fx_prefixes: Vec<String> = effect_uuids.iter().map(|u| format!("fx_{}:", u)).collect();

    for (key, mods) in engine.assignments_iter() {
        // Match generator params: "deck_{uuid}:brightness" → relative "brightness".
        let relative_param = if let Some(rel) = key.strip_prefix(&prefix_colon) {
            Some(rel.to_string())
        } else {
            // Match effect params: "fx_{fx_uuid}:param" → store the full effect key
            // as-is so it can be re-applied with the same UUID.
            fx_prefixes
                .iter()
                .find(|p| key.starts_with(p.as_str()))
                .map(|_| key.clone())
        };

        if let Some(relative_param) = relative_param {
            for m in mods {
                source_map.entry(m.source_id.clone()).or_default().push(
                    crate::scene::ModulationRecipeAssignment {
                        param: relative_param.clone(),
                        amount: m.amount,
                        component: m.component,
                    },
                );
            }
        }
    }

    source_map
        .into_iter()
        .filter_map(|(source_uuid, assignments)| {
            engine
                .find_source_by_uuid(&source_uuid)
                .map(|entry| crate::scene::ModulationRecipe {
                    source_uuid: entry.uuid.clone(),
                    source: entry.source.clone(),
                    assignments,
                })
        })
        .collect()
}

/// Apply modulation recipes to the global engine for a newly loaded deck.
/// UUID-is-identity: if a source with the recipe's UUID exists, wire up to it.
/// Otherwise create a new source with that UUID.
fn apply_modulation_recipes(
    recipes: &[crate::scene::ModulationRecipe],
    prefix: &str,
    engine: &mut crate::modulation::ModulationEngine,
) {
    for recipe in recipes {
        let source_uuid = if engine.has_source(&recipe.source_uuid) {
            recipe.source_uuid.clone()
        } else {
            let uuid =
                engine.add_source_with_uuid(recipe.source_uuid.clone(), recipe.source.clone());
            log::info!("Created new modulation source {} for preset", uuid);
            uuid
        };
        for assignment in &recipe.assignments {
            // Effect params stored as "fx_{uuid}:param" (already fully qualified).
            // Generator params stored as "brightness" → key "deck_{uuid}:brightness".
            let full_key = if assignment.param.starts_with("fx_") {
                assignment.param.clone()
            } else {
                format!("{}:{}", prefix, assignment.param)
            };
            engine.assign(
                &full_key,
                &source_uuid,
                assignment.amount,
                assignment.component,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulation::{ModulationEngine, ModulationSource};
    use crate::scene::{ModulationRecipe, ModulationRecipeAssignment};

    #[test]
    fn extract_captures_generator_and_effect_params() {
        let mut engine = ModulationEngine::new();
        let src_uuid = engine.add_source(ModulationSource::sine_lfo(2.0));
        // Generator param: deck_abc12345:brightness
        engine.assign("deck_abc12345:brightness", &src_uuid, 0.5, None);
        // Effect param: fx_effuuid1:amount (new format uses effect UUID)
        engine.assign("fx_effuuid1:amount", &src_uuid, 0.3, None);
        // Unrelated key from another deck — should NOT be captured
        engine.assign("deck_def67890:brightness", &src_uuid, 1.0, None);

        let effect_uuids = vec!["effuuid1".to_string()];
        let recipes = extract_modulation_recipes(&engine, "deck_abc12345", &effect_uuids);
        assert_eq!(
            recipes.len(),
            1,
            "should group into one recipe (one source)"
        );
        let recipe = &recipes[0];
        let mut params: Vec<&str> = recipe
            .assignments
            .iter()
            .map(|a| a.param.as_str())
            .collect();
        params.sort();
        assert_eq!(params, vec!["brightness", "fx_effuuid1:amount"]);
    }

    #[test]
    fn apply_restores_generator_and_effect_keys() {
        let mut engine = ModulationEngine::new();
        let recipes = vec![ModulationRecipe {
            source_uuid: "test0001".to_string(),
            source: ModulationSource::sine_lfo(2.0),
            assignments: vec![
                ModulationRecipeAssignment {
                    param: "brightness".into(),
                    amount: 0.5,
                    component: None,
                },
                ModulationRecipeAssignment {
                    param: "fx_effuuid1:amount".into(),
                    amount: 0.3,
                    component: None,
                },
            ],
        }];

        apply_modulation_recipes(&recipes, "deck_newuuid1", &mut engine);

        assert_eq!(engine.source_count(), 1);
        assert!(
            engine.has_modulation("deck_newuuid1:brightness"),
            "generator key missing"
        );
        assert!(
            engine.has_modulation("fx_effuuid1:amount"),
            "effect key missing"
        );
    }

    #[test]
    fn roundtrip_extract_then_apply_preserves_effect_modulation() {
        // Simulate save: create engine with assignments, extract recipes
        let mut save_engine = ModulationEngine::new();
        let src_uuid = save_engine.add_source(ModulationSource::sine_lfo(3.0));
        save_engine.assign("deck_saveuuid:contrast", &src_uuid, 0.7, None);
        save_engine.assign("fx_fxuuid01:mix", &src_uuid, 0.4, None);

        let effect_uuids = vec!["fxuuid01".to_string()];
        let recipes = extract_modulation_recipes(&save_engine, "deck_saveuuid", &effect_uuids);

        // Simulate load: fresh engine, apply recipes into a different slot
        let mut load_engine = ModulationEngine::new();
        apply_modulation_recipes(&recipes, "deck_loaduuid", &mut load_engine);

        assert_eq!(load_engine.source_count(), 1);
        assert!(load_engine.has_modulation("deck_loaduuid:contrast"));
        assert!(load_engine.has_modulation("fx_fxuuid01:mix"));
    }
}
