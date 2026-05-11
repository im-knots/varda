//! Modulation source and assignment mutations.

use crate::modulation::ModulationSource;
use crate::usecases::ui::{ModulationAction, UIActions};
use super::super::VardaApp;

impl VardaApp {
    /// Apply modulation actions
    pub(crate) fn apply_modulation_actions(&mut self, actions: &UIActions) {
        let mixer = &mut self.mixer;
        for action in &actions.modulation_actions {
            match action {
                ModulationAction::AddLFO { waveform, frequency } => {
                    let source = ModulationSource::LFO {
                        waveform: *waveform, frequency: *frequency, phase: 0.0, amplitude: 1.0, bipolar: false,
                    };
                    let uuid = mixer.modulation_mut().add_source(source);
                    log::info!("Added LFO modulation source {}", uuid);
                }
                ModulationAction::AddAudioFFT { preset, source_id } => {
                    let (freq_low, freq_high) = preset.freq_range();
                    let source = ModulationSource::AudioBand { source_id: *source_id, freq_low, freq_high, gain: 1.0, smoothing: 0.6, mode: crate::modulation::AudioReactMode::Direct, noise_gate: 0.1 };
                    let uuid = mixer.modulation_mut().add_source(source);
                    log::info!("Added Audio FFT modulation source {} ({:?}, {}-{}Hz)", uuid, preset, freq_low, freq_high);
                }
                ModulationAction::RemoveSource { source_id } => {
                    mixer.modulation_mut().remove_source(source_id);
                    log::info!("Removed modulation source {}", source_id);
                }
            ModulationAction::UpdateLFOFrequency { source_id, frequency } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::LFO { frequency: ref mut f, .. } = source { *f = *frequency; }
                }
            }
            ModulationAction::UpdateLFOWaveform { source_id, waveform } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::LFO { waveform: ref mut w, .. } = source { *w = *waveform; }
                }
            }
            ModulationAction::UpdateLFOPhase { source_id, phase } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::LFO { phase: ref mut p, .. } = source { *p = *phase; }
                }
            }
            ModulationAction::UpdateLFOAmplitude { source_id, amplitude } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::LFO { amplitude: ref mut a, .. } = source { *a = *amplitude; }
                }
            }
            ModulationAction::UpdateLFOBipolar { source_id, bipolar } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::LFO { bipolar: ref mut b, .. } = source { *b = *bipolar; }
                }
            }
            ModulationAction::UpdateAudioSmoothing { source_id, smoothing } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { smoothing: ref mut s, .. } = source { *s = *smoothing; }
                }
            }
            ModulationAction::UpdateAudioFreqLow { source_id, freq_low } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, .. } = source { *fl = *freq_low; }
                }
            }
            ModulationAction::UpdateAudioFreqHigh { source_id, freq_high } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { freq_high: ref mut fh, .. } = source { *fh = *freq_high; }
                }
            }
            ModulationAction::UpdateAudioGain { source_id, gain } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { gain: ref mut g, .. } = source { *g = *gain; }
                }
            }
            ModulationAction::UpdateAudioPreset { source_id, preset } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, freq_high: ref mut fh, .. } = source {
                        let (lo, hi) = preset.freq_range();
                        *fl = lo; *fh = hi;
                    }
                }
            }
            ModulationAction::UpdateAudioSource { source_id, source_id_audio } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { source_id: ref mut sid, .. } = source { *sid = *source_id_audio; }
                }
            }
            ModulationAction::UpdateAudioMode { source_id, mode } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { mode: ref mut m, .. } = source { *m = *mode; }
                }
            }
            ModulationAction::UpdateAudioNoiseGate { source_id, noise_gate } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::AudioBand { noise_gate: ref mut ng, .. } = source { *ng = *noise_gate; }
                }
            }
            ModulationAction::AddADSR { attack, decay, sustain, release } => {
                let source = ModulationSource::adsr(*attack, *decay, *sustain, *release);
                let uuid = mixer.modulation_mut().add_source(source);
                log::info!("Added ADSR modulation source {}", uuid);
            }
            ModulationAction::AddStepSequencer { num_steps, rate } => {
                let source = ModulationSource::step_sequencer(*num_steps, *rate);
                let uuid = mixer.modulation_mut().add_source(source);
                log::info!("Added StepSequencer modulation source {} ({} steps)", uuid, num_steps);
            }
            ModulationAction::UpdateADSRAttack { source_id, attack } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::ADSR { attack: ref mut a, .. } = source { *a = *attack; }
                }
            }
            ModulationAction::UpdateADSRDecay { source_id, decay } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::ADSR { decay: ref mut d, .. } = source { *d = *decay; }
                }
            }
            ModulationAction::UpdateADSRSustain { source_id, sustain } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::ADSR { sustain: ref mut s, .. } = source { *s = *sustain; }
                }
            }
            ModulationAction::UpdateADSRRelease { source_id, release } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::ADSR { release: ref mut r, .. } = source { *r = *release; }
                }
            }
            ModulationAction::TriggerADSR { source_id } => {
                mixer.modulation_mut().trigger_adsr(source_id);
            }
            ModulationAction::ReleaseADSR { source_id } => {
                mixer.modulation_mut().release_adsr(source_id);
            }
            ModulationAction::UpdateStepValue { source_id, step_idx, value } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::StepSequencer { steps, .. } = source {
                        if *step_idx < steps.len() { steps[*step_idx] = *value; }
                    }
                }
            }
            ModulationAction::UpdateStepRate { source_id, rate } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::StepSequencer { rate: ref mut r, .. } = source { *r = *rate; }
                }
            }
            ModulationAction::UpdateStepInterpolation { source_id, interpolation } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::StepSequencer { interpolation: ref mut interp, .. } = source { *interp = *interpolation; }
                }
            }
            ModulationAction::UpdateStepBipolar { source_id, bipolar } => {
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::StepSequencer { bipolar: ref mut b, .. } = source { *b = *bipolar; }
                }
            }
            ModulationAction::SetStepCount { source_id, count } => {
                let count = (*count).max(2).min(64);
                if let Some(source) = mixer.modulation_mut().source_mut(source_id) {
                    if let ModulationSource::StepSequencer { steps, .. } = source {
                        steps.resize(count, 0.0);
                    }
                }
            }
            ModulationAction::AssignModOnMod { target_source_id, param_name, modulator_id, amount } => {
                mixer.modulation_mut().assign_mod_on_mod(target_source_id, param_name, modulator_id, *amount);
                log::info!("Assigned mod-on-mod: {} modulates {} param {} (amount {})", modulator_id, target_source_id, param_name, amount);
            }
            ModulationAction::RemoveModOnMod { target_source_id, param_name } => {
                mixer.modulation_mut().clear_mod_on_mod(target_source_id, param_name);
                log::info!("Removed mod-on-mod from source {} param {}", target_source_id, param_name);
            }
            ModulationAction::AssignModulation { deck_uuid, param_name, source_id, amount } => {
                mixer.modulation_mut().assign(&format!("deck_{}:{}", deck_uuid, param_name), source_id, *amount, None);
                log::info!("Assigned modulation source {} to deck {} param {} with amount {}", source_id, deck_uuid, param_name, amount);
            }
            ModulationAction::RemoveAssignment { deck_uuid, param_name, .. } => {
                mixer.modulation_mut().clear_assignments(&format!("deck_{}:{}", deck_uuid, param_name));
                log::info!("Removed modulation assignment from deck {} param {}", deck_uuid, param_name);
            }
            ModulationAction::AssignEffectModulation { effect_uuid, param_name, source_id, amount } => {
                let key = format!("fx_{}:{}", effect_uuid, param_name);
                mixer.modulation_mut().assign(&key, source_id, *amount, None);
                log::info!("Assigned modulation source {} to effect {} param {}", source_id, effect_uuid, param_name);
            }
            ModulationAction::RemoveEffectAssignment { effect_uuid, param_name } => {
                let key = format!("fx_{}:{}", effect_uuid, param_name);
                mixer.modulation_mut().clear_assignments(&key);
            }
            }
        }
    }
}
