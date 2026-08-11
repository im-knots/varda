//! Giving a restored config a fresh identity.
//!
//! A UUID names one live entity ([`/spec/entity-identity.md`]): every command,
//! modulation key, MIDI path, and API route resolves by UUID and takes the first
//! match. Rebuilding a scene restores the identities it saved, but building a
//! *second* copy of something that is already on stage has to mint new ones, or
//! the two share every address they have.
//!
//! `taken` answers "is this UUID already live", so the same pass serves both
//! callers: paste always mints (a copy is a second entity), while preset load
//! mints only on collision, which leaves a preset restored into a scene where
//! its UUID is free still answering to the mappings that point at it.
//!
//! See [`/spec/clipboard.md`] § Paste reidentifies.

use super::{ChannelConfig, DeckConfig, EffectConfig, ModulationRecipe};
use crate::deck::generate_short_uuid;
use std::collections::HashMap;

/// Rename map from old UUID to new, for the effects a pass has reidentified.
type Renames = HashMap<String, String>;

/// Reidentify a deck and its effects, remapping the modulation it carries.
pub fn deck(config: &mut DeckConfig, taken: &dyn Fn(&str) -> bool) {
    mint_if_taken(&mut config.uuid, taken);

    let mut renames = Renames::new();
    for effect in &mut config.effects {
        let old = effect.uuid.clone();
        if mint_if_taken(&mut effect.uuid, taken) {
            renames.insert(old, effect.uuid.clone());
        }
    }
    recipes(&mut config.modulation, &renames, taken);
}

/// Reidentify a channel, its own effects, and every deck inside it.
pub fn channel(config: &mut ChannelConfig, taken: &dyn Fn(&str) -> bool) {
    mint_if_taken(&mut config.uuid, taken);

    let mut renames = Renames::new();
    for effect in &mut config.effects {
        let old = effect.uuid.clone();
        if mint_if_taken(&mut effect.uuid, taken) {
            renames.insert(old, effect.uuid.clone());
        }
    }
    recipes(&mut config.modulation, &renames, taken);

    for deck_config in &mut config.decks {
        deck(deck_config, taken);
    }
}

/// Reidentify a lone effect and the assignments that name it.
pub fn effect(
    config: &mut EffectConfig,
    modulation: &mut [ModulationRecipe],
    taken: &dyn Fn(&str) -> bool,
) {
    let old = config.uuid.clone();
    let mut renames = Renames::new();
    if mint_if_taken(&mut config.uuid, taken) {
        renames.insert(old, config.uuid.clone());
    }
    recipes(modulation, &renames, taken);
}

/// Point assignments at the renamed effects, and give curves an identity of
/// their own.
///
/// Live modulators are deliberately left alone: a recipe naming an LFO that is
/// already in the scene wires up to that LFO, so a pasted deck rides the same
/// one as the deck it came from. An envelope is the exception, because a curve
/// belongs to the one parameter it was drawn for
/// ([`/spec/automation.md`] § One envelope per parameter), so a copy gets its
/// own curve with the same shape rather than a shared source that either lane
/// could rewrite.
fn recipes(recipes: &mut [ModulationRecipe], renames: &Renames, taken: &dyn Fn(&str) -> bool) {
    for recipe in recipes {
        if recipe.source.is_envelope() {
            mint_if_taken(&mut recipe.source_uuid, taken);
        }
        for assignment in &mut recipe.assignments {
            if let Some(renamed) = rename_effect_key(&assignment.param, renames) {
                assignment.param = renamed;
            }
        }
    }
}

/// `fx_{old}:{param}` becomes `fx_{new}:{param}`. Bare generator params
/// (`speed`) carry no UUID and are re-prefixed by the caller that knows the new
/// deck, so they are left alone here.
fn rename_effect_key(param: &str, renames: &Renames) -> Option<String> {
    let rest = param.strip_prefix("fx_")?;
    let (uuid, name) = rest.split_once(':')?;
    let new = renames.get(uuid)?;
    Some(format!("fx_{new}:{name}"))
}

