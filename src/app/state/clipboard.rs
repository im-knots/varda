//! Copy and paste of scene objects.
//!
//! A copy is not a clone: a live deck owns GPU textures, a decode thread, and
//! sometimes a device, none of which can be shared. What is captured is the
//! recipe (the same `DeckConfig` / `ChannelConfig` / `EffectConfig` a preset
//! saves), and paste rebuilds from it through the same restore path, with every
//! UUID reidentified so the copy is a second entity rather than a second name
//! for the first. See /spec/clipboard.md.

use super::presets::{apply_modulation_recipes, extract_modulation_recipes, Identity};
use crate::app::resolve::EffectChain;
use crate::app::VardaApp;
use crate::arrangement::RegionConfig;
use crate::engine::{
    ClipboardKind, ClipboardSource, ClipboardSummary, CommandResult, ErrorCode, MixerCommands,
    PasteTarget,
};
use crate::scene::{ChannelConfig, DeckConfig, EffectConfig, ModulationRecipe};

/// What the clipboard holds. Configs, never live objects.
#[derive(Debug, Clone)]
pub(crate) enum ClipboardPayload {
    Effect {
        config: Box<EffectConfig>,
        modulation: Vec<ModulationRecipe>,
    },
    /// Regions ride alongside the config because they belong to the
    /// arrangement, not the deck, and are captured only when the copy was made
    /// on the timeline.
    Deck {
        config: Box<DeckConfig>,
        regions: Vec<RegionConfig>,
    },
    Channel(Box<ChannelConfig>),
}

impl ClipboardPayload {
    pub(crate) fn summary(&self) -> ClipboardSummary {
        match self {
            Self::Effect { config, .. } => ClipboardSummary {
                kind: ClipboardKind::Effect,
                label: effect_label(&config.path),
            },
            Self::Deck { config, .. } => ClipboardSummary {
                kind: ClipboardKind::Deck,
                label: config.name.clone(),
            },
            Self::Channel(config) => ClipboardSummary {
                kind: ClipboardKind::Channel,
                label: config.name.clone(),
            },
        }
    }
}

/// `shaders/rgb_glitch.fs` reads as `rgb_glitch` in a menu.
fn effect_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_string(), |stem| stem.to_string_lossy().into())
}

impl VardaApp {
    pub(crate) fn cmd_copy(
        &mut self,
        source: &ClipboardSource,
        include_arrangement: bool,
    ) -> CommandResult {
        match self.capture(source, include_arrangement) {
            Ok(payload) => {
                let summary = payload.summary();
                self.session.clipboard = Some(payload);
                self.session
                    .notifications
                    .info(format!("Copied {}", summary.label));
                CommandResult::Ok
            }
            Err(result) => result,
        }
    }

