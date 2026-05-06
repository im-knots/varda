//! Parameter value update mutations.

use crate::params::ParamValue;
use crate::usecases::ui::{ParamUpdate, UIActions};
use super::super::VardaApp;

impl VardaApp {
    /// Apply parameter value updates (generator, effect, master effect params)
    pub(crate) fn apply_param_updates(&mut self, actions: &UIActions) {
        let mixer = &mut self.mixer;
        for update in &actions.param_updates {
            match update {
                ParamUpdate::GeneratorFloat { ch_idx, deck_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            ch.decks[*deck_idx].deck.generator_params.set(name, ParamValue::Float(*value));
                        }
                    }
                }
                ParamUpdate::GeneratorBool { ch_idx, deck_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            ch.decks[*deck_idx].deck.generator_params.set(name, ParamValue::Bool(*value));
                        }
                    }
                }
                ParamUpdate::GeneratorColor { ch_idx, deck_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            ch.decks[*deck_idx].deck.generator_params.set(name, ParamValue::Color(*value));
                        }
                    }
                }
                ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            ch.decks[*deck_idx].deck.generator_params.reset_to_defaults();
                        }
                    }
                }
                ParamUpdate::EffectFloat { ch_idx, deck_idx, effect_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            let deck = &mut ch.decks[*deck_idx].deck;
                            if *effect_idx < deck.effects.len() {
                                deck.effects[*effect_idx].params.set(name, ParamValue::Float(*value));
                            }
                        }
                    }
                }
                ParamUpdate::EffectBool { ch_idx, deck_idx, effect_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            let deck = &mut ch.decks[*deck_idx].deck;
                            if *effect_idx < deck.effects.len() {
                                deck.effects[*effect_idx].params.set(name, ParamValue::Bool(*value));
                            }
                        }
                    }
                }
                ParamUpdate::EffectColor { ch_idx, deck_idx, effect_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *deck_idx < ch.decks.len() {
                            let deck = &mut ch.decks[*deck_idx].deck;
                            if *effect_idx < deck.effects.len() {
                                deck.effects[*effect_idx].params.set(name, ParamValue::Color(*value));
                            }
                        }
                    }
                }
                ParamUpdate::ChannelEffectFloat { ch_idx, effect_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *effect_idx < ch.effects.len() {
                            ch.effects[*effect_idx].params.set(name, ParamValue::Float(*value));
                        }
                    }
                }
                ParamUpdate::ChannelEffectBool { ch_idx, effect_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *effect_idx < ch.effects.len() {
                            ch.effects[*effect_idx].params.set(name, ParamValue::Bool(*value));
                        }
                    }
                }
                ParamUpdate::ChannelEffectColor { ch_idx, effect_idx, name, value } => {
                    if let Some(ch) = mixer.channel_mut(*ch_idx) {
                        if *effect_idx < ch.effects.len() {
                            ch.effects[*effect_idx].params.set(name, ParamValue::Color(*value));
                        }
                    }
                }
                ParamUpdate::MasterEffectFloat { effect_idx, name, value } => {
                    if *effect_idx < mixer.master_effects().len() {
                        mixer.master_effects_mut()[*effect_idx].params.set(name, ParamValue::Float(*value));
                    }
                }
                ParamUpdate::MasterEffectBool { effect_idx, name, value } => {
                    if *effect_idx < mixer.master_effects().len() {
                        mixer.master_effects_mut()[*effect_idx].params.set(name, ParamValue::Bool(*value));
                    }
                }
                ParamUpdate::MasterEffectColor { effect_idx, name, value } => {
                    if *effect_idx < mixer.master_effects().len() {
                        mixer.master_effects_mut()[*effect_idx].params.set(name, ParamValue::Color(*value));
                    }
                }
            }
        }
    }
}
