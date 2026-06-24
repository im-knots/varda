//! Shared parameter path router for external control protocols (MIDI, OSC).
//!
//! Maps path strings like `deck/<uuid>/opacity` or `mod/0/frequency` to
//! concrete mixer mutations. All values are normalized 0.0–1.0 and scaled
//! to the target parameter's native range.

use crate::deck::ScalingMode;
use crate::mixer::Mixer;
use crate::modulation::ModulationSource;
use crate::params::ParamValue;
use crate::video::LoopMode;

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
        ["deck", uuid, "video", "play"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                return mixer.channels_mut()[ch].decks[dk]
                    .deck
                    .video_set_playing(value > 0.5);
            }
            false
        }
        ["deck", uuid, "video", "speed"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                return mixer.channels_mut()[ch].decks[dk]
                    .deck
                    .video_set_speed(scale_speed(value));
            }
            false
        }
        ["deck", uuid, "video", "seek"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                let deck = &mixer.channels_mut()[ch].decks[dk].deck;
                if let Some(snap) = deck.playback_snapshot() {
                    return deck.video_seek(scale_to_duration(value, snap.duration));
                }
            }
            false
        }
        ["deck", uuid, "video", "in_point"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                let deck = &mixer.channels_mut()[ch].decks[dk].deck;
                if let Some(snap) = deck.playback_snapshot() {
                    return deck.video_set_in_point(scale_to_duration(value, snap.duration));
                }
            }
            false
        }
        ["deck", uuid, "video", "out_point"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                let deck = &mixer.channels_mut()[ch].decks[dk].deck;
                if let Some(snap) = deck.playback_snapshot() {
                    return deck.video_set_out_point(scale_to_duration(value, snap.duration));
                }
            }
            false
        }
        ["deck", uuid, "video", "clear"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                if value > 0.5 {
                    return mixer.channels_mut()[ch].decks[dk]
                        .deck
                        .video_clear_in_out_points();
                }
                return true;
            }
            false
        }
        ["deck", uuid, "video", "loop_mode"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                return mixer.channels_mut()[ch].decks[dk]
                    .deck
                    .video_set_loop_mode(loop_mode_from_value(value));
            }
            false
        }
        ["deck", uuid, "scaling_mode"] => {
            if let Some((ch, dk)) = mixer.find_deck_by_uuid(uuid) {
                mixer.channels_mut()[ch].decks[dk]
                    .deck
                    .set_scaling_mode(scaling_mode_from_value(value));
                return true;
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

/// Clamp a normalized value to 0.0–1.0, treating non-finite input as 0.0.
fn clamp_norm(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Map a normalized 0.0–1.0 value to a discrete variant index via fader bucketing.
/// Splits the range into `n` equal segments: `index = min(floor(value * n), n - 1)`.
fn bucket_index(value: f32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    ((clamp_norm(value) * n as f32).floor() as usize).min(n - 1)
}

/// Map a normalized value to a video playback speed multiplier (0.1×–4.0×).
fn scale_speed(value: f32) -> f64 {
    (0.1 + clamp_norm(value) * 3.9) as f64
}

/// Scale a normalized value to an absolute time in seconds against a clip duration.
fn scale_to_duration(value: f32, duration: f64) -> f64 {
    clamp_norm(value) as f64 * duration.max(0.0)
}

/// Map a normalized value to a `LoopMode` via fader bucketing.
fn loop_mode_from_value(value: f32) -> LoopMode {
    match bucket_index(value, 4) {
        0 => LoopMode::Loop,
        1 => LoopMode::PingPong,
        2 => LoopMode::OneShot,
        _ => LoopMode::HoldLast,
    }
}

/// Map a normalized value to a `ScalingMode` via fader bucketing.
fn scaling_mode_from_value(value: f32) -> ScalingMode {
    match bucket_index(value, 4) {
        0 => ScalingMode::Fill,
        1 => ScalingMode::Fit,
        2 => ScalingMode::Stretch,
        _ => ScalingMode::Center,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_index_splits_range_evenly() {
        assert_eq!(bucket_index(0.0, 4), 0);
        assert_eq!(bucket_index(0.1, 4), 0);
        assert_eq!(bucket_index(0.25, 4), 1);
        assert_eq!(bucket_index(0.4, 4), 1);
        assert_eq!(bucket_index(0.5, 4), 2);
        assert_eq!(bucket_index(0.74, 4), 2);
        assert_eq!(bucket_index(0.75, 4), 3);
        assert_eq!(bucket_index(1.0, 4), 3);
    }

    #[test]
    fn bucket_index_clamps_out_of_range() {
        assert_eq!(bucket_index(-1.0, 4), 0);
        assert_eq!(bucket_index(2.0, 4), 3);
        assert_eq!(bucket_index(f32::NAN, 4), 0);
        assert_eq!(bucket_index(0.5, 0), 0);
    }

    #[test]
    fn scale_speed_maps_to_range() {
        assert!((scale_speed(0.0) - 0.1).abs() < 1e-6);
        assert!((scale_speed(1.0) - 4.0).abs() < 1e-6);
        assert!((scale_speed(0.5) - 2.05).abs() < 1e-6);
        assert!((scale_speed(2.0) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn scale_to_duration_scales_against_clip() {
        assert!((scale_to_duration(0.0, 10.0) - 0.0).abs() < 1e-9);
        assert!((scale_to_duration(1.0, 10.0) - 10.0).abs() < 1e-9);
        assert!((scale_to_duration(0.5, 10.0) - 5.0).abs() < 1e-9);
        assert!((scale_to_duration(0.5, -4.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn loop_mode_buckets() {
        assert_eq!(loop_mode_from_value(0.0), LoopMode::Loop);
        assert_eq!(loop_mode_from_value(0.3), LoopMode::PingPong);
        assert_eq!(loop_mode_from_value(0.6), LoopMode::OneShot);
        assert_eq!(loop_mode_from_value(1.0), LoopMode::HoldLast);
    }

    #[test]
    fn scaling_mode_buckets() {
        assert_eq!(scaling_mode_from_value(0.0), ScalingMode::Fill);
        assert_eq!(scaling_mode_from_value(0.3), ScalingMode::Fit);
        assert_eq!(scaling_mode_from_value(0.6), ScalingMode::Stretch);
        assert_eq!(scaling_mode_from_value(1.0), ScalingMode::Center);
    }
}
