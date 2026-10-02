//! Builds the framework-free `EngineState` from live `VardaApp` state. The
//! egui mapping (`UIData`) lives in `usecases::ui::snapshot`.
//!
//! PERF: each snapshot clones the full state (dozens of allocations at 8+
//! decks). Fine today; profile if deck counts pass ~16.

use super::VardaApp;
use crate::channel::{DeckTransitionPhase, DurationSpec, TransitionTrigger};
use crate::engine::types::{
    AnalyzerScalarInfo, AnalyzerTypeInfo, AudioDeviceSnapshot, AudioPassthroughSnapshot,
    AudioSnapshot, DeliveryHealthSnapshot, ModulationAssignmentSnapshot, ModulationSnapshot,
    ModulationSourceSnapshot, ModulationSourceSnapshotEntry, MonitorSnapshot, OutputSnapshot,
    OutputWindowSnapshot, SurfaceAssignmentSnapshot, SurfaceSnapshot,
};
use crate::engine::types::{
    AutoTransitionSnapshot, CameraSnapshot, ChannelSnapshot, ClockSnapshot, DeckSnapshot,
    DepthPreproParamsSnapshot, EffectSnapshot, EngineState, MidiDeviceSnapshot,
    MidiMappingSnapshot, MidiSnapshot, MixerSnapshot, ParamSnapshot, RegistrySnapshot,
    RunningAnalyzerSnapshot, SequenceSnapshot, SequenceStepKindSnapshot, SequenceStepSnapshot,
    ShaderParamsSnapshot,
};
use crate::modulation::ModulationSource;

/// Build a `MixerSnapshot` from the current `VardaApp` state.
pub(crate) fn build_mixer_snapshot(app: &VardaApp) -> MixerSnapshot {
    let mixer = &app.mixer;
    let channel_labels = app.mixer.channel_labels();
    let query = app.sources.query(&channel_labels);
    #[cfg(feature = "html")]
    let interactive_deck = app.interactive_active_deck();
    #[cfg(not(feature = "html"))]
    let interactive_deck: Option<&str> = None;

    let channels = mixer
        .channels()
        .iter()
        .enumerate()
        .map(|(ch_idx, ch)| {
            let decks = ch
                .decks
                .iter()
                .enumerate()
                .map(|(deck_idx, slot)| {
                    let gen_params = build_shader_params(
                        slot.deck.source_name(),
                        &slot.deck.generator_params,
                        slot.deck
                            .shader()
                            .map_or(&[][..], |shader| &shader.metadata.columns),
                    );
                    let effects = slot
                        .deck
                        .effects
                        .iter()
                        .map(|e| EffectSnapshot {
                            uuid: e.uuid().to_owned(),
                            name: e.shader.name(),
                            enabled: e.enabled,
                            params: build_shader_params(&e.shader.name(), &e.params, &[]),
                        })
                        .collect();

                    let auto_transition =
                        slot.auto_transition
                            .as_ref()
                            .map(|at| AutoTransitionSnapshot {
                                enabled: at.enabled,
                                trigger_is_clip_end: at.trigger == TransitionTrigger::ClipEnd,
                                play_duration_value: at.play_duration.value(),
                                play_duration_is_beats: matches!(
                                    at.play_duration,
                                    DurationSpec::Beats(_)
                                ),
                                transition_duration_value: at.transition_duration.value(),
                                transition_duration_is_beats: matches!(
                                    at.transition_duration,
                                    DurationSpec::Beats(_)
                                ),
                                transition_shader_name: at.transition_shader_name.clone(),
                                phase: at.phase,
                            });

                    let depth_prepro_params =
                        slot.deck
                            .depth_prepro
                            .as_ref()
                            .map(|s| DepthPreproParamsSnapshot {
                                sensor_name: s.sensor_name.clone(),
                                near: normalized_prepro_param(&s.params, "near"),
                                far: normalized_prepro_param(&s.params, "far"),
                                smoothing: normalized_prepro_param(&s.params, "smoothing"),
                                hole_fill: normalized_prepro_param(&s.params, "hole_fill"),
                                mask_feather: normalized_prepro_param(&s.params, "mask_feather"),
                                motion_gain: normalized_prepro_param(&s.params, "motion_gain"),
                                mirror: s.params.mirror,
                            });

                    let effective_opacity = match slot.transition_phase() {
                        DeckTransitionPhase::Transitioning { progress } => {
                            slot.opacity * (1.0 - progress as f32)
                        }
                        _ => slot.opacity,
                    };

                    DeckSnapshot {
                        idx: deck_idx,
                        uuid: slot.deck.uuid().to_string(),
                        name: slot.deck.source_name().to_string(),
                        source: app
                            .sources
                            .providers
                            .deck_snapshot(slot.deck.source(), &query),
                        is_interactive: interactive_deck == Some(slot.deck.uuid()),
                        has_depth_prepro: slot.deck.depth_prepro.is_some(),
                        depth_prepro_params,
                        opacity: slot.opacity,
                        effective_opacity,
                        blend_mode: slot.blend_mode,
                        solo: slot.solo,
                        mute: slot.mute,
                        transparent: slot.deck.transparent(),
                        generator: gen_params,
                        effects,
                        auto_transition,
                        render_fps: slot.render_fps,
                        effective_render_fps: if slot.render_cost_us > 0.0 {
                            1_000_000.0 / slot.render_cost_us
                        } else {
                            0.0
                        },
                        render_cost_us: slot.render_cost_us,
                        gpu_render_cost_us: slot.gpu_render_cost_us,
                        fps: slot.deck.fps(),
                        source_asleep: !slot.source_demand.wants_frames(),
                        running_analyzers: slot
                            .deck
                            .analyzers
                            .running_types()
                            .into_iter()
                            .map(|t| RunningAnalyzerSnapshot { analyzer_type: t })
                            .collect(),
                    }
                })
                .collect();

            let ch_effects = ch
                .effects
                .iter()
                .map(|e| EffectSnapshot {
                    uuid: e.uuid().to_owned(),
                    name: e.shader.name(),
                    enabled: e.enabled,
                    params: build_shader_params(&e.shader.name(), &e.params, &[]),
                })
                .collect();

            ChannelSnapshot {
                idx: ch_idx,
                uuid: ch.uuid().to_string(),
                name: ch.name.clone(),
                opacity: ch.opacity,
                blend_mode: ch.blend_mode,
                decks,
                effects: ch_effects,
                render_time_ms: ch.render_time_ms,
                active_deck_count: ch.active_deck_count,
            }
        })
        .collect();

    let master_effects = mixer
        .master_effects()
        .iter()
        .map(|e| EffectSnapshot {
            uuid: e.uuid().to_owned(),
            name: e.shader.name(),
            enabled: e.enabled,
            params: build_shader_params(&e.shader.name(), &e.params, &[]),
        })
        .collect();

    let auto_crossfade_active = mixer.is_crossfading();
    let auto_crossfade_progress = mixer
        .auto_crossfade()
        .as_ref()
        .map_or(0.0, |a| a.progress());

    let transition_names = app
        .sources
        .registry
        .transitions()
        .iter()
        .map(|s| s.name())
        .collect();
    let active_transition_name = mixer.active_transition().as_ref().map(|t| t.name.clone());

    let sequences = build_sequence_snapshots(mixer);

    MixerSnapshot {
        channels,
        crossfader: mixer.crossfader(),
        auto_crossfade_active,
        auto_crossfade_progress,
        master_effects,
        active_transition_name,
        transition_names,
        sequences,
        tonemap_mode: mixer.tonemap_mode(),
        active_lut: mixer
            .active_lut_filename()
            .map(std::string::ToString::to_string),
        look_lut: mixer
            .look_lut_filename()
            .map(std::string::ToString::to_string),
    }
}

