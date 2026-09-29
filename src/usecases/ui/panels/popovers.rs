//! Status-bar popovers and the global MIDI-learn popup: FPS, GPU, clock,
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

    if ctx.input(|i| i.pointer.secondary_clicked())
        && let Some(pos) = ctx.input(|i| i.pointer.interact_pos())
    {
        if popup_pos.is_some() {
            ctx.memory_mut(|mem| {
                mem.data.remove::<egui::Pos2>(popup_id);
                mem.data.remove::<bool>(popup_fresh_id);
            });
        } else if !egui::Popup::is_any_open(ctx)
            && ctx
                .layer_id_at(pos)
                .is_none_or(|layer| layer.order == egui::Order::Background)
        {
            // A widget with its own context menu, or anything already floating, keeps the
            // right-click. This popup is drawn last, so opening it would cover that menu
            // and block presses on its items.
            ctx.memory_mut(|mem| {
                mem.data.insert_temp(popup_id, pos);
                mem.data.insert_temp(popup_fresh_id, true);
            });
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
                        actions.commands.push(EngineCommand::MidiLearnToggle);
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
                        actions.commands.push(EngineCommand::KeyboardLearnToggle);
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
            if let Some(click) = click_pos
                && !popup_rect.contains(click)
            {
                ctx.memory_mut(|mem| {
                    mem.data.remove::<egui::Pos2>(popup_id);
                    mem.data.remove::<bool>(popup_fresh_id);
                });
            }
        }
    }
}

/// A status-bar popover that stays open while its own dropdowns are used.
///
/// egui tracks one open popup per window, so a `ComboBox` inside would close
/// the popover (patching LTC takes two choices in a row). This keeps its own
/// open state: a click in a floating layer belongs to the popover or a list it
/// opened, and only a click on the main UI closes it. That also keeps the
/// status bar's popovers mutually exclusive, since their triggers are in the
/// main UI.
pub(super) fn status_popover(button: &egui::Response, content: impl FnOnce(&mut egui::Ui)) {
    let ctx = button.ctx.clone();
    let id = button.id.with("status_popover");
    let mut open: bool = ctx.data(|d| d.get_temp(id).unwrap_or(false));
    let toggled = button.clicked();
    if toggled {
        open = !open;
    }

    if open {
        egui::Popup::from_response(button)
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .show(content);
    }

    // egui already closes on Escape and writes that back through the bool above.
    // The press that opened the popover does not close it.
    if open && !toggled {
        let clicked_away = ctx.input(|i| i.pointer.any_click())
            && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| {
                ctx.layer_id_at(pos)
                    .is_some_and(|layer| layer.order == egui::Order::Background)
            });
        if clicked_away {
            open = false;
        }
    }

    ctx.data_mut(|d| d.insert_temp(id, open));
}

/// FPS details popover, opened from the FPS readout in the top bar.
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

/// GPU details popover, opened from the GPU device in the top bar.
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

/// Color for the top bar position readout. Chasing timecode is highlighted,
/// since the position is then out of the operator's hands.
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

/// Whether the tempo readout is driving anything.
///
/// Tempo and position are always shown; the one nothing reads is dimmed. This
/// follows engine state, not UI mode, so it stays correct when a show runs on
/// both clocks.
pub(super) fn clock_is_live(data: &UIData) -> bool {
    data.clock_active && data.clock_beat_followers > 0
}

/// Hover text naming what follows each clock.
///
/// There are three motion sources (free-running, beat-locked,
/// transport-locked) and only two readouts, so each readout lists what depends
/// on it.
pub(super) fn followers_hint(count: usize, what: &str) -> String {
    match count {
        0 => format!("nothing is locked to {what}"),
        1 => format!("1 modulator locked to {what}"),
        n => format!("{n} modulators locked to {what}"),
    }
}

/// Idle, armed and writing each get a color, readable without the tooltip.
fn record_colour(ui: &egui::Ui, data: &UIData) -> egui::Color32 {
    if !data.transport.recording_params.is_empty() {
        egui::Color32::from_rgb(255, 80, 80)
    } else if data.transport.record_armed {
        egui::Color32::from_rgb(190, 60, 60)
    } else {
        ui.visuals().weak_text_color()
    }
}

