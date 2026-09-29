//! Shared parameter path router for external control protocols (MIDI, OSC).
//!
//! Maps paths like `deck/<uuid>/opacity` or `mod/<uuid>/frequency` to mixer
//! mutations. Values are normalized 0.0–1.0 and scaled to each parameter's
//! range.
//!
//! Entities are addressed by UUID, so reordering a chain does not retarget a
//! saved binding. Failures return a [`ParamRouteError`] with the reason.

use crate::engine::value::param::{DeckTarget, ModulatorTarget, ParamAddress};
use crate::mixer::Mixer;
use crate::modulation::ModulationSource;
use crate::params::{ParamValue, clamp_norm};

/// The kind of entity a path segment addresses, for [`ParamRouteError`].
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

/// Why a parameter path failed to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamRouteError {
    /// The path did not match any known parameter route.
    UnknownPath { path: String },
    /// A valid path named an entity UUID that does not exist.
    UnknownEntity { kind: EntityKind, id: String },
    /// An index (e.g. a step-sequencer step) is out of range for its container.
    IndexOutOfRange {
        kind: EntityKind,
        index: usize,
        len: usize,
    },
    /// The entity cannot accept this mutation in its current state (a deck
    /// with no auto-transition, a modulator that is not a step sequencer).
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

/// Map a mixer op's success flag to a `Result`; `false` becomes
/// [`ParamRouteError::WrongState`] with `reason`.
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

/// The modulation key a live write to `path` overrides: its canonical path
/// (`video/seek` becomes `video/position`). `None` for triggers, in and out
/// points, and macro and modulator values, which a live write does not
/// override.
pub fn modulation_key_for_path(path: &str) -> Option<String> {
    let address: ParamAddress = path.parse().ok()?;
    let overridable = address.is_modulatable()
        && !matches!(
            address,
            ParamAddress::MacroValue { .. } | ParamAddress::Modulator { .. }
        );
    overridable.then(|| address.to_string())
}

/// The canonical modulation key for a client target: a router path or the
/// pre-v8 `deck_<uuid>:<name>` form.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownPath`] when `target` names nothing
/// modulation can drive.
pub fn canonical_modulation_key(target: &str) -> Result<String, ParamRouteError> {
    target
        .parse::<ParamAddress>()
        .ok()
        .or_else(|| ParamAddress::from_legacy_modulation_key(target))
        .filter(ParamAddress::is_modulatable)
        .map(|address| address.to_string())
        .ok_or_else(|| ParamRouteError::UnknownPath {
            path: target.to_string(),
        })
}

/// The normalized (0.0–1.0) value at `address`, read from the mixer for
/// controller feedback. Mute and solo read as 0 or 1; a shader parameter
/// without a declared range reads raw. `None` when nothing readable exists.
pub fn read_param(mixer: &Mixer, address: &ParamAddress) -> Option<f32> {
    match address {
        ParamAddress::Crossfader => Some(mixer.crossfader()),
        ParamAddress::ChannelOpacity { channel } => mixer
            .channel(mixer.find_channel_by_uuid(channel)?)
            .map(|c| c.opacity),
        ParamAddress::Deck { deck, target } => {
            let (ch, dk) = mixer.find_deck_by_uuid(deck)?;
            let slot = mixer.channel(ch)?.decks.get(dk)?;
            match target {
                DeckTarget::Opacity | DeckTarget::Trigger => Some(slot.opacity),
                DeckTarget::Mute => Some(f32::from(u8::from(slot.mute))),
                DeckTarget::Solo => Some(f32::from(u8::from(slot.solo))),
                DeckTarget::Transparent => Some(f32::from(u8::from(slot.deck.transparent()))),
                DeckTarget::Param(name) => read_normalized(&slot.deck.generator_params, name),
                DeckTarget::Source(route) => crate::source::read_route(slot.deck.source(), route),
                _ => None,
            }
        }
        ParamAddress::EffectParam { effect, param } => {
            let location = mixer.find_effect_by_uuid(effect)?;
            read_normalized(&mixer.effect_at(location)?.params, param)
        }
        // Outputs and surfaces are read by the app, not the mixer.
        ParamAddress::Action(_)
        | ParamAddress::CueFire { .. }
        | ParamAddress::Modulator { .. }
        | ParamAddress::MacroValue { .. }
        | ParamAddress::Output { .. }
        | ParamAddress::SurfaceSource { .. } => None,
    }
}

