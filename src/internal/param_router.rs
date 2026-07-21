//! Shared parameter path router for external control protocols (MIDI, OSC).
//!
//! Maps path strings like `deck/<uuid>/opacity` or `mod/<uuid>/frequency` to
//! concrete mixer mutations. All values are normalized 0.0–1.0 and scaled
//! to the target parameter's native range.
//!
//! Entities are addressed by **stable UUID**, never positional index — decks,
//! channels, effects, and modulation sources all carry 8-char hex UUIDs (see
//! `/spec/entity-identity.md`). Reordering a chain or rack therefore never
//! retargets a saved binding. Resolution failures return a structured
//! [`ParamRouteError`] so callers can log or surface the specific reason
//! rather than a silent no-op (see `/spec/parameter-routing.md`).

use crate::deck::ScalingMode;
use crate::mixer::Mixer;
use crate::modulation::ModulationSource;
use crate::params::ParamValue;
use crate::video::LoopMode;

/// The class of entity a path segment addresses. Used in [`ParamRouteError`]
/// to describe *what* failed to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Deck,
    Channel,
    Effect,
    Modulator,
    Step,
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EntityKind::Deck => "deck",
            EntityKind::Channel => "channel",
            EntityKind::Effect => "effect",
            EntityKind::Modulator => "modulator",
            EntityKind::Step => "step",
        };
        f.write_str(s)
    }
}

/// Why a parameter path failed to apply. Replaces the previous bare `bool`
/// so MIDI/OSC/API callers can log the specific reason instead of silently
/// dropping the mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamRouteError {
    /// The path did not match any known parameter route.
    UnknownPath { path: String },
    /// A structurally-valid path referenced an entity UUID that does not exist.
    UnknownEntity { kind: EntityKind, id: String },
    /// An index (e.g. a step-sequencer step) is out of range for its container.
    IndexOutOfRange {
        kind: EntityKind,
        index: usize,
        len: usize,
    },
    /// The entity resolved but is in a state that can't accept this mutation
    /// (e.g. a deck with no auto-transition, or a non-step-sequencer modulator).
    WrongState { path: String, reason: &'static str },
    /// The entity resolved but the named sub-parameter is unknown.
    UnknownParam { scope: &'static str, name: String },
}

impl ParamRouteError {
    fn unknown_entity(kind: EntityKind, id: &str) -> Self {
        ParamRouteError::UnknownEntity {
            kind,
            id: id.to_string(),
        }
    }
}

impl std::fmt::Display for ParamRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamRouteError::UnknownPath { path } => write!(f, "unknown parameter path: {path}"),
            ParamRouteError::UnknownEntity { kind, id } => {
                write!(f, "unknown {kind}: {id}")
            }
            ParamRouteError::IndexOutOfRange { kind, index, len } => {
                write!(f, "{kind} index {index} out of range (len {len})")
            }
            ParamRouteError::WrongState { path, reason } => {
                write!(f, "cannot apply {path}: {reason}")
            }
            ParamRouteError::UnknownParam { scope, name } => {
                write!(f, "unknown {scope} param: {name}")
            }
        }
    }
}

impl std::error::Error for ParamRouteError {}

/// Convert an inner "did the mixer op succeed" bool into a `Result`, attributing
/// a `false` to a [`ParamRouteError::WrongState`] with the given reason.
fn ok_or_state(applied: bool, path: &str, reason: &'static str) -> Result<(), ParamRouteError> {
    if applied {
        Ok(())
    } else {
        Err(ParamRouteError::WrongState {
            path: path.to_string(),
            reason,
        })
    }
}

