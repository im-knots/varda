//! UIData builder — derives the UI consumer's view-model from `EngineState`.
//!
//! Lives here (not `app/snapshot.rs`) because it is presentation mapping:
//! it names egui's `TextureId` and constructs `usecases::ui::UIData` itself.
//! See `/spec/app-presentation-boundary.md`. `app::VardaApp::build_engine_state()`
//! remains the one legitimate cross-layer read — it returns a plain,
//! framework-free `EngineState` that this function then maps from.

use super::*;
use crate::app::VardaApp;
use crate::engine::types::*;

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
    let engine = app.build_engine_state();

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
                        is_html: d.is_html,
                        is_html_interactive: d.is_html_interactive,
                        opacity: d.opacity,
                        effective_opacity: d.effective_opacity,
                        blend_mode: d.blend_mode,
                        solo: d.solo,
                        mute: d.mute,
                        transparent: d.transparent,
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
                        crate::renderer::context::CalibrationMode::Off,
                    )
                }
            };
            let edge_blend_mode = o.edge_blend_mode();
            let edge_blend = o.edge_blend();
            let audio_passthrough = match o {
                crate::renderer::context::UnifiedOutput::Headless(h) => {
                    h.audio_pcm.as_ref().map(|p| AudioPassthroughUI {
                        device: h.target.audio_device().unwrap_or_default().to_string(),
                        frames_written: h
                            .subprocess
                            .as_ref()
                            .and_then(|s| s.audio_frames_written())
                            .unwrap_or(0),
                        frames_dropped: p.dropped.load(std::sync::atomic::Ordering::Relaxed),
                    })
                }
                _ => None,
            };
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
                audio_passthrough,
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
            warp: s.warp.clone(),
            warp_bound: s.warp_bound,
            path: s.path.clone(),
            holes: s.holes.clone(),
            hole_contours: s.hole_contours.clone(),
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
        macros: engine.macros.clone(),
        audio,
        deck_preview_textures: deck_preview_textures.clone(),
        channel_preview_textures: channel_preview_textures.clone(),
        output_preview_textures: output_preview_textures.clone(),
        main_output_texture,
        notifications,
        crossfader: engine.mixer.crossfader,
        auto_crossfade_active: engine.mixer.auto_crossfade_active,
        auto_crossfade_progress: engine.mixer.auto_crossfade_progress,
        tonemap_mode: engine.mixer.tonemap_mode,
        active_lut_filename: engine.mixer.active_lut.clone(),
        available_luts: list_available_luts(&app.session.workspace),
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
        selected_macro: layout.selected_macro.clone(),
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
        html_library_configs: app
            .external_io
            .html_library
            .iter()
            .map(|url| crate::usecases::ui::HtmlLibraryEntry {
                url: url.clone(),
                active: (0..app.external_io.html_manager.instance_count())
                    .any(|i| app.external_io.html_manager.instance_url(i) == Some(url.as_str())),
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
            let info = app.gpu_context().adapter.get_info();
            info.name
        },
        gpu_backend: format!("{:?}", app.gpu_context().adapter.get_info().backend),
        gpu_driver: app.gpu_context().adapter.get_info().driver,
        gpu_driver_info: app.gpu_context().adapter.get_info().driver_info,
        gpu_device_type: format!("{:?}", app.gpu_context().adapter.get_info().device_type),
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
        render_width: app.render_width(),
        render_height: app.render_height(),
        max_render_dimension: app.max_render_dimension(),
        target_fps: app.target_fps(),
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

/// Scan `.varda/luts/` for available LUT files (.cube, .3dl).
fn list_available_luts(workspace: &crate::persistence::Workspace) -> Vec<String> {
    let dir = workspace.luts_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".cube") || lower.ends_with(".3dl") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
}
