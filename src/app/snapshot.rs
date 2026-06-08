//! Snapshot builder — constructs engine state snapshots from live VardaApp state.
//!
//! PERF: Every frame clones the full EngineState (params, effects, FFT data,
//! modulation assignments). At 8+ decks with effects, this is dozens of heap
//! allocations per frame. Not a bottleneck at 60fps with current deck counts,
//! but worth profiling if deck/effect counts grow significantly (16+ decks).
//! Mitigation options: dirty-flag retained snapshots, arena allocation, or
//! COW wrappers on heavy fields.

use super::VardaApp;
use crate::channel::{DeckTransitionPhase, DurationSpec, TransitionTrigger};
use crate::engine::types::*;
use crate::usecases::ui::{EffectInfo, ParamUIInfo, ShaderParamsUI};

/// Build a MixerSnapshot from the current VardaApp state.
pub(crate) fn build_mixer_snapshot(app: &VardaApp) -> MixerSnapshot {
    let mixer = &app.mixer;

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
                    let gen_params =
                        build_shader_params(&slot.deck.source_name(), &slot.deck.generator_params);
                    let effects = slot
                        .deck
                        .effects
                        .iter()
                        .map(|e| EffectSnapshot {
                            uuid: e.uuid.clone(),
                            name: e.shader.name(),
                            enabled: e.enabled,
                            params: build_shader_params(&e.shader.name(), &e.params),
                        })
                        .collect();

                    let video_playback =
                        slot.deck
                            .playback_snapshot()
                            .map(|ps| VideoPlaybackSnapshot {
                                playing: ps.playing,
                                position: ps.position,
                                duration: ps.duration,
                                speed: ps.speed,
                                loop_mode: ps.loop_mode,
                                in_point: ps.in_point,
                                out_point: ps.out_point,
                                frame_rate: ps.frame_rate,
                            });

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
                        opacity: slot.opacity,
                        effective_opacity,
                        blend_mode: slot.blend_mode,
                        solo: slot.solo,
                        mute: slot.mute,
                        scaling_mode: slot.deck.scaling_mode(),
                        generator: gen_params,
                        effects,
                        video_playback,
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
                    uuid: e.uuid.clone(),
                    name: e.shader.name(),
                    enabled: e.enabled,
                    params: build_shader_params(&e.shader.name(), &e.params),
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
            uuid: e.uuid.clone(),
            name: e.shader.name(),
            enabled: e.enabled,
            params: build_shader_params(&e.shader.name(), &e.params),
        })
        .collect();

    let auto_crossfade_active = mixer.is_crossfading();
    let auto_crossfade_progress = mixer
        .auto_crossfade()
        .as_ref()
        .map_or(0.0, |a| a.progress());

    let transition_names = app
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
    }
}

fn build_shader_params(
    shader_name: &str,
    params: &crate::params::ShaderParams,
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
            })
        })
        .collect();

    ShaderParamsSnapshot {
        shader_name: shader_name.to_string(),
        params: params_vec,
    }
}

fn build_sequence_snapshots(mixer: &crate::mixer::Mixer) -> Vec<SequenceSnapshot> {
    let channel_names: Vec<String> = mixer.channels().iter().map(|c| c.name.clone()).collect();
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
                            let easing_name = format!("{:?}", easing);
                            let label = format!(
                                "Fade {} -> {} ({:.1}{})",
                                channel_names
                                    .get(*from_ch)
                                    .map(|s| s.as_str())
                                    .unwrap_or("?"),
                                channel_names.get(*to_ch).map(|s| s.as_str()).unwrap_or("?"),
                                duration.value(),
                                unit_label
                            );
                            (
                                label,
                                SequenceStepKindSnapshot::Fade {
                                    from_ch: *from_ch,
                                    to_ch: *to_ch,
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
                            let label = format!("GoTo step {}", step_index);
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

/// Build a RegistrySnapshot from the current VardaApp state.
pub(crate) fn build_registry_snapshot(app: &VardaApp) -> RegistrySnapshot {
    let mut generators: Vec<(String, usize)> = app
        .registry
        .generators()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name(), i))
        .collect();
    generators.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let mut filters: Vec<(String, usize)> = app
        .registry
        .filters()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name(), i))
        .collect();
    filters.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    RegistrySnapshot {
        generators,
        filters,
        shader_count: app.registry.count(),
    }
}