/// Apply a normalized value (0.0–1.0) to the parameter at the given path.
/// Returns `Ok(())` if the path resolved and the mutation was applied, or a
/// [`ParamRouteError`] describing why it did not.
pub fn apply_param_by_path(
    mixer: &mut Mixer,
    path: &str,
    value: f32,
) -> Result<(), ParamRouteError> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["crossfader"] => {
            mixer.snap_crossfader(value);
            Ok(())
        }
        ["deck", uuid, "opacity"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            mixer.channels_mut()[ch].decks[dk].opacity = clamp_or_full(value);
            Ok(())
        }
        ["deck", uuid, "mute"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                let m = mixer.channels_mut()[ch].decks[dk].mute;
                mixer.channels_mut()[ch].decks[dk].mute = !m;
            }
            Ok(())
        }
        ["deck", uuid, "solo"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                let s = mixer.channels_mut()[ch].decks[dk].solo;
                mixer.channels_mut()[ch].decks[dk].solo = !s;
            }
            Ok(())
        }
        ["deck", uuid, "trigger"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                mixer.channels_mut()[ch].decks[dk].opacity = 1.0;
            }
            Ok(())
        }
        ["deck", uuid, "at", "play_duration"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            let at = slot
                .auto_transition
                .as_mut()
                .ok_or(ParamRouteError::WrongState {
                    path: path.to_string(),
                    reason: "deck has no auto-transition",
                })?;
            let max = if at.play_duration.is_beats() {
                128.0
            } else {
                300.0
            };
            at.play_duration.set_value(0.5 + value as f64 * (max - 0.5));
            Ok(())
        }
        ["deck", uuid, "at", "trans_duration"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            let at = slot
                .auto_transition
                .as_mut()
                .ok_or(ParamRouteError::WrongState {
                    path: path.to_string(),
                    reason: "deck has no auto-transition",
                })?;
            let max = if at.transition_duration.is_beats() {
                32.0
            } else {
                30.0
            };
            at.transition_duration
                .set_value(0.1 + value as f64 * (max - 0.1));
            Ok(())
        }
        ["deck", uuid, "video", "play"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .video_set_playing(value > 0.5);
            ok_or_state(applied, path, "deck has no video source")
        }
        ["deck", uuid, "video", "speed"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .video_set_speed(scale_speed(value));
            ok_or_state(applied, path, "deck has no video source")
        }
        ["deck", uuid, "video", "seek"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let deck = &mixer.channels_mut()[ch].decks[dk].deck;
            let snap = deck
                .playback_snapshot()
                .ok_or(ParamRouteError::WrongState {
                    path: path.to_string(),
                    reason: "deck has no playable video",
                })?;
            let applied = deck.video_seek(scale_to_duration(value, snap.duration));
            ok_or_state(applied, path, "video seek failed")
        }
        ["deck", uuid, "video", "in_point"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let deck = &mixer.channels_mut()[ch].decks[dk].deck;
            let snap = deck
                .playback_snapshot()
                .ok_or(ParamRouteError::WrongState {
                    path: path.to_string(),
                    reason: "deck has no playable video",
                })?;
            let applied = deck.video_set_in_point(scale_to_duration(value, snap.duration));
            ok_or_state(applied, path, "set in-point failed")
        }
        ["deck", uuid, "video", "out_point"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let deck = &mixer.channels_mut()[ch].decks[dk].deck;
            let snap = deck
                .playback_snapshot()
                .ok_or(ParamRouteError::WrongState {
                    path: path.to_string(),
                    reason: "deck has no playable video",
                })?;
            let applied = deck.video_set_out_point(scale_to_duration(value, snap.duration));
            ok_or_state(applied, path, "set out-point failed")
        }
        ["deck", uuid, "video", "clear"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                let applied = mixer.channels_mut()[ch].decks[dk]
                    .deck
                    .video_clear_in_out_points();
                ok_or_state(applied, path, "clear in/out points failed")
            } else {
                Ok(())
            }
        }
        ["deck", uuid, "video", "loop_mode"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .video_set_loop_mode(loop_mode_from_value(value));
            ok_or_state(applied, path, "deck has no video source")
        }
        ["deck", uuid, "scaling_mode"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            mixer.channels_mut()[ch].decks[dk]
                .deck
                .set_scaling_mode(scaling_mode_from_value(value));
            Ok(())
        }
        ["deck", uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            apply_float_param_scaled(
                &mut mixer.channels_mut()[ch].decks[dk].deck.generator_params,
                name,
                value,
            );
            Ok(())
        }
        ["deck", uuid, "effect", fx_uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let effects = &mut mixer.channels_mut()[ch].decks[dk].deck.effects;
            let ek = effects
                .iter()
                .position(|e| e.uuid == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_float_param_scaled(&mut effects[ek].params, name, value);
            Ok(())
        }
        ["ch", ch_uuid, "opacity"] => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            mixer.channels_mut()[ch].opacity = clamp_or_full(value);
            Ok(())
        }
        ["ch", ch_uuid, "effect", fx_uuid, "param", name] => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            let effects = &mut mixer.channels_mut()[ch].effects;
            let ek = effects
                .iter()
                .position(|e| e.uuid == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_float_param_scaled(&mut effects[ek].params, name, value);
            Ok(())
        }
        ["master", "effect", fx_uuid, "param", name] => {
            let effects = mixer.master_effects_mut();
            let ek = effects
                .iter()
                .position(|e| e.uuid == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_float_param_scaled(&mut effects[ek].params, name, value);
            Ok(())
        }
        ["mod", mod_uuid, "step", step_s] => {
            let step_idx = step_s
                .parse::<usize>()
                .map_err(|_| ParamRouteError::UnknownPath {
                    path: path.to_string(),
                })?;
            let entry = mixer
                .modulation_mut()
                .find_source_by_uuid_mut(mod_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Modulator, mod_uuid))?;
            if let ModulationSource::StepSequencer { steps, .. } = &mut entry.source {
                if step_idx < steps.len() {
                    steps[step_idx] = value.clamp(0.0, 1.0);
                    Ok(())
                } else {
                    Err(ParamRouteError::IndexOutOfRange {
                        kind: EntityKind::Step,
                        index: step_idx,
                        len: steps.len(),
                    })
                }
            } else {
                Err(ParamRouteError::WrongState {
                    path: path.to_string(),
                    reason: "modulator is not a step sequencer",
                })
            }
        }
        ["mod", mod_uuid, param_name] => {
            let entry = mixer
                .modulation_mut()
                .find_source_by_uuid_mut(mod_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Modulator, mod_uuid))?;
            apply_mod_param(&mut entry.source, param_name, value)
        }
        _ => Err(ParamRouteError::UnknownPath {
            path: path.to_string(),
        }),
    }
}

