//! Modulation source and assignment mutations.

use crate::engine::EngineCommand;
use crate::usecases::ui::{ModulationAction, UIActions};
use super::super::VardaApp;

impl VardaApp {
    /// Apply modulation actions
    pub(crate) fn apply_modulation_actions(&mut self, actions: &UIActions) {
        for action in &actions.modulation_actions {
            let cmd = match action {
                ModulationAction::AddLFO { waveform, frequency } =>
                    EngineCommand::AddLfo { waveform: *waveform, frequency: *frequency },
                ModulationAction::AddAudioFFT { preset, source_id } =>
                    EngineCommand::AddAudioBand { preset: *preset, source_id: *source_id },
                ModulationAction::RemoveSource { source_id } =>
                    EngineCommand::RemoveModulationSource { uuid: source_id.clone() },
                ModulationAction::UpdateLFOFrequency { source_id, frequency } =>
                    EngineCommand::UpdateLfoFrequency { uuid: source_id.clone(), frequency: *frequency },
                ModulationAction::UpdateLFOWaveform { source_id, waveform } =>
                    EngineCommand::UpdateLfoWaveform { uuid: source_id.clone(), waveform: *waveform },
                ModulationAction::UpdateLFOPhase { source_id, phase } =>
                    EngineCommand::UpdateLfoPhase { uuid: source_id.clone(), phase: *phase },
                ModulationAction::UpdateLFOAmplitude { source_id, amplitude } =>
                    EngineCommand::UpdateLfoAmplitude { uuid: source_id.clone(), amplitude: *amplitude },
                ModulationAction::UpdateLFOBipolar { source_id, bipolar } =>
                    EngineCommand::UpdateLfoBipolar { uuid: source_id.clone(), bipolar: *bipolar },
                ModulationAction::UpdateAudioSmoothing { source_id, smoothing } =>
                    EngineCommand::UpdateAudioSmoothing { uuid: source_id.clone(), smoothing: *smoothing },
                ModulationAction::UpdateAudioFreqLow { source_id, freq_low } =>
                    EngineCommand::UpdateAudioFreqLow { uuid: source_id.clone(), freq_low: *freq_low },
                ModulationAction::UpdateAudioFreqHigh { source_id, freq_high } =>
                    EngineCommand::UpdateAudioFreqHigh { uuid: source_id.clone(), freq_high: *freq_high },
                ModulationAction::UpdateAudioGain { source_id, gain } =>
                    EngineCommand::UpdateAudioGain { uuid: source_id.clone(), gain: *gain },
                ModulationAction::UpdateAudioPreset { source_id, preset } =>
                    EngineCommand::UpdateAudioPreset { uuid: source_id.clone(), preset: *preset },
                ModulationAction::UpdateAudioSource { source_id, source_id_audio } =>
                    EngineCommand::UpdateAudioSource { uuid: source_id.clone(), source_id: *source_id_audio },
                ModulationAction::UpdateAudioMode { source_id, mode } =>
                    EngineCommand::UpdateAudioMode { uuid: source_id.clone(), mode: *mode },
                ModulationAction::UpdateAudioNoiseGate { source_id, noise_gate } =>
                    EngineCommand::UpdateAudioNoiseGate { uuid: source_id.clone(), noise_gate: *noise_gate },
                ModulationAction::AddADSR { attack, decay, sustain, release } =>
                    EngineCommand::AddAdsr { attack: *attack, decay: *decay, sustain: *sustain, release: *release },
                ModulationAction::AddStepSequencer { num_steps, rate } =>
                    EngineCommand::AddStepSequencer { num_steps: *num_steps, rate: *rate },
                ModulationAction::UpdateADSRAttack { source_id, attack } =>
                    EngineCommand::UpdateAdsrAttack { uuid: source_id.clone(), attack: *attack },
                ModulationAction::UpdateADSRDecay { source_id, decay } =>
                    EngineCommand::UpdateAdsrDecay { uuid: source_id.clone(), decay: *decay },
                ModulationAction::UpdateADSRSustain { source_id, sustain } =>
                    EngineCommand::UpdateAdsrSustain { uuid: source_id.clone(), sustain: *sustain },
                ModulationAction::UpdateADSRRelease { source_id, release } =>
                    EngineCommand::UpdateAdsrRelease { uuid: source_id.clone(), release: *release },
                ModulationAction::TriggerADSR { source_id } =>
                    EngineCommand::TriggerAdsr { uuid: source_id.clone() },
                ModulationAction::ReleaseADSR { source_id } =>
                    EngineCommand::ReleaseAdsr { uuid: source_id.clone() },
                ModulationAction::UpdateStepValue { source_id, step_idx, value } =>
                    EngineCommand::UpdateStepSeqValue { uuid: source_id.clone(), step_idx: *step_idx, value: *value },
                ModulationAction::UpdateStepRate { source_id, rate } =>
                    EngineCommand::UpdateStepSeqRate { uuid: source_id.clone(), rate: *rate },
                ModulationAction::UpdateStepInterpolation { source_id, interpolation } =>
                    EngineCommand::UpdateStepSeqInterpolation { uuid: source_id.clone(), interpolation: *interpolation },
                ModulationAction::UpdateStepBipolar { source_id, bipolar } =>
                    EngineCommand::UpdateStepSeqBipolar { uuid: source_id.clone(), bipolar: *bipolar },
                ModulationAction::SetStepCount { source_id, count } =>
                    EngineCommand::SetStepSeqCount { uuid: source_id.clone(), count: *count },
                ModulationAction::AssignModOnMod { target_source_id, param_name, modulator_id, amount } =>
                    EngineCommand::AssignModOnMod { target_source_id: target_source_id.clone(), param_name: param_name.clone(), modulator_id: modulator_id.clone(), amount: *amount },
                ModulationAction::RemoveModOnMod { target_source_id, param_name } =>
                    EngineCommand::RemoveModOnMod { target_source_id: target_source_id.clone(), param_name: param_name.clone() },
                ModulationAction::AssignModulation { deck_uuid, param_name, source_id, amount } =>
                    EngineCommand::AssignModulation { target: format!("deck_{}:{}", deck_uuid, param_name), source_id: source_id.clone(), amount: *amount },
                ModulationAction::RemoveAssignment { deck_uuid, param_name, .. } =>
                    EngineCommand::ClearModulation { target: format!("deck_{}:{}", deck_uuid, param_name) },
                ModulationAction::AssignEffectModulation { effect_uuid, param_name, source_id, amount } =>
                    EngineCommand::AssignModulation { target: format!("fx_{}:{}", effect_uuid, param_name), source_id: source_id.clone(), amount: *amount },
                ModulationAction::RemoveEffectAssignment { effect_uuid, param_name } =>
                    EngineCommand::ClearModulation { target: format!("fx_{}:{}", effect_uuid, param_name) },
            };
            self.execute_command(cmd);
        }
    }
}