    pub(crate) fn cmd_paste(&mut self, target: &PasteTarget) -> CommandResult {
        let Some(payload) = self.session.clipboard.clone() else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Clipboard is empty".to_string(),
            };
        };
        self.place(payload, target)
    }

    /// Copy and paste beside the original in one step, leaving the clipboard
    /// alone: `Cmd+D` must not destroy what was copied a minute ago.
    pub(crate) fn cmd_duplicate(&mut self, source: &ClipboardSource) -> CommandResult {
        // A duplicate is the object as it stands, which on the timeline includes
        // where it plays.
        let payload = match self.capture(source, true) {
            Ok(payload) => payload,
            Err(result) => return result,
        };
        let target = match source {
            ClipboardSource::Deck(uuid) => PasteTarget::AfterDeck(uuid.clone()),
            ClipboardSource::Effect(uuid) => PasteTarget::AfterEffect(uuid.clone()),
            ClipboardSource::Channel(_) => PasteTarget::NewChannel,
        };
        self.place(payload, &target)
    }

    pub(crate) fn clipboard_summary(&self) -> Option<ClipboardSummary> {
        self.session
            .clipboard
            .as_ref()
            .map(ClipboardPayload::summary)
    }

    // ── Capture ──────────────────────────────────────────────────────

    fn capture(
        &self,
        source: &ClipboardSource,
        include_arrangement: bool,
    ) -> Result<ClipboardPayload, CommandResult> {
        match source {
            ClipboardSource::Deck(uuid) => self.capture_deck(uuid, include_arrangement),
            ClipboardSource::Channel(uuid) => self.capture_channel(uuid),
            ClipboardSource::Effect(uuid) => self.capture_effect(uuid),
        }
    }

    fn capture_deck(
        &self,
        deck_uuid: &str,
        include_arrangement: bool,
    ) -> Result<ClipboardPayload, CommandResult> {
        let (channel_idx, deck_idx) = self.resolve_deck(deck_uuid).map_err(CommandResult::from)?;
        let scene = crate::persistence::snapshot_scene(
            &self.mixer,
            None,
            self.render_width,
            self.render_height,
        );
        let mut config = scene
            .channels
            .get(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .cloned()
            .ok_or_else(|| missing("deck", deck_uuid))?;
        config.modulation = self.deck_recipes(channel_idx, deck_idx, deck_uuid);

        let regions = if include_arrangement {
            self.mixer
                .arrangement()
                .and_then(|arrangement| arrangement.lane(deck_uuid))
                .map(|lane| lane.regions.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(ClipboardPayload::Deck {
            config: Box::new(config),
            regions,
        })
    }

    fn capture_channel(&self, channel_uuid: &str) -> Result<ClipboardPayload, CommandResult> {
        let channel_idx = self
            .resolve_channel(channel_uuid)
            .map_err(CommandResult::from)?;
        let config = self
            .channel_config(channel_idx)
            .ok_or_else(|| missing("channel", channel_uuid))?;
        Ok(ClipboardPayload::Channel(Box::new(config)))
    }

    /// A channel as a portable config: the scene's own snapshot, plus the
    /// modulation recipes that a scene keeps in the engine instead.
    ///
    /// Shared with channel-preset saving, which is where the gap showed: only
    /// decks carried recipes, so a channel's own effects arrived unmodulated.
    pub(crate) fn channel_config(&self, channel_idx: usize) -> Option<ChannelConfig> {
        let scene = crate::persistence::snapshot_scene(
            &self.mixer,
            None,
            self.render_width,
            self.render_height,
        );
        let mut config = scene.channels.get(channel_idx).cloned()?;

        for (deck_idx, deck_config) in config.decks.iter_mut().enumerate() {
            let deck_uuid = deck_config.uuid.clone();
            deck_config.modulation = self.deck_recipes(channel_idx, deck_idx, &deck_uuid);
        }
        let effect_uuids: Vec<String> = config.effects.iter().map(|e| e.uuid.clone()).collect();
        config.modulation =
            extract_modulation_recipes(self.mixer.modulation(), None, &effect_uuids);
        Some(config)
    }

    fn capture_effect(&self, effect_uuid: &str) -> Result<ClipboardPayload, CommandResult> {
        let location = self
            .resolve_effect(effect_uuid)
            .map_err(CommandResult::from)?;
        let effect = self
            .mixer
            .effect_at(location)
            .ok_or_else(|| missing("effect", effect_uuid))?;
        let config = EffectConfig {
            uuid: effect.uuid().to_owned(),
            path: effect.shader.file_path.clone().unwrap_or_default(),
            enabled: effect.enabled,
            params: effect.params.values.clone(),
        };
        let modulation = extract_modulation_recipes(
            self.mixer.modulation(),
            None,
            std::slice::from_ref(&config.uuid),
        );
        Ok(ClipboardPayload::Effect {
            config: Box::new(config),
            modulation,
        })
    }

    fn deck_recipes(
        &self,
        channel_idx: usize,
        deck_idx: usize,
        deck_uuid: &str,
    ) -> Vec<ModulationRecipe> {
        let effect_uuids: Vec<String> = self
            .mixer
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
        extract_modulation_recipes(
            self.mixer.modulation(),
            Some(&format!("deck_{deck_uuid}")),
            &effect_uuids,
        )
    }

    // ── Place ────────────────────────────────────────────────────────

    fn place(&mut self, payload: ClipboardPayload, target: &PasteTarget) -> CommandResult {
        match payload {
            ClipboardPayload::Deck { config, regions } => {
                self.place_deck(&config, &regions, target)
            }
            ClipboardPayload::Effect { config, modulation } => {
                self.place_effect(*config, modulation, target)
            }
            ClipboardPayload::Channel(config) => self.place_channel(*config, target),
        }
    }

    fn place_deck(
        &mut self,
        config: &DeckConfig,
        regions: &[RegionConfig],
        target: &PasteTarget,
    ) -> CommandResult {
        let (channel_idx, at) = match target {
            PasteTarget::AfterDeck(uuid) => match self.resolve_deck(uuid) {
                Ok((channel_idx, deck_idx)) => (channel_idx, Some(deck_idx + 1)),
                Err(e) => return e.into(),
            },
            PasteTarget::IntoChannel(uuid) => match self.resolve_channel(uuid) {
                Ok(channel_idx) => (channel_idx, None),
                Err(e) => return e.into(),
            },
            _ => return wrong_target("A deck lands in a channel"),
        };

        match self.restore_deck_at(config, channel_idx, at, Identity::Fresh) {
            Ok(deck_uuid) => {
                // Through the region command, so the lane is created and the
                // opacity envelope recompiled the one way they ever are.
                for region in regions {
                    self.cmd_add_region(&deck_uuid, *region);
                }
                CommandResult::OkWithId { uuid: deck_uuid }
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::InternalError,
                message: e.to_string(),
            },
        }
    }

    fn place_effect(
        &mut self,
        config: EffectConfig,
        modulation: Vec<ModulationRecipe>,
        target: &PasteTarget,
    ) -> CommandResult {
        let (chain, at) = match target {
            PasteTarget::AfterEffect(uuid) => match self.resolve_effect(uuid) {
                Ok(location) => (chain_of(location), Some(index_of(location) + 1)),
                Err(e) => return e.into(),
            },
            PasteTarget::IntoChain(effect_target) => {
                match self.resolve_effect_target(effect_target) {
                    Ok(chain) => (chain, None),
                    Err(e) => return e.into(),
                }
            }
            _ => return wrong_target("An effect lands in an effect chain"),
        };

        let mut config = config;
        let mut modulation = modulation;
        let taken = self.mixer.uuids_in_use();
        crate::scene::reidentify::effect(&mut config, &mut modulation, &|uuid| {
            taken.contains(uuid)
        });

        let format = self.context.compositing_format;
        let effect = match crate::persistence::restore_effect(&config, &self.context, format) {
            Ok(effect) => effect,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                }
            }
        };
        let uuid = effect.uuid().to_owned();

        let Some(chain_effects) = self.effects_of_mut(chain) else {
            return missing("effect chain", "target");
        };
        let at = at.unwrap_or(chain_effects.len()).min(chain_effects.len());
        chain_effects.insert(at, effect);

        apply_modulation_recipes(&modulation, "", self.mixer.modulation_mut());
        CommandResult::OkWithId { uuid }
    }

    fn place_channel(&mut self, config: ChannelConfig, target: &PasteTarget) -> CommandResult {
        if !matches!(target, PasteTarget::NewChannel) {
            return wrong_target("A channel lands in the mixer as a new channel");
        }
        // One reidentification for the whole tree, before anything is built, so
        // the recipes it carries already name the effects that are about to
        // exist. The restores below then find every UUID free.
        let mut config = config;
        let taken = self.mixer.uuids_in_use();
        crate::scene::reidentify::channel(&mut config, &|uuid| taken.contains(uuid));

        let uuid = match self.add_channel() {
            Ok(uuid) => uuid,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                }
            }
        };
        let Ok(channel_idx) = self.resolve_channel(&uuid) else {
            return missing("channel", &uuid);
        };
        if let Some(channel) = self.mixer.channel_mut(channel_idx) {
            channel.name.clone_from(&config.name);
            channel.opacity = config.opacity;
            channel.blend_mode = config.blend_mode.into();
        }

        self.fill_channel(channel_idx, &config);
        CommandResult::OkWithId { uuid }
    }

    /// Build a channel's decks, effects, and modulation from a config whose
    /// UUIDs are already free. Shared with channel-preset loading.
    pub(crate) fn fill_channel(&mut self, channel_idx: usize, config: &ChannelConfig) -> bool {
        let mut ok = true;
        for deck_config in &config.decks {
            if let Err(e) =
                self.restore_deck_at(deck_config, channel_idx, None, Identity::RestoreIfFree)
            {
                log::warn!("Restoring deck '{}' failed: {e}", deck_config.name);
                self.session.notifications.warn(format!(
                    "Could not restore deck '{}': {e}",
                    deck_config.name
                ));
                ok = false;
            }
        }

        let format = self.context.compositing_format;
        for effect_config in &config.effects {
            match crate::persistence::restore_effect(effect_config, &self.context, format) {
                Ok(effect) => {
                    if let Some(channel) = self.mixer.channel_mut(channel_idx) {
                        channel.add_effect(effect);
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Restoring channel effect '{}' failed: {e}",
                        effect_config.path
                    );
                    ok = false;
                }
            }
        }
        // After the effects exist, since the recipes name them.
        if !config.modulation.is_empty() {
            apply_modulation_recipes(&config.modulation, "", self.mixer.modulation_mut());
        }
        ok
    }

    fn effects_of_mut(&mut self, chain: EffectChain) -> Option<&mut Vec<crate::deck::Effect>> {
        match chain {
            EffectChain::Deck {
                channel_idx,
                deck_idx,
            } => self
                .mixer
                .channel_mut(channel_idx)?
                .decks
                .get_mut(deck_idx)
                .map(|slot| &mut slot.deck.effects),
            EffectChain::Channel { channel_idx } => {
                Some(&mut self.mixer.channel_mut(channel_idx)?.effects)
            }
            EffectChain::Master => Some(self.mixer.master_effects_mut()),
        }
    }
}