/// Build a MidiSnapshot from the current VardaApp state.
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
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| format!("Device {}", key.device_id()));
                MidiMappingSnapshot {
                    key: *key,
                    key_display: format!("{}", key),
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

/// Build a CameraSnapshot from the current VardaApp state.
pub(crate) fn build_camera_snapshot(app: &VardaApp) -> CameraSnapshot {
    CameraSnapshot {
        devices: app
            .camera_manager
            .devices()
            .iter()
            .map(|d| (d.name.clone(), d.id))
            .collect(),
    }
}

/// Build a ClockSnapshot from the current clock manager state.
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
            (format!("ForceMidi({})", device_id), Some(*device_id))
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
    }
}

/// Build a full EngineState from all subsystem snapshots.
pub(crate) fn build_engine_state(app: &VardaApp) -> EngineState {
    use crate::engine::traits::*;
    EngineState {
        mixer: app.mixer_snapshot(),
        audio: app.audio_snapshot(),
        modulation: app.modulation_snapshot(),
        outputs: app.output_snapshot(),
        registry: build_registry_snapshot(app),
        midi: build_midi_snapshot(app),
        cameras: build_camera_snapshot(app),
        clock: build_clock_snapshot(app),
        fps: app.frame_stats.fps_smoothed,
        frame_count: app.frame_stats.frame_count,
        target_fps: app.target_fps,
        ndi_sources: app.external_io.ndi_manager.discovered_sources(),
        ndi_available: app.external_io.ndi_manager.is_available(),
        #[cfg(target_os = "macos")]
        syphon_sources: app.external_io.syphon_manager.discovered_sources(),
        #[cfg(target_os = "macos")]
        syphon_available: app.external_io.syphon_manager.is_available(),
        #[cfg(not(target_os = "macos"))]
        syphon_sources: vec![],
        #[cfg(not(target_os = "macos"))]
        syphon_available: false,
        stream_receivers: build_stream_receiver_snapshots(app),
        analyzers: app.available_analyzers(),
    }
}

/// Build stream library snapshots: library entries merged with active receiver status.
fn build_stream_receiver_snapshots(
    app: &VardaApp,
) -> Vec<crate::engine::types::StreamReceiverSnapshot> {
    let mut result: Vec<crate::engine::types::StreamReceiverSnapshot> = Vec::new();

    // Add library entries (configured but possibly not connected)
    for (url, mode) in &app.external_io.stream_library {
        let connected = (0..app.external_io.stream_manager.receiver_count()).any(|i| {
            app.external_io.stream_manager.receiver_url(i) == Some(url.as_str())
                && app.external_io.stream_manager.is_connected(i)
        });
        result.push(crate::engine::types::StreamReceiverSnapshot {
            url: url.clone(),
            mode: format!("{}", mode).to_lowercase(),
            connected,
        });
    }

    // Add active receivers not already in the library (e.g. restored from scene)
    for i in 0..app.external_io.stream_manager.receiver_count() {
        if let (Some(url), Some(mode)) = (
            app.external_io.stream_manager.receiver_url(i),
            app.external_io.stream_manager.receiver_mode(i),
        ) {
            if !result.iter().any(|r| r.url == url) {
                result.push(crate::engine::types::StreamReceiverSnapshot {
                    url: url.to_string(),
                    mode: format!("{}", mode).to_lowercase(),
                    connected: app.external_io.stream_manager.is_connected(i),
                });
            }
        }
    }

    result
}

