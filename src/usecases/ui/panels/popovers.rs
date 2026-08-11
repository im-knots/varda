//! Status-bar popovers and the global MIDI-learn popup.
//!
//! Transient overlays rendered above the main layout: FPS, GPU, clock,
//! transport, resolution and target FPS readouts, plus the right-click
//! MIDI-learn toggle.

use super::super::{UIActions, UIData};
use crate::engine::EngineCommand;

/// Global right-click popup for toggling MIDI learn mode.
pub(super) fn handle_midi_learn_popup(ctx: &egui::Context, data: &UIData, actions: &mut UIActions) {
    let popup_id = egui::Id::new("global_midi_learn_popup");
    let popup_fresh_id = egui::Id::new("global_midi_learn_popup_fresh");

    let popup_pos: Option<egui::Pos2> = ctx.memory(|mem| mem.data.get_temp(popup_id));
    let popup_fresh: bool = ctx.memory(|mem| mem.data.get_temp(popup_fresh_id).unwrap_or(false));

    if ctx.input(|i| i.pointer.secondary_clicked()) {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            if popup_pos.is_some() {
                ctx.memory_mut(|mem| {
                    mem.data.remove::<egui::Pos2>(popup_id);
                    mem.data.remove::<bool>(popup_fresh_id);
                });
            } else if !egui::Popup::is_any_open(ctx) {
                // A widget with its own context menu keeps the click. This popup
                // is drawn last, so opening it on the same right-click would
                // cover that menu and eat every press aimed at its items.
                ctx.memory_mut(|mem| {
                    mem.data.insert_temp(popup_id, pos);
                    mem.data.insert_temp(popup_fresh_id, true);
                });
            }
        }
    }

    let popup_pos: Option<egui::Pos2> = ctx.memory(|mem| mem.data.get_temp(popup_id));
    if let Some(pos) = popup_pos {
        let label = if data.midi_learn_active {
            "🎹 Exit MIDI Learn"
        } else {
            "🎹 Enter MIDI Learn"
        };

        let area_resp = egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    if ui.button(label).clicked() {
                        actions.session.midi_learn_toggle = true;
                        ctx.memory_mut(|mem| {
                            mem.data.remove::<egui::Pos2>(popup_id);
                            mem.data.remove::<bool>(popup_fresh_id);
                        });
                    }
                    let kb_label = if data.keyboard_learn_active {
                        "⌨ Exit Keyboard Learn"
                    } else {
                        "⌨ Enter Keyboard Learn"
                    };
                    if ui.button(kb_label).clicked() {
                        actions.session.keyboard_learn_toggle = true;
                        ctx.memory_mut(|mem| {
                            mem.data.remove::<egui::Pos2>(popup_id);
                            mem.data.remove::<bool>(popup_fresh_id);
                        });
                    }
                });
            });

        if popup_fresh {
            ctx.memory_mut(|mem| {
                mem.data.insert_temp(popup_fresh_id, false);
            });
        } else if ctx.input(|i| i.pointer.primary_clicked()) {
            let popup_rect = area_resp.response.rect;
            let click_pos = ctx.input(|i| i.pointer.interact_pos());
            if let Some(click) = click_pos {
                if !popup_rect.contains(click) {
                    ctx.memory_mut(|mem| {
                        mem.data.remove::<egui::Pos2>(popup_id);
                        mem.data.remove::<bool>(popup_fresh_id);
                    });
                }
            }
        }
    }
}

