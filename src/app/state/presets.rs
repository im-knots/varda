//! Deck/channel preset load + save commands.
//!
//! Preset operations are pure engine mutations (build decks, create channels,
//! read/write preset files) with no egui coupling: preset *loads* only ever
//! append decks or channels, so the GUI drain reacts to the returned
//! `CommandOutcome::DecksCreated` (or the per-frame `refresh_textures` pass)
//! to register the new previews — the handlers themselves never touch a texture.

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};

impl VardaApp {
    /// Load a deck preset by name as a new deck appended to the channel.
    pub(crate) fn cmd_load_deck_preset(
        &mut self,
        channel_uuid: &str,
        preset_name: &str,
    ) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        let Some(preset) = self
            .session
            .preset_library
            .deck_presets
            .iter()
            .find(|p| p.name == preset_name)
            .cloned()
        else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Deck preset '{preset_name}' not found"),
            };
        };
        match self.restore_deck_at(&preset.config, channel_idx, None, Identity::RestoreIfFree) {
            Ok(_) => {
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

    /// Load a channel preset by name. Fills `target_channel_uuid` when it is
    /// supplied and empty; otherwise appends a new channel.
    pub(crate) fn cmd_load_channel_preset(
        &mut self,
        target_channel_uuid: Option<&str>,
        preset_name: &str,
    ) -> CommandResult {
        let target_channel = match target_channel_uuid {
            Some(uuid) => match self.resolve_channel(uuid) {
                Ok(idx) => Some(idx),
                Err(e) => return e.into(),
            },
            None => None,
        };
        let Some(preset) = self
            .session
            .preset_library
            .channel_presets
            .iter()
            .find(|p| p.name == preset_name)
            .cloned()
        else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Channel preset '{preset_name}' not found"),
            };
        };

        // One identity pass over the whole tree before anything is built, so a
        // preset loaded twice does not put two entities behind one UUID, and so
        // its recipes name the effects that are about to exist.
        // See /spec/clipboard.md § Paste reidentifies.
        let mut config = preset.config.clone();
        let taken = self.mixer.uuids_in_use();
        crate::scene::reidentify::channel(&mut config, &|uuid| taken.contains(uuid));

        // Only fill into the target channel if it's empty (no decks); otherwise
        // create a new channel to avoid clobbering existing content.
        let use_existing = target_channel.and_then(|idx| {
            self.mixer
                .channel_mut(idx)
                .filter(|ch| ch.decks.is_empty())
                .map(|_| idx)
        });

        let resolved = if let Some(ch_idx) = use_existing {
            if let Some(channel) = self.mixer.channel_mut(ch_idx) {
                channel.opacity = config.opacity;
                channel.blend_mode = config.blend_mode.into();
            }
            Some((ch_idx, false))
        } else {
            let ch_name = self.mixer.take_next_channel_name();
            match crate::channel::Channel::new(
                ch_name,
                &self.context,
                self.render_width,
                self.render_height,
            ) {
                Ok(mut channel) => {
                    channel.opacity = config.opacity;
                    channel.blend_mode = config.blend_mode.into();
                    let idx = self.mixer.channels().len();
                    self.mixer.channels_mut().push(channel);
                    Some((idx, true))
                }
                Err(e) => {
                    log::error!("Failed to create channel for preset: {e}");
                    self.session
                        .notifications
                        .error(format!("Failed to load channel preset: {e}"));
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

        if !created_new {
            // The channel being filled has an effect chain of its own that the
            // preset must not append to, so only its decks come across.
            config.effects.clear();
            config.modulation.clear();
        }
        let had_errors = !self.fill_channel(ch_idx, &config);

        let target_desc = if created_new {
            "new channel".to_string()
        } else {
            format!("ch{ch_idx}")
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
    pub(crate) fn cmd_save_deck_preset(&mut self, deck_uuid: &str, name: &str) -> CommandResult {
        let (channel_idx, deck_idx) = match self.resolve_deck(deck_uuid) {
            Ok(loc) => loc,
            Err(e) => return e.into(),
        };
        let mixer = &mut self.mixer;
        let scene =
            crate::persistence::snapshot_scene(mixer, None, self.render_width, self.render_height);
        let Some(ch_config) = scene.channels.get(channel_idx) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Channel {channel_idx} not found"),
            };
        };
        let Some(deck_config) = ch_config.decks.get(deck_idx) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Deck {deck_idx} not found in channel {channel_idx}"),
            };
        };
        let mut preset_config = deck_config.clone();
        preset_config.name = name.to_string();
        let effect_uuids: Vec<String> = mixer
            .channel(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .map(|slot| {
                slot.deck
                    .effects
                    .iter()
                    .map(|e| e.uuid().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let prefix = format!("deck_{deck_uuid}");
        preset_config.modulation =
            extract_modulation_recipes(mixer.modulation(), Some(&prefix), &effect_uuids);
        match crate::persistence::presets::PresetLibrary::save_deck_preset(
            &self.session.workspace,
            name,
            &preset_config,
        ) {
            Ok(()) => {
                // Update the deck's display name to match the saved preset name.
                if let Some(ch) = mixer.channel_mut(channel_idx)
                    && let Some(slot) = ch.decks.get_mut(deck_idx)
                {
                    slot.deck.set_source_name(name.to_string());
                }
                self.session.preset_library.refresh(&self.session.workspace);
                self.session
                    .notifications
                    .info(format!("💾 Saved deck preset '{name}'"));
                CommandResult::Ok
            }
            Err(e) => {
                log::error!("Failed to save deck preset: {e}");
                self.session
                    .notifications
                    .error(format!("Failed to save preset: {e}"));
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
        channel_uuid: &str,
        name: &str,
    ) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        let Some(preset_ch_config) = self.channel_config(channel_idx) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Channel {channel_idx} not found"),
            };
        };
        match crate::persistence::presets::PresetLibrary::save_channel_preset(
            &self.session.workspace,
            name,
            &preset_ch_config,
        ) {
            Ok(()) => {
                self.session.preset_library.refresh(&self.session.workspace);
                self.session
                    .notifications
                    .info(format!("💾 Saved channel preset '{name}'"));
                CommandResult::Ok
            }
            Err(e) => {
                log::error!("Failed to save channel preset: {e}");
                self.session
                    .notifications
                    .error(format!("Failed to save preset: {e}"));
                CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        }
    }

    /// Restore a `DeckConfig` into a channel, at `at` or appended, and return
    /// the UUID it ended up with.
    ///
    /// Shared by preset loading and by paste, which differ only in identity:
    /// see [`Identity`].
    pub(crate) fn restore_deck_at(
        &mut self,
        config: &crate::scene::DeckConfig,
        ch_idx: usize,
        at: Option<usize>,
        identity: Identity,
    ) -> anyhow::Result<String> {
        Self::restore_deck_into_channel(
            config,
            ch_idx,
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
            &mut self.mixer,
            at,
            identity,
        )
    }

    /// Restore a single `DeckConfig` into an existing channel. Shared by
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
        screen_capture_manager: &mut crate::screen_capture::ScreenCaptureManager,
        depth_manager: &mut crate::depth::DepthSensorManager,
        ndi_manager: &mut crate::ndi::NdiManager,
        stream_manager: &mut crate::stream::StreamManager,
        html_manager: &mut crate::html::HtmlManager,
        render_width: u32,
        render_height: u32,
        mixer: &mut crate::mixer::Mixer,
        at: Option<usize>,
        identity: Identity,
    ) -> anyhow::Result<String> {
        // A config restores the identity it was saved with, which collides when
        // the thing it names is already on stage: loading one preset twice, or
        // loading it back into the scene it came from. See
        // /spec/clipboard.md § Paste reidentifies.
        let taken = mixer.uuids_in_use();
        let always = matches!(identity, Identity::Fresh);
        let mut config = config.clone();
        crate::scene::reidentify::deck(&mut config, &|uuid| always || taken.contains(uuid));
        let config = &config;

        let mut deck = crate::persistence::restore_deck(
            config,
            context,
            registry,
            camera_manager,
            screen_capture_manager,
            depth_manager,
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
                .ok_or_else(|| anyhow::anyhow!("Channel {ch_idx} not found"))?;
            let mut slot = crate::channel::DeckSlot::new(deck);
            slot.opacity = config.opacity;
            slot.blend_mode = config.blend_mode.into();
            slot.mute = config.mute;
            slot.solo = config.solo;
            slot.z_index = config.z_index;
            ch.add_deck_slot(slot);
            // Appended, then moved into place, so the slot goes through the one
            // insertion path the channel maintains.
            let appended = ch.decks.len() - 1;
            match at {
                Some(at) if at < appended => {
                    let slot = ch.decks.remove(appended);
                    ch.decks.insert(at, slot);
                    at
                }
                _ => appended,
            }
        };
        let deck_uuid = mixer
            .channel(ch_idx)
            .and_then(|ch| ch.decks.get(dk_idx))
            .map(|slot| slot.deck.uuid().to_string())
            .unwrap_or_default();
        // Apply modulation recipes with deduplication.
        if !config.modulation.is_empty() {
            let new_prefix = format!("deck_{deck_uuid}");
            apply_modulation_recipes(&config.modulation, &new_prefix, mixer.modulation_mut());
        }
        Ok(deck_uuid)
    }
}

/// Whether a restored config keeps the identity it was saved with.
///
/// A preset restores it when it is free, so the mappings that point at that
/// deck keep working; a paste never does, because a copy is a second entity.
/// See /spec/clipboard.md § Paste reidentifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Identity {
    RestoreIfFree,
    Fresh,
}

/// Extract modulation recipes for one entity from the global engine.
/// Scans all assignments matching the owner's prefix and effect UUIDs,
/// groups by source, and strips prefixes to make them portable.
///
/// `prefix` is absent when the entity has no params of its own, which is the
/// case for an effect and for a channel: both own only effect assignments.
pub(crate) fn extract_modulation_recipes(
    engine: &crate::modulation::ModulationEngine,
    prefix: Option<&str>,
    effect_uuids: &[String],
) -> Vec<crate::scene::ModulationRecipe> {
    let prefix_colon = prefix.map(|p| format!("{p}:"));
    let mut source_map: std::collections::HashMap<
        String,
        Vec<crate::scene::ModulationRecipeAssignment>,
    > = std::collections::HashMap::new();

    // Build a set of effect key prefixes for this deck's effects.
    let fx_prefixes: Vec<String> = effect_uuids.iter().map(|u| format!("fx_{u}:")).collect();

    for (key, mods) in engine.assignments_iter() {
        // Match generator params: "deck_{uuid}:brightness" → relative "brightness".
        let own_param = prefix_colon
            .as_ref()
            .and_then(|p| key.strip_prefix(p.as_str()));
        let relative_param = if let Some(rel) = own_param {
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
                    timebase: entry.timebase,
                    assignments,
                })
        })
        .collect()
}