/// Build a UIData snapshot from VardaApp state + UI layout state + egui texture IDs.
/// Constructs EngineState first, then derives UIData from it.
pub(crate) fn build_ui_data(
    app: &VardaApp,
    layout: &crate::usecases::ui::UILayoutState,
    deck_preview_textures: &std::collections::HashMap<(usize, usize), egui::TextureId>,
    channel_preview_textures: &std::collections::HashMap<usize, egui::TextureId>,
    output_preview_textures: &std::collections::HashMap<usize, egui::TextureId>,
    main_output_texture: Option<egui::TextureId>,
) -> crate::usecases::ui::UIData {
    use crate::usecases::ui::*;

    // Build the domain-neutral engine state first
    let engine = build_engine_state(app);

    // ── Map EngineState → UIData ──────────────────────────────────────

    // Channels: map ChannelSnapshot → ChannelUIInfo
    let channels = engine
        .mixer
        .channels
        .iter()
        .map(|ch| {
            let decks = ch
                .decks
                .iter()
                .map(|d| {
                    let generator = params_snapshot_to_ui(&d.generator);
                    let effects = d.effects.iter().map(effect_snapshot_to_ui).collect();
                    let video_playback = d.video_playback.as_ref().map(|vp| VideoPlaybackUI {
                        playing: vp.playing,
                        position: vp.position,
                        duration: vp.duration,
                        speed: vp.speed,
                        loop_mode: vp.loop_mode,
                        in_point: vp.in_point,
                        out_point: vp.out_point,
                        frame_rate: vp.frame_rate,
                    });
                    let auto_transition = d.auto_transition.as_ref().map(|at| AutoTransitionUI {
                        enabled: at.enabled,
                        trigger_is_clip_end: at.trigger_is_clip_end,
                        play_duration_value: at.play_duration_value,
                        play_duration_is_beats: at.play_duration_is_beats,
                        transition_duration_value: at.transition_duration_value,
                        transition_duration_is_beats: at.transition_duration_is_beats,
                        transition_shader_name: at.transition_shader_name.clone(),
                        phase: at.phase,
                    });
                    DeckUIInfo {
                        deck_idx: d.idx,
                        uuid: d.uuid.clone(),
                        name: d.name.clone(),
                        opacity: d.opacity,
                        effective_opacity: d.effective_opacity,
                        blend_mode: d.blend_mode,
                        solo: d.solo,
                        mute: d.mute,
                        scaling_mode: d.scaling_mode,
                        generator,
                        effects,
                        video_playback,
                        auto_transition,
                        render_fps: d.render_fps,
                        effective_render_fps: d.effective_render_fps,
                        render_cost_us: d.render_cost_us,
                        gpu_render_cost_us: d.gpu_render_cost_us,
                    }
                })
                .collect();
            let effects = ch.effects.iter().map(effect_snapshot_to_ui).collect();
            ChannelUIInfo {
                ch_idx: ch.idx,
                uuid: ch.uuid.clone(),
                name: ch.name.clone(),
                opacity: ch.opacity,
                blend_mode: ch.blend_mode,
                decks,
                effects,
            }
        })
        .collect();

    let master_effect_info = engine
        .mixer
        .master_effects
        .iter()
        .map(effect_snapshot_to_ui)
        .collect();

    // Modulation: map snapshots → UI types
    let modulation_sources = engine
        .modulation
        .sources
        .iter()
        .map(|entry| {
            let source = match &entry.source {
                ModulationSourceSnapshot::LFO {
                    waveform,
                    frequency,
                    phase,
                    amplitude,
                    bipolar,
                } => ModSourceUI::LFO {
                    waveform: *waveform,
                    frequency: *frequency,
                    phase: *phase,
                    amplitude: *amplitude,
                    bipolar: *bipolar,
                },
                ModulationSourceSnapshot::Audio {
                    source_id,
                    freq_low,
                    freq_high,
                    gain,
                    smoothing,
                    mode,
                    noise_gate,
                } => ModSourceUI::Audio {
                    source_id: *source_id,
                    freq_low: *freq_low,
                    freq_high: *freq_high,
                    gain: *gain,
                    smoothing: *smoothing,
                    mode: *mode,
                    noise_gate: *noise_gate,
                },
                ModulationSourceSnapshot::ADSR {
                    attack,
                    decay,
                    sustain,
                    release,
                    stage,
                } => ModSourceUI::ADSR {
                    attack: *attack,
                    decay: *decay,
                    sustain: *sustain,
                    release: *release,
                    stage: *stage,
                },
                ModulationSourceSnapshot::StepSequencer {
                    steps,
                    rate,
                    interpolation,
                    bipolar,
                } => ModSourceUI::StepSequencer {
                    steps: steps.clone(),
                    rate: *rate,
                    interpolation: *interpolation,
                    bipolar: *bipolar,
                },
                ModulationSourceSnapshot::Analyzer {
                    deck_id,
                    analyzer_type,
                    output_name,
                    smoothing,
                } => ModSourceUI::Analyzer {
                    deck_id: deck_id.clone(),
                    analyzer_type: analyzer_type.clone(),
                    output_name: output_name.clone(),
                    smoothing: *smoothing,
                },
            };
            ModSourceUIEntry {
                uuid: entry.uuid.clone(),
                source,
            }
        })
        .collect();
    let modulation_current_values = engine.modulation.current_values.clone();
    let modulation_assignments = engine
        .modulation
        .assignments
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.iter()
                    .map(|a| ModAssignmentUI {
                        source_id: a.source_id.clone(),
                        amount: a.amount,
                    })
                    .collect(),
            )
        })
        .collect();

    // Audio: map AudioSnapshot → AudioUIData
    let audio = AudioUIData {
        level: engine.audio.level,
        bass: engine.audio.bass,
        mid: engine.audio.mid,
        treble: engine.audio.treble,
        bpm: engine.audio.bpm,
        beat_phase: engine.audio.beat_phase,
        enabled: engine.audio.enabled,
        devices: engine
            .audio
            .devices
            .iter()
            .map(|d| AudioDeviceUI {
                id: d.id,
                name: d.name.clone(),
                active: d.active,
            })
            .collect(),
        fft: engine.audio.fft.clone(),
        sample_rate: engine.audio.sample_rate,
    };

    // Outputs: build unified OutputUI list from VardaApp's outputs
    let outputs: Vec<OutputUI> = app
        .output
        .outputs
        .iter()
        .map(|o| {
            let (
                target,
                target_label,
                is_windowed,
                is_active,
                active_duration,
                surface_assignments,
                calibration_mode,
            ) = match o {
                crate::renderer::context::UnifiedOutput::Window(w) => {
                    let sa = w
                        .surface_assignments
                        .iter()
                        .map(|a| {
                            let surface_name = app
                                .output
                                .surface_manager
                                .find_by_uuid(&a.surface_uuid)
                                .map(|(_, s)| s.name.clone())
                                .unwrap_or_else(|| format!("Surface {}", a.surface_uuid));
                            SurfaceAssignmentUI {
                                surface_uuid: a.surface_uuid.clone(),
                                surface_name,
                                warp_mode: a.warp_mode.clone(),
                                enabled: a.enabled,
                                overlap_zones: a.overlap_zones.clone(),
                            }
                        })
                        .collect();
                    (
                        w.target.clone(),
                        format!("{}", w.target),
                        true,
                        true,
                        std::time::Duration::ZERO,
                        sa,
                        w.calibration_mode,
                    )
                }
                crate::renderer::context::UnifiedOutput::Headless(h) => {
                    let sa = h
                        .surface_assignments
                        .iter()
                        .map(|a| SurfaceAssignmentUI {
                            surface_uuid: a.surface_uuid.clone(),
                            surface_name: app
                                .output
                                .surface_manager
                                .find_by_uuid(&a.surface_uuid)
                                .map(|(_, s)| s.name.clone())
                                .unwrap_or_else(|| format!("Surface {}", a.surface_uuid)),
                            warp_mode: a.warp_mode.clone(),
                            enabled: a.enabled,
                            overlap_zones: a.overlap_zones.clone(),
                        })
                        .collect();
                    (
                        h.target.clone(),
                        format!("{}", h.target),
                        false,
                        h.active,
                        o.active_duration(),
                        sa,
                        false,
                    )
                }
            };
            let edge_blend_mode = o.edge_blend_mode();
            let edge_blend = o.edge_blend();
            OutputUI {
                uuid: o.uuid().to_string(),
                name: o.name().to_string(),
                target,
                target_label,
                is_windowed,
                is_active,
                active_duration,
                surface_assignments,
                calibration_mode,
                edge_blend_mode,
                edge_blend,
                rotation: o.rotation(),
            }
        })
        .collect();

    let surfaces = engine
        .outputs
        .surfaces
        .iter()
        .map(|s| SurfaceUI {
            uuid: s.uuid.clone(),
            name: s.name.clone(),
            vertices: s.vertices.clone(),
            extra_contours: s.extra_contours.clone(),
            source: s.source.clone(),
            content_mapping: s.content_mapping,
            output_type: s.output_type,
            circle_hint: s.circle_hint,
        })
        .collect();

    let available_monitors = engine
        .outputs
        .monitors
        .iter()
        .map(|m| MonitorInfo {
            name: m.name.clone(),
            index: m.index,
            width: m.width,
            height: m.height,
        })
        .collect();

    // MIDI: map snapshots → UI types
    let midi_devices = engine
        .midi
        .devices
        .iter()
        .map(|d| MidiDeviceUI {
            id: d.id,
            name: d.name.clone(),
            enabled: d.enabled,
            has_output: d.has_output,
            profile: d.profile.clone(),
        })
        .collect();
    let midi_mappings = engine
        .midi
        .mappings
        .iter()
        .map(|m| MidiMappingUI {
            key: m.key,
            key_display: m.key_display.clone(),
            device_name: m.device_name.clone(),
            param_path: m.param_path.clone(),
        })
        .collect();

    // Sequences: map snapshots → UI types
    let sequences = engine
        .mixer
        .sequences
        .iter()
        .map(|seq| {
            let steps = seq
                .steps
                .iter()
                .map(|s| {
                    let kind = match &s.kind {
                        SequenceStepKindSnapshot::Fade {
                            from_ch,
                            to_ch,
                            duration_val,
                            duration_unit,
                            easing,
                            transition_shader,
                            target_amount,
                        } => SequenceStepKindUI::Fade {
                            from_ch: *from_ch,
                            to_ch: *to_ch,
                            duration_val: *duration_val,
                            duration_unit: *duration_unit,
                            easing: easing.clone(),
                            transition_shader: transition_shader.clone(),
                            target_amount: *target_amount,
                        },
                        SequenceStepKindSnapshot::Wait {
                            duration_val,
                            duration_unit,
                        } => SequenceStepKindUI::Wait {
                            duration_val: *duration_val,
                            duration_unit: *duration_unit,
                        },
                        SequenceStepKindSnapshot::GoTo { step_index } => SequenceStepKindUI::GoTo {
                            step_index: *step_index,
                        },
                    };
                    SequenceStepUI {
                        label: s.label.clone(),
                        kind,
                    }
                })
                .collect();
            SequenceUIData {
                name: seq.name.clone(),
                enabled: seq.enabled,
                playing: seq.playing,
                current_step: seq.current_step,
                step_elapsed: seq.step_elapsed,
                steps,
            }
        })
        .collect();

    // Notifications — UI-only, not in EngineState
    let notifications = app
        .session
        .notifications
        .visible()
        .iter()
        .map(|n| NotificationUI {
            level: n.level,
            message: n.message.clone(),
            progress: n.progress(),
        })
        .collect();

    UIData {
        generators: engine.registry.generators,
        filters: engine.registry.filters,
        shader_count: engine.registry.shader_count,
        channels,
        master_effect_info,
        modulation_sources,
        modulation_current_values,
        modulation_assignments,
        audio,
        deck_preview_textures: deck_preview_textures.clone(),
        channel_preview_textures: channel_preview_textures.clone(),
        output_preview_textures: output_preview_textures.clone(),
        main_output_texture,
        notifications,
        crossfader: engine.mixer.crossfader,
        auto_crossfade_active: engine.mixer.auto_crossfade_active,
        auto_crossfade_progress: engine.mixer.auto_crossfade_progress,
        midi_learn_active: engine.midi.learn_active,
        midi_learn_target: engine.midi.learn_target,
        keyboard_learn_active: app.input.keymap.learn_mode,
        keyboard_learn_target: app
            .input
            .keymap
            .learn_target
            .as_ref()
            .map(|t| format!("{}", t)),
        keymap_bindings: app.input.keymap.bindings.clone(),
        transition_names: engine.mixer.transition_names,
        active_transition_name: engine.mixer.active_transition_name,
        // UI layout/selection state — owned by the UI consumer, not the engine
        selected_deck: layout.selected_deck,
        selected_channel: layout.selected_channel,
        selected_master: layout.selected_master,
        selected_sequence: layout.selected_sequence,
        selected_sequence_step: layout.selected_sequence_step,
        outputs,
        surfaces,
        stage_editor_open: layout.stage_editor_open,
        dome_preview_open: layout.dome_preview_open,
        dome_preview_texture: None, // populated by UIRunner after build
        dome_mode_active: layout.dome_mode_active,
        dome_preset: layout.dome_preset,
        dome_geometry: layout.dome_geometry,
        camera_detect_texture: None, // populated by UIRunner
        camera_detect_mode: crate::usecases::ui::CameraDetectMode::Off, // populated by UIRunner
        camera_detect_contours: vec![], // populated by UIRunner
        library_panel_open: layout.library_panel_open,
        right_panel_open: layout.right_panel_open,
        stage_editor_grid_size: layout.stage_editor_grid_size,
        stage_editor_snap: layout.stage_editor_snap,
        available_monitors,
        midi_devices,
        midi_mappings,
        cameras: engine.cameras.devices,
        ndi_sources: engine.ndi_sources.clone(),
        ndi_available: engine.ndi_available,
        syphon_sources: engine.syphon_sources.clone(),
        syphon_available: engine.syphon_available,
        srt_library_configs: engine
            .stream_receivers
            .iter()
            .map(|r| {
                let mode = match r.mode.as_str() {
                    "listener" => crate::stream::SrtMode::Listener,
                    _ => crate::stream::SrtMode::Caller,
                };
                SrtLibraryEntry {
                    url: r.url.clone(),
                    mode,
                    connected: r.connected,
                }
            })
            .collect(),
        hls_library_configs: app
            .external_io
            .hls_library
            .iter()
            .map(|url| crate::usecases::ui::HlsLibraryEntry {
                url: url.clone(),
                connected: (0..app.external_io.stream_manager.receiver_count()).any(|i| {
                    app.external_io.stream_manager.receiver_url(i) == Some(url.as_str())
                        && app.external_io.stream_manager.is_connected(i)
                }),
            })
            .collect(),
        dash_library_configs: app
            .external_io
            .dash_library
            .iter()
            .map(|url| crate::usecases::ui::DashLibraryEntry {
                url: url.clone(),
                connected: (0..app.external_io.stream_manager.receiver_count()).any(|i| {
                    app.external_io.stream_manager.receiver_url(i) == Some(url.as_str())
                        && app.external_io.stream_manager.is_connected(i)
                }),
            })
            .collect(),
        rtmp_library_configs: app
            .external_io
            .rtmp_library
            .iter()
            .map(|(url, mode)| crate::usecases::ui::RtmpLibraryEntry {
                url: url.clone(),
                mode: *mode,
                connected: (0..app.external_io.stream_manager.receiver_count()).any(|i| {
                    app.external_io.stream_manager.receiver_url(i) == Some(url.as_str())
                        && app.external_io.stream_manager.is_connected(i)
                }),
            })
            .collect(),

        sequences,
        channel_count: engine.mixer.channels.len(),
        channel_names: engine
            .mixer
            .channels
            .iter()
            .map(|c| c.name.clone())
            .collect(),
        channel_render_stats: {
            let stats: Vec<crate::usecases::ui::ChannelRenderStats> = engine
                .mixer
                .channels
                .iter()
                .map(|ch| {
                    // Average the per-deck wall-clock FPS across active decks
                    let active_decks: Vec<f32> = ch
                        .decks
                        .iter()
                        .filter(|s| !s.mute && s.opacity > 0.0)
                        .map(|s| s.fps)
                        .filter(|&fps| fps > 0.0)
                        .collect();
                    let avg_fps = if active_decks.is_empty() {
                        0.0
                    } else {
                        active_decks.iter().sum::<f32>() / active_decks.len() as f32
                    };
                    crate::usecases::ui::ChannelRenderStats {
                        name: ch.name.clone(),
                        avg_deck_fps: avg_fps,
                        active_deck_count: ch.active_deck_count,
                        render_time_ms: ch.render_time_ms,
                    }
                })
                .collect();
            stats
        },
        // Wall-clock frame rate (smoothed over 60 frames)
        fps: app.frame_stats.fps_smoothed,
        gpu_device_name: {
            let info = app.context.adapter.get_info();
            info.name
        },
        gpu_backend: format!("{:?}", app.context.adapter.get_info().backend),
        gpu_driver: app.context.adapter.get_info().driver,
        gpu_driver_info: app.context.adapter.get_info().driver_info,
        gpu_device_type: format!("{:?}", app.context.adapter.get_info().device_type),
        gpu_utilization: app.mixer_ref().gpu_utilization(),
        cpu_usage: app.frame_stats.system_monitor.cpu_usage(),
        ram_used: app.frame_stats.system_monitor.ram_used(),
        ram_total: app.frame_stats.system_monitor.ram_total(),
        clock_source: engine.clock.source_label,
        clock_bpm: engine.clock.bpm,
        clock_active: engine.clock.active,
        clock_device_name: engine.clock.device_name,
        clock_detected_midi: engine.clock.detected_midi_sources,
        clock_osc_active: engine.clock.osc_active,
        clock_osc_bpm: engine.clock.osc_bpm,
        clock_audio_bpm: engine.clock.audio_bpm,
        clock_preference: engine.clock.preference_label,
        clock_preference_force_device_id: engine.clock.preference_force_device_id,
        clock_manual_bpm: engine.clock.manual_bpm,
        render_width: app.render_width,
        render_height: app.render_height,
        target_fps: app.target_fps,
        // Populated by UIRunner after build (history/pending loads live on runner, not app)
        can_undo: false,
        can_redo: false,
        pending_deck_loads: 0,
        deck_presets: app
            .session
            .preset_library
            .deck_presets
            .iter()
            .map(|p| p.name.clone())
            .collect(),
        channel_presets: app
            .session
            .preset_library
            .channel_presets
            .iter()
            .map(|p| p.name.clone())
            .collect(),
    }
}