/// Apply a typed [`ParamValue`] to the parameter at the given path.
///
/// For the shader/effect **param** paths this preserves the value's type —
/// `Color`/`Point2D`/`Bool`/`Long` are written intact, and `Float` is
/// normalized-scaled against the ISF definition exactly as the fader path does.
/// Every other (inherently scalar) path delegates to [`apply_param_by_path`]
/// after flattening the value to a normalized f32.
///
/// This is the entry point for the engine `set_param` trait; MIDI/OSC continue
/// to use the normalized-f32 [`apply_param_by_path`].
pub fn apply_typed_param_by_path(
    mixer: &mut Mixer,
    path: &str,
    value: ParamValue,
) -> Result<(), ParamRouteError> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["deck", uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            apply_typed_param(
                &mut mixer.channels_mut()[ch].decks[dk].deck.generator_params,
                name,
                value,
            );
            Ok(())
        }
        ["deck", uuid, "effect", fx_uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let effects = &mut mixer.channels_mut()[ch].decks[dk].deck.effects;
            let ek = effects
                .iter()
                .position(|e| e.uuid == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_typed_param(&mut effects[ek].params, name, value);
            Ok(())
        }
        ["ch", ch_uuid, "effect", fx_uuid, "param", name] => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            let effects = &mut mixer.channels_mut()[ch].effects;
            let ek = effects
                .iter()
                .position(|e| e.uuid == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_typed_param(&mut effects[ek].params, name, value);
            Ok(())
        }
        ["master", "effect", fx_uuid, "param", name] => {
            let effects = mixer.master_effects_mut();
            let ek = effects
                .iter()
                .position(|e| e.uuid == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_typed_param(&mut effects[ek].params, name, value);
            Ok(())
        }
        // Inherently-scalar paths (opacity, crossfader, video, mod, …): flatten.
        _ => apply_param_by_path(mixer, path, param_value_to_norm_f32(&value)),
    }
}

/// Flatten a [`ParamValue`] to the normalized f32 the scalar router expects.
/// Non-scalar values collapse to their first component (colors → R, points → x);
/// this is only used for paths that are inherently scalar.
pub fn param_value_to_norm_f32(value: &ParamValue) -> f32 {
    match value {
        ParamValue::Float(v) => *v,
        ParamValue::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        ParamValue::Long(i) => *i as f32,
        ParamValue::Color(c) => c[0],
        ParamValue::Point2D(p) => p[0],
    }
}

/// Set a typed value on a shader param: `Float` is normalized-scaled against the
/// param definition (matching the fader path); every other variant is written
/// as-is so colors and 2D points keep their type and full channel data.
fn apply_typed_param(params: &mut crate::ShaderParams, name: &str, value: ParamValue) {
    match value {
        ParamValue::Float(v) => apply_float_param_scaled(params, name, v),
        other => params.set(name, other),
    }
}

/// Apply a normalized value to a modulation source parameter.
fn apply_mod_param(
    source: &mut ModulationSource,
    param_name: &str,
    value: f32,
) -> Result<(), ParamRouteError> {
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
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "LFO",
                    name: param_name.to_string(),
                })
            }
        },
        ModulationSource::AudioBand {
            freq_low,
            freq_high,
            gain,
            smoothing,
            noise_gate,
            ..
        } => match param_name {
            // Native ranges match the UI sliders (see modulation panel).
            "freq_low" => *freq_low = 20.0 + clamp_norm(value) * (20000.0 - 20.0),
            "freq_high" => *freq_high = 20.0 + clamp_norm(value) * (20000.0 - 20.0),
            "gain" => *gain = clamp_norm(value) * 4.0,
            "smoothing" => *smoothing = (value * 0.99).clamp(0.0, 0.99),
            "noise_gate" => *noise_gate = clamp_norm(value) * 0.5,
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "Audio",
                    name: param_name.to_string(),
                })
            }
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
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "ADSR",
                    name: param_name.to_string(),
                })
            }
        },
        ModulationSource::StepSequencer { rate, .. } => match param_name {
            "rate" => *rate = 0.1 + value * 19.9,
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "StepSeq",
                    name: param_name.to_string(),
                })
            }
        },
        ModulationSource::Analyzer { smoothing, .. } => match param_name {
            "smoothing" => *smoothing = (value * 0.99).clamp(0.0, 0.99),
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "Analyzer",
                    name: param_name.to_string(),
                })
            }
        },
    }
    Ok(())
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

