//! Shared parameter path router for external control protocols (MIDI, OSC).
//!
//! Maps path strings like `deck/<uuid>/opacity` or `mod/0/frequency` to
//! concrete mixer mutations. All values are normalized 0.0–1.0 and scaled
//! to the target parameter's native range.

use crate::mixer::Mixer;
use crate::modulation::ModulationSource;
use crate::params::ParamValue;

/// Apply a normalized value (0.0–1.0) to the parameter at the given path.
/// Returns true if the path resolved successfully.
pub fn apply_param_by_path(mixer: &mut Mixer, path: &str, value: f32) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["crossfader"] => {
            mixer.snap_crossfader(value);
            true
        }
        ["deck", uuid, "opacity"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                let v = if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    1.0
                };
                mixer.channels_mut()[ch].decks[dk].opacity = v;
                return true;
            }
            false
        }
        ["deck", uuid, "mute"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if value > 0.5 {
                    let m = mixer.channels_mut()[ch].decks[dk].mute;
                    mixer.channels_mut()[ch].decks[dk].mute = !m;
                }
                return true;
            }
            false
        }
        ["deck", uuid, "solo"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if value > 0.5 {
                    let s = mixer.channels_mut()[ch].decks[dk].solo;
                    mixer.channels_mut()[ch].decks[dk].solo = !s;
                }
                return true;
            }
            false
        }
        ["deck", uuid, "trigger"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if value > 0.5 {
                    mixer.channels_mut()[ch].decks[dk].opacity = 1.0;
                }
                return true;
            }
            false
        }
        ["deck", uuid, "at", "play_duration"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if let Some(ref mut at) = mixer.channels_mut()[ch].decks[dk].auto_transition {
                    let max = if at.play_duration.is_beats() {
                        128.0
                    } else {
                        300.0
                    };
                    at.play_duration.set_value(0.5 + value as f64 * (max - 0.5));
                    return true;
                }
            }
            false
        }
        ["deck", uuid, "at", "trans_duration"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if let Some(ref mut at) = mixer.channels_mut()[ch].decks[dk].auto_transition {
                    let max = if at.transition_duration.is_beats() {
                        32.0
                    } else {
                        30.0
                    };
                    at.transition_duration
                        .set_value(0.1 + value as f64 * (max - 0.1));
                    return true;
                }
            }
            false
        }
        ["deck", uuid, "param", name] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                apply_float_param_scaled(
                    &mut mixer.channels_mut()[ch].decks[dk].deck.generator_params,
                    name,
                    value,
                );
                return true;
            }
            false
        }
        ["deck", uuid, "effect", ek_s, "param", name] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if let Ok(ek) = ek_s.parse::<usize>() {
                    let decks = &mut mixer.channels_mut()[ch].decks;
                    if ek < decks[dk].deck.effects.len() {
                        apply_float_param_scaled(
                            &mut decks[dk].deck.effects[ek].params,
                            name,
                            value,
                        );
                        return true;
                    }
                }
            }
            false
        }
        ["ch", ch_uuid, "opacity"] => {
            if let Some(ch) = mixer.find_channel_by_uuid(ch_uuid) {
                let v = if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    1.0
                };
                mixer.channels_mut()[ch].opacity = v;
                return true;
            }
            false
        }
        ["ch", ch_uuid, "effect", ek_s, "param", name] => {
            if let Some(ch) = mixer.find_channel_by_uuid(ch_uuid) {
                if let Ok(ek) = ek_s.parse::<usize>() {
                    if ek < mixer.channels_mut()[ch].effects.len() {
                        apply_float_param_scaled(
                            &mut mixer.channels_mut()[ch].effects[ek].params,
                            name,
                            value,
                        );
                        return true;
                    }
                }
            }
            false
        }
        ["master", "effect", ek_s, "param", name] => {
            if let Ok(ek) = ek_s.parse::<usize>() {
                if ek < mixer.master_effects().len() {
                    apply_float_param_scaled(
                        &mut mixer.master_effects_mut()[ek].params,
                        name,
                        value,
                    );
                    return true;
                }
            }
            false
        }
        ["mod", idx_s, param_name] => {
            if let Ok(idx) = idx_s.parse::<usize>() {
                if idx < mixer.modulation_mut().sources.len() {
                    apply_mod_param(
                        &mut mixer.modulation_mut().sources[idx].source,
                        param_name,
                        value,
                    );
                    return true;
                }
            }
            false
        }
        ["mod", idx_s, "step", step_s] => {
            if let (Ok(idx), Ok(step_idx)) = (idx_s.parse::<usize>(), step_s.parse::<usize>()) {
                if idx < mixer.modulation_mut().sources.len() {
                    if let ModulationSource::StepSequencer { steps, .. } =
                        &mut mixer.modulation_mut().sources[idx].source
                    {
                        if step_idx < steps.len() {
                            steps[step_idx] = value.clamp(0.0, 1.0);
                            return true;
                        }
                    }
                }
            }
            false
        }
        _ => {
            log::warn!("Unknown parameter path: {}", path);
            false
        }
    }
}

/// Apply a normalized value to a modulation source parameter.
fn apply_mod_param(source: &mut ModulationSource, param_name: &str, value: f32) {
    match source {
        ModulationSource::LFO {
            frequency,
            amplitude,
            phase,
            ..
        } => match param_name {
            "frequency" => *frequency = 0.01 + value * 9.99,
            "amplitude" => *amplitude = value.clamp(0.0, 1.0),
            "phase" => *phase = value.clamp(0.0, 1.0),
            _ => log::warn!("Unknown LFO param: {}", param_name),
        },
        ModulationSource::AudioBand { smoothing, .. } => match param_name {
            "smoothing" => *smoothing = (value * 0.99).clamp(0.0, 0.99),
            _ => log::warn!("Unknown Audio param: {}", param_name),
        },
        ModulationSource::ADSR {
            attack,
            decay,
            sustain,
            release,
            ..
        } => match param_name {
            "attack" => *attack = 0.001 + value * 4.999,
            "decay" => *decay = 0.001 + value * 4.999,
            "sustain" => *sustain = value.clamp(0.0, 1.0),
            "release" => *release = 0.001 + value * 4.999,
            "gate" => {
                if value > 0.5 {
                    source.gate_on();
                } else {
                    source.gate_off();
                }
            }
            _ => log::warn!("Unknown ADSR param: {}", param_name),
        },
        ModulationSource::StepSequencer { rate, .. } => match param_name {
            "rate" => *rate = 0.1 + value * 19.9,
            _ => log::warn!("Unknown StepSeq param: {}", param_name),
        },
        ModulationSource::Analyzer { smoothing, .. } => match param_name {
            "smoothing" => *smoothing = (value * 0.99).clamp(0.0, 0.99),
            _ => log::warn!("Unknown Analyzer param: {}", param_name),
        },
    }
}

/// Apply a normalized 0.0–1.0 value to a float param, scaling to the param's min/max range.
fn apply_float_param_scaled(params: &mut crate::ShaderParams, name: &str, normalized: f32) {
    if let Some(def) = params.definitions.get(name) {
        let min = def.min.unwrap_or(0.0);
        let max = def.max.unwrap_or(1.0);
        let scaled = min + normalized * (max - min);
        params.set(name, ParamValue::Float(scaled));
    } else {
        params.set(name, ParamValue::Float(normalized));
    }
}