/// The automation record arm, beside the position in both modes, so it is
/// reachable without opening the transport popover.
pub(super) fn record_button(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let t = &data.transport;
    let writing = t.recording_params.len();
    let color = record_colour(ui, data);

    let hover = if writing > 0 {
        format!(
            "Recording {}. Press to end the pass.",
            match writing {
                1 => "1 parameter".to_string(),
                n => format!("{n} parameters"),
            }
        )
    } else if t.record_armed {
        "Armed. Move any control to write it into the arrangement.".to_string()
    } else {
        "Record automation. Arms and rolls the show; whatever you touch is kept as a curve."
            .to_string()
    };

    if ui
        .add(egui::Button::new(egui::RichText::new("⏺").color(color)).frame(t.record_armed))
        .on_hover_text(hover)
        .clicked()
    {
        actions.commands.push(EngineCommand::SetRecordArmed {
            armed: !t.record_armed,
        });
    }
}

/// Transport popover, opened from the position readout in the top bar.
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

    // Position is read-only while chasing, so the controls are hidden.
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

    if t.source == TransportSource::Timecode {
        ui.separator();
        render_timecode_inputs(ui, data, actions);
    }
}

/// The signals the transport can follow.
///
/// Only devices sending timecode are offered. LTC is always offered, because
/// its input is patched in settings rather than discovered.
fn follow_choices(
    tc: &crate::engine::types::TimecodeSnapshot,
) -> Vec<(crate::timecode::TimecodePreference, String)> {
    use crate::timecode::TimecodePreference;

    let mut choices = vec![
        (TimecodePreference::Auto, "Auto".to_string()),
        (TimecodePreference::ForceLtc, "LTC".to_string()),
    ];
    for input in &tc.inputs {
        if let Some(device_id) = input.key.strip_prefix("mtc:").and_then(|d| d.parse().ok()) {
            choices.push((
                TimecodePreference::ForceMtc { device_id },
                input.label.clone(),
            ));
        }
    }
    choices.push((TimecodePreference::Off, "Off".to_string()));
    choices
}

/// Which audio input, and which channel of it, carries LTC.
///
/// A channel, because a typical field rig sends music on one channel and
/// timecode on the other. No device is opened until an input is chosen.
fn render_ltc_patch(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    use crate::timecode::LtcInput;

    let current = data.timecode.ltc_input;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("LTC in").small());

        let selected = current
            .and_then(|input| {
                data.audio
                    .devices
                    .iter()
                    .find(|device| device.id == input.source_id)
            })
            .map_or("None", |device| device.name.as_str());

        egui::ComboBox::from_id_salt("ltc_device")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "None").clicked() && current.is_some() {
                    actions
                        .commands
                        .push(EngineCommand::SetLtcInput { input: None });
                }
                for device in &data.audio.devices {
                    let chosen = current.is_some_and(|i| i.source_id == device.id);
                    if ui.selectable_label(chosen, &device.name).clicked() && !chosen {
                        actions.commands.push(EngineCommand::SetLtcInput {
                            input: Some(LtcInput {
                                source_id: device.id,
                                channel: current.map_or(0, |i| i.channel),
                                rate: current.and_then(|i| i.rate),
                            }),
                        });
                    }
                }
            });

        if let Some(input) = current {
            egui::ComboBox::from_id_salt("ltc_channel")
                .selected_text(format!("Ch {}", input.channel + 1))
                .show_ui(ui, |ui| {
                    // A stereo pair; the device channel count is not in this snapshot.
                    for channel in 0..2_u16 {
                        if ui
                            .selectable_label(
                                input.channel == channel,
                                format!("Ch {}", channel + 1),
                            )
                            .clicked()
                            && input.channel != channel
                        {
                            actions.commands.push(EngineCommand::SetLtcInput {
                                input: Some(LtcInput { channel, ..input }),
                            });
                        }
                    }
                });
        }
    });
}