/// Apply modulation recipes to the global engine for a newly loaded deck.
/// UUID-is-identity: if a source with the recipe's UUID exists, wire up to it.
/// Otherwise create a new source with that UUID.
pub(crate) fn apply_modulation_recipes(
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
            // The clock the source follows lives on the engine's entry, so a
            // recipe that did not carry it restored an arrangement curve as
            // free-running. See /spec/timebase.md.
            engine.set_timebase(&uuid, recipe.timebase);
            log::info!("Created new modulation source {uuid} for preset");
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
    use crate::engine::{EngineCommand as C, MixerQueries};
    use crate::modulation::{ModulationEngine, ModulationSource};
    use crate::scene::{ModulationRecipe, ModulationRecipeAssignment};

    /// A preset saved from a deck that is still on stage used to restore that
    /// deck's UUID onto a second one, so every command, modulation key, and MIDI
    /// path addressed at it hit whichever resolved first.
    /// See /spec/clipboard.md § Bug this fixes.
    #[test]
    fn loading_one_preset_twice_makes_two_decks() {
        let Some(gpu) = crate::renderer::context::GpuContext::new_headless().ok() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let Ok(mut app) = VardaApp::new(gpu, &crate::testing::headless_config()) else {
            return;
        };
        let channel_uuid = app.mixer_snapshot().channels[0].uuid.clone();
        let result = app.execute_command(C::AddSolidColorDeck {
            channel_uuid: channel_uuid.clone(),
            color: [0.0, 0.0, 1.0, 1.0],
        });
        let CommandResult::OkWithId { uuid: deck } = result else {
            panic!("no deck: {result:?}");
        };

        app.execute_command(C::SaveDeckPreset {
            deck_uuid: deck.clone(),
            name: "twice".into(),
        });
        app.execute_command(C::LoadDeckPreset {
            channel_uuid: channel_uuid.clone(),
            preset_name: "twice".into(),
        });
        app.execute_command(C::LoadDeckPreset {
            channel_uuid,
            preset_name: "twice".into(),
        });

        let uuids: Vec<String> = app.mixer_snapshot().channels[0]
            .decks
            .iter()
            .map(|d| d.uuid.clone())
            .collect();
        assert_eq!(uuids.len(), 3, "the original and two loads");
        let unique: std::collections::HashSet<&String> = uuids.iter().collect();
        assert_eq!(unique.len(), 3, "each deck answers to its own address");
    }

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
        let recipes = extract_modulation_recipes(&engine, Some("deck_abc12345"), &effect_uuids);
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
        params.sort_unstable();
        assert_eq!(params, vec!["brightness", "fx_effuuid1:amount"]);
    }

    #[test]
    fn apply_restores_generator_and_effect_keys() {
        let mut engine = ModulationEngine::new();
        let recipes = vec![ModulationRecipe {
            source_uuid: "test0001".to_string(),
            source: ModulationSource::sine_lfo(2.0),
            timebase: crate::timebase::Timebase::FreeRun,
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
        let recipes =
            extract_modulation_recipes(&save_engine, Some("deck_saveuuid"), &effect_uuids);

        // Simulate load: fresh engine, apply recipes into a different slot
        let mut load_engine = ModulationEngine::new();
        apply_modulation_recipes(&recipes, "deck_loaduuid", &mut load_engine);

        assert_eq!(load_engine.source_count(), 1);
        assert!(load_engine.has_modulation("deck_loaduuid:contrast"));
        assert!(load_engine.has_modulation("fx_fxuuid01:mix"));
    }
}