/// Render the FPS details popover (shown when clicking FPS in the top bar).
pub(super) fn render_fps_popover(ui: &mut egui::Ui, data: &UIData) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("⏱ Render Pipeline").strong());
    ui.separator();

    let fps_color = |fps: f32| {
        if fps > 55.0 {
            egui::Color32::from_rgb(100, 220, 100)
        } else if fps > 30.0 {
            egui::Color32::from_rgb(220, 200, 60)
        } else {
            egui::Color32::from_rgb(220, 60, 60)
        }
    };

    ui.horizontal(|ui| {
        ui.label("Avg pipeline FPS:");
        ui.label(
            egui::RichText::new(format!("{:.0}", data.fps))
                .color(fps_color(data.fps))
                .monospace()
                .strong(),
        );
    });
    ui.add_space(4.0);

    if data.channel_render_stats.is_empty() {
        ui.label(egui::RichText::new("No channels").weak());
    } else {
        egui::Grid::new("fps_channel_grid")
            .striped(true)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Channel").strong().small());
                ui.label(egui::RichText::new("Avg FPS").strong().small());
                ui.label(egui::RichText::new("Decks").strong().small());
                ui.label(egui::RichText::new("Time").strong().small());
                ui.end_row();

                for stat in &data.channel_render_stats {
                    ui.label(&stat.name);
                    if stat.avg_deck_fps > 0.0 {
                        ui.label(
                            egui::RichText::new(format!("{:.0}", stat.avg_deck_fps))
                                .color(fps_color(stat.avg_deck_fps))
                                .monospace(),
                        );
                    } else {
                        ui.label(egui::RichText::new("--").weak());
                    }
                    ui.label(format!("{}", stat.active_deck_count));
                    ui.label(
                        egui::RichText::new(format!("{:.1}ms", stat.render_time_ms)).monospace(),
                    );
                    ui.end_row();
                }
            });

        let total_active: u32 = data
            .channel_render_stats
            .iter()
            .map(|s| s.active_deck_count)
            .sum();
        let total_ms: f32 = data
            .channel_render_stats
            .iter()
            .map(|s| s.render_time_ms)
            .sum();
        ui.add_space(4.0);
        ui.separator();
        ui.label(format!(
            "{total_active} active decks · {total_ms:.1}ms total render"
        ));
    }
}

/// Render the GPU details popover (shown when clicking GPU device in the top bar).
pub(super) fn render_gpu_popover(ui: &mut egui::Ui, data: &UIData) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("🖥 GPU Details").strong());
    ui.separator();

    egui::Grid::new("gpu_details_grid").show(ui, |ui| {
        ui.label(egui::RichText::new("Device").strong().small());
        ui.label(&data.gpu_device_name);
        ui.end_row();

        ui.label(egui::RichText::new("Type").strong().small());
        ui.label(&data.gpu_device_type);
        ui.end_row();

        ui.label(egui::RichText::new("Backend").strong().small());
        ui.label(&data.gpu_backend);
        ui.end_row();

        if !data.gpu_driver.is_empty() {
            ui.label(egui::RichText::new("Driver").strong().small());
            ui.label(&data.gpu_driver);
            ui.end_row();
        }

        if !data.gpu_driver_info.is_empty() {
            ui.label(egui::RichText::new("Driver info").strong().small());
            ui.label(&data.gpu_driver_info);
            ui.end_row();
        }

        let util = data.gpu_utilization;
        ui.label(egui::RichText::new("Utilization").strong().small());
        let util_color = if util < 50.0 {
            egui::Color32::from_rgb(100, 220, 100)
        } else if util < 80.0 {
            egui::Color32::from_rgb(220, 200, 60)
        } else {
            egui::Color32::from_rgb(220, 60, 60)
        };
        ui.label(
            egui::RichText::new(format!("{util:.0}%"))
                .color(util_color)
                .monospace(),
        );
        ui.end_row();
    });
}

/// Colour for the top bar position readout.
///
/// Chasing timecode is deliberately loud: it is the state where the position is
/// out of the operator's hands, and finding that out by scrubbing and having
/// nothing move is a bad way to learn it.
pub(super) fn transport_color(data: &UIData) -> egui::Color32 {
    if data.transport.source == crate::transport::TransportSource::Timecode {
        egui::Color32::from_rgb(120, 180, 255)
    } else if data.transport.running {
        egui::Color32::from_rgb(100, 220, 100)
    } else if data.transport.has_run {
        egui::Color32::from_rgb(220, 200, 60)
    } else {
        egui::Color32::GRAY
    }
}

/// Whether the tempo readout is currently driving anything.
///
/// Tempo and position are both always shown, in both modes; the weaker of the
/// two is the one nothing is reading. That is a statement about engine state,
/// not about which UI mode is open, so it stays honest when a show is running
/// on both clocks at once. See /spec/transport.md § Tempo and position are both
/// shown.
pub(super) fn clock_is_live(data: &UIData) -> bool {
    data.clock_active && data.clock_beat_followers > 0
}