fn build_shader_params(
    shader_name: &str,
    params: &crate::params::ShaderParams,
    columns: &[crate::isf::ParamColumn],
) -> ShaderParamsSnapshot {
    let params_vec = params
        .param_order
        .iter()
        .filter_map(|name| {
            let value = params.values.get(name)?;
            let def = params.definitions.get(name);
            Some(ParamSnapshot {
                name: name.clone(),
                label: def.and_then(|d| d.label.clone()),
                value: *value,
                min: def.and_then(|d| d.min),
                max: def.and_then(|d| d.max),
                group: def.and_then(|d| d.group.clone()),
                choices: def.map(crate::isf::ISFInput::choices).and_then(|c| {
                    (!c.is_empty()).then(|| {
                        c.into_iter()
                            .map(|(value, label)| crate::engine::ParamChoice { value, label })
                            .collect()
                    })
                }),
            })
        })
        .collect();

    ShaderParamsSnapshot {
        shader_name: shader_name.to_string(),
        params: params_vec,
        columns: columns
            .iter()
            .map(|c| crate::engine::types::ParamColumnSnapshot {
                title: c.title.clone(),
                groups: c.groups.clone(),
            })
            .collect(),
    }
}

/// Normalized (`0..1`) value of a depth-preprocessor param. `name` is one of the
/// router names in `src/internal/depth/preprocess.rs`; unknown names read as `0`.
fn normalized_prepro_param(
    params: &crate::depth::preprocess::DepthPreprocessParams,
    name: &str,
) -> f32 {
    params.normalized_param(name).unwrap_or_default()
}

