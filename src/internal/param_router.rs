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
    Macro,
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EntityKind::Deck => "deck",
            EntityKind::Channel => "channel",
            EntityKind::Effect => "effect",
            EntityKind::Modulator => "modulator",
            EntityKind::Step => "step",
            EntityKind::Macro => "macro",
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

/// The modulation engine's key for a router path, when the two address the
/// same parameter.
///
/// Routes use slashes and the modulation graph uses `prefix:name`, so the live
/// override (which arrives as a path from OSC, MIDI, or the API) needs a
/// translation to find out whether it just landed on an automated parameter.
/// Returns `None` for routes nothing can be assigned to, such as triggers.
///
/// The video arms map an action onto a state: the route `video/seek` means "seek
/// to here", while the modulation target `video_position` means "the playhead is
/// here". In and out points stay unassignable on purpose. They define the loop
/// region that a position offset is scaled against, so modulating them would
/// make position modulation depend on its own scaling reference.
///
/// See /spec/arrangement.md § Live override and
/// /spec/video-playback-modulation.md § Router and Addressing.
pub fn modulation_key_for_path(path: &str) -> Option<String> {
    use crate::video::modulation as vm;
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["deck", uuid, "opacity"] => Some(format!("deck_{uuid}:opacity")),
        ["deck", uuid, "param", name] => Some(format!("deck_{uuid}:{name}")),
        ["ch", uuid, "opacity"] => Some(format!("ch_{uuid}:opacity")),
        ["deck" | "ch", _, "effect", fx, "param", name]
        | ["master", "effect", fx, "param", name] => Some(format!("fx_{fx}:{name}")),
        ["deck", uuid, "video", "speed"] => Some(format!("deck_{uuid}:{}", vm::SPEED)),
        ["deck", uuid, "video", "seek"] => Some(format!("deck_{uuid}:{}", vm::POSITION)),
        ["deck", uuid, "video", "play"] => Some(format!("deck_{uuid}:{}", vm::PLAY)),
        ["deck", uuid, "video", "loop_mode"] => Some(format!("deck_{uuid}:{}", vm::LOOP_MODE)),
        ["deck", uuid, "scaling_mode"] => Some(format!("deck_{uuid}:{}", vm::SCALING_MODE)),
        _ => None,
    }
}