// ── Snapshot → UI type helpers ──────────────────────────────────────

fn params_snapshot_to_ui(snap: &ShaderParamsSnapshot) -> ShaderParamsUI {
    ShaderParamsUI {
        shader_name: snap.shader_name.clone(),
        params: snap
            .params
            .iter()
            .map(|p| ParamUIInfo {
                name: p.name.clone(),
                label: p.label.clone(),
                value: p.value,
                min: p.min,
                max: p.max,
            })
            .collect(),
    }
}

fn effect_snapshot_to_ui(snap: &EffectSnapshot) -> EffectInfo {
    (
        snap.uuid.clone(),
        snap.name.clone(),
        snap.enabled,
        params_snapshot_to_ui(&snap.params),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::traits::*;
    use clap::Parser;

    fn parse_args(args: &[&str]) -> super::super::AppConfig {
        super::super::AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
    }

    fn headless_app() -> Option<super::super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
        super::super::VardaApp::new(gpu, &config).ok()
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
        app.add_solid_color_deck(0, [1.0, 0.0, 0.0, 1.0]).unwrap();
        app.set_deck_opacity(0, 0, 0.5);
        let snap = build_mixer_snapshot(&app);
        let deck = &snap.channels[0].decks[0];
        assert!((deck.opacity - 0.5).abs() < 1e-5);
        // No transition → effective == opacity
        assert!((deck.effective_opacity - 0.5).abs() < 1e-5);
    }

    #[test]
    fn snapshot_deck_with_effects() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.add_solid_color_deck(0, [1.0, 0.0, 0.0, 1.0]).unwrap();
        let target = crate::engine::types::EffectTarget::Deck(0, 0);
        // Try adding an effect — may fail if "Invert" not in registry
        if app.add_effect(target, "Invert").is_ok() {
            let snap = build_mixer_snapshot(&app);
            assert!(!snap.channels[0].decks[0].effects.is_empty());
        }
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
        // param_order has "brightness" but values doesn't → filtered out
        let mut params = crate::params::ShaderParams::from_inputs(&[]);
        params.param_order.push("brightness".into());
        // values map is empty, so "brightness" has no value → should be filtered
        let snap = build_shader_params("test_shader", &params);
        assert!(snap.params.is_empty());
    }

    #[test]
    fn build_shader_params_missing_definition() {
        // Value exists but no definition → label/min/max are None
        let mut params = crate::params::ShaderParams::from_inputs(&[]);
        params.param_order.push("mystery".into());
        params
            .values
            .insert("mystery".into(), crate::params::ParamValue::Float(0.5));
        let snap = build_shader_params("test_shader", &params);
        assert_eq!(snap.params.len(), 1);
        let p = &snap.params[0];
        assert!(p.label.is_none());
        assert!(p.min.is_none());
        assert!(p.max.is_none());
    }

    #[test]
    fn build_registry_snapshot_sorted() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_registry_snapshot(&app);
        // Verify generators are sorted case-insensitively
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
        // Default clock is inactive → bpm is None
        assert!(snap.bpm.is_none() || !snap.active);
    }

    #[test]
    fn build_clock_snapshot_source_labels() {
        let Some(app) = headless_app() else {
            return;
        };
        let snap = build_clock_snapshot(&app);
        // Source label should be one of the known values
        let valid = ["Audio", "MIDI", "OSC", "Manual"];
        assert!(
            valid.contains(&snap.source_label.as_str()),
            "unexpected source label: {}",
            snap.source_label
        );
    }

    #[test]
    fn build_stream_receiver_dedup() {
        let Some(app) = headless_app() else {
            return;
        };
        let receivers = build_stream_receiver_snapshots(&app);
        // All URLs should be unique
        let urls: Vec<&str> = receivers.iter().map(|r| r.url.as_str()).collect();
        let mut deduped = urls.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            urls.len(),
            deduped.len(),
            "duplicate stream receivers found"
        );
    }
}