fn build_sequence_snapshots(mixer: &crate::mixer::Mixer) -> Vec<SequenceSnapshot> {
    let channel_names: std::collections::HashMap<&str, &str> = mixer
        .channels()
        .iter()
        .map(|c| (c.uuid(), c.name.as_str()))
        .collect();
    mixer
        .transition_sequences()
        .iter()
        .map(|seq| {
            let steps = seq
                .steps
                .iter()
                .map(|step| {
                    let (label, kind) = match &step.kind {
                        crate::mixer::StepKind::Fade {
                            from_ch,
                            to_ch,
                            duration,
                            easing,
                            transition_shader,
                            target_amount,
                        } => {
                            let unit_label = duration.unit().label();
                            let easing_name = format!("{easing:?}");
                            let label = format!(
                                "Fade {} -> {} ({:.1}{})",
                                channel_names.get(from_ch.as_str()).copied().unwrap_or("?"),
                                channel_names.get(to_ch.as_str()).copied().unwrap_or("?"),
                                duration.value(),
                                unit_label
                            );
                            (
                                label,
                                SequenceStepKindSnapshot::Fade {
                                    from_ch: from_ch.clone(),
                                    to_ch: to_ch.clone(),
                                    duration_val: duration.value(),
                                    duration_unit: duration.unit(),
                                    easing: easing_name,
                                    transition_shader: transition_shader.clone(),
                                    target_amount: *target_amount,
                                },
                            )
                        }
                        crate::mixer::StepKind::Wait { duration } => {
                            let unit_label = duration.unit().label();
                            let label = format!("Wait {:.1}{}", duration.value(), unit_label);
                            (
                                label,
                                SequenceStepKindSnapshot::Wait {
                                    duration_val: duration.value(),
                                    duration_unit: duration.unit(),
                                },
                            )
                        }
                        crate::mixer::StepKind::GoTo { step_index } => {
                            let label = format!("GoTo step {step_index}");
                            (
                                label,
                                SequenceStepKindSnapshot::GoTo {
                                    step_index: *step_index,
                                },
                            )
                        }
                    };
                    SequenceStepSnapshot { label, kind }
                })
                .collect();
            SequenceSnapshot {
                uuid: seq.uuid.clone(),
                name: seq.name.clone(),
                enabled: seq.enabled,
                playing: seq.state.playing,
                current_step: seq.state.current_step,
                step_elapsed: seq.state.step_elapsed,
                steps,
            }
        })
        .collect()
}

/// Build a `RegistrySnapshot` from the current `VardaApp` state.
pub(crate) fn build_registry_snapshot(app: &VardaApp) -> RegistrySnapshot {
    let mut generators: Vec<(String, usize)> = app
        .sources
        .registry
        .generators()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name(), i))
        .collect();
    generators.sort_by_key(|a| a.0.to_lowercase());
    let mut filters: Vec<(String, usize)> = app
        .sources
        .registry
        .filters()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name(), i))
        .collect();
    filters.sort_by_key(|a| a.0.to_lowercase());
    RegistrySnapshot {
        generators,
        filters,
        shader_count: app.sources.registry.count(),
    }
}