/// Apply a normalized value (0.0–1.0) to the parameter at the given path.
/// Returns `Ok(())` if the path resolved and the mutation was applied, or a
/// [`ParamRouteError`] describing why it did not.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownPath`] if `path` matches no route,
/// [`ParamRouteError::UnknownEntity`] if a UUID or index in the path does not
/// resolve to a live channel/deck/effect/macro,
/// [`ParamRouteError::IndexOutOfRange`] if an index exceeds its container,
/// [`ParamRouteError::UnknownParam`] if the named sub-parameter does not exist,
/// and [`ParamRouteError::WrongState`] if the route resolved but the entity is
/// in a state that cannot accept the mutation.
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
            at.play_duration
                .set_value(0.5 + f64::from(value) * (max - 0.5));
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
                .set_value(0.1 + f64::from(value) * (max - 0.1));
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
        // Screen/window capture params. The deck holds the desired config and
        // the render loop pushes it to the capture manager, so every path here
        // is MIDI-learnable, OSC-addressable, and macro-drivable.
        //
        // Not modulation targets: a route makes a value writable on demand,
        // while a modulation target is re-evaluated every frame, which each
        // consumer has to opt into (see `modulation_key_for_path`). Whether
        // they should be is an open question in
        // spec/video-playback-modulation.md.
        // See spec/screen-capture.md § Parameters and Router Paths.
        ["deck", uuid, "capture", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .set_capture_param(name, clamp_norm(value));
            ok_or_state(applied, path, "deck is not a screen capture source")
        }
        ["deck", uuid, "depth", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .set_depth_param(name, clamp_norm(value));
            ok_or_state(applied, path, "deck is not a depth sensor source")
        }
        // Depth-sensor *preprocessor* params, distinct from the point-cloud
        // params above: these configure the fields fed to a shader that declared
        // `depth_sensor`. See spec/depth-sensor-preprocessor.md.
        ["deck", uuid, "depth_prepro", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .set_depth_prepro_param(name, clamp_norm(value));
            ok_or_state(applied, path, "deck has no depth-sensor preprocessor")
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
                .position(|e| e.uuid() == *fx_uuid)
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
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_float_param_scaled(&mut effects[ek].params, name, value);
            Ok(())
        }
        ["master", "effect", fx_uuid, "param", name] => {
            let effects = mixer.master_effects_mut();
            let ek = effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
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
        ["macro", macro_uuid, "value"] => {
            // Feed the macro; it returns the parameter writes to fan out. Global
            // app actions (undo/save/tap) are queued on the bank for the app layer
            // to drain (see app/inputs.rs). Targets are never `macro/*` paths
            // (filtered in the fan-out), so recursion depth is bounded at 1.
            let fanout = mixer
                .macros_mut()
                .apply_input(macro_uuid, value)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Macro, macro_uuid))?;
            for (target_path, target_value) in fanout {
                if let Err(e) = apply_param_by_path(mixer, &target_path, target_value) {
                    // A macro target may reference a deleted/absent entity; log at
                    // debug rather than failing the whole macro turn.
                    log::debug!("macro {macro_uuid} target '{target_path}' skipped: {e}");
                }
            }
            Ok(())
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
///
/// # Errors
///
/// Same failure modes as [`apply_param_by_path`]: an unknown path, a UUID/index
/// that no longer resolves, or a resolved route that the mixer refused.
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
            )
        }
        ["deck", uuid, "effect", fx_uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let effects = &mut mixer.channels_mut()[ch].decks[dk].deck.effects;
            let ek = effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_typed_param(&mut effects[ek].params, name, value)
        }
        ["ch", ch_uuid, "effect", fx_uuid, "param", name] => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            let effects = &mut mixer.channels_mut()[ch].effects;
            let ek = effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_typed_param(&mut effects[ek].params, name, value)
        }
        ["master", "effect", fx_uuid, "param", name] => {
            let effects = mixer.master_effects_mut();
            let ek = effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            apply_typed_param(&mut effects[ek].params, name, value)
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

/// Set a typed value on a shader param, coerced to the param's declared ISF type.
///
/// The stored value's variant *is* the declared type, and it decides how the
/// incoming value applies: a `long` takes a discrete choice index, a `bool` a
/// flag, a `float` a normalized 0.0–1.0 fraction scaled against the param's
/// range (matching the fader path). `Color` and `Point2D` keep their full
/// channel data.
///
/// [`ParamValue`] is `#[serde(untagged)]` with `Float` listed first, so every JSON number an API
/// client sends deserializes as `Float` whatever the param really is. Writing
/// that straight into a `long` leaves the shader reading a float's bit pattern
/// as an integer: `2.0` arrives as 1073741824, no `mode ==` branch matches, and
/// the effect silently passes its input through. Only index 0 ever worked,
/// because `0.0f32` and `0i32` share a bit pattern.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownParam`] if the shader has no param by that
/// name, and [`ParamRouteError::WrongState`] if a scalar is aimed at a `color`
/// or `point2D` param, rather than dropping either write silently.
fn apply_typed_param(
    params: &mut crate::ShaderParams,
    name: &str,
    value: ParamValue,
) -> Result<(), ParamRouteError> {
    let Some(declared) = params.values.get(name) else {
        return Err(ParamRouteError::UnknownParam {
            scope: "shader",
            name: name.to_string(),
        });
    };
    match declared {
        ParamValue::Long(_) => params.set(name, ParamValue::Long(param_value_to_index(&value))),
        ParamValue::Bool(_) => params.set(name, ParamValue::Bool(param_value_to_bool(&value))),
        ParamValue::Float(_) => {
            apply_float_param_scaled(params, name, param_value_to_norm_f32(&value));
        }
        // Color and point params carry per-channel data of a fixed width, so
        // only the matching variant is written: a scalar cannot describe one,
        // and a 4-channel color is not a 2-channel point.
        ParamValue::Color(_) => {
            let ParamValue::Color(c) = value else {
                return Err(ParamRouteError::WrongState {
                    path: name.to_string(),
                    reason: "only a color value can set a color parameter",
                });
            };
            params.set(name, ParamValue::Color(c));
        }
        ParamValue::Point2D(_) => {
            let ParamValue::Point2D(p) = value else {
                return Err(ParamRouteError::WrongState {
                    path: name.to_string(),
                    reason: "only a point2D value can set a point2D parameter",
                });
            };
            params.set(name, ParamValue::Point2D(p));
        }
    }
    Ok(())
}

/// Flatten a [`ParamValue`] to the discrete index a `long` param stores.
/// Floats round to nearest so a fader landing on 1.999 still selects variant 2.
fn param_value_to_index(value: &ParamValue) -> i32 {
    match value {
        ParamValue::Long(i) => *i,
        ParamValue::Bool(b) => i32::from(*b),
        ParamValue::Float(v) => {
            if v.is_finite() {
                v.round() as i32
            } else {
                0
            }
        }
        ParamValue::Color(c) => c[0] as i32,
        ParamValue::Point2D(p) => p[0] as i32,
    }
}

/// Flatten a [`ParamValue`] to the flag a `bool` param stores.
fn param_value_to_bool(value: &ParamValue) -> bool {
    match value {
        ParamValue::Bool(b) => *b,
        other => param_value_to_norm_f32(other) >= 0.5,
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
                });
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
                });
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
                });
            }
        },
        ModulationSource::StepSequencer { rate, .. } => match param_name {
            "rate" => *rate = 0.1 + value * 19.9,
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "StepSeq",
                    name: param_name.to_string(),
                });
            }
        },
        ModulationSource::Analyzer { smoothing, .. } => match param_name {
            "smoothing" => *smoothing = (value * 0.99).clamp(0.0, 0.99),
            _ => {
                return Err(ParamRouteError::UnknownParam {
                    scope: "Analyzer",
                    name: param_name.to_string(),
                });
            }
        },
        // An envelope's shape is its breakpoints, which are edited as a curve
        // rather than driven as a scalar. Keeping it out of the router is what
        // keeps envelope parameters unmodulatable, which /spec/automation.md
        // § Performance relies on.
        ModulationSource::Envelope { .. } => {
            return Err(ParamRouteError::UnknownParam {
                scope: "Envelope",
                name: param_name.to_string(),
            });
        }
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
    f64::from(0.1 + clamp_norm(value) * 3.9)
}