/// Incoming timecode signals, separate from the transport position.
///
/// Lets a performer tell a master that stopped from a master nobody is
/// listening to.
pub(super) fn render_timecode_inputs(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let tc = &data.timecode;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Follow").small());
        let choices = follow_choices(tc);

        egui::ComboBox::from_id_salt("timecode_preference")
            .selected_text(
                choices
                    .iter()
                    .find(|(p, _)| *p == tc.preference)
                    .map_or("Auto", |(_, label)| label.as_str()),
            )
            .show_ui(ui, |ui| {
                for (preference, label) in &choices {
                    if ui
                        .selectable_label(tc.preference == *preference, label)
                        .clicked()
                        && tc.preference != *preference
                    {
                        actions.commands.push(EngineCommand::SetTimecodePreference {
                            preference: *preference,
                        });
                    }
                }
            });
    });

    render_ltc_patch(ui, data, actions);

    if tc.inputs.is_empty() {
        ui.label(
            egui::RichText::new(match tc.ltc_input {
                Some(_) => "No timecode arriving.",
                None => {
                    "No timecode arriving. MTC is listened for on every MIDI port; \
                         for LTC, choose the audio input it is patched to."
                }
            })
            .small()
            .weak(),
        );
        return;
    }

    for input in &tc.inputs {
        let resolved = tc.resolved.as_deref() == Some(input.key.as_str());
        ui.horizontal(|ui| {
            let (mark, colour) = if !input.running {
                ("○", ui.visuals().weak_text_color())
            } else if input.freewheeling {
                ("◐", egui::Color32::from_rgb(220, 170, 60))
            } else {
                ("●", egui::Color32::from_rgb(90, 200, 120))
            };
            ui.label(egui::RichText::new(mark).color(colour));
            let name = egui::RichText::new(&input.label).small();
            ui.label(if resolved { name.strong() } else { name.weak() });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&input.timecode).monospace().small())
                    .on_hover_text(format!(
                        "{} at {}{}",
                        input.label,
                        input.rate.label(),
                        if (input.speed - 1.0).abs() > 0.05 && input.running {
                            format!(", running at {:.2}×", input.speed)
                        } else {
                            String::new()
                        }
                    ));
            });
        });
    }
}

/// Clock source popover, opened from the BPM readout in the top bar.
pub(super) fn render_clock_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("🕐 Clock Source").strong());
    ui.separator();

    let is_auto = data.clock_preference == "Auto";

    if ui.radio(is_auto, "Auto (recommended)").clicked() && !is_auto {
        actions.commands.push(EngineCommand::SetClockPreference {
            preference: crate::clock::ClockPreference::Auto,
        });
    }

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

    // OSC option, only while OSC is active.
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

    // Current status line.
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

/// Resolution popover, opened from the resolution readout in the top bar.
pub(super) fn render_resolution_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.set_min_width(200.0);
    ui.label(egui::RichText::new("📐 Render Resolution").strong());
    ui.separator();

    let current_w = data.render_width;
    let current_h = data.render_height;

    // Landscape presets, then short-form video shapes: 1080×1920 (9:16) is the
    // master for Reels, TikTok, Shorts and Stories; 1080×1350 is the 4:5
    // Instagram feed post; 1080×1080 is the 1:1 fallback.
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

    // Custom W×H input, persisted in egui memory.
    let custom_width_id = ui.id().with("custom_res_w");
    let custom_height_id = ui.id().with("custom_res_h");
    let mut custom_w: u32 = ui
        .data(|d| d.get_temp(custom_width_id))
        .unwrap_or(current_w);
    let mut custom_h: u32 = ui
        .data(|d| d.get_temp(custom_height_id))
        .unwrap_or(current_h);

    // The upper bound is the GPU's max texture dimension, as in the engine and API.
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