/// Build a `MidiSnapshot` from the current `VardaApp` state.
pub(crate) fn build_midi_snapshot(app: &VardaApp) -> MidiSnapshot {
    let devices = app
        .input
        .midi_devices
        .as_ref()
        .map(|mgr| {
            mgr.device_list()
                .iter()
                .map(|d| MidiDeviceSnapshot {
                    id: d.id,
                    name: d.name.clone(),
                    enabled: d.enabled,
                    has_output: d.has_output,
                    profile: d.profile_name().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let mappings = {
        let sorted = app.input.midi_mappings.sorted_mappings();
        sorted
            .iter()
            .map(|(key, path)| {
                let dev_name = app
                    .input
                    .midi_devices
                    .as_ref()
                    .and_then(|mgr| mgr.device(key.device_id()))
                    .map_or_else(|| format!("Device {}", key.device_id()), |d| d.name.clone());
                MidiMappingSnapshot {
                    key: *key,
                    key_display: format!("{key}"),
                    device_name: dev_name,
                    param_path: path.clone(),
                }
            })
            .collect()
    };

    MidiSnapshot {
        devices,
        mappings,
        learn_active: app.input.midi_mappings.learn_mode,
        learn_target: app.input.midi_mappings.learn_target.clone(),
    }
}

/// Build a `CameraSnapshot` from the current `VardaApp` state.
pub(crate) fn build_camera_snapshot(app: &VardaApp) -> CameraSnapshot {
    CameraSnapshot {
        devices: app
            .camera_manager()
            .devices()
            .iter()
            .map(|d| (d.name.clone(), d.id))
            .collect(),
    }
}

/// Build a `ClockSnapshot` from the current clock manager state.
pub(crate) fn build_clock_snapshot(app: &VardaApp) -> ClockSnapshot {
    use crate::engine::types::DetectedClockSourceSnapshot;

    let clock = app.input.clock_manager.state();
    let (source_label, device_name) = match &clock.source {
        crate::clock::ClockSource::Audio => ("Audio".to_string(), None),
        crate::clock::ClockSource::MidiClock { device_name, .. } => {
            ("MIDI".to_string(), Some(device_name.clone()))
        }
        crate::clock::ClockSource::OscClock => ("OSC".to_string(), None),
        crate::clock::ClockSource::Manual => ("Manual".to_string(), None),
    };

    let detected_midi_sources = app
        .input
        .clock_manager
        .detected_midi_sources()
        .into_iter()
        .map(|s| DetectedClockSourceSnapshot {
            device_id: s.device_id,
            device_name: s.device_name,
            bpm: s.bpm,
        })
        .collect();

    let preference = app.input.clock_manager.preference();
    let (preference_label, preference_force_device_id) = match preference {
        crate::clock::ClockPreference::Auto => ("Auto".to_string(), None),
        crate::clock::ClockPreference::ForceMidi { device_id } => {
            (format!("ForceMidi({device_id})"), Some(*device_id))
        }
        crate::clock::ClockPreference::ForceOsc => ("ForceOsc".to_string(), None),
        crate::clock::ClockPreference::ForceAudio => ("ForceAudio".to_string(), None),
        crate::clock::ClockPreference::ForceManual { .. } => ("ForceManual".to_string(), None),
    };

    ClockSnapshot {
        bpm: if clock.active { Some(clock.bpm) } else { None },
        beat_phase: clock.beat_phase,
        source_label,
        device_name,
        active: clock.active,
        detected_midi_sources,
        osc_active: app.input.clock_manager.osc_active(),
        osc_bpm: app.input.clock_manager.osc_bpm(),
        audio_bpm: if clock.active && matches!(clock.source, crate::clock::ClockSource::Audio) {
            Some(clock.bpm)
        } else {
            None
        },
        preference_label,
        preference_force_device_id,
        manual_bpm: app.input.clock_manager.manual_bpm(),
        beat_followers: app
            .mixer
            .modulation()
            .followers_of(crate::timebase::Timebase::Beat),
    }
}

/// Build the transport snapshot, adding the follower count (from the
/// modulation engine) and the record state, which the `From` impl can't see.
pub(crate) fn build_transport_snapshot(app: &VardaApp) -> crate::engine::types::TransportSnapshot {
    let mut snapshot: crate::engine::types::TransportSnapshot = (&app.show.transport).into();
    snapshot.followers = app
        .mixer
        .modulation()
        .followers_of(crate::timebase::Timebase::Transport);
    snapshot.record_armed = app.show.recorder.armed();
    snapshot.recording_params = app.show.recorder.recording_params();
    snapshot
}

/// Build the timecode diagnostics: every input heard, and which one drives.
pub(crate) fn build_timecode_snapshot(app: &VardaApp) -> crate::engine::types::TimecodeSnapshot {
    let manager = &app.input.timecode;
    crate::engine::types::TimecodeSnapshot {
        inputs: manager
            .inputs()
            .iter()
            .map(|input| crate::engine::types::TimecodeInputSnapshot {
                key: input.source.key(),
                label: input.source.label(),
                position: input.position,
                timecode: input.label(),
                rate: input.rate,
                running: input.running,
                freewheeling: input.freewheeling,
                speed: input.speed,
            })
            .collect(),
        resolved: manager.resolved_key(),
        preference: manager.preference(),
        ltc_input: manager.ltc_input(),
    }
}

/// Build the arrangement snapshot, or `None` for a Performance-only scene.
pub(crate) fn build_arrangement_snapshot(
    app: &VardaApp,
) -> Option<crate::engine::types::ArrangementSnapshot> {
    let config = app.mixer.arrangement()?;
    Some(crate::engine::types::ArrangementSnapshot {
        engaged: app.arrangement_authority().is_engaged(),
        overridden_params: app
            .mixer
            .modulation()
            .overridden_params()
            .map(String::from)
            .collect(),
        duration: config.duration(),
        config: config.clone(),
    })
}

pub(crate) fn build_audio_snapshot(app: &VardaApp) -> AudioSnapshot {
    let primary_audio = app.audio.manager.get_primary_data();
    let active_ids = app.audio.manager.active_source_ids();
    AudioSnapshot {
        level: primary_audio.level,
        bass: primary_audio.bass(),
        mid: primary_audio.mid(),
        treble: primary_audio.treble(),
        bpm: primary_audio.bpm,
        beat_phase: primary_audio.beat_phase(),
        enabled: app.audio.manager.has_active_source(),
        devices: app
            .audio
            .manager
            .devices()
            .iter()
            .map(|d| AudioDeviceSnapshot {
                id: d.id,
                name: d.name.clone(),
                active: active_ids.contains(&d.id),
            })
            .collect(),
        fft: primary_audio.fft.to_vec(),
        sample_rate: primary_audio.sample_rate,
    }
}

pub(crate) fn build_modulation_snapshot(app: &VardaApp) -> ModulationSnapshot {
    let m = &app.mixer;
    let sources = m
        .modulation()
        .sources
        .iter()
        .map(|entry| {
            let snapshot = match &entry.source {
                ModulationSource::LFO {
                    waveform,
                    frequency,
                    phase,
                    amplitude,
                    bipolar,
                } => ModulationSourceSnapshot::LFO {
                    waveform: *waveform,
                    frequency: *frequency,
                    phase: *phase,
                    amplitude: *amplitude,
                    bipolar: *bipolar,
                },
                ModulationSource::AudioBand {
                    source_id,
                    freq_low,
                    freq_high,
                    gain,
                    smoothing,
                    mode,
                    noise_gate,
                } => ModulationSourceSnapshot::Audio {
                    source_id: *source_id,
                    freq_low: *freq_low,
                    freq_high: *freq_high,
                    gain: *gain,
                    smoothing: *smoothing,
                    mode: *mode,
                    noise_gate: *noise_gate,
                },
                ModulationSource::ADSR {
                    attack,
                    decay,
                    sustain,
                    release,
                    stage,
                    ..
                } => ModulationSourceSnapshot::ADSR {
                    attack: *attack,
                    decay: *decay,
                    sustain: *sustain,
                    release: *release,
                    stage: *stage,
                },
                ModulationSource::StepSequencer {
                    steps,
                    rate,
                    interpolation,
                    bipolar,
                } => ModulationSourceSnapshot::StepSequencer {
                    steps: steps.clone(),
                    rate: *rate,
                    interpolation: *interpolation,
                    bipolar: *bipolar,
                },
                ModulationSource::Analyzer {
                    deck_id,
                    analyzer_type,
                    output_name,
                    smoothing,
                } => ModulationSourceSnapshot::Analyzer {
                    deck_id: deck_id.clone(),
                    analyzer_type: analyzer_type.clone(),
                    output_name: output_name.clone(),
                    smoothing: *smoothing,
                },
                ModulationSource::Envelope { breakpoints, .. } => {
                    ModulationSourceSnapshot::Envelope {
                        breakpoints: breakpoints.clone(),
                    }
                }
            };
            ModulationSourceSnapshotEntry {
                uuid: entry.uuid.clone(),
                source: snapshot,
                timebase: entry.timebase,
            }
        })
        .collect();
    let current_values: std::collections::HashMap<String, f32> = m
        .modulation()
        .sources
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            (
                entry.uuid.clone(),
                m.modulation()
                    .current_values()
                    .get(i)
                    .copied()
                    .unwrap_or(0.0),
            )
        })
        .collect();
    let assignments = m
        .modulation()
        .assignments
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.iter()
                    .map(|pm| ModulationAssignmentSnapshot {
                        source_id: pm.source_id.clone(),
                        amount: pm.amount,
                    })
                    .collect(),
            )
        })
        .collect();
    ModulationSnapshot {
        sources,
        current_values,
        assignments,
    }
}