/// Hover text naming what follows each clock.
///
/// This is the actual answer to "why is something moving when I have not
/// pressed play": there are three motion sources (free-running, beat-locked,
/// transport-locked) and only two have readouts, so the readouts have to say
/// what depends on them.
pub(super) fn followers_hint(count: usize, what: &str) -> String {
    match count {
        0 => format!("nothing is locked to {what}"),
        1 => format!("1 modulator locked to {what}"),
        n => format!("{n} modulators locked to {what}"),
    }
}

/// Render the transport popover (shown when clicking the position in the top bar).
pub(super) fn render_transport_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    use crate::transport::{TimecodeRate, TransportSource};

    let t = &data.transport;
    ui.set_min_width(240.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("⏱ Transport").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(&t.status_label).small().weak());
        });
    });
    ui.separator();

    ui.label(
        egui::RichText::new(&t.timecode)
            .monospace()
            .size(20.0)
            .color(transport_color(data)),
    );

    // Position is read-only while chasing, so offering the controls would be
    // offering a lie.
    let scrubbable = t.source == TransportSource::Internal;

    ui.horizontal(|ui| {
        ui.add_enabled_ui(scrubbable, |ui| {
            let play_label = if t.running { "⏸ Pause" } else { "▶ Play" };
            if ui.button(play_label).clicked() {
                actions.commands.push(if t.running {
                    EngineCommand::TransportStop
                } else {
                    EngineCommand::TransportPlay
                });
            }
            if ui
                .button("⏮ Zero")
                .on_hover_text("Return to 00:00:00:00")
                .clicked()
            {
                actions
                    .commands
                    .push(EngineCommand::TransportLocate { position: 0.0 });
            }
        });
    });

    ui.separator();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Source").small());
        for (source, label) in [
            (TransportSource::Internal, "Internal"),
            (TransportSource::Timecode, "Timecode"),
        ] {
            if ui.radio(t.source == source, label).clicked() && t.source != source {
                actions
                    .commands
                    .push(EngineCommand::SetTransportSource { source });
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Rate").small());
        egui::ComboBox::from_id_salt("transport_rate")
            .selected_text(t.timecode_rate.label())
            .show_ui(ui, |ui| {
                for rate in TimecodeRate::ALL {
                    if ui
                        .selectable_label(t.timecode_rate == rate, rate.label())
                        .clicked()
                        && t.timecode_rate != rate
                    {
                        actions
                            .commands
                            .push(EngineCommand::SetTimecodeRate { rate });
                    }
                }
            });
    });
}

