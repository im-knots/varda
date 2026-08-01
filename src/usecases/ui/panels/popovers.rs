//! Status-bar popovers and the global MIDI-learn popup.
//!
//! Transient overlays rendered above the main layout: FPS, GPU, clock,
//! resolution, target FPS and tonemap readouts, plus the right-click MIDI-learn
//! toggle.

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
            } else {
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

pub(super) fn tonemap_short_name(mode: crate::renderer::tonemap::TonemapMode) -> &'static str {
    use crate::renderer::tonemap::TonemapMode;
    match mode {
        TonemapMode::Bypass => "TM:Off",
        TonemapMode::Aces => "TM:ACES",
        TonemapMode::Reinhard => "TM:Rein",
        TonemapMode::ReinhardExtended => "TM:ReinX",
        TonemapMode::HableFilmic => "TM:Hable",
        TonemapMode::Uchimura => "TM:Uchi",
        TonemapMode::Lottes => "TM:Lottes",
        TonemapMode::AgX => "TM:AgX",
        TonemapMode::KhronosPbrNeutral => "TM:PBR",
    }
}

const TONEMAP_PRESETS: &[(&str, crate::renderer::tonemap::TonemapMode)] = {
    use crate::renderer::tonemap::TonemapMode;
    &[
        ("Bypass (clamp)", TonemapMode::Bypass),
        ("ACES Filmic", TonemapMode::Aces),
        ("Reinhard", TonemapMode::Reinhard),
        ("Reinhard Extended", TonemapMode::ReinhardExtended),
        ("Hable Filmic", TonemapMode::HableFilmic),
        ("Uchimura (GT)", TonemapMode::Uchimura),
        ("Lottes (AMD)", TonemapMode::Lottes),
        ("AgX", TonemapMode::AgX),
        ("PBR Neutral", TonemapMode::KhronosPbrNeutral),
    ]
};

/// Render the tonemap mode popover (shown when clicking tonemap label in the top bar).
pub(super) fn render_tonemap_popover(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    use crate::renderer::tonemap::TonemapMode;

    ui.set_min_width(180.0);
    ui.label(egui::RichText::new("🎨 Tonemap Mode").strong());
    ui.separator();

    let current = data.tonemap_mode;

    for &(label, mode) in TONEMAP_PRESETS {
        if ui.radio(current == mode, label).clicked() && current != mode {
            actions.commands.push(EngineCommand::SetTonemapMode(mode));
        }
    }

    ui.separator();
    ui.label(
        egui::RichText::new(match current {
            TonemapMode::Bypass => "Values >1.0 are clamped at the output boundary",
            TonemapMode::Aces => "Cinematic rolloff, warm highlight shift",
            TonemapMode::Reinhard => "Gentle curve, never reaches pure white",
            TonemapMode::ReinhardExtended => "Reinhard with white point, full SDR range",
            TonemapMode::HableFilmic => "Nice toe and shoulder, game-industry standard",
            TonemapMode::Uchimura => "Gran Turismo style, tunable shoulder",
            TonemapMode::Lottes => "Fast, invertible, high contrast",
            TonemapMode::AgX => "Neutral, minimal hue shift",
            TonemapMode::KhronosPbrNeutral => "Color-accurate, minimal look modification",
        })
        .weak()
        .small(),
    );

    // ── LUT Section ──
    ui.separator();
    ui.label(egui::RichText::new("🎞 3D LUT").strong());

    let active_lut = data.active_lut_filename.as_deref();

    // "None" option
    if ui.radio(active_lut.is_none(), "None").clicked() && active_lut.is_some() {
        actions.commands.push(EngineCommand::UnloadLut);
    }

    // Available LUT files
    for lut_name in &data.available_luts {
        let is_active = active_lut == Some(lut_name.as_str());
        if ui.radio(is_active, lut_name).clicked() && !is_active {
            actions.commands.push(EngineCommand::LoadLut {
                filename: lut_name.clone(),
            });
        }
    }

    if data.available_luts.is_empty() {
        ui.label(
            egui::RichText::new("Place .cube/.3dl files in .varda/luts/")
                .weak()
                .small(),
        );
    }
}