/// Scale a normalized value to an absolute time in seconds against a clip duration.
fn scale_to_duration(value: f32, duration: f64) -> f64 {
    f64::from(clamp_norm(value)) * duration.max(0.0)
}

/// The normalized value at the centre of bucket `index` of `n`.
///
/// Inverse of [`bucket_index`] in the only sense a bucketing has one: it returns
/// a value that maps back to the same bucket, and picks the centre so rounding
/// at either edge cannot land in a neighbour.
fn bucket_center(index: usize, n: usize) -> f32 {
    if n == 0 {
        return 0.0;
    }
    (index.min(n - 1) as f32 + 0.5) / n as f32
}

/// Inverse of [`scale_speed`]: the normalized value a fader or curve would need
/// to hold to produce `speed`.
pub(crate) fn speed_to_norm(speed: f64) -> f32 {
    clamp_norm(((speed - 0.1) / 3.9) as f32)
}

/// Inverse of [`scale_to_duration`]. A clip of unknown length has no meaningful
/// normalized position, so it reports the start rather than dividing by zero.
pub(crate) fn duration_to_norm(secs: f64, duration: f64) -> f32 {
    if duration <= 0.0 {
        return 0.0;
    }
    clamp_norm((secs / duration) as f32)
}

/// Inverse of [`loop_mode_from_value`].
pub(crate) fn loop_mode_to_value(mode: LoopMode) -> f32 {
    let index = match mode {
        LoopMode::Loop => 0,
        LoopMode::PingPong => 1,
        LoopMode::OneShot => 2,
        LoopMode::HoldLast => 3,
    };
    bucket_center(index, 4)
}