pub(crate) fn build_output_snapshot(app: &VardaApp) -> OutputSnapshot {
    OutputSnapshot {
        windows: app
            .output
            .outputs
            .iter()
            .map(|o| {
                let surface_assignments = o
                    .surface_assignments
                    .iter()
                    .map(|a| {
                        let surface_name = app
                            .output
                            .surface_manager
                            .find_by_uuid(&a.surface_uuid)
                            .map_or_else(
                                || format!("Surface {}", a.surface_uuid),
                                |(_, s)| s.name.clone(),
                            );
                        SurfaceAssignmentSnapshot {
                            surface_uuid: a.surface_uuid.clone(),
                            surface_name,
                            enabled: a.enabled,
                            overlap_zones: a.overlap_zones.clone(),
                        }
                    })
                    .collect();
                let sink = o.sink();
                // Encoder counts plus drops seen by the output's own subscription.
                let audio_passthrough = o.audio.as_ref().and_then(|pass| {
                    let health = sink.audio_health()?;
                    Some(AudioPassthroughSnapshot {
                        device: sink.audio_device().unwrap_or_default().to_string(),
                        frames_written: health.frames_written,
                        frames_dropped: pass.dropped.load(std::sync::atomic::Ordering::Relaxed),
                        silence_spliced: health.silence_spliced,
                    })
                });
                let delivery = sink.encoder_health().map(|health| DeliveryHealthSnapshot {
                    frames_written: health.frames_written,
                    frames_dropped: health.frames_dropped,
                    frames_padded: health.frames_padded,
                });
                let (width, height) = o.size();
                OutputWindowSnapshot {
                    uuid: o.uuid.clone(),
                    name: o.name.clone(),
                    sink: crate::engine::types::OutputSinkSnapshot {
                        type_id: sink.sink_type().to_string(),
                        label: sink.label(),
                        available: !sink.as_any().is::<crate::output::UnavailableSink>(),
                        startable: sink.startable(),
                        status: sink.status(),
                    },
                    is_active: o.active,
                    unassigned: o.effective_unassigned(),
                    surface_assignments,
                    calibration_mode: o.calibration_mode,
                    presentation_request: o.presentation_request(),
                    resolved_presentation: o.resolved_presentation().clone(),
                    mode_availability: o.mode_availability().to_vec(),
                    tonemap_override: o.tonemap_override,
                    audio_passthrough,
                    delivery,
                    edge_blend_mode: o.edge_blend_mode,
                    edge_blend: o.edge_blend,
                    rotation: o.rotation,
                    active_seconds: crate::app::Outputs::active_duration(o).as_secs_f64(),
                    width,
                    height,
                }
            })
            .collect(),
        surfaces: app
            .output
            .surface_manager
            .surfaces
            .iter()
            .map(|s| SurfaceSnapshot {
                uuid: s.uuid.clone(),
                name: s.name.clone(),
                vertices: s.vertices.clone(),
                extra_contours: s.extra_contours.clone(),
                source: s.source.clone(),
                content_mapping: s.content_mapping,
                output_type: s.output_type,
                circle_hint: s.circle_hint,
                warp: s.effective_warp(),
                warp_bound: s.warp_bound,
                path: s.path.clone(),
                holes: s.holes.clone(),
                hole_contours: s.hole_contours.clone(),
            })
            .collect(),
        monitors: app
            .sources
            .services
            .get::<crate::output::window::Monitors>()
            .map(|m| m.list.as_slice())
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, (name, handle))| {
                let size = handle.size();
                MonitorSnapshot {
                    name: name.clone(),
                    index: i,
                    width: size.width,
                    height: size.height,
                }
            })
            .collect(),
    }
}