fn toggle_transparent(mixer: &mut Mixer, uuid: &str) -> Result<(), ParamRouteError> {
    let (ch, dk) = mixer
        .find_deck_by_uuid(uuid)
        .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
    let deck = &mut mixer.channels_mut()[ch].decks[dk].deck;
    deck.set_transparent(!deck.transparent());
    Ok(())
}

fn read_normalized(params: &crate::ShaderParams, name: &str) -> Option<f32> {
    if let Some((base, component)) = crate::engine::value::param::Component::split(name)
        && params.component_kind(base) == Some(component.kind())
    {
        return params.component(base, component);
    }
    let value = params.values.get(name)?;
    params
        .normalize(name, value)
        .or_else(|| params.get_float(name))
}

/// Write one target of a macro's modulated fan-out, skipping targets that do
/// not resolve. Used as the per-frame [`crate::mixer::ParamWriter`].
pub fn write_macro_target(mixer: &mut Mixer, path: &str, value: f32) {
    if let Err(e) = apply_param_by_path(mixer, path, value) {
        log::debug!("macro modulation target '{path}' skipped: {e}");
    }
}

/// Parse a router path, reporting a malformed one as `UnknownPath`.
fn parse_address(path: &str) -> Result<ParamAddress, ParamRouteError> {
    path.parse().map_err(|_| ParamRouteError::UnknownPath {
        path: path.to_string(),
    })
}

/// The parameters of the effect with `uuid`, whichever chain holds it.
fn effect_params_mut<'m>(
    mixer: &'m mut Mixer,
    uuid: &str,
) -> Result<&'m mut crate::ShaderParams, ParamRouteError> {
    let location = mixer
        .find_effect_by_uuid(uuid)
        .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Effect, uuid))?;
    let (chain, idx) = mixer.effect_chain_at_mut(location);
    Ok(&mut chain[idx].params)
}