/// Returns whether a new UUID was minted.
fn mint_if_taken(uuid: &mut String, taken: &dyn Fn(&str) -> bool) -> bool {
    if uuid.is_empty() || !taken(uuid) {
        return false;
    }
    *uuid = generate_short_uuid();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulation::ModulationSource;
    use crate::scene::{BlendModeConfig, ModulationRecipeAssignment, SourceConfig};

    fn everything_is_taken(_: &str) -> bool {
        true
    }

    fn nothing_is_taken(_: &str) -> bool {
        false
    }

    fn an_effect(uuid: &str) -> EffectConfig {
        EffectConfig {
            uuid: uuid.to_string(),
            path: "shaders/blur.fs".to_string(),
            enabled: true,
            params: std::collections::HashMap::new(),
        }
    }

    fn a_deck(uuid: &str) -> DeckConfig {
        DeckConfig {
            uuid: uuid.to_string(),
            name: "waves".to_string(),
            source: SourceConfig::Shader {
                path: "shaders/waves.fs".to_string(),
                params: std::collections::HashMap::new(),
                depth_prepro: None,
            },
            effects: vec![an_effect("fx000001")],
            opacity: 1.0,
            transparent: false,
            blend_mode: BlendModeConfig::Normal,
            mute: false,
            solo: false,
            z_index: 0,
            render_fps: crate::channel::DeckRenderFps::Auto,
            auto_transition: None,
            modulation: Vec::new(),
        }
    }

    fn an_lfo_on(param: &str) -> ModulationRecipe {
        ModulationRecipe {
            source_uuid: "lfo00001".to_string(),
            source: ModulationSource::sine_lfo(1.0),
            timebase: crate::timebase::Timebase::FreeRun,
            assignments: vec![ModulationRecipeAssignment {
                param: param.to_string(),
                amount: 0.5,
                component: None,
            }],
        }
    }

    fn a_curve_on(param: &str) -> ModulationRecipe {
        ModulationRecipe {
            source_uuid: "env00001".to_string(),
            source: ModulationSource::envelope(vec![crate::modulation::Breakpoint {
                position: 1.0,
                value: 0.5,
                curve: crate::modulation::CurveKind::default(),
            }]),
            timebase: crate::timebase::Timebase::Transport,
            assignments: vec![ModulationRecipeAssignment {
                param: param.to_string(),
                amount: 1.0,
                component: None,
            }],
        }
    }

    #[test]
    fn a_deck_already_on_stage_gets_new_identity_throughout() {
        let mut config = a_deck("deck0001");
        deck(&mut config, &everything_is_taken);

        assert_ne!(config.uuid, "deck0001");
        assert_ne!(config.effects[0].uuid, "fx000001");
        assert_eq!(
            config.uuid.len(),
            8,
            "short hex UUIDs, like every other one"
        );
    }

    /// Restoring a deck whose UUID is free is a restore, not a copy, so the
    /// mappings pointing at it keep working.
    #[test]
    fn a_deck_whose_identity_is_free_keeps_it() {
        let mut config = a_deck("deck0001");
        deck(&mut config, &nothing_is_taken);

        assert_eq!(config.uuid, "deck0001");
        assert_eq!(config.effects[0].uuid, "fx000001");
    }

    /// The whole point of the pass: an assignment that named the old effect has
    /// to name the new one, or the copy's effect param is unmodulated while the
    /// original's is driven twice.
    #[test]
    fn effect_assignments_follow_the_effect_they_name() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![an_lfo_on("fx_fx000001:amount")];
        deck(&mut config, &everything_is_taken);

        let new_fx = &config.effects[0].uuid;
        assert_eq!(
            config.modulation[0].assignments[0].param,
            format!("fx_{new_fx}:amount")
        );
    }

    /// Generator params are stored relative to the deck and re-prefixed at
    /// restore, so they have no UUID to rewrite.
    #[test]
    fn generator_assignments_are_left_relative() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![an_lfo_on("speed")];
        deck(&mut config, &everything_is_taken);

        assert_eq!(config.modulation[0].assignments[0].param, "speed");
    }

    /// A pasted deck rides the same LFO as the deck it came from. That is what
    /// a performer means by copying something that is being modulated.
    #[test]
    fn a_live_modulator_stays_shared() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![an_lfo_on("speed")];
        deck(&mut config, &everything_is_taken);

        assert_eq!(config.modulation[0].source_uuid, "lfo00001");
    }

    /// A curve belongs to one parameter, so the copy gets its own with the same
    /// shape rather than a source both lanes would rewrite.
    #[test]
    fn a_curve_is_cloned_rather_than_shared() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![a_curve_on("speed")];
        deck(&mut config, &everything_is_taken);

        assert_ne!(config.modulation[0].source_uuid, "env00001");
        let ModulationSource::Envelope { breakpoints, .. } = &config.modulation[0].source else {
            panic!("still an envelope");
        };
        assert_eq!(breakpoints.len(), 1, "the shape comes along unchanged");
        assert_eq!(
            config.modulation[0].timebase,
            crate::timebase::Timebase::Transport,
            "a curve that followed the show still follows the show"
        );
    }

    /// Loading a curve into a scene that has never seen it is a restore.
    #[test]
    fn a_curve_keeps_its_identity_when_it_is_free() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![a_curve_on("speed")];
        deck(&mut config, &nothing_is_taken);

        assert_eq!(config.modulation[0].source_uuid, "env00001");
    }

    #[test]
    fn a_channel_reidentifies_everything_under_it() {
        let mut config = ChannelConfig {
            uuid: "chan0001".to_string(),
            name: "A".to_string(),
            opacity: 1.0,
            blend_mode: BlendModeConfig::Normal,
            decks: vec![a_deck("deck0001"), a_deck("deck0002")],
            effects: vec![an_effect("fx000009")],
            modulation: Vec::new(),
        };
        channel(&mut config, &everything_is_taken);

        assert_ne!(config.uuid, "chan0001");
        assert_ne!(config.effects[0].uuid, "fx000009");
        assert_ne!(config.decks[0].uuid, "deck0001");
        assert_ne!(config.decks[1].uuid, "deck0002");
        assert_ne!(
            config.decks[0].uuid, config.decks[1].uuid,
            "two decks in one channel cannot share an identity"
        );
        assert_ne!(
            config.decks[0].effects[0].uuid, config.decks[1].effects[0].uuid,
            "nor can their effects, which started as the same UUID"
        );
    }

    #[test]
    fn a_lone_effect_carries_its_assignments_across() {
        let mut config = an_effect("fx000001");
        let mut modulation = vec![an_lfo_on("fx_fx000001:amount")];
        effect(&mut config, &mut modulation, &everything_is_taken);

        assert_ne!(config.uuid, "fx000001");
        assert_eq!(
            modulation[0].assignments[0].param,
            format!("fx_{}:amount", config.uuid)
        );
    }

    /// A tap deck names the channel it is watching. That channel is somebody
    /// else and must not be renamed by a pass that is copying this deck.
    #[test]
    fn a_reference_to_another_entity_is_left_alone() {
        let mut config = a_deck("deck0001");
        config.source = SourceConfig::Tap {
            source: crate::scene::TapSourceConfig::Channel {
                uuid: "chan0001".to_string(),
            },
            scaling_mode: crate::deck::ScalingMode::default(),
        };
        deck(&mut config, &everything_is_taken);

        let SourceConfig::Tap {
            source: crate::scene::TapSourceConfig::Channel { uuid },
            ..
        } = &config.source
        else {
            panic!("still a tap");
        };
        assert_eq!(uuid, "chan0001", "the tapped channel is not ours to rename");
    }

    /// A scene written before UUIDs, or hand-edited, can carry an empty one.
    /// Minting there would give it an identity the rest of the file does not
    /// name, so it is left empty and the caller's own repair path deals with it.
    #[test]
    fn an_entity_with_no_identity_is_left_without_one() {
        let mut config = a_deck("");
        deck(&mut config, &everything_is_taken);
        assert!(config.uuid.is_empty());
    }

    /// Modulation keys that are not effect params (a generator param, a stray
    /// string) must survive a copy untouched: renaming what we do not recognise
    /// would break assignments rather than move them.
    #[test]
    fn a_key_that_names_no_effect_is_carried_across_unchanged() {
        let renames: Renames = [("fx000001".to_string(), "fx000002".to_string())]
            .into_iter()
            .collect();

        for key in [
            "speed",
            "fx_",
            "fx_fx000001",
            "fx_unknown:amount",
            ":amount",
        ] {
            assert_eq!(rename_effect_key(key, &renames), None, "{key}");
        }
        assert_eq!(
            rename_effect_key("fx_fx000001:amount", &renames).as_deref(),
            Some("fx_fx000002:amount"),
        );
    }
}