pub(crate) fn build_analyzer_types(app: &VardaApp) -> Vec<AnalyzerTypeInfo> {
    app.sources
        .analyzer_registry
        .available_types()
        .into_iter()
        .filter_map(|t| {
            let schema = app.sources.analyzer_registry.schema_for(t)?;
            Some(AnalyzerTypeInfo {
                analyzer_type: t.to_owned(),
                scalar_outputs: schema
                    .scalars
                    .iter()
                    .map(|s| AnalyzerScalarInfo {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        range: s.range,
                        default_smoothing: s.default_smoothing,
                    })
                    .collect(),
                texture_outputs: schema.textures.iter().map(|t| t.name.clone()).collect(),
            })
        })
        .collect()
}

/// Build a full `EngineState` from all subsystem snapshots.
pub(crate) fn build_engine_state(app: &VardaApp) -> EngineState {
    EngineState {
        mixer: build_mixer_snapshot(app),
        deck_loads: app.sources.deck_loader.snapshot(),
        dome: app.dome_config(),
        audio: build_audio_snapshot(app),
        modulation: build_modulation_snapshot(app),
        outputs: build_output_snapshot(app),
        registry: build_registry_snapshot(app),
        midi: build_midi_snapshot(app),
        cameras: build_camera_snapshot(app),
        sources: app.sources.type_snapshots(&app.mixer.channel_labels()),
        sinks: app.output.sink_type_cache.get_or_build(|| {
            app.output.sinks.type_snapshots(&crate::output::SinkQuery {
                services: &app.sources.services,
            })
        }),
        clock: build_clock_snapshot(app),
        transport: build_transport_snapshot(app),
        timecode: build_timecode_snapshot(app),
        arrangement: build_arrangement_snapshot(app),
        fps: app.frame_stats.fps_smoothed,
        frame_count: app.frame_stats.frame_count,
        target_fps: app.render.target_fps,
        analyzers: build_analyzer_types(app),
        macros: app.mixer.macros().macros().to_vec(),
        can_undo: app.history_can_undo(),
        can_redo: app.history_can_redo(),
        keymap: build_keymap_snapshot(app),
        presets: crate::engine::types::PresetsSnapshot {
            deck: app
                .session
                .preset_library
                .deck_presets
                .iter()
                .map(|p| p.name.clone())
                .collect(),
            channel: app
                .session
                .preset_library
                .channel_presets
                .iter()
                .map(|p| p.name.clone())
                .collect(),
        },
        notifications: app
            .session
            .notifications
            .visible()
            .iter()
            .map(|n| crate::engine::types::NotificationSnapshot {
                id: n.id,
                level: n.level,
                message: n.message.clone(),
                progress: n.progress(),
            })
            .collect(),
        clipboard: app.clipboard_summary(),
        render: crate::engine::types::RenderSnapshot {
            width: app.render.width,
            height: app.render.height,
            max_dimension: app.max_render_dimension(),
            domemaster_resolution: app.output.domemaster_resolution,
        },
        system: crate::engine::types::SystemSnapshot {
            cpu_usage: app.frame_stats.system_monitor.cpu_usage(),
            ram_used: app.frame_stats.system_monitor.ram_used(),
            ram_total: app.frame_stats.system_monitor.ram_total(),
            gpu_utilization: app.mixer.gpu_utilization(),
            gpu: app.render.gpu_info.clone(),
        },
    }
}