/// Apply a normalized value (0.0–1.0) to the parameter at `path`.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownPath`] if `path` matches no route,
/// [`ParamRouteError::UnknownEntity`] if a UUID or index does not resolve,
/// [`ParamRouteError::IndexOutOfRange`] if an index exceeds its container,
/// [`ParamRouteError::UnknownParam`] if the sub-parameter does not exist,
/// and [`ParamRouteError::WrongState`] if the entity cannot accept the write.
pub fn apply_param_by_path(
    mixer: &mut Mixer,
    path: &str,
    value: f32,
) -> Result<(), ParamRouteError> {
    match &parse_address(path)? {
        ParamAddress::Crossfader => {
            mixer.snap_crossfader(value);
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Opacity,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            mixer.channels_mut()[ch].decks[dk].opacity = clamp_or_full(value);
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Mute,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                let m = mixer.channels_mut()[ch].decks[dk].mute;
                mixer.channels_mut()[ch].decks[dk].mute = !m;
            }
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Solo,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                let s = mixer.channels_mut()[ch].decks[dk].solo;
                mixer.channels_mut()[ch].decks[dk].solo = !s;
            }
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Trigger,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            if value > 0.5 {
                mixer.channels_mut()[ch].decks[dk].opacity = 1.0;
            }
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::AutoTransitionPlay,
        } => {
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
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::AutoTransitionFade,
        } => {
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
        // A source control (`video/speed`, `capture/rate`, ...). The source
        // maps the normalized value onto its own range.
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Source(route),
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let source = mixer.channels_mut()[ch].decks[dk].deck.source_mut();
            crate::source::write_route(source, route, clamp_norm(value)).map_err(|e| match e {
                crate::source::ControlError::Unknown(_) => ParamRouteError::UnknownPath {
                    path: path.to_string(),
                },
                crate::source::ControlError::Invalid(_) | crate::source::ControlError::State(_) => {
                    ParamRouteError::WrongState {
                        path: path.to_string(),
                        reason: "the deck's source refused the value",
                    }
                }
            })
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Transparent,
        } => {
            if value > 0.5 {
                toggle_transparent(mixer, uuid)?;
            }
            Ok(())
        }
        // Depth-sensor preprocessor params for a shader that declares
        // `depth_sensor`.
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::DepthPreprocess(name),
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let applied = mixer.channels_mut()[ch].decks[dk]
                .deck
                .set_depth_prepro_param(name, clamp_norm(value));
            ok_or_state(applied, path, "deck has no depth-sensor preprocessor")
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Param(name),
        } => {
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
        ParamAddress::EffectParam {
            effect,
            param: name,
        } => {
            apply_float_param_scaled(effect_params_mut(mixer, effect)?, name, value);
            Ok(())
        }
        ParamAddress::ChannelOpacity { channel: ch_uuid } => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            mixer.channels_mut()[ch].opacity = clamp_or_full(value);
            Ok(())
        }
        ParamAddress::Modulator {
            source: mod_uuid,
            target: ModulatorTarget::Step(step_idx),
        } => {
            let step_idx = *step_idx;
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
        ParamAddress::Modulator {
            source: mod_uuid,
            target: ModulatorTarget::Param(param_name),
        } => {
            let entry = mixer
                .modulation_mut()
                .find_source_by_uuid_mut(mod_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Modulator, mod_uuid))?;
            apply_mod_param(&mut entry.source, param_name, value)
        }
        ParamAddress::MacroValue { macro_uuid } => {
            // The macro returns the writes to fan out. App actions (undo, save,
            // tap) are queued on the bank for app/inputs.rs. Targets are never
            // `macro/*` paths, so recursion depth is at most 1.
            let fanout = mixer
                .macros_mut()
                .apply_input(macro_uuid, value)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Macro, macro_uuid))?;
            for (target_path, target_value) in fanout {
                if let Err(e) = apply_param_by_path(mixer, &target_path, target_value) {
                    // A target may name a deleted entity; log and keep going.
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
/// Shader and effect `param` paths keep the value's type; `Float` is scaled
/// against the ISF range like a fader. Other paths flatten the value and call
/// [`apply_param_by_path`]. Used by the engine's `set_param`.
///
/// # Errors
///
/// Same as [`apply_param_by_path`].
pub fn apply_typed_param_by_path(
    mixer: &mut Mixer,
    path: &str,
    value: ParamValue,
) -> Result<(), ParamRouteError> {
    match &parse_address(path)? {
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Param(name),
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            apply_typed_param(
                &mut mixer.channels_mut()[ch].decks[dk].deck.generator_params,
                name,
                value,
            )
        }
        ParamAddress::EffectParam {
            effect,
            param: name,
        } => apply_typed_param(effect_params_mut(mixer, effect)?, name, value),
        // Scalar paths (opacity, crossfader, video, mod, ...).
        _ => apply_param_by_path(mixer, path, param_value_to_norm_f32(&value)),
    }
}

/// Flatten a [`ParamValue`] to the normalized f32 the scalar router expects.
/// Non-scalar values collapse to their first component (colors to R, points
/// to x).
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
/// The stored variant is the declared type: a `long` takes a choice index, a
/// `bool` a flag, a `float` a normalized fraction of its range. `Color` and
/// `Point2D` keep all channels.
///
/// [`ParamValue`] is untagged with `Float` first, so every JSON number arrives
/// as `Float`. It must be converted before writing to a `long`, or the shader
/// reads the float's bit pattern as an integer.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownParam`] if the shader has no param by that
/// name, and [`ParamRouteError::WrongState`] if a scalar targets a `color` or
/// `point2D` param.
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
        // Color and point params accept only their own variant.
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
/// Floats round to nearest, so 1.999 selects variant 2.
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
            // Native ranges match the modulation panel sliders.
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
        // Envelope breakpoints are edited as a curve, not routed, so envelope
        // parameters stay unmodulatable.
        ModulationSource::Envelope { .. } => {
            return Err(ParamRouteError::UnknownParam {
                scope: "Envelope",
                name: param_name.to_string(),
            });
        }
    }
    Ok(())
}

