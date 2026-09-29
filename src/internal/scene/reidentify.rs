//! Giving a restored config fresh UUIDs.
//!
//! Commands, modulation keys, MIDI paths and API routes resolve by UUID and take
//! the first match, so a second copy of an entity needs new UUIDs.
//!
//! `taken` reports whether a UUID is already live. Paste passes one that always
//! returns true; preset load mints only on collision, so mappings to a restored
//! preset keep working.

use super::{ChannelConfig, DeckConfig, EffectConfig, ModulationRecipe};
use crate::ids::generate_short_uuid;
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

/// Point assignments at the renamed effects, and give curves new UUIDs.
///
/// Live modulators keep their UUIDs, so a pasted deck follows the same LFO as
/// the original. A curve belongs to one parameter, so a copy gets its own curve
/// with the same shape.
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

/// `effect/{old}/param/{name}` becomes `effect/{new}/param/{name}`. Owner-relative
/// params (`param/speed`) have no UUID; the caller re-prefixes them.
fn rename_effect_key(param: &str, renames: &Renames) -> Option<String> {
    use crate::engine::value::param::ParamAddress;
    let ParamAddress::EffectParam { effect, param } = param.parse().ok()? else {
        return None;
    };
    let new = renames.get(&effect)?;
    Some(ParamAddress::effect_param(new, &param).to_string())
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
            source: SourceConfig::new("Shader").with("path", "shaders/waves.fs"),
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
                legacy_component: None,
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
                legacy_component: None,
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

    /// A deck whose UUID is free keeps it, so mappings to it keep working.
    #[test]
    fn a_deck_whose_identity_is_free_keeps_it() {
        let mut config = a_deck("deck0001");
        deck(&mut config, &nothing_is_taken);

        assert_eq!(config.uuid, "deck0001");
        assert_eq!(config.effects[0].uuid, "fx000001");
    }

    /// An assignment naming the old effect must name the new one, or the
    /// original's param is driven twice and the copy's not at all.
    #[test]
    fn effect_assignments_follow_the_effect_they_name() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![an_lfo_on("effect/fx000001/param/amount")];
        deck(&mut config, &everything_is_taken);

        let new_fx = &config.effects[0].uuid;
        assert_eq!(
            config.modulation[0].assignments[0].param,
            format!("effect/{new_fx}/param/amount")
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

    /// A pasted deck follows the same LFO as the original.
    #[test]
    fn a_live_modulator_stays_shared() {
        let mut config = a_deck("deck0001");
        config.modulation = vec![an_lfo_on("speed")];
        deck(&mut config, &everything_is_taken);

        assert_eq!(config.modulation[0].source_uuid, "lfo00001");
    }

    /// A curve belongs to one parameter, so the copy gets its own with the same shape.
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

    /// A curve whose UUID is free keeps it.
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
        let mut modulation = vec![an_lfo_on("effect/fx000001/param/amount")];
        effect(&mut config, &mut modulation, &everything_is_taken);

        assert_ne!(config.uuid, "fx000001");
        assert_eq!(
            modulation[0].assignments[0].param,
            format!("effect/{}/param/amount", config.uuid)
        );
    }

    /// A tap deck's watched channel UUID is not renamed when the deck is copied.
    #[test]
    fn a_reference_to_another_entity_is_left_alone() {
        let mut config = a_deck("deck0001");
        config.source = SourceConfig::new("Tap").with(
            "source",
            serde_json::json!({"kind": "channel", "uuid": "chan0001"}),
        );
        deck(&mut config, &everything_is_taken);

        assert_eq!(
            config.source.get("source"),
            Some(&serde_json::json!({"kind": "channel", "uuid": "chan0001"})),
            "the tapped channel is not ours to rename"
        );
    }

    /// Scenes from before UUIDs, or hand-edited ones, can hold an empty UUID.
    /// It is left empty for the caller's repair path.
    #[test]
    fn an_entity_with_no_identity_is_left_without_one() {
        let mut config = a_deck("");
        deck(&mut config, &everything_is_taken);
        assert!(config.uuid.is_empty());
    }

    /// Modulation keys that are not effect params survive a copy unchanged.
    #[test]
    fn a_key_that_names_no_effect_is_carried_across_unchanged() {
        let renames: Renames = [("fx000001".to_string(), "fx000002".to_string())]
            .into_iter()
            .collect();

        for key in [
            "param/speed",
            "effect/",
            "effect/fx000001",
            "effect/unknown/param/amount",
            "/param/amount",
        ] {
            assert_eq!(rename_effect_key(key, &renames), None, "{key}");
        }
        assert_eq!(
            rename_effect_key("effect/fx000001/param/amount", &renames).as_deref(),
            Some("effect/fx000002/param/amount"),
        );
    }
}