fn chain_of(location: crate::mixer::EffectLocation) -> EffectChain {
    match location {
        crate::mixer::EffectLocation::Deck {
            channel_idx,
            deck_idx,
            ..
        } => EffectChain::Deck {
            channel_idx,
            deck_idx,
        },
        crate::mixer::EffectLocation::Channel { channel_idx, .. } => {
            EffectChain::Channel { channel_idx }
        }
        crate::mixer::EffectLocation::Master { .. } => EffectChain::Master,
    }
}

fn index_of(location: crate::mixer::EffectLocation) -> usize {
    match location {
        crate::mixer::EffectLocation::Deck { effect_idx, .. }
        | crate::mixer::EffectLocation::Channel { effect_idx, .. }
        | crate::mixer::EffectLocation::Master { effect_idx } => effect_idx,
    }
}

fn missing(kind: &str, uuid: &str) -> CommandResult {
    CommandResult::Err {
        code: ErrorCode::NotFound,
        message: format!("Unknown {kind} '{uuid}'"),
    }
}

fn wrong_target(expected: &str) -> CommandResult {
    CommandResult::Err {
        code: ErrorCode::InvalidInput,
        message: format!("{expected}, so it cannot be pasted there"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{EffectTarget, EngineCommand as C, MixerQueries};

    fn headless_app() -> Option<VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        VardaApp::new(gpu, &crate::testing::headless_config()).ok()
    }

    /// A blue deck in channel 0, returning its UUID.
    fn a_deck(app: &mut VardaApp) -> String {
        let channel_uuid = app.mixer_snapshot().channels[0].uuid.clone();
        let result = app.execute_command(C::AddSolidColorDeck {
            channel_uuid,
            color: [0.0, 0.0, 1.0, 1.0],
        });
        let CommandResult::OkWithId { uuid } = result else {
            panic!("deck was not created: {result:?}");
        };
        uuid
    }

    fn deck_uuids(app: &VardaApp, channel: usize) -> Vec<String> {
        app.mixer_snapshot().channels[channel]
            .decks
            .iter()
            .map(|d| d.uuid.clone())
            .collect()
    }

    fn envelope_count(app: &VardaApp) -> usize {
        app.mixer_ref()
            .modulation()
            .sources
            .iter()
            .filter(|entry| entry.source.is_envelope())
            .count()
    }

    #[test]
    fn a_pasted_deck_is_a_second_entity() {
        let Some(mut app) = headless_app() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let deck = a_deck(&mut app);
        let channel = app.mixer_snapshot().channels[0].uuid.clone();

        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(deck.clone()),
            include_arrangement: false,
        });
        app.execute_command(C::Paste {
            target: PasteTarget::IntoChannel(channel),
        });

        let uuids = deck_uuids(&app, 0);
        assert_eq!(uuids.len(), 2);
        assert_ne!(
            uuids[0], uuids[1],
            "a copy answers to its own address, or every command hits the first one"
        );
    }

    /// Copying a deck to build up a second channel is the reason the mixer's
    /// menu exists, so the target channel is usually not the source's.
    #[test]
    fn a_deck_pastes_into_a_channel_it_did_not_come_from() {
        let Some(mut app) = headless_app() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let deck = a_deck(&mut app);
        let CommandResult::OkWithId { uuid: elsewhere } = app.execute_command(C::AddChannel) else {
            panic!("a second channel was not created");
        };

        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(deck.clone()),
            include_arrangement: false,
        });
        let result = app.execute_command(C::Paste {
            target: PasteTarget::IntoChannel(elsewhere.clone()),
        });

        let CommandResult::OkWithId { uuid: copy } = result else {
            panic!("pasting into another channel failed: {result:?}");
        };
        assert_eq!(deck_uuids(&app, 0), vec![deck], "the original stays put");
        let target = app
            .mixer_snapshot()
            .channels
            .iter()
            .position(|ch| ch.uuid == elsewhere)
            .expect("the channel that was pasted into");
        assert_eq!(
            deck_uuids(&app, target),
            vec![copy],
            "the copy belongs to the channel it was pasted into"
        );
    }

    #[test]
    fn a_pasted_deck_lands_below_the_one_it_was_taken_from() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let first = a_deck(&mut app);
        let second = a_deck(&mut app);

        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(first.clone()),
            include_arrangement: false,
        });
        app.execute_command(C::Paste {
            target: PasteTarget::AfterDeck(first.clone()),
        });

        let uuids = deck_uuids(&app, 0);
        assert_eq!(uuids.len(), 3);
        assert_eq!(uuids[0], first);
        assert_eq!(
            uuids[2], second,
            "the copy went between them, not at the end"
        );
    }

    #[test]
    fn an_empty_clipboard_refuses_to_paste() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel = app.mixer_snapshot().channels[0].uuid.clone();
        let result = app.execute_command(C::Paste {
            target: PasteTarget::IntoChannel(channel),
        });
        assert!(matches!(
            result,
            CommandResult::Err {
                code: ErrorCode::NotFound,
                ..
            }
        ));
    }

    /// A menu can outlive the thing it was opened on: delete the deck, then
    /// press the copy that was already on screen. Nothing should be captured,
    /// and whatever was on the clipboard before must survive the miss.
    #[test]
    fn copying_something_that_is_gone_leaves_the_clipboard_alone() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(deck),
            include_arrangement: false,
        });
        let held = app.clipboard_summary().map(|s| s.label);
        assert!(held.is_some());

        for source in [
            ClipboardSource::Deck("nosuch01".into()),
            ClipboardSource::Channel("nosuch02".into()),
            ClipboardSource::Effect("nosuch03".into()),
        ] {
            let result = app.execute_command(C::Copy {
                source,
                include_arrangement: false,
            });
            assert!(
                matches!(
                    result,
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        ..
                    }
                ),
                "{result:?}"
            );
        }

        assert_eq!(
            app.clipboard_summary().map(|s| s.label),
            held,
            "a failed copy must not empty the clipboard"
        );
    }

    #[test]
    fn a_deck_cannot_be_pasted_into_an_effect_chain() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(deck),
            include_arrangement: false,
        });
        let result = app.execute_command(C::Paste {
            target: PasteTarget::IntoChain(EffectTarget::Master),
        });
        assert!(
            matches!(
                result,
                CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    ..
                }
            ),
            "refused before anything was built: {result:?}"
        );
        assert_eq!(app.mixer_ref().master_effects().len(), 0);
    }

    /// One LFO, two decks moving together. That is what a performer means by
    /// copying something that is being modulated.
    #[test]
    fn a_live_modulator_drives_both_copies() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        app.execute_command(C::AddLfo {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0,
        });
        let lfo = app.mixer_ref().modulation().sources[0].uuid.clone();
        app.execute_command(C::AssignModulation {
            target: format!("deck_{deck}:opacity"),
            source_id: lfo.clone(),
            amount: 0.5,
        });

        app.execute_command(C::Duplicate {
            source: ClipboardSource::Deck(deck.clone()),
        });

        let uuids = deck_uuids(&app, 0);
        let copy = uuids.iter().find(|u| **u != deck).expect("a second deck");
        let modulation = app.mixer_ref().modulation();
        assert!(
            modulation.has_source(&lfo),
            "the LFO is shared, not duplicated"
        );
        assert_eq!(
            modulation.sources.len(),
            1,
            "no second LFO was created for the copy"
        );
        assert!(
            modulation
                .assignments_iter()
                .any(|(key, _)| key == &format!("deck_{copy}:opacity")),
            "the copy is modulated too"
        );
    }

    /// A curve belongs to one parameter, so the copy gets its own.
    #[test]
    fn a_curve_is_cloned_for_the_copy() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        app.execute_command(C::AddAutomationLane {
            target: format!("deck_{deck}:opacity"),
            timebase: crate::timebase::Timebase::Transport,
        });
        assert_eq!(envelope_count(&app), 1);

        app.execute_command(C::Duplicate {
            source: ClipboardSource::Deck(deck),
        });

        assert_eq!(
            envelope_count(&app),
            2,
            "editing the copy's lane must not rewrite the original's"
        );
    }

    #[test]
    fn regions_travel_only_when_the_copy_was_made_on_the_timeline() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        let channel = app.mixer_snapshot().channels[0].uuid.clone();
        app.execute_command(C::AddRegion {
            deck_uuid: deck.clone(),
            region: RegionConfig::new(0.0, 4.0),
        });

        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(deck.clone()),
            include_arrangement: false,
        });
        app.execute_command(C::Paste {
            target: PasteTarget::IntoChannel(channel.clone()),
        });
        let bare = deck_uuids(&app, 0)[1].clone();
        assert!(
            app.mixer_ref()
                .arrangement()
                .and_then(|a| a.lane(&bare))
                .is_none(),
            "a copy from the mixer is a source, without a placement"
        );

        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(deck),
            include_arrangement: true,
        });
        app.execute_command(C::Paste {
            target: PasteTarget::IntoChannel(channel),
        });
        let arranged = deck_uuids(&app, 0)[2].clone();
        let lane = app
            .mixer_ref()
            .arrangement()
            .and_then(|a| a.lane(&arranged))
            .expect("a lane came with it");
        assert_eq!(lane.regions.len(), 1);
        assert!((lane.regions[0].end - 4.0).abs() < f64::EPSILON);
    }

    /// `Cmd+D` must not destroy what was copied a minute ago.
    #[test]
    fn duplicating_leaves_the_clipboard_alone() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let first = a_deck(&mut app);
        let second = a_deck(&mut app);
        app.execute_command(C::Copy {
            source: ClipboardSource::Deck(first),
            include_arrangement: false,
        });
        let held = app.clipboard_summary().expect("something is held");

        app.execute_command(C::Duplicate {
            source: ClipboardSource::Deck(second),
        });

        assert_eq!(app.clipboard_summary(), Some(held));
    }

    /// Order is the whole point of a chain, so a pasted effect lands directly
    /// after the one whose menu was open rather than at the end.
    #[test]
    fn a_pasted_effect_lands_after_the_one_it_was_taken_from() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        let first = app.execute_command(C::AddEffect {
            target: EffectTarget::Deck(deck.clone()),
            shader_name: "invert".into(),
        });
        let CommandResult::OkWithId { uuid: first } = first else {
            eprintln!("Skipping: the invert filter is not in this registry");
            return;
        };
        app.execute_command(C::AddEffect {
            target: EffectTarget::Deck(deck.clone()),
            shader_name: "invert".into(),
        });

        app.execute_command(C::Copy {
            source: ClipboardSource::Effect(first.clone()),
            include_arrangement: false,
        });
        let result = app.execute_command(C::Paste {
            target: PasteTarget::AfterEffect(first.clone()),
        });
        let CommandResult::OkWithId { uuid: pasted } = result else {
            panic!("nothing was pasted: {result:?}");
        };

        let snapshot = app.mixer_snapshot();
        let chain: Vec<String> = snapshot.channels[0].decks[0]
            .effects
            .iter()
            .map(|e| e.uuid.clone())
            .collect();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], first);
        assert_eq!(chain[1], pasted);
        assert_ne!(chain[2], pasted, "the copy is its own effect");
    }

    /// A channel's own effects had no way to carry their modulation: only decks
    /// held recipes, so a copied channel arrived with its effects unmodulated.
    #[test]
    fn a_channels_own_effect_modulation_travels_with_it() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel = app.mixer_snapshot().channels[0].uuid.clone();
        let added = app.execute_command(C::AddEffect {
            target: EffectTarget::Channel(channel.clone()),
            shader_name: "invert".into(),
        });
        let CommandResult::OkWithId { uuid: effect } = added else {
            eprintln!("Skipping: the invert filter is not in this registry");
            return;
        };
        let param = app
            .mixer_snapshot()
            .channels
            .iter()
            .flat_map(|ch| ch.effects.iter())
            .find(|e| e.uuid == effect)
            .and_then(|e| e.params.params.first().map(|p| p.name.clone()));
        let Some(param) = param else {
            eprintln!("Skipping: that filter exposes no parameters to modulate");
            return;
        };
        app.execute_command(C::AddLfo {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0,
        });
        let lfo = app.mixer_ref().modulation().sources[0].uuid.clone();
        app.execute_command(C::AssignModulation {
            target: format!("fx_{effect}:{param}"),
            source_id: lfo.clone(),
            amount: 0.5,
        });

        app.execute_command(C::Copy {
            source: ClipboardSource::Channel(channel),
            include_arrangement: false,
        });
        let CommandResult::OkWithId { uuid: pasted } = app.execute_command(C::Paste {
            target: PasteTarget::NewChannel,
        }) else {
            panic!("the channel was not pasted");
        };

        let snapshot = app.mixer_snapshot();
        let copy = snapshot
            .channels
            .iter()
            .find(|ch| ch.uuid == pasted)
            .expect("the pasted channel");
        let copied_effect = copy.effects.first().expect("its effect came with it");
        assert_ne!(copied_effect.uuid, effect);
        assert!(
            app.mixer_ref()
                .modulation()
                .assignments_iter()
                .any(|(key, _)| key == &format!("fx_{}:{param}", copied_effect.uuid)),
            "the copy's effect is modulated too"
        );
    }

    /// Loading one deck preset twice used to put two decks behind one UUID, so
    /// every command, mapping, and API route reached only the first.
    #[test]
    fn the_same_preset_loaded_twice_makes_two_decks() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        let channel = app.mixer_snapshot().channels[0].uuid.clone();
        app.execute_command(C::SaveDeckPreset {
            deck_uuid: deck.clone(),
            name: "twice".into(),
        });

        for _ in 0..2 {
            app.execute_command(C::LoadDeckPreset {
                channel_uuid: channel.clone(),
                preset_name: "twice".into(),
            });
        }

        let uuids = deck_uuids(&app, 0);
        assert_eq!(uuids.len(), 3, "the original plus two loads");
        let unique: std::collections::HashSet<&String> = uuids.iter().collect();
        assert_eq!(unique.len(), 3, "each deck answers to its own address");
    }

    #[test]
    fn a_pasted_channel_is_a_second_channel() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let deck = a_deck(&mut app);
        let channel = app.mixer_snapshot().channels[0].uuid.clone();

        app.execute_command(C::Copy {
            source: ClipboardSource::Channel(channel.clone()),
            include_arrangement: false,
        });
        let result = app.execute_command(C::Paste {
            target: PasteTarget::NewChannel,
        });
        let CommandResult::OkWithId { uuid: pasted } = result else {
            panic!("no channel came back: {result:?}");
        };

        assert_ne!(pasted, channel);
        let snapshot = app.mixer_snapshot();
        assert_eq!(snapshot.channels.len(), 3, "two defaults plus the copy");
        let copy = snapshot
            .channels
            .iter()
            .find(|ch| ch.uuid == pasted)
            .expect("the pasted channel");
        assert_eq!(copy.decks.len(), 1, "its deck came with it");
        assert_ne!(copy.decks[0].uuid, deck);
    }
}