/// Apply a normalized 0.0–1.0 value to a float param, scaled to its range.
fn apply_float_param_scaled(params: &mut crate::ShaderParams, name: &str, normalized: f32) {
    // `tint/r`: one channel of a color or axis of a point.
    if let Some((base, component)) = crate::engine::value::param::Component::split(name)
        && params.component_kind(base) == Some(component.kind())
    {
        params.set_component(base, component, normalized);
        return;
    }
    if let Some(def) = params.definitions.get(name) {
        let min = def.min.unwrap_or(0.0);
        let max = def.max.unwrap_or(1.0);
        let scaled = min + normalized * (max - min);
        params.set(name, ParamValue::Float(scaled));
    } else {
        params.set(name, ParamValue::Float(normalized));
    }
}

/// Clamp to 0.0–1.0. Non-finite input becomes `1.0`, so a bad opacity value
/// leaves the layer visible.
fn clamp_or_full(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Toggle a parameter between its extremes, for keyboard shortcuts.
///
/// Floats snap 0↔1, bools and mute/solo flip, and trigger sets opacity to 1.0.
/// Modulation paths are rejected.
///
/// # Errors
///
/// Returns [`ParamRouteError::UnknownPath`] if `path` matches no toggle route,
/// [`ParamRouteError::UnknownEntity`] if a UUID in the path does not resolve,
/// [`ParamRouteError::UnknownParam`] if the named shader parameter does not exist,
/// and [`ParamRouteError::WrongState`] for modulation paths that cannot toggle.
pub fn toggle_param_by_path(mixer: &mut Mixer, path: &str) -> Result<(), ParamRouteError> {
    match &parse_address(path)? {
        ParamAddress::Crossfader => {
            let current = mixer.crossfader();
            mixer.snap_crossfader(if current > 0.5 { 0.0 } else { 1.0 });
            Ok(())
        }
        ParamAddress::ChannelOpacity { channel: ch_uuid } => {
            let ch = mixer
                .find_channel_by_uuid(ch_uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Channel, ch_uuid))?;
            let channel = &mut mixer.channels_mut()[ch];
            channel.opacity = if channel.opacity > 0.01 { 0.0 } else { 1.0 };
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Opacity,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            slot.opacity = if slot.opacity > 0.01 { 0.0 } else { 1.0 };
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Mute,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            slot.mute = !slot.mute;
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Solo,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let slot = &mut mixer.channels_mut()[ch].decks[dk];
            slot.solo = !slot.solo;
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Trigger,
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            mixer.channels_mut()[ch].decks[dk].opacity = 1.0;
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Param(name),
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let val = mixer.channels_mut()[ch].decks[dk]
                .deck
                .generator_params
                .values
                .get_mut(name.as_str())
                .ok_or_else(|| ParamRouteError::UnknownParam {
                    scope: "deck",
                    name: name.clone(),
                })?;
            toggle_param_value(val);
            Ok(())
        }
        ParamAddress::EffectParam {
            effect,
            param: name,
        } => {
            let val = effect_params_mut(mixer, effect)?
                .values
                .get_mut(name.as_str())
                .ok_or_else(|| ParamRouteError::UnknownParam {
                    scope: "effect",
                    name: name.clone(),
                })?;
            toggle_param_value(val);
            Ok(())
        }
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Transparent,
        } => toggle_transparent(mixer, uuid),
        // A source control snaps between its extremes; an action fires.
        ParamAddress::Deck {
            deck: uuid,
            target: DeckTarget::Source(route),
        } => {
            let (ch, dk) = mixer
                .find_deck_by_uuid(uuid)
                .ok_or_else(|| ParamRouteError::unknown_entity(EntityKind::Deck, uuid))?;
            let source = mixer.channels_mut()[ch].decks[dk].deck.source_mut();
            let next = match crate::source::read_route(source, route) {
                Some(current) if current > 0.5 => 0.0,
                _ => 1.0,
            };
            crate::source::write_route(source, route, next).map_err(|_| {
                ParamRouteError::UnknownPath {
                    path: path.to_string(),
                }
            })
        }
        ParamAddress::Modulator { .. } => Err(ParamRouteError::WrongState {
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

    /// Every routable path must map to a modulation key, or a fader cannot
    /// override automation.
    #[test]
    fn a_path_that_names_an_automatable_parameter_finds_its_key() {
        for (path, key) in [
            ("deck/d0000001/opacity", "deck/d0000001/opacity"),
            ("deck/d0000001/param/speed", "deck/d0000001/param/speed"),
            ("ch/c0000001/opacity", "ch/c0000001/opacity"),
            (
                "ch/c0000001/effect/f0000001/param/amount",
                "effect/f0000001/param/amount",
            ),
            (
                "master/effect/f0000001/param/amount",
                "effect/f0000001/param/amount",
            ),
            (
                "effect/f0000001/param/amount",
                "effect/f0000001/param/amount",
            ),
            ("deck/d0000001/video/speed", "deck/d0000001/video/speed"),
            ("deck/d0000001/video/seek", "deck/d0000001/video/position"),
            ("deck/d0000001/video/play", "deck/d0000001/video/play"),
            (
                "deck/d0000001/video/loop_mode",
                "deck/d0000001/video/loop_mode",
            ),
            ("deck/d0000001/scaling_mode", "deck/d0000001/scaling_mode"),
        ] {
            assert_eq!(
                modulation_key_for_path(path).as_deref(),
                Some(key),
                "{path}"
            );
        }
    }

    /// Video playback keys do not collide with shader inputs, which live under
    /// `param/`.
    #[test]
    fn a_shader_param_named_speed_is_not_the_video_speed_target() {
        assert_ne!(
            modulation_key_for_path("deck/d0000001/param/speed"),
            modulation_key_for_path("deck/d0000001/video/speed")
        );
    }

    /// The crossfader and triggers have no key. Source routes always have one;
    /// the engine checks the source schema when assigning.
    #[test]
    fn a_path_that_names_nothing_automatable_has_no_key() {
        for path in [
            "crossfader",
            "deck/d0000001/trigger",
            "macro/m0000001/value",
            "mod/m0000001/frequency",
            "ch/c0000001",
        ] {
            assert_eq!(modulation_key_for_path(path), None, "{path}");
        }
    }

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
        let mut src =
            ModulationSource::audio_from_preset(crate::modulation::AudioBandPreset::Low, None);
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

    // ── Typed value path preserves non-scalar params ──────────

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

    #[test]
    fn a_component_path_writes_one_channel_of_a_shader_color() {
        let mut params = color_params();
        let tint = |params: &crate::ShaderParams| match params.values.get("tint") {
            Some(ParamValue::Color(c)) => *c,
            other => panic!("tint is a color, got {other:?}"),
        };
        apply_float_param_scaled(&mut params, "tint/g", 0.6);
        assert_eq!(tint(&params), [0.0, 0.6, 0.0, 1.0]);
        assert_eq!(read_normalized(&params, "tint/g"), Some(0.6));
        assert_eq!(read_normalized(&params, "tint/a"), Some(1.0));
        // A point suffix on a color, or a suffix on nothing, writes nothing.
        apply_float_param_scaled(&mut params, "tint/x", 0.9);
        apply_float_param_scaled(&mut params, "nope/r", 0.9);
        assert_eq!(tint(&params), [0.0, 0.6, 0.0, 1.0]);
        assert_eq!(read_normalized(&params, "tint/x"), None);
    }

    #[test]
    fn shader_color_modulation_resolves_per_component_path() {
        use crate::modulation::{AnalyzerValues, AudioValues, ModulationEngine, ModulationSource};
        let mut params = color_params();
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update_free_running(0.25, &AudioValues::default(), &AnalyzerValues::default());
        engine.assign("deck/d1/param/tint/b", &uuid, 1.0);
        let Some(ParamValue::Color(c)) =
            params.get_modulated("tint", &engine, Some("deck/d1/param"))
        else {
            panic!("tint is a color");
        };
        assert!(c[2] > 0.0, "blue is modulated: {c:?}");
        assert_eq!([c[0], c[1], c[3]], [0.0, 0.0, 1.0]);
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
        // `{"value": 2}` from the API deserializes as `Float(2.0)` and must
        // land in a `long` as index 2.
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
        // The original color must survive the refused write.
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
        let gpu = crate::testing::headless_gpu()?;
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