/// Target FPS popover, opened from the FPS target in the top bar.
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

    /// The tempo readout is live only when a clock exists and something reads it.
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

    /// Chasing timecode does not dim the tempo readout: beat-locked modulators keep
    /// running, and timecode carries no tempo.
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

    /// The record button toggles, so the same press starts and ends a pass.
    #[test]
    fn the_record_button_arms_and_disarms() {
        use egui_kittest::kittest::Queryable;

        for armed in [false, true] {
            let mut data = UIData::test_fixture();
            data.transport.record_armed = armed;
            let mut actions = UIActions::new();
            {
                let mut harness =
                    egui_kittest::Harness::new_ui(|ui| record_button(ui, &data, &mut actions));
                harness.get_by_label("⏺").click();
                harness.run();
            }
            match actions.commands.as_slice() {
                [EngineCommand::SetRecordArmed { armed: asked }] => {
                    assert_eq!(*asked, !armed, "the press asks for the other state");
                }
                other => panic!("expected one record arm, got {other:?}"),
            }
        }
    }

    /// A pass that is writing looks distinct from one that is only armed.
    #[test]
    fn a_pass_being_written_looks_different_from_being_armed() {
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let idle = UIData::test_fixture();
            let mut armed = UIData::test_fixture();
            armed.transport.record_armed = true;
            let mut writing = UIData::test_fixture();
            writing.transport.record_armed = true;
            writing.transport.recording_params = vec!["deck/a/opacity".to_string()];

            let colours: Vec<egui::Color32> = [&idle, &armed, &writing]
                .iter()
                .map(|data| record_colour(ui, data))
                .collect();
            assert_ne!(colours[0], colours[1]);
            assert_ne!(colours[1], colours[2]);
        });
        harness.run();
    }

    /// One input to build the diagnostics from.
    fn an_input(
        key: &str,
        label: &str,
        running: bool,
    ) -> crate::engine::types::TimecodeInputSnapshot {
        crate::engine::types::TimecodeInputSnapshot {
            key: key.to_string(),
            label: label.to_string(),
            position: 3600.0,
            timecode: "01:00:00:00".to_string(),
            rate: crate::transport::TimecodeRate::Fps25,
            running,
            freewheeling: false,
            speed: 1.0,
        }
    }

    /// Every input is listed, including ones that are not resolving.
    #[test]
    fn every_input_is_shown_not_only_the_one_driving() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.transport.source = TransportSource::Timecode;
        data.timecode.inputs = vec![
            an_input("ltc", "LTC (channel 2)", false),
            an_input("mtc:1", "MTC (Tascam Model 12)", true),
        ];
        data.timecode.resolved = Some("mtc:1".to_string());

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_timecode_inputs(ui, &data, &mut actions);
        });
        harness.run();

        harness.get_by_label_contains("LTC (channel 2)");
        harness.get_by_label_contains("MTC (Tascam Model 12)");
    }

    /// With nothing arriving, the panel says so and what to do.
    #[test]
    fn silence_says_what_to_do_about_it() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.transport.source = TransportSource::Timecode;

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_timecode_inputs(ui, &data, &mut actions);
        });
        harness.run();

        harness.get_by_label_contains("No timecode arriving");
    }

    /// Only devices sending timecode are offered to follow.
    #[test]
    fn only_devices_sending_timecode_are_offered() {
        use crate::timecode::TimecodePreference;

        let mut tc = crate::engine::types::TimecodeSnapshot::default();
        assert_eq!(
            follow_choices(&tc)
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>(),
            vec![
                TimecodePreference::Auto,
                TimecodePreference::ForceLtc,
                TimecodePreference::Off
            ],
            "with nothing arriving there is no device to name"
        );

        tc.inputs = vec![
            an_input("mtc:4", "MTC (Model 12)", true),
            an_input("ltc", "LTC (channel 2)", true),
        ];
        let choices = follow_choices(&tc);
        assert!(choices.iter().any(|(p, label)| *p
            == TimecodePreference::ForceMtc { device_id: 4 }
            && label == "MTC (Model 12)"));
        assert_eq!(
            choices.len(),
            4,
            "the LTC input is already covered by the LTC entry, got {choices:?}"
        );
    }

    /// LTC is only listened for on a named input, so naming one starts it. The
    /// channel is part of the choice.
    #[test]
    fn patching_ltc_names_a_device_and_a_channel() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.audio.devices = vec![crate::usecases::ui::data::AudioDeviceUI {
            id: 4,
            name: "Scarlett 2i2".to_string(),
            active: false,
        }];

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_ltc_patch(ui, &data, &mut actions);
        });
        harness.run();
        // A ComboBox exposes its selected text as AccessKit `value`, not `label`.
        harness.get_by_value("None").click();
        harness.run();
        harness.get_by_label("Scarlett 2i2").click();
        harness.run();
        drop(harness);

        assert!(
            matches!(
                actions.commands.first(),
                Some(EngineCommand::SetLtcInput {
                    input: Some(crate::timecode::LtcInput {
                        source_id: 4,
                        channel: 0,
                        ..
                    })
                })
            ),
            "got {:?}",
            actions.commands.first()
        );
    }

    /// Switching channel keeps the same device.
    #[test]
    fn changing_the_ltc_channel_keeps_the_device() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.audio.devices = vec![crate::usecases::ui::data::AudioDeviceUI {
            id: 4,
            name: "Scarlett 2i2".to_string(),
            active: true,
        }];
        data.timecode.ltc_input = Some(crate::timecode::LtcInput {
            source_id: 4,
            channel: 0,
            rate: None,
        });

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_ltc_patch(ui, &data, &mut actions);
        });
        harness.run();
        harness.get_by_value("Ch 1").click();
        harness.run();
        harness.get_by_label("Ch 2").click();
        harness.run();
        drop(harness);

        assert!(
            matches!(
                actions.commands.first(),
                Some(EngineCommand::SetLtcInput {
                    input: Some(crate::timecode::LtcInput {
                        source_id: 4,
                        channel: 1,
                        ..
                    })
                })
            ),
            "got {:?}",
            actions.commands.first()
        );
    }

    /// Patching LTC takes two choices (device, then channel), so the popover must
    /// stay open after the first, even though the chooser is in its own layer.
    #[test]
    fn choosing_an_input_leaves_the_transport_popover_open() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.transport.source = TransportSource::Timecode;
        data.audio.devices = vec![crate::usecases::ui::data::AudioDeviceUI {
            id: 4,
            name: "Scarlett 2i2".to_string(),
            active: false,
        }];

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let opener = ui.button("Transport");
            status_popover(&opener, |ui| {
                render_transport_popover(ui, &data, &mut actions);
            });
        });
        harness.run();
        harness.get_by_label("Transport").click();
        harness.run();
        assert!(
            harness.query_by_label_contains("LTC in").is_some(),
            "the popover should be open before we touch anything"
        );

        harness.get_by_value("None").click();
        harness.run();
        assert!(
            harness.query_by_label_contains("LTC in").is_some(),
            "opening the picker dismissed the popover"
        );
        harness.get_by_label("Scarlett 2i2").click();
        harness.run();

        assert!(
            harness.query_by_label_contains("LTC in").is_some(),
            "picking a device dismissed the popover the picker was opened from"
        );
        drop(harness);

        assert!(
            matches!(
                actions.commands.as_slice(),
                [EngineCommand::SetLtcInput {
                    input: Some(crate::timecode::LtcInput { source_id: 4, .. })
                }]
            ),
            "surviving the picker is worth nothing if the choice is lost, got {:?}",
            actions.commands
        );
    }

    /// A popover still closes on a click on the UI behind it.
    #[test]
    fn a_click_in_the_ui_behind_puts_a_popover_away() {
        use egui_kittest::kittest::Queryable;

        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = ui.button("Elsewhere");
            let opener = ui.button("Open");
            status_popover(&opener, |ui| {
                let _ = ui.button("Inside");
            });
        });
        harness.run();
        harness.get_by_label("Open").click();
        harness.run();
        assert!(harness.query_by_label("Inside").is_some(), "it opened");

        harness.get_by_label("Inside").click();
        harness.run();
        assert!(
            harness.query_by_label("Inside").is_some(),
            "using the popover closed it"
        );

        harness.get_by_label("Elsewhere").click();
        harness.run();
        assert!(
            harness.query_by_label("Inside").is_none(),
            "a click in the UI behind it should put it away"
        );
    }

    /// Escape closes the popover.
    #[test]
    fn escape_puts_a_status_popover_away() {
        use egui_kittest::kittest::Queryable;

        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let opener = ui.button("Open");
            status_popover(&opener, |ui| {
                let _ = ui.button("Inside");
            });
        });
        harness.run();
        harness.get_by_label("Open").click();
        harness.run();
        assert!(harness.query_by_label("Inside").is_some(), "it opened");

        harness.key_press(egui::Key::Escape);
        harness.run();

        assert!(
            harness.query_by_label("Inside").is_none(),
            "escape should dismiss a popover without hunting for its trigger"
        );
    }

    /// Opening another status-bar popover closes the current one. The popovers
    /// own their open state, so egui's one-popup rule does not enforce this.
    #[test]
    fn opening_one_status_popover_closes_the_other() {
        use egui_kittest::kittest::Queryable;

        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let (alpha, beta) = ui
                .horizontal(|ui| (ui.button("Alpha"), ui.button("Beta")))
                .inner;
            status_popover(&alpha, |ui| {
                let _ = ui.button("In Alpha");
            });
            status_popover(&beta, |ui| {
                let _ = ui.button("In Beta");
            });
        });
        harness.run();
        harness.get_by_label("Alpha").click();
        harness.run();
        assert!(harness.query_by_label("In Alpha").is_some(), "Alpha opened");

        harness.get_by_label("Beta").click();
        harness.run();

        assert!(
            harness.query_by_label("In Beta").is_some(),
            "the second press should open the one it was aimed at"
        );
        assert!(
            harness.query_by_label("In Alpha").is_none(),
            "two status popovers open at once cover the readouts they belong to"
        );
    }

    /// Unpatching is supported; a stale input would keep a dead channel open and
    /// report silence as the master's.
    #[test]
    fn clearing_the_ltc_patch_asks_for_no_input() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.audio.devices = vec![crate::usecases::ui::data::AudioDeviceUI {
            id: 4,
            name: "Scarlett 2i2".to_string(),
            active: true,
        }];
        data.timecode.ltc_input = Some(crate::timecode::LtcInput {
            source_id: 4,
            channel: 1,
            rate: None,
        });

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_ltc_patch(ui, &data, &mut actions);
        });
        harness.run();
        harness.get_by_value("Scarlett 2i2").click();
        harness.run();
        harness.get_by_label("None").click();
        harness.run();
        drop(harness);

        assert!(
            matches!(
                actions.commands.as_slice(),
                [EngineCommand::SetLtcInput { input: None }]
            ),
            "got {:?}",
            actions.commands
        );
    }

    /// Auto picks a master by priority; naming a device overrides it, so the
    /// choice carries the device.
    #[test]
    fn choosing_a_named_master_from_the_ui_forces_that_device() {
        use egui_kittest::kittest::Queryable;

        let mut data = UIData::test_fixture();
        data.transport.source = TransportSource::Timecode;
        data.timecode.inputs = vec![an_input("mtc:7", "MTC (Model 12)", true)];

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_timecode_inputs(ui, &data, &mut actions);
        });
        harness.run();
        harness.get_by_value("Auto").click();
        harness.run();
        // The same text also appears as a label in the arriving-inputs list below.
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "MTC (Model 12)")
            .click();
        harness.run();
        drop(harness);

        assert!(
            matches!(
                actions.commands.as_slice(),
                [EngineCommand::SetTimecodePreference {
                    preference: crate::timecode::TimecodePreference::ForceMtc { device_id: 7 }
                }]
            ),
            "got {:?}",
            actions.commands
        );
    }

    /// Freewheeling is marked, since the position keeps moving and the numbers
    /// alone can't show the master was lost.
    #[test]
    fn a_freewheeling_input_is_marked_differently_from_a_healthy_one() {
        use egui_kittest::kittest::Queryable;

        let mut coasting = an_input("mtc:2", "MTC (coasting)", true);
        coasting.freewheeling = true;

        let mut data = UIData::test_fixture();
        data.transport.source = TransportSource::Timecode;
        data.timecode.inputs = vec![
            an_input("mtc:1", "MTC (healthy)", true),
            coasting,
            an_input("ltc", "LTC (silent)", false),
        ];

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_timecode_inputs(ui, &data, &mut actions);
        });
        harness.run();

        for (mark, state) in [("●", "running"), ("◐", "freewheeling"), ("○", "stopped")] {
            assert_eq!(
                harness.query_all_by_label(mark).count(),
                1,
                "{state} should draw exactly one {mark}"
            );
        }
    }

    /// MIDI learn is opened by right-clicking the layout.
    #[test]
    fn right_clicking_the_main_ui_offers_midi_learn() {
        use egui_kittest::kittest::Queryable;

        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = ui.button("Somewhere");
            handle_midi_learn_popup(ui.ctx(), &data, &mut actions);
        });
        harness.run();
        assert!(
            harness.query_by_label("🎹 Enter MIDI Learn").is_none(),
            "nothing was pressed yet"
        );

        harness.get_by_label("Somewhere").click_secondary();
        harness.run();

        assert!(
            harness.query_by_label("🎹 Enter MIDI Learn").is_some(),
            "a right click on the layout should offer the toggle"
        );
    }

    /// The same gesture that opened the popup closes it.
    #[test]
    fn a_second_right_click_puts_the_midi_learn_popup_away() {
        use egui_kittest::kittest::Queryable;

        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = ui.button("Somewhere");
            handle_midi_learn_popup(ui.ctx(), &data, &mut actions);
        });
        harness.run();
        harness.get_by_label("Somewhere").click_secondary();
        harness.run();
        assert!(harness.query_by_label("🎹 Enter MIDI Learn").is_some());

        harness.get_by_label("Somewhere").click_secondary();
        harness.run();

        assert!(
            harness.query_by_label("🎹 Enter MIDI Learn").is_none(),
            "the same right click should take it back"
        );
    }

    /// A right-click on a floating layer belongs to that layer; this popup, drawn
    /// last, would cover it.
    #[test]
    fn right_clicking_something_floating_does_not_open_the_midi_learn_popup() {
        use egui_kittest::kittest::Queryable;

        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            egui::Area::new(egui::Id::new("floating_for_test"))
                .order(egui::Order::Middle)
                .fixed_pos(egui::pos2(40.0, 40.0))
                .show(ui.ctx(), |ui| {
                    let _ = ui.button("Floating");
                });
            handle_midi_learn_popup(ui.ctx(), &data, &mut actions);
        });
        harness.run();

        harness.get_by_label("Floating").click_secondary();
        harness.run();

        assert!(
            harness.query_by_label("🎹 Enter MIDI Learn").is_none(),
            "the floating thing owns that click, and this popup would cover it"
        );
    }

    /// Both learn modes are entered and left through this popup.
    #[test]
    fn the_midi_learn_popup_asks_for_the_mode_it_names() {
        use egui_kittest::kittest::Queryable;

        for (label, midi, keyboard) in [
            ("🎹 Enter MIDI Learn", true, false),
            ("⌨ Enter Keyboard Learn", false, true),
        ] {
            let data = UIData::test_fixture();
            let mut actions = UIActions::new();
            {
                let mut harness = egui_kittest::Harness::new_ui(|ui| {
                    let _ = ui.button("Somewhere");
                    handle_midi_learn_popup(ui.ctx(), &data, &mut actions);
                });
                harness.run();
                harness.get_by_label("Somewhere").click_secondary();
                harness.run();
                harness.get_by_label(label).click();
                harness.run();
            }
            let sent = |wanted: &EngineCommand| {
                actions
                    .commands
                    .iter()
                    .any(|c| std::mem::discriminant(c) == std::mem::discriminant(wanted))
            };
            assert_eq!(sent(&EngineCommand::MidiLearnToggle), midi, "{label}");
            assert_eq!(
                sent(&EngineCommand::KeyboardLearnToggle),
                keyboard,
                "{label}"
            );
        }
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