/// Render the clock source popover (shown when clicking BPM in the top bar).
pub(super) fn render_clock_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("🕐 Clock Source").strong());
    ui.separator();

    let is_auto = data.clock_preference == "Auto";

    // Auto option
    if ui.radio(is_auto, "Auto (recommended)").clicked() && !is_auto {
        actions.commands.push(EngineCommand::SetClockPreference {
            preference: crate::clock::ClockPreference::Auto,
        });
    }

    // Detected MIDI devices
    for src in &data.clock_detected_midi {
        let is_selected = data.clock_preference_force_device_id == Some(src.device_id);
        let bpm_str = src.bpm.map_or("--".to_string(), |b| format!("{b:.0}"));
        let label = format!("🟣 {}  {} BPM", src.device_name, bpm_str);
        if ui.radio(is_selected, label).clicked() && !is_selected {
            actions.commands.push(EngineCommand::SetClockPreference {
                preference: crate::clock::ClockPreference::ForceMidi {
                    device_id: src.device_id,
                },
            });
        }
    }

    // OSC option (only shown if OSC is active)
    if data.clock_osc_active {
        let is_osc = data.clock_preference == "ForceOsc";
        let bpm_str = data
            .clock_osc_bpm
            .map_or("--".to_string(), |b| format!("{b:.0}"));
        let label = format!("🔵 OSC  {bpm_str} BPM");
        if ui.radio(is_osc, label).clicked() && !is_osc {
            actions.commands.push(EngineCommand::SetClockPreference {
                preference: crate::clock::ClockPreference::ForceOsc,
            });
        }
    }

    // Audio only option
    let is_audio = data.clock_preference == "ForceAudio";
    let audio_bpm_str = data
        .clock_audio_bpm
        .map_or("--".to_string(), |b| format!("{b:.0}"));
    let label = format!("🟢 Audio only  {audio_bpm_str} BPM");
    if ui.radio(is_audio, label).clicked() && !is_audio {
        actions.commands.push(EngineCommand::SetClockPreference {
            preference: crate::clock::ClockPreference::ForceAudio,
        });
    }

    // Manual BPM option
    let is_manual = data.clock_preference == "ForceManual";
    let mut manual_bpm = data.clock_manual_bpm.unwrap_or(120.0);
    ui.horizontal(|ui| {
        if ui.radio(is_manual, "🟠 Manual").clicked() && !is_manual {
            actions.commands.push(EngineCommand::SetClockPreference {
                preference: crate::clock::ClockPreference::ForceManual { bpm: manual_bpm },
            });
        }
        if is_manual {
            let drag = ui.add(
                egui::DragValue::new(&mut manual_bpm)
                    .range(20.0..=300.0)
                    .speed(0.5)
                    .suffix(" BPM"),
            );
            if drag.changed() {
                actions
                    .commands
                    .push(EngineCommand::SetManualBpm { bpm: manual_bpm });
            }
        }
    });

    // Current status line
    ui.separator();
    let status = match data.clock_source.as_str() {
        "MIDI" => {
            let dev = data.clock_device_name.as_deref().unwrap_or("Unknown");
            format!(
                "Currently: {} ({})",
                dev,
                if is_auto { "auto" } else { "forced" }
            )
        }
        "OSC" => format!(
            "Currently: OSC ({})",
            if is_auto { "auto" } else { "forced" }
        ),
        "Audio" => format!(
            "Currently: Audio ({})",
            if is_auto { "auto" } else { "forced" }
        ),
        "Manual" => format!("Currently: Manual ({manual_bpm:.0} BPM)"),
        _ => "Currently: No clock".to_string(),
    };
    ui.label(egui::RichText::new(status).weak().small());
}

/// A named render resolution offered in the popover: label, width, height.
type ResolutionPreset = (&'static str, u32, u32);

/// Render the resolution popover (shown when clicking resolution in the top bar).
pub(super) fn render_resolution_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(200.0);
    ui.label(egui::RichText::new("📐 Render Resolution").strong());
    ui.separator();

    let current_w = data.render_width;
    let current_h = data.render_height;

    // Landscape presets, then the shapes short-form video actually ships in.
    // 1080×1920 is the single 9:16 master every vertical platform takes —
    // Reels, TikTok, Shorts, Stories and Facebook Reels all specify exactly
    // that. 1080×1350 is the 4:5 Instagram feed post, which claims more of the
    // scroll than square does, and 1080×1080 is the 1:1 fallback.
    let landscape: &[ResolutionPreset] = &[
        ("720p", 1280, 720),
        ("1080p", 1920, 1080),
        ("1440p", 2560, 1440),
        ("4K", 3840, 2160),
    ];
    let vertical: &[ResolutionPreset] = &[
        ("9:16 Reels / TikTok / Shorts", 1080, 1920),
        ("9:16 4K vertical", 2160, 3840),
        ("4:5 Instagram feed", 1080, 1350),
        ("1:1 Square", 1080, 1080),
    ];

    for (heading, presets) in [("Landscape", landscape), ("Vertical & square", vertical)] {
        ui.label(egui::RichText::new(heading).strong().small());
        for &(label, w, h) in presets {
            let is_current = current_w == w && current_h == h;
            let text = format!("{label} ({w}×{h})");
            if ui.radio(is_current, text).clicked() && !is_current {
                actions.commands.push(EngineCommand::SetRenderResolution {
                    width: w,
                    height: h,
                });
            }
        }
    }

    ui.separator();
    ui.label(egui::RichText::new("Custom").strong().small());

    // Custom W×H input — use persistent state via egui memory
    let custom_width_id = ui.id().with("custom_res_w");
    let custom_height_id = ui.id().with("custom_res_h");
    let mut custom_w: u32 = ui
        .data(|d| d.get_temp(custom_width_id))
        .unwrap_or(current_w);
    let mut custom_h: u32 = ui
        .data(|d| d.get_temp(custom_height_id))
        .unwrap_or(current_h);

    // No artificial cap: the upper bound is the GPU's max texture dimension,
    // matching what the engine/API accept (spec/resolution-and-scaling.md).
    let max_dim = data.max_render_dimension;
    ui.horizontal(|ui| {
        ui.label("W:");
        ui.add(
            egui::DragValue::new(&mut custom_w)
                .range(64..=max_dim)
                .speed(16),
        );
        ui.label("H:");
        ui.add(
            egui::DragValue::new(&mut custom_h)
                .range(64..=max_dim)
                .speed(16),
        );
    });

    ui.data_mut(|d| {
        d.insert_temp(custom_width_id, custom_w);
        d.insert_temp(custom_height_id, custom_h);
    });

    let is_custom_different = custom_w != current_w || custom_h != current_h;
    if ui
        .add_enabled(
            is_custom_different && custom_w > 0 && custom_h > 0,
            egui::Button::new("Apply"),
        )
        .clicked()
    {
        actions.commands.push(EngineCommand::SetRenderResolution {
            width: custom_w,
            height: custom_h,
        });
    }

    ui.separator();
    ui.label(
        egui::RichText::new(format!("Current: {current_w}×{current_h}"))
            .weak()
            .small(),
    );
}

