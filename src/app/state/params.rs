//! Parameter value update mutations.

use super::super::VardaApp;
use crate::engine::EngineCommand;
use crate::params::ParamValue;
use crate::usecases::ui::{ParamUpdate, UIActions};

impl VardaApp {
    /// Apply parameter value updates (generator, effect, master effect params)
    pub(crate) fn apply_param_updates(&mut self, actions: &UIActions) {
        for update in &actions.param_updates {
            let cmd = match update {
                ParamUpdate::GeneratorFloat {
                    ch_idx,
                    deck_idx,
                    name,
                    value,
                } => EngineCommand::SetGeneratorParam {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    name: name.clone(),
                    value: ParamValue::Float(*value),
                },
                ParamUpdate::GeneratorBool {
                    ch_idx,
                    deck_idx,
                    name,
                    value,
                } => EngineCommand::SetGeneratorParam {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    name: name.clone(),
                    value: ParamValue::Bool(*value),
                },
                ParamUpdate::GeneratorColor {
                    ch_idx,
                    deck_idx,
                    name,
                    value,
                } => EngineCommand::SetGeneratorParam {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    name: name.clone(),
                    value: ParamValue::Color(*value),
                },
                ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx } => {
                    EngineCommand::ResetGeneratorParamsToDefaults {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                    }
                }
                ParamUpdate::EffectFloat {
                    ch_idx,
                    deck_idx,
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetEffectParam {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Float(*value),
                },
                ParamUpdate::EffectBool {
                    ch_idx,
                    deck_idx,
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetEffectParam {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Bool(*value),
                },
                ParamUpdate::EffectColor {
                    ch_idx,
                    deck_idx,
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetEffectParam {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Color(*value),
                },
                ParamUpdate::ChannelEffectFloat {
                    ch_idx,
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetChannelEffectParam {
                    channel_idx: *ch_idx,
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Float(*value),
                },
                ParamUpdate::ChannelEffectBool {
                    ch_idx,
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetChannelEffectParam {
                    channel_idx: *ch_idx,
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Bool(*value),
                },
                ParamUpdate::ChannelEffectColor {
                    ch_idx,
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetChannelEffectParam {
                    channel_idx: *ch_idx,
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Color(*value),
                },
                ParamUpdate::MasterEffectFloat {
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetMasterEffectParam {
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Float(*value),
                },
                ParamUpdate::MasterEffectBool {
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetMasterEffectParam {
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Bool(*value),
                },
                ParamUpdate::MasterEffectColor {
                    effect_idx,
                    name,
                    value,
                } => EngineCommand::SetMasterEffectParam {
                    effect_idx: *effect_idx,
                    name: name.clone(),
                    value: ParamValue::Color(*value),
                },
            };
            self.execute_command(cmd);
        }
    }
}
