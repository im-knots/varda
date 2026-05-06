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
                    let idx = mixer.modulation.add_source(source);
                    log::info!("Added LFO modulation source {}", idx);
                }
                ModulationAction::AddAudioFFT { preset, source_id } => {
                    let (freq_low, freq_high) = preset.freq_range();
                    let source = ModulationSource::AudioBand { source_id: *source_id, freq_low, freq_high, gain: 1.0, smoothing: 0.6, mode: crate::modulation::AudioReactMode::Direct, noise_gate: 0.1 };
                    let idx = mixer.modulation.add_source(source);
                    log::info!("Added Audio FFT modulation source {} ({:?}, {}-{}Hz)", idx, preset, freq_low, freq_high);
                }
                ModulationAction::RemoveSource { idx } => {
                    mixer.modulation.remove_source(*idx);
                    log::info!("Removed modulation source {}", idx);
                }
            ModulationAction::UpdateLFOFrequency { idx, frequency } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { frequency: ref mut f, .. } = mixer.modulation.sources[*idx] {
                        *f = *frequency;
                    }
                }
            }
            ModulationAction::UpdateLFOWaveform { idx, waveform } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { waveform: ref mut w, .. } = mixer.modulation.sources[*idx] {
                        *w = *waveform;
                    }
                }
            }
            ModulationAction::UpdateLFOPhase { idx, phase } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { phase: ref mut p, .. } = mixer.modulation.sources[*idx] {
                        *p = *phase;
                    }
                }
            }
            ModulationAction::UpdateLFOAmplitude { idx, amplitude } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { amplitude: ref mut a, .. } = mixer.modulation.sources[*idx] {
                        *a = *amplitude;
                    }
                }
            }
            ModulationAction::UpdateLFOBipolar { idx, bipolar } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { bipolar: ref mut b, .. } = mixer.modulation.sources[*idx] {
                        *b = *bipolar;
                    }
                }
            }
            ModulationAction::UpdateAudioSmoothing { idx, smoothing } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { smoothing: ref mut s, .. } = mixer.modulation.sources[*idx] {
                        *s = *smoothing;
                    }
                }
            }
            ModulationAction::UpdateAudioFreqLow { idx, freq_low } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, .. } = mixer.modulation.sources[*idx] {
                        *fl = *freq_low;
                    }
                }
            }
            ModulationAction::UpdateAudioFreqHigh { idx, freq_high } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { freq_high: ref mut fh, .. } = mixer.modulation.sources[*idx] {
                        *fh = *freq_high;
                    }
                }
            }
            ModulationAction::UpdateAudioGain { idx, gain } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { gain: ref mut g, .. } = mixer.modulation.sources[*idx] {
                        *g = *gain;
                    }
                }
            }
            ModulationAction::UpdateAudioPreset { idx, preset } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, freq_high: ref mut fh, .. } = mixer.modulation.sources[*idx] {
                        let (lo, hi) = preset.freq_range();
                        *fl = lo;
                        *fh = hi;
                    }
                }
            }
            ModulationAction::UpdateAudioSource { idx, source_id } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { source_id: ref mut sid, .. } = mixer.modulation.sources[*idx] {
                        *sid = *source_id;
                    }
                }
            }
            ModulationAction::UpdateAudioMode { idx, mode } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { mode: ref mut m, .. } = mixer.modulation.sources[*idx] {
                        *m = *mode;
                    }
                }
            }
            ModulationAction::UpdateAudioNoiseGate { idx, noise_gate } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { noise_gate: ref mut ng, .. } = mixer.modulation.sources[*idx] {
                        *ng = *noise_gate;
                    }
                }
            }
            ModulationAction::AddADSR { attack, decay, sustain, release } => {
                let source = ModulationSource::adsr(*attack, *decay, *sustain, *release);
                let idx = mixer.modulation.add_source(source);
                log::info!("Added ADSR modulation source {}", idx);
            }
            ModulationAction::AddStepSequencer { num_steps, rate } => {
                let source = ModulationSource::step_sequencer(*num_steps, *rate);
                let idx = mixer.modulation.add_source(source);
                log::info!("Added StepSequencer modulation source {} ({} steps)", idx, num_steps);
            }
            ModulationAction::UpdateADSRAttack { idx, attack } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { attack: ref mut a, .. } = mixer.modulation.sources[*idx] {
                        *a = *attack;
                    }
                }
            }
            ModulationAction::UpdateADSRDecay { idx, decay } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { decay: ref mut d, .. } = mixer.modulation.sources[*idx] {
                        *d = *decay;
                    }
                }
            }
            ModulationAction::UpdateADSRSustain { idx, sustain } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { sustain: ref mut s, .. } = mixer.modulation.sources[*idx] {
                        *s = *sustain;
                    }
                }
            }
            ModulationAction::UpdateADSRRelease { idx, release } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { release: ref mut r, .. } = mixer.modulation.sources[*idx] {
                        *r = *release;
                    }
                }
            }
            ModulationAction::TriggerADSR { idx } => {
                mixer.modulation.trigger_adsr(*idx);
            }
            ModulationAction::ReleaseADSR { idx } => {
                mixer.modulation.release_adsr(*idx);
            }
            ModulationAction::UpdateStepValue { idx, step_idx, value } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { steps, .. } = &mut mixer.modulation.sources[*idx] {
                        if *step_idx < steps.len() {
                            steps[*step_idx] = *value;
                        }
                    }
                }
            }
            ModulationAction::UpdateStepRate { idx, rate } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { rate: ref mut r, .. } = mixer.modulation.sources[*idx] {
                        *r = *rate;
                    }
                }
            }
            ModulationAction::UpdateStepInterpolation { idx, interpolation } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { interpolation: ref mut interp, .. } = mixer.modulation.sources[*idx] {
                        *interp = *interpolation;
                    }
                }
            }
            ModulationAction::UpdateStepBipolar { idx, bipolar } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { bipolar: ref mut b, .. } = mixer.modulation.sources[*idx] {
                        *b = *bipolar;
                    }
                }
            }
            ModulationAction::AssignModOnMod { target_source_idx, param_name, modulator_idx, amount } => {
                mixer.modulation.assign_mod_on_mod(*target_source_idx, param_name, *modulator_idx, *amount);
                log::info!("Assigned mod-on-mod: source {} modulates source {} param {} (amount {})", modulator_idx, target_source_idx, param_name, amount);
            }
            ModulationAction::RemoveModOnMod { target_source_idx, param_name } => {
                mixer.modulation.clear_mod_on_mod(*target_source_idx, param_name);
                log::info!("Removed mod-on-mod from source {} param {}", target_source_idx, param_name);
            }
            ModulationAction::AssignModulation { ch_idx, deck_idx, param_name, source_idx, amount } => {
                mixer.modulation.assign(&format!("ch{}_deck{}:{}", ch_idx, deck_idx, param_name), *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to ch{} deck {} param {} with amount {}", source_idx, ch_idx, deck_idx, param_name, amount);
            }
            ModulationAction::RemoveAssignment { ch_idx, deck_idx, param_name, .. } => {
                mixer.modulation.clear_assignments(&format!("ch{}_deck{}:{}", ch_idx, deck_idx, param_name));
                log::info!("Removed modulation assignment from ch{} deck {} param {}", ch_idx, deck_idx, param_name);
            }
            ModulationAction::AssignEffectModulation { ch_idx, deck_idx, effect_idx, param_name, source_idx, amount } => {
                let key = format!("ch{}_deck{}_fx{}:{}", ch_idx, deck_idx, effect_idx, param_name);
                mixer.modulation.assign(&key, *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to ch{} deck {} effect {} param {}", source_idx, ch_idx, deck_idx, effect_idx, param_name);
            }
            ModulationAction::RemoveEffectAssignment { ch_idx, deck_idx, effect_idx, param_name } => {
                let key = format!("ch{}_deck{}_fx{}:{}", ch_idx, deck_idx, effect_idx, param_name);
                mixer.modulation.clear_assignments(&key);
                log::info!("Removed effect modulation from ch{} deck {} effect {} param {}", ch_idx, deck_idx, effect_idx, param_name);
            }
            ModulationAction::AssignChannelEffectModulation { ch_idx, effect_idx, param_name, source_idx, amount } => {
                let key = format!("ch{}_fx{}:{}", ch_idx, effect_idx, param_name);
                mixer.modulation.assign(&key, *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to ch{} channel effect {} param {}", source_idx, ch_idx, effect_idx, param_name);
            }
            ModulationAction::RemoveChannelEffectAssignment { ch_idx, effect_idx, param_name } => {
                let key = format!("ch{}_fx{}:{}", ch_idx, effect_idx, param_name);
                mixer.modulation.clear_assignments(&key);
                log::info!("Removed channel effect modulation from ch{} effect {} param {}", ch_idx, effect_idx, param_name);
            }
            ModulationAction::AssignMasterEffectModulation { effect_idx, param_name, source_idx, amount } => {
                let key = format!("master_fx{}:{}", effect_idx, param_name);
                mixer.modulation.assign(&key, *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to master effect {} param {}", source_idx, effect_idx, param_name);
            }
            ModulationAction::RemoveMasterEffectAssignment { effect_idx, param_name } => {
                let key = format!("master_fx{}:{}", effect_idx, param_name);
                mixer.modulation.clear_assignments(&key);
                log::info!("Removed master effect modulation from effect {} param {}", effect_idx, param_name);
            }
            }
        }
    }
}
