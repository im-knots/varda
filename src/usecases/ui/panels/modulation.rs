//! Modulation sources panel.

use crate::modulation::{LFOWaveform, StepInterpolation};
use super::super::{UIData, UIActions, ModulationAction, ModSourceUI, modulator_color, widgets};

pub(super) fn render_modulation_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.horizontal(|ui| {
        if ui.button("➕ LFO").clicked() {
            actions.modulation_actions.push(ModulationAction::AddLFO {
                waveform: LFOWaveform::Sine,
                frequency: 1.0,
            });
        }
        if ui.button("➕ Audio").clicked() {
            actions.modulation_actions.push(ModulationAction::AddAudioFFT {
                preset: crate::modulation::AudioBandPreset::Low,
                source_id: None,
            });
        }
        if ui.button("➕ ADSR").clicked() {
            actions.modulation_actions.push(ModulationAction::AddADSR {
                attack: 0.1, decay: 0.3, sustain: 0.7, release: 0.5,
            });
        }
        if ui.button("➕ StepSeq").clicked() {
            actions.modulation_actions.push(ModulationAction::AddStepSequencer {
                num_steps: 8, rate: 2.0,
            });
        }
    });


    if data.modulation_sources.is_empty() {
        ui.label(egui::RichText::new("No modulation sources").small().weak());
    } else {
        egui::ScrollArea::vertical().id_salt("mod_sources_vscroll").show(ui, |ui| {
            for (idx, src) in data.modulation_sources.iter().enumerate() {
                let mod_color = modulator_color(idx);
                let dim_color = egui::Color32::from_rgba_premultiplied(
                    mod_color.r() / 4, mod_color.g() / 4, mod_color.b() / 4, 40
                );
                let header_label = match src {
                    ModSourceUI::LFO { .. } => format!("LFO {}", idx + 1),
                    ModSourceUI::Audio { .. } => format!("Audio {}", idx + 1),
                    ModSourceUI::ADSR { stage, .. } => {
                        let stage_icon = match stage {
                            crate::modulation::ADSRStage::Idle => "⏹",
                            crate::modulation::ADSRStage::Attack => "▲",
                            crate::modulation::ADSRStage::Decay => "▼",
                            crate::modulation::ADSRStage::Sustain => "━",
                            crate::modulation::ADSRStage::Release => "↘",
                        };
                        format!("ADSR {} {}", idx + 1, stage_icon)
                    }
                    ModSourceUI::StepSequencer { .. } => format!("StepSeq {}", idx + 1),
                };
                // Show current value in header if available
                let value_text = data.modulation_current_values.get(idx)
                    .map(|v| format!(" ({:.2})", v))
                    .unwrap_or_default();

                egui::Frame::default()
                    .inner_margin(4.0)
                    .corner_radius(4.0)
                    .fill(dim_color)
                    .stroke(egui::Stroke::new(1.0, mod_color))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}{}", header_label, value_text)).strong().color(mod_color));
                            if ui.small_button("x").clicked() {
                                actions.modulation_actions.push(ModulationAction::RemoveSource { idx });
                            }
                        });
                        match src {
                            ModSourceUI::LFO { waveform, frequency, phase, amplitude, bipolar } => {
                                let waveforms = ["Sine", "Square", "Triangle", "Saw", "Random"];
                                let current_wf = match waveform {
                                    LFOWaveform::Sine => 0,
                                    LFOWaveform::Square => 1,
                                    LFOWaveform::Triangle => 2,
                                    LFOWaveform::Sawtooth => 3,
                                    LFOWaveform::Random => 4,
                                };
                                let mut selected_wf = current_wf;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Wave:").small());
                                    egui::ComboBox::from_id_salt(format!("wf_{}", idx))
                                        .selected_text(waveforms[selected_wf])
                                        .width(70.0)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in waveforms.iter().enumerate() {
                                                if ui.selectable_value(&mut selected_wf, i, *name).changed() {
                                                    let new_wf = match i {
                                                        0 => LFOWaveform::Sine,
                                                        1 => LFOWaveform::Square,
                                                        2 => LFOWaveform::Triangle,
                                                        3 => LFOWaveform::Sawtooth,
                                                        _ => LFOWaveform::Random,
                                                    };
                                                    actions.modulation_actions.push(ModulationAction::UpdateLFOWaveform { idx, waveform: new_wf });
                                                }
                                            }
                                        });
                                });
                                let mut freq = *frequency;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Freq:").small());
                                    let path = format!("mod/{}/frequency", idx);
                                    if render_mod_learn_slider(ui, &mut freq, 0.01..=10.0, |s| s.logarithmic(true).show_value(true).suffix("Hz"), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOFrequency { idx, frequency: freq });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "frequency");
                                });
                                let mut amp = *amplitude;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Amp:").small());
                                    let path = format!("mod/{}/amplitude", idx);
                                    if render_mod_learn_slider(ui, &mut amp, 0.0..=1.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOAmplitude { idx, amplitude: amp });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "amplitude");
                                });
                                let mut ph = *phase;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Phase:").small());
                                    let path = format!("mod/{}/phase", idx);
                                    if render_mod_learn_slider(ui, &mut ph, 0.0..=1.0, |s| s.show_value(false), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOPhase { idx, phase: ph });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "phase");
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar (-1 to 1)").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateLFOBipolar { idx, bipolar: bp });
                                }
                                // LFO waveform visualization
                                let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(180.0), 30.0), egui::Sense::hover());
                                let rect = response.rect;
                                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                                let n_points = 60;
                                let points: Vec<egui::Pos2> = (0..=n_points).map(|i| {
                                    let t = i as f32 / n_points as f32;
                                    let raw = match waveform {
                                        LFOWaveform::Sine => (t * std::f32::consts::TAU).sin(),
                                        LFOWaveform::Square => if t < 0.5 { 1.0 } else { -1.0 },
                                        LFOWaveform::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                                        LFOWaveform::Sawtooth => 2.0 * t - 1.0,
                                        LFOWaveform::Random => {
                                            let step = (t * 8.0).floor();
                                            let seed = step as u32;
                                            let hash = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                                            (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
                                        }
                                    };
                                    let y = rect.center().y - raw * *amplitude * rect.height() * 0.4;
                                    egui::pos2(rect.left() + t * rect.width(), y)
                                }).collect();
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    let y = rect.center().y - cur_val * rect.height() * 0.4;
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::Audio { source_id, freq_low, freq_high, gain, smoothing, mode, noise_gate } => {
                                // Mode selector (Direct/Increase/Decrease)
                                use crate::modulation::AudioReactMode;
                                let mode_label = match mode {
                                    AudioReactMode::Direct => "Direct",
                                    AudioReactMode::Increase => "Increase",
                                    AudioReactMode::Decrease => "Decrease",
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Mode:").small());
                                    egui::ComboBox::from_id_salt(format!("audio_mode_{}", idx))
                                        .selected_text(mode_label)
                                        .width(70.0)
                                        .show_ui(ui, |ui| {
                                            for (name, m) in [("Direct", AudioReactMode::Direct), ("Increase", AudioReactMode::Increase), ("Decrease", AudioReactMode::Decrease)] {
                                                if ui.selectable_label(*mode == m, name).clicked() {
                                                    actions.modulation_actions.push(ModulationAction::UpdateAudioMode { idx, mode: m });
                                                }
                                            }
                                        });
                                });
                                // Preset selector (Low/Mid/High/Full/Custom)
                                let preset_label = if (*freq_low - 20.0).abs() < 1.0 && (*freq_high - 250.0).abs() < 1.0 {
                                    "Low"
                                } else if (*freq_low - 250.0).abs() < 1.0 && (*freq_high - 2000.0).abs() < 1.0 {
                                    "Mid"
                                } else if (*freq_low - 2000.0).abs() < 1.0 && (*freq_high - 20000.0).abs() < 1.0 {
                                    "High"
                                } else if (*freq_low - 20.0).abs() < 1.0 && (*freq_high - 20000.0).abs() < 1.0 {
                                    "Full"
                                } else {
                                    "Custom"
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Range:").small());
                                    egui::ComboBox::from_id_salt(format!("audio_preset_{}", idx))
                                        .selected_text(preset_label)
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            use crate::modulation::AudioBandPreset;
                                            for (name, preset) in [("Low", AudioBandPreset::Low), ("Mid", AudioBandPreset::Mid), ("High", AudioBandPreset::High), ("Full", AudioBandPreset::Full)] {
                                                if ui.selectable_label(preset_label == name, name).clicked() {
                                                    actions.modulation_actions.push(ModulationAction::UpdateAudioPreset { idx, preset });
                                                }
                                            }
                                        });
                                });
                                // Frequency range sliders (bookshelf)
                                let mut fl = *freq_low;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Lo:").small());
                                    let path = format!("mod/{}/freq_low", idx);
                                    if render_mod_learn_slider(ui, &mut fl, 20.0..=20000.0, |s| s.logarithmic(true).suffix("Hz"), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioFreqLow { idx, freq_low: fl });
                                    }
                                });
                                let mut fh = *freq_high;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Hi:").small());
                                    let path = format!("mod/{}/freq_high", idx);
                                    if render_mod_learn_slider(ui, &mut fh, 20.0..=20000.0, |s| s.logarithmic(true).suffix("Hz"), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioFreqHigh { idx, freq_high: fh });
                                    }
                                });
                                // Gain slider
                                let mut g = *gain;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Gain:").small());
                                    let path = format!("mod/{}/gain", idx);
                                    if render_mod_learn_slider(ui, &mut g, 0.0..=4.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioGain { idx, gain: g });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "gain");
                                });
                                // Smoothing slider
                                let mut sm = *smoothing;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Smooth:").small());
                                    let path = format!("mod/{}/smoothing", idx);
                                    if render_mod_learn_slider(ui, &mut sm, 0.0..=0.99, |s| s.show_value(false), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioSmoothing { idx, smoothing: sm });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "smoothing");
                                });
                                // Noise gate slider
                                let mut ng = *noise_gate;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Gate:").small());
                                    if ui.add(egui::Slider::new(&mut ng, 0.0..=0.5).show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioNoiseGate { idx, noise_gate: ng });
                                    }
                                });
                                // Source selector (if multiple audio devices)
                                if data.audio.devices.len() > 1 {
                                    let src_label = source_id
                                        .and_then(|sid| data.audio.devices.iter().find(|d| d.id == sid))
                                        .map(|d| d.name.as_str())
                                        .unwrap_or("Default");
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Src:").small());
                                        egui::ComboBox::from_id_salt(format!("audio_src_{}", idx))
                                            .selected_text(src_label)
                                            .width(80.0)
                                            .show_ui(ui, |ui| {
                                                if ui.selectable_label(source_id.is_none(), "Default").clicked() {
                                                    actions.modulation_actions.push(ModulationAction::UpdateAudioSource { idx, source_id: None });
                                                }
                                                for dev in &data.audio.devices {
                                                    if ui.selectable_label(*source_id == Some(dev.id), &dev.name).clicked() {
                                                        actions.modulation_actions.push(ModulationAction::UpdateAudioSource { idx, source_id: Some(dev.id) });
                                                    }
                                                }
                                            });
                                    });
                                }
                                // Audio level bar
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    ui.add(egui::ProgressBar::new(cur_val).desired_width(140.0).fill(mod_color));
                                }
                            }
                            ModSourceUI::ADSR { attack, decay, sustain, release, stage: _ } => {
                                ui.horizontal(|ui| {
                                    if ui.button(egui::RichText::new("▶ Gate").small()).clicked() {
                                        actions.modulation_actions.push(ModulationAction::TriggerADSR { idx });
                                    }
                                    if ui.button(egui::RichText::new("⏹ Release").small()).clicked() {
                                        actions.modulation_actions.push(ModulationAction::ReleaseADSR { idx });
                                    }
                                });
                                let mut a = *attack;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("A:").small());
                                    let path = format!("mod/{}/attack", idx);
                                    if render_mod_learn_slider(ui, &mut a, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRAttack { idx, attack: a });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "attack");
                                });
                                let mut d = *decay;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("D:").small());
                                    let path = format!("mod/{}/decay", idx);
                                    if render_mod_learn_slider(ui, &mut d, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRDecay { idx, decay: d });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "decay");
                                });
                                let mut s = *sustain;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("S:").small());
                                    let path = format!("mod/{}/sustain", idx);
                                    if render_mod_learn_slider(ui, &mut s, 0.0..=1.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRSustain { idx, sustain: s });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "sustain");
                                });
                                let mut r = *release;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("R:").small());
                                    let path = format!("mod/{}/release", idx);
                                    if render_mod_learn_slider(ui, &mut r, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRRelease { idx, release: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "release");
                                });
                                // ADSR envelope visualization
                                let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().min(180.0), 30.0), egui::Sense::hover());
                                let rect = response.rect;
                                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                                let total_time = a + d + 0.3 + r; // sustain shown as ~0.3 segment
                                let ax = rect.left() + (a / total_time) * rect.width();
                                let dx = ax + (d / total_time) * rect.width();
                                let sx = dx + (0.3 / total_time) * rect.width();
                                let rx = sx + (r / total_time) * rect.width();
                                let top = rect.top() + 2.0;
                                let bot = rect.bottom() - 2.0;
                                let sus_y = top + (1.0 - s) * (bot - top);
                                let points = vec![
                                    egui::pos2(rect.left(), bot),  // start at 0
                                    egui::pos2(ax, top),           // attack peak
                                    egui::pos2(dx, sus_y),         // decay to sustain
                                    egui::pos2(sx, sus_y),         // sustain hold
                                    egui::pos2(rx, bot),           // release to 0
                                ];
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(idx) {
                                    let y = bot - cur_val * (bot - top);
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::StepSequencer { steps, rate, interpolation, bipolar } => {
                                let mut r = *rate;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Rate:").small());
                                    let path = format!("mod/{}/rate", idx);
                                    if render_mod_learn_slider(ui, &mut r, 0.1..=20.0, |s| s.logarithmic(true).suffix("Hz").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateStepRate { idx, rate: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, idx, "rate");
                                });
                                // Interpolation mode
                                let interp_names = ["None", "Linear", "Smooth"];
                                let current_interp = match interpolation {
                                    StepInterpolation::None => 0,
                                    StepInterpolation::Linear => 1,
                                    StepInterpolation::Smooth => 2,
                                };
                                let mut selected_interp = current_interp;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Interp:").small());
                                    egui::ComboBox::from_id_salt(format!("step_interp_{}", idx))
                                        .selected_text(interp_names[selected_interp])
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in interp_names.iter().enumerate() {
                                                if ui.selectable_value(&mut selected_interp, i, *name).changed() {
                                                    let new_interp = match i {
                                                        0 => StepInterpolation::None,
                                                        1 => StepInterpolation::Linear,
                                                        _ => StepInterpolation::Smooth,
                                                    };
                                                    actions.modulation_actions.push(ModulationAction::UpdateStepInterpolation { idx, interpolation: new_interp });
                                                }
                                            }
                                        });
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateStepBipolar { idx, bipolar: bp });
                                }
                                // Step value sliders (compact)
                                ui.horizontal_wrapped(|ui| {
                                    for (step_idx, step_val) in steps.iter().enumerate() {
                                        let mut val = *step_val;
                                        let slider = egui::Slider::new(&mut val, 0.0..=1.0)
                                            .vertical()
                                            .show_value(false);
                                        if data.midi_learn_active {
                                            let inner = ui.scope(|ui| {
                                                ui.disable();
                                                ui.add_sized([12.0, 30.0], slider)
                                            });
                                            let step_path = format!("mod/{}/step/{}", idx, step_idx);
                                            let is_target = data.midi_learn_target.as_deref() == Some(step_path.as_str());
                                            if is_target {
                                                widgets::draw_midi_learn_selected(ui, inner.inner.rect);
                                            } else {
                                                widgets::draw_midi_learn_glow(ui, inner.inner.rect);
                                            }
                                            let click_id = ui.id().with(("midi_learn_step", idx, step_idx));
                                            if ui.interact(inner.inner.rect, click_id, egui::Sense::click()).clicked() {
                                                actions.midi_learn_select = Some(step_path);
                                            }
                                        } else {
                                            if ui.add_sized([12.0, 30.0], slider).on_hover_text(format!("Step {}", step_idx + 1)).changed() {
                                                actions.modulation_actions.push(ModulationAction::UpdateStepValue { idx, step_idx, value: val });
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    });
                ui.add_space(4.0);
            }
        });
    }
}