/// Render the target FPS popover (shown when clicking FPS target in the top bar).
pub(super) fn render_target_fps_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(180.0);
    ui.label(egui::RichText::new("🎯 Target FPS").strong());
    ui.separator();

    let current = data.target_fps;

    let presets: &[(&str, u32)] = &[
        ("30 FPS", 30),
        ("60 FPS", 60),
        ("120 FPS", 120),
        ("Uncapped", 0),
    ];

    for &(label, fps) in presets {
        let is_current = current == fps;
        if ui.radio(is_current, label).clicked() && !is_current {
            actions.commands.push(EngineCommand::SetTargetFps { fps });
        }
    }

    ui.separator();
    ui.label(
        egui::RichText::new(if current == 0 {
            "Current: Uncapped".to_string()
        } else {
            format!("Current: {current} FPS")
        })
        .weak()
        .small(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportSource;

    /// Emphasis follows engine state, not UI mode: the tempo readout is only
    /// live when a clock exists *and* something reads it.
    #[test]
    fn tempo_is_live_only_when_a_clock_has_followers() {
        let mut data = UIData::test_fixture();

        data.clock_active = true;
        data.clock_beat_followers = 2;
        assert!(clock_is_live(&data));

        data.clock_beat_followers = 0;
        assert!(
            !clock_is_live(&data),
            "a clock nothing reads is not driving"
        );

        data.clock_active = false;
        data.clock_beat_followers = 2;
        assert!(
            !clock_is_live(&data),
            "beat-locked sources are frozen with no clock source"
        );
    }

    /// Chasing timecode must not dim the tempo readout. Beat-locked modulators
    /// keep running through a timecode-chased section, and timecode carries no
    /// tempo to replace them with. See /spec/transport.md.
    #[test]
    fn chasing_timecode_does_not_dim_the_tempo() {
        let mut data = UIData::test_fixture();
        data.clock_active = true;
        data.clock_beat_followers = 1;
        data.transport.source = TransportSource::Timecode;

        assert!(clock_is_live(&data));
    }

    #[test]
    fn follower_hints_read_as_sentences() {
        assert_eq!(
            followers_hint(0, "the beat"),
            "nothing is locked to the beat"
        );
        assert_eq!(
            followers_hint(1, "the beat"),
            "1 modulator locked to the beat"
        );
        assert_eq!(
            followers_hint(4, "the transport"),
            "4 modulators locked to the transport"
        );
    }

    #[test]
    fn transport_colour_distinguishes_never_run_from_stopped() {
        let mut data = UIData::test_fixture();

        let never_run = transport_color(&data);
        data.transport.has_run = true;
        let stopped = transport_color(&data);
        data.transport.running = true;
        let running = transport_color(&data);
        data.transport.source = TransportSource::Timecode;
        let chasing = transport_color(&data);

        assert_ne!(never_run, stopped, "idle and stopped must not look alike");
        assert_ne!(stopped, running);
        assert_ne!(running, chasing);
    }
}