/// Keyboard shortcuts and keyboard learn.
fn build_keymap_snapshot(app: &VardaApp) -> crate::engine::types::KeymapSnapshot {
    let keymap = &app.input.keymap;
    crate::engine::types::KeymapSnapshot {
        bindings: keymap
            .bindings
            .iter()
            .map(|(combo, target)| crate::engine::types::KeyBindingSnapshot {
                combo: combo.clone(),
                target: target.clone(),
            })
            .collect(),
        learn_active: keymap.learn_mode,
        learn_target: keymap.learn_target.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> Option<super::super::VardaApp> {
        crate::testing::headless_app()
    }

    /// The published state carries what the GUI reads: source libraries, key
    /// bindings, presets, render size, and the GPU adapter.
    #[test]
    fn the_published_state_carries_what_the_gui_shows() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.execute_command(crate::engine::EngineCommand::AddSourceLibraryEntry {
            entry: crate::source::SourceConfig::new("Hls")
                .with("url", "https://example.invalid/a.m3u8"),
        });
        let state = build_engine_state(&app);
        let hls = state
            .sources
            .iter()
            .find(|t| t.type_id == "Hls")
            .expect("every registered type is published");
        assert_eq!(hls.library.entries.len(), 1);
        assert_eq!(
            hls.library.entries[0].connected,
            Some(false),
            "no deck receives it"
        );
        assert!(!state.keymap.bindings.is_empty(), "the default bindings");
        assert_eq!(state.render.width, app.render.width);
        assert!(state.render.max_dimension >= state.render.width);
        assert!(!state.system.gpu.name.is_empty());
        let _json = serde_json::to_value(&state).expect("the state serializes");
    }

    #[test]
    fn snapshot_default_mixer_two_channels() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_mixer_snapshot(&app);
        assert_eq!(snap.channels.len(), 2);
        assert_eq!(snap.crossfader, 0.0);
        for ch in &snap.channels {
            assert!(ch.decks.is_empty());
        }
    }

    #[test]
    fn snapshot_deck_opacity_and_effective() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch = build_mixer_snapshot(&app).channels[0].uuid.clone();
        let deck_uuid = app
            .add_deck(
                &ch,
                &crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            )
            .unwrap();
        app.mixer.set_deck_opacity(&deck_uuid, 0.5).unwrap();
        let snap = build_mixer_snapshot(&app);
        let deck = &snap.channels[0].decks[0];
        assert!((deck.opacity - 0.5).abs() < 1e-5);
        // No transition, so effective equals opacity.
        assert!((deck.effective_opacity - 0.5).abs() < 1e-5);
    }

    #[test]
    fn snapshot_deck_with_effects() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch = build_mixer_snapshot(&app).channels[0].uuid.clone();
        let deck_uuid = app
            .add_deck(
                &ch,
                &crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            )
            .unwrap();
        let target = crate::engine::types::EffectTarget::Deck(deck_uuid);
        let effect_uuid = app.add_effect(&target, "invert").unwrap();

        let snap = build_mixer_snapshot(&app);
        let effects = &snap.channels[0].decks[0].effects;
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].uuid, effect_uuid);
    }

    #[test]
    fn snapshot_empty_channel_has_no_decks() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_mixer_snapshot(&app);
        assert!(snap.channels[0].decks.is_empty());
    }

    #[test]
    fn build_shader_params_filters_missing() {
        // A param in param_order with no value is filtered out.
        let mut params = crate::params::ShaderParams::from_inputs(&[]);
        params.param_order.push("brightness".into());
        let snap = build_shader_params("test_shader", &params, &[]);
        assert!(snap.params.is_empty());
    }

    #[test]
    fn build_shader_params_missing_definition() {
        // A value with no definition has no label/min/max.
        let mut params = crate::params::ShaderParams::from_inputs(&[]);
        params.param_order.push("mystery".into());
        params
            .values
            .insert("mystery".into(), crate::params::ParamValue::Float(0.5));
        let snap = build_shader_params("test_shader", &params, &[]);
        assert_eq!(snap.params.len(), 1);
        let p = &snap.params[0];
        assert!(p.label.is_none());
        assert!(p.min.is_none());
        assert!(p.max.is_none());
    }

    #[test]
    fn shader_columns_reach_the_snapshot() {
        let params = crate::params::ShaderParams::from_inputs(&[]);
        let columns = [crate::isf::ParamColumn {
            title: "Light".into(),
            groups: vec!["Lighting".into(), "Palette".into()],
        }];
        let snap = build_shader_params("test_shader", &params, &columns);
        assert_eq!(snap.columns.len(), 1);
        assert_eq!(snap.columns[0].title, "Light");
        assert_eq!(snap.columns[0].groups, ["Lighting", "Palette"]);
    }

    #[test]
    fn build_registry_snapshot_sorted() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_registry_snapshot(&app);
        // Sorted case-insensitively.
        for pair in snap.generators.windows(2) {
            assert!(
                pair[0].0.to_lowercase() <= pair[1].0.to_lowercase(),
                "generators not sorted: {} > {}",
                pair[0].0,
                pair[1].0,
            );
        }
        for pair in snap.filters.windows(2) {
            assert!(
                pair[0].0.to_lowercase() <= pair[1].0.to_lowercase(),
                "filters not sorted: {} > {}",
                pair[0].0,
                pair[1].0,
            );
        }
    }

    #[test]
    fn build_clock_snapshot_inactive() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_clock_snapshot(&app);
        assert!(snap.bpm.is_none() || !snap.active);
    }

    #[test]
    fn build_clock_snapshot_source_labels() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_clock_snapshot(&app);
        let valid = ["Audio", "MIDI", "OSC", "Manual"];
        assert!(
            valid.contains(&snap.source_label.as_str()),
            "unexpected source label: {}",
            snap.source_label
        );
    }

    /// Every timecode input heard appears with its own position and run state,
    /// including ones that are not resolving.
    #[test]
    fn build_timecode_snapshot_lists_every_input_it_heard() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let now = std::time::Instant::now();
        let at = |seconds| {
            crate::timecode::TimecodeFrame::at(seconds, crate::transport::TimecodeRate::Fps25)
        };
        app.input.timecode.ingest(
            crate::timecode::TimecodeSource::Ltc {
                source_id: 3,
                channel: 1,
            },
            at(90.0),
            now,
        );
        app.input.timecode.ingest(
            crate::timecode::TimecodeSource::Mtc {
                device_id: 7,
                device_name: "Tascam DA-6400".to_string(),
            },
            at(5.0),
            now,
        );
        app.input.timecode.update(now);

        let snap = build_timecode_snapshot(&app);

        assert_eq!(
            snap.inputs
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["ltc", "mtc:7"]
        );
        assert_eq!(
            snap.inputs
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            vec!["LTC (channel 2)", "MTC (Tascam DA-6400)"],
            "the labels are what a performer reads off the panel"
        );
        let ltc = &snap.inputs[0];
        assert!((ltc.position - 90.0).abs() < 0.05);
        assert_eq!(ltc.timecode, "00:01:30:00");
        assert_eq!(ltc.rate, crate::transport::TimecodeRate::Fps25);
        assert!(ltc.running);
        assert!(!ltc.freewheeling, "a frame just arrived");
        assert!((ltc.speed - 1.0).abs() < 1e-9);
        assert_eq!(
            snap.resolved.as_deref(),
            Some("ltc"),
            "LTC outranks MTC, and the snapshot has to say which one is driving"
        );

        // A master that stops is still listed, holding where it stopped.
        app.input
            .timecode
            .update(now + std::time::Duration::from_secs(2));
        let stopped = build_timecode_snapshot(&app);
        assert_eq!(stopped.inputs.len(), 2, "a silent input is still an input");
        assert!(stopped.inputs.iter().all(|i| !i.running));
        assert!(stopped.inputs.iter().all(|i| !i.freewheeling));
    }

    /// The patch comes from the same snapshot as the positions.
    #[test]
    fn build_timecode_snapshot_reports_the_patch_it_is_reading() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let patch = crate::timecode::LtcInput {
            source_id: 2,
            channel: 1,
            rate: Some(crate::transport::TimecodeRate::Fps2997),
        };
        app.input.timecode.set_ltc_input(Some(patch));
        app.input
            .timecode
            .set_preference(crate::timecode::TimecodePreference::ForceLtc);

        let snap = build_timecode_snapshot(&app);

        assert_eq!(snap.ltc_input, Some(patch));
        assert_eq!(
            snap.preference,
            crate::timecode::TimecodePreference::ForceLtc
        );
        assert!(snap.inputs.is_empty(), "nothing has arrived on it yet");
        assert_eq!(snap.resolved, None);
    }
}