/// Inverse of [`scaling_mode_from_value`].
pub(crate) fn scaling_mode_to_value(mode: ScalingMode) -> f32 {
    let index = match mode {
        ScalingMode::Fill => 0,
        ScalingMode::Fit => 1,
        ScalingMode::Stretch => 2,
        ScalingMode::Center => 3,
    };
    bucket_center(index, 4)
}

/// Map a normalized value to a `LoopMode` via fader bucketing.
pub(crate) fn loop_mode_from_value(value: f32) -> LoopMode {
    match bucket_index(value, 4) {
        0 => LoopMode::Loop,
        1 => LoopMode::PingPong,
        2 => LoopMode::OneShot,
        _ => LoopMode::HoldLast,
    }
}

/// Map a normalized value to a `ScalingMode` via fader bucketing.
pub(crate) fn scaling_mode_from_value(value: f32) -> ScalingMode {
    match bucket_index(value, 4) {
        0 => ScalingMode::Fill,
        1 => ScalingMode::Fit,
        2 => ScalingMode::Stretch,
        _ => ScalingMode::Center,
    }
}

/// Toggle a parameter between its two extremes (keyboard shortcut affordance).
///
/// Floats snap 0↔1. Bools invert. Mute/solo flip. Trigger forces opacity to 1.0.
/// Modulation paths are rejected: those values are continuous, not two-state.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownPath`] if `path` matches no toggle route,
/// [`ParamRouteError::UnknownEntity`] if a UUID in the path does not resolve,
/// [`ParamRouteError::UnknownParam`] if the named shader parameter does not exist,
/// and [`ParamRouteError::WrongState`] for modulation paths that cannot toggle.
pub fn toggle_param_by_path(mixer: &mut Mixer, path: &str) -> Result<(), ParamRouteError> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["crossfader"] => {
            let current = mixer.crossfader();
            mixer.snap_crossfader(if current > 0.5 { 0.0 } else { 1.0 });
            Ok(())
        }
        ["ch", ch_uuid, "opacity"] => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            let channel = &mut mixer.channels_mut()[ch];
            channel.opacity = if channel.opacity > 0.01 { 0.0 } else { 1.0 };
            Ok(())
        }
        ["deck", uuid, "opacity"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            slot.opacity = if slot.opacity > 0.01 { 0.0 } else { 1.0 };
            Ok(())
        }
        ["deck", uuid, "mute"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            slot.mute = !slot.mute;
            Ok(())
        }
        ["deck", uuid, "solo"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            slot.solo = !slot.solo;
            Ok(())
        }
        ["deck", uuid, "trigger"] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            mixer.channels_mut()[ch].decks[dk].opacity = 1.0;
            Ok(())
        }
        ["deck", uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let val = mixer.channels_mut()[ch].decks[dk]
                .deck
                .generator_params
                .values
                .get_mut(*name)
                .ok_or_else(|| ParamRouteError::UnknownParam {
                    scope: "deck",
                    name: (*name).to_string(),
                })?;
            toggle_param_value(val);
            Ok(())
        }
        ["deck", uuid, "effect", fx_uuid, "param", name] => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            let ek = slot
                .deck
                .effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            let val = slot.deck.effects[ek]
                .params
                .values
                .get_mut(*name)
                .ok_or_else(|| ParamRouteError::UnknownParam {
                    scope: "effect",
                    name: (*name).to_string(),
                })?;
            toggle_param_value(val);
            Ok(())
        }
        ["ch", ch_uuid, "effect", fx_uuid, "param", name] => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            let ek = mixer.channels_mut()[ch]
                .effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            let val = mixer.channels_mut()[ch].effects[ek]
                .params
                .values
                .get_mut(*name)
                .ok_or_else(|| ParamRouteError::UnknownParam {
                    scope: "effect",
                    name: (*name).to_string(),
                })?;
            toggle_param_value(val);
            Ok(())
        }
        ["master", "effect", fx_uuid, "param", name] => {
            let effects = mixer.master_effects_mut();
            let ek = effects
                .iter()
                .position(|e| e.uuid() == *fx_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, fx_uuid))?;
            let val = effects[ek].params.values.get_mut(*name).ok_or_else(|| {
                ParamRouteError::UnknownParam {
                    scope: "effect",
                    name: (*name).to_string(),
                }
            })?;
            toggle_param_value(val);
            Ok(())
        }
        ["mod", _, _] => Err(ParamRouteError::WrongState {
            path: path.to_string(),
            reason: "modulation params are continuous; keyboard toggle does not apply",
        }),
        _ => Err(ParamRouteError::UnknownPath {
            path: path.to_string(),
        }),
    }
}