/// Clamp a value to 0.0–1.0, treating non-finite input as `1.0` (used for
/// opacity, where a garbled value should fail safe to fully-visible).
fn clamp_or_full(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
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

    // ── apply_mod_param: structured-result behavior (no GPU) ──────────

    #[test]
    fn mod_param_known_returns_ok_and_sets_value() {
        let mut src = ModulationSource::sine_lfo(1.0);
        assert!(apply_mod_param(&mut src, "frequency", 0.5).is_ok());
        if let ModulationSource::LFO { frequency, .. } = src {
            // 0.01 + 0.5 * 9.99 = 5.005
            assert!((frequency - 5.005).abs() < 1e-4, "frequency = {frequency}");
        } else {
            panic!("expected LFO");
        }
    }

    #[test]
    fn mod_param_unknown_returns_unknown_param() {
        let mut src = ModulationSource::sine_lfo(1.0);
        let err = apply_mod_param(&mut src, "bogus", 0.5).unwrap_err();
        assert_eq!(
            err,
            ParamRouteError::UnknownParam {
                scope: "LFO",
                name: "bogus".to_string(),
            }
        );
    }

    #[test]
    fn mod_param_audio_band_params_are_routable() {
        // Regression: freq_low/freq_high/gain/noise_gate were previously silent
        // no-ops (only `smoothing` was handled).
        let mut src = ModulationSource::audio_from_preset(crate::modulation::AudioBandPreset::Low);
        assert!(apply_mod_param(&mut src, "freq_low", 0.0).is_ok());
        assert!(apply_mod_param(&mut src, "gain", 1.0).is_ok());
        assert!(apply_mod_param(&mut src, "noise_gate", 1.0).is_ok());
        if let ModulationSource::AudioBand {
            freq_low,
            gain,
            noise_gate,
            ..
        } = src
        {
            assert!((freq_low - 20.0).abs() < 1e-3, "freq_low = {freq_low}");
            assert!((gain - 4.0).abs() < 1e-4, "gain = {gain}");
            assert!((noise_gate - 0.5).abs() < 1e-4, "noise_gate = {noise_gate}");
        } else {
            panic!("expected AudioBand");
        }
    }

    // ── WS3(a): typed value path preserves non-scalar params ──────────

    fn color_params() -> crate::ShaderParams {
        let input: crate::isf::ISFInput = serde_json::from_value(serde_json::json!({
            "NAME": "tint",
            "TYPE": "color",
            "DEFAULT": [0.0, 0.0, 0.0, 1.0],
        }))
        .unwrap();
        crate::ShaderParams::from_inputs(&[input])
    }

    #[test]
    fn typed_param_preserves_color_channels() {
        let mut params = color_params();
        apply_typed_param(&mut params, "tint", ParamValue::Color([0.1, 0.2, 0.3, 0.4]));
        match params.values.get("tint") {
            Some(ParamValue::Color(c)) => {
                assert_eq!(*c, [0.1, 0.2, 0.3, 0.4], "all channels must survive");
            }
            other => panic!("expected Color, got {other:?}"),
        }
    }

    #[test]
    fn param_value_to_norm_f32_flattens_all_variants() {
        assert_eq!(param_value_to_norm_f32(&ParamValue::Float(0.7)), 0.7);
        assert_eq!(param_value_to_norm_f32(&ParamValue::Bool(true)), 1.0);
        assert_eq!(param_value_to_norm_f32(&ParamValue::Bool(false)), 0.0);
        assert_eq!(param_value_to_norm_f32(&ParamValue::Long(3)), 3.0);
        assert_eq!(
            param_value_to_norm_f32(&ParamValue::Color([0.9, 0.1, 0.2, 1.0])),
            0.9
        );
        assert_eq!(
            param_value_to_norm_f32(&ParamValue::Point2D([0.25, 0.75])),
            0.25
        );
    }

    #[test]
    fn error_display_is_human_readable() {
        let e = ParamRouteError::unknown_entity(EntityKind::Deck, "abc123");
        assert_eq!(e.to_string(), "unknown deck: abc123");
        let e = ParamRouteError::IndexOutOfRange {
            kind: EntityKind::Step,
            index: 9,
            len: 8,
        };
        assert_eq!(e.to_string(), "step index 9 out of range (len 8)");
        let e = ParamRouteError::UnknownPath {
            path: "foo/bar".to_string(),
        };
        assert_eq!(e.to_string(), "unknown parameter path: foo/bar");
    }
}