/// Render a modulation source slider with MIDI learn support.
/// Returns true if the slider value changed (only in non-learn mode).
pub(super) fn render_mod_learn_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    slider_opts: impl FnOnce(egui::Slider<'_>) -> egui::Slider<'_>,
    midi_path: &str,
    data: &UIData,
    actions: &mut UIActions,
) -> bool {
    let mut changed = false;
    if data.midi_learn_active {
        let inner = ui.scope(|ui| {
            ui.disable();
            ui.add(slider_opts(egui::Slider::new(value, range)))
        });
        let slider_rect = inner.inner.rect;
        let is_target = data.midi_learn_target.as_deref() == Some(midi_path);
        if is_target {
            widgets::draw_midi_learn_selected(ui, slider_rect);
        } else {
            widgets::draw_midi_learn_glow(ui, slider_rect);
        }
        let click_id = ui.id().with(("midi_learn_mod", midi_path));
        let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
        if click_resp.clicked() {
            actions.midi_learn_select = Some(midi_path.to_string());
        }
    } else {
        if ui.add(slider_opts(egui::Slider::new(value, range))).changed() {
            changed = true;
        }
    }
    changed
}

/// Render a mod-on-mod assignment dropdown for a modulator's parameter.
/// `target_idx` is the modulator whose parameter is being targeted.
/// `param_name` is the parameter name (e.g., "frequency", "amplitude", "phase").
/// Shows a 🎛 combo listing all other modulators that can modulate this parameter.
pub(super) fn render_mod_on_mod_dropdown(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    target_idx: usize,
    param_name: &str,
) {
    let key = format!("mod:{}:{}", target_idx, param_name);
    let has_assignment = data.modulation_assignments.get(&key).map_or(false, |v| !v.is_empty());
    let btn_text = if has_assignment { "🎛" } else { "🎛" };
    let btn_color = if has_assignment {
        modulator_color(data.modulation_assignments.get(&key)
            .and_then(|v| v.first())
            .map(|a| a.source_idx)
            .unwrap_or(0))
    } else {
        egui::Color32::GRAY
    };

    egui::ComboBox::from_id_salt(format!("mom_{}_{}", target_idx, param_name))
        .selected_text(egui::RichText::new(btn_text).color(btn_color).small())
        .width(30.0)
        .show_ui(ui, |ui| {
            ui.label(egui::RichText::new(format!("Modulate {}", param_name)).small().strong());
            for (src_idx, src) in data.modulation_sources.iter().enumerate() {
                if src_idx == target_idx { continue; } // can't modulate yourself
                let color = modulator_color(src_idx);
                let src_name = match src {
                    ModSourceUI::LFO { .. } => format!("LFO {}", src_idx + 1),
                    ModSourceUI::Audio { freq_low, freq_high, .. } => format!("Audio {:.0}-{:.0}Hz", freq_low, freq_high),
                    ModSourceUI::ADSR { .. } => format!("ADSR {}", src_idx + 1),
                    ModSourceUI::StepSequencer { .. } => format!("StepSeq {}", src_idx + 1),
                };
                if ui.button(egui::RichText::new(format!("+ {}", src_name)).color(color).small()).clicked() {
                    actions.modulation_actions.push(ModulationAction::AssignModOnMod {
                        target_source_idx: target_idx,
                        param_name: param_name.to_string(),
                        modulator_idx: src_idx,
                        amount: 1.0,
                    });
                }
            }
            if has_assignment {
                ui.separator();
                if ui.button(egui::RichText::new("x Remove").small().color(egui::Color32::from_rgb(255, 100, 100))).clicked() {
                    actions.modulation_actions.push(ModulationAction::RemoveModOnMod {
                        target_source_idx: target_idx,
                        param_name: param_name.to_string(),
                    });
                }
            }
        });
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_modulation_section_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_modulation_section(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_modulation_section_smoke_empty() {
        let mut data = UIData::test_fixture();
        data.modulation_sources.clear();
        data.modulation_current_values.clear();
        data.modulation_assignments.clear();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_modulation_section(ui, &data, &mut actions);
        });
    }
}