/// Floats snap between 0.0 and 1.0. Bools invert. Other variants are left as-is.
fn toggle_param_value(val: &mut ParamValue) {
    match val {
        ParamValue::Float(v) => *v = if v.abs() > 0.01 { 0.0 } else { 1.0 },
        ParamValue::Bool(b) => *b = !*b,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A control surface writes a path and the modulation graph is keyed by
    /// something else, so a missing translation here is a fader that silently
    /// fails to take its parameter back from a curve.
    #[test]
    fn a_path_that_names_an_automatable_parameter_finds_its_key() {
        for (path, key) in [
            ("deck/d0000001/opacity", "deck_d0000001:opacity"),
            ("deck/d0000001/param/speed", "deck_d0000001:speed"),
            ("ch/c0000001/opacity", "ch_c0000001:opacity"),
            (
                "ch/c0000001/effect/f0000001/param/amount",
                "fx_f0000001:amount",
            ),
            ("master/effect/f0000001/param/amount", "fx_f0000001:amount"),
            ("deck/d0000001/video/speed", "deck_d0000001:video_speed"),
            ("deck/d0000001/video/seek", "deck_d0000001:video_position"),
            ("deck/d0000001/video/play", "deck_d0000001:video_play"),
            (
                "deck/d0000001/video/loop_mode",
                "deck_d0000001:video_loop_mode",
            ),
            ("deck/d0000001/scaling_mode", "deck_d0000001:scaling_mode"),
        ] {
            assert_eq!(
                modulation_key_for_path(path).as_deref(),
                Some(key),
                "{path}"
            );
        }
    }

    /// A video playback key must not be able to collide with a shader input of
    /// the same nickname on the same deck, which is what the reserved `video_`
    /// prefix buys. `tests/shader_pipeline_guard.rs` holds the other half.
    #[test]
    fn a_shader_param_named_speed_is_not_the_video_speed_target() {
        assert_ne!(
            modulation_key_for_path("deck/d0000001/param/speed"),
            modulation_key_for_path("deck/d0000001/video/speed")
        );
    }

    /// The crossfader is routable but deliberately not a modulation target, a
    /// trigger is an event rather than a value, and in/out points define the
    /// region a position offset is measured against.
    #[test]
    fn a_path_that_names_nothing_automatable_has_no_key() {
        for path in [
            "crossfader",
            "deck/d0000001/trigger",
            "deck/d0000001/video/in_point",
            "deck/d0000001/video/out_point",
            "deck/d0000001/video/clear",
            "macro/m0000001/value",
            "ch/c0000001",
        ] {
            assert_eq!(modulation_key_for_path(path), None, "{path}");
        }
    }

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
    fn discrete_modes_round_trip_through_their_buckets() {
        // The two directions are written out by hand, so a reordering of either
        // match arm has to show up here rather than as a mode that silently
        // becomes its neighbour when a live gesture is recorded.
        for mode in [
            LoopMode::Loop,
            LoopMode::PingPong,
            LoopMode::OneShot,
            LoopMode::HoldLast,
        ] {
            assert_eq!(loop_mode_from_value(loop_mode_to_value(mode)), mode);
        }
        for mode in [
            ScalingMode::Fill,
            ScalingMode::Fit,
            ScalingMode::Stretch,
            ScalingMode::Center,
        ] {
            assert_eq!(scaling_mode_from_value(scaling_mode_to_value(mode)), mode);
        }
    }

    #[test]
    fn speed_and_position_round_trip_through_their_scales() {
        for speed in [0.1_f64, 0.5, 1.0, 2.05, 4.0] {
            assert!((scale_speed(speed_to_norm(speed)) - speed).abs() < 1e-6);
        }
        for secs in [0.0_f64, 7.5, 30.0] {
            assert!((scale_to_duration(duration_to_norm(secs, 30.0), 30.0) - secs).abs() < 1e-4);
        }
    }

    #[test]
    fn a_clip_of_unknown_length_normalizes_to_the_start() {
        assert_eq!(duration_to_norm(5.0, 0.0), 0.0);
        assert_eq!(duration_to_norm(5.0, -2.0), 0.0);
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
        apply_typed_param(&mut params, "tint", ParamValue::Color([0.1, 0.2, 0.3, 0.4])).unwrap();
        match params.values.get("tint") {
            Some(ParamValue::Color(c)) => {
                assert_eq!(*c, [0.1, 0.2, 0.3, 0.4], "all channels must survive");
            }
            other => panic!("expected Color, got {other:?}"),
        }
    }

    // ── Declared-type coercion for typed param writes ─────────────────

    fn params_from(json: serde_json::Value) -> crate::ShaderParams {
        let input: crate::isf::ISFInput = serde_json::from_value(json).unwrap();
        crate::ShaderParams::from_inputs(&[input])
    }

    fn mode_params() -> crate::ShaderParams {
        params_from(serde_json::json!({
            "NAME": "mode", "TYPE": "long", "DEFAULT": 0,
            "VALUES": [0, 1, 2, 3, 4],
            "LABELS": ["Horizontal", "Vertical", "Quad", "Diagonal", "Radial"],
        }))
    }

    #[test]
    fn long_param_survives_a_json_number_arriving_as_float() {
        // Regression: `ParamValue` is `#[serde(untagged)]` with `Float` first, so
        // `{"value": 2}` from the HTTP API deserializes as `Float(2.0)`. Writing
        // that into a `long` left the shader reading 2.0f32's bit pattern as an
        // int (1073741824), matching no branch, so mirror/blend/mode-style
        // effects silently passed their input through for every index but 0.
        let mut params = mode_params();
        apply_typed_param(&mut params, "mode", ParamValue::Float(2.0)).unwrap();
        assert!(
            matches!(params.values.get("mode"), Some(ParamValue::Long(2))),
            "expected Long(2), got {:?}",
            params.values.get("mode")
        );
    }

    #[test]
    fn long_param_accepts_a_typed_long_unchanged() {
        // The GUI sends `ParamValue::Long(index)`; the API must land identically.
        let mut params = mode_params();
        apply_typed_param(&mut params, "mode", ParamValue::Long(4)).unwrap();
        assert!(matches!(
            params.values.get("mode"),
            Some(ParamValue::Long(4))
        ));
    }

    #[test]
    fn long_param_is_never_normalized_against_a_range() {
        // A choice index is discrete: 3 means variant 3, not 3% of a range.
        let mut params = mode_params();
        for idx in 0u8..=4 {
            apply_typed_param(&mut params, "mode", ParamValue::Float(f32::from(idx))).unwrap();
            assert!(
                matches!(params.values.get("mode"), Some(ParamValue::Long(v)) if *v == i32::from(idx)),
                "index {idx} did not round-trip"
            );
        }
    }

    #[test]
    fn bool_param_coerces_from_a_number() {
        let mut params = params_from(serde_json::json!({
            "NAME": "flip_side", "TYPE": "bool", "DEFAULT": false,
        }));
        apply_typed_param(&mut params, "flip_side", ParamValue::Float(1.0)).unwrap();
        assert!(matches!(
            params.values.get("flip_side"),
            Some(ParamValue::Bool(true))
        ));
        apply_typed_param(&mut params, "flip_side", ParamValue::Float(0.0)).unwrap();
        assert!(matches!(
            params.values.get("flip_side"),
            Some(ParamValue::Bool(false))
        ));
    }

    #[test]
    fn float_param_still_normalizes_against_its_range() {
        // Guard the fader contract the coercion must not disturb.
        let mut params = params_from(serde_json::json!({
            "NAME": "fly_speed", "TYPE": "float",
            "DEFAULT": 0.35, "MIN": 0.0, "MAX": 3.0,
        }));
        apply_typed_param(&mut params, "fly_speed", ParamValue::Float(0.5)).unwrap();
        match params.values.get("fly_speed") {
            Some(ParamValue::Float(v)) => assert!((v - 1.5).abs() < 1e-5, "got {v}"),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn unknown_shader_param_is_reported_not_dropped() {
        let mut params = mode_params();
        assert!(matches!(
            apply_typed_param(&mut params, "no_such_param", ParamValue::Float(1.0)),
            Err(ParamRouteError::UnknownParam {
                scope: "shader",
                ..
            })
        ));
    }

    #[test]
    fn scalar_aimed_at_a_color_param_is_refused() {
        let mut params = color_params();
        assert!(matches!(
            apply_typed_param(&mut params, "tint", ParamValue::Float(0.5)),
            Err(ParamRouteError::WrongState { .. })
        ));
        // The original colour must survive the refused write.
        assert!(matches!(
            params.values.get("tint"),
            Some(ParamValue::Color([0.0, 0.0, 0.0, 1.0]))
        ));
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

    fn maybe_mixer() -> Option<(crate::renderer::GpuContext, Mixer)> {
        let gpu = crate::renderer::GpuContext::new_headless().ok()?;
        Mixer::new(&gpu, 64, 64).ok().map(|mixer| (gpu, mixer))
    }

    #[test]
    fn toggle_param_value_snaps_float_and_inverts_bool() {
        let mut v = ParamValue::Float(0.8);
        toggle_param_value(&mut v);
        match v {
            ParamValue::Float(x) => assert!(x.abs() < 1e-5),
            other => panic!("expected Float(0), got {other:?}"),
        }
        toggle_param_value(&mut v);
        match v {
            ParamValue::Float(x) => assert!((x - 1.0).abs() < 1e-5),
            other => panic!("expected Float(1), got {other:?}"),
        }

        let mut b = ParamValue::Bool(false);
        toggle_param_value(&mut b);
        assert!(matches!(b, ParamValue::Bool(true)));
    }

    #[test]
    fn toggle_crossfader_snaps_between_extremes() {
        let Some((_gpu, mut mixer)) = maybe_mixer() else {
            return;
        };
        assert!(mixer.crossfader() < 0.01);
        toggle_param_by_path(&mut mixer, "crossfader").unwrap();
        assert!((mixer.crossfader() - 1.0).abs() < 1e-5);
        toggle_param_by_path(&mut mixer, "crossfader").unwrap();
        assert!(mixer.crossfader() < 1e-5);
    }

    #[test]
    fn toggle_channel_opacity_snaps_between_extremes() {
        let Some((_gpu, mut mixer)) = maybe_mixer() else {
            return;
        };
        let uuid = mixer.channel(0).unwrap().uuid().to_string();
        mixer.channels_mut()[0].opacity = 0.4;
        toggle_param_by_path(&mut mixer, &format!("ch/{uuid}/opacity")).unwrap();
        assert!(mixer.channels()[0].opacity.abs() < 1e-5);
        toggle_param_by_path(&mut mixer, &format!("ch/{uuid}/opacity")).unwrap();
        assert!((mixer.channels()[0].opacity - 1.0).abs() < 1e-5);
    }

    #[test]
    fn toggle_unknown_path_and_mod_path_error() {
        let Some((_gpu, mut mixer)) = maybe_mixer() else {
            return;
        };
        assert!(matches!(
            toggle_param_by_path(&mut mixer, "no/such/path"),
            Err(ParamRouteError::UnknownPath { .. })
        ));
        assert!(matches!(
            toggle_param_by_path(&mut mixer, "mod/abc/frequency"),
            Err(ParamRouteError::WrongState { .. })
        ));
    }
}
