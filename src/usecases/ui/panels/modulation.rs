//! Modulation sources panel.

use super::super::{modulator_color, widgets, ModSourceUI, ModulationAction, UIActions, UIData};
use crate::modulation::{LFOWaveform, StepInterpolation};

pub(super) fn render_modulation_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.horizontal(|ui| {
        if ui.button("➕ LFO").clicked() {
            actions.modulation_actions.push(ModulationAction::AddLFO {
                waveform: LFOWaveform::Sine,
                frequency: 1.0,
            });
        }
        if ui.button("➕ Audio").clicked() {
            actions
                .modulation_actions
                .push(ModulationAction::AddAudioFFT {
                    preset: crate::modulation::AudioBandPreset::Low,
                    source_id: None,
                });
        }
        if ui.button("➕ ADSR").clicked() {
            actions.modulation_actions.push(ModulationAction::AddADSR {
                attack: 0.1,
                decay: 0.3,
                sustain: 0.7,
                release: 0.5,
            });
        }
        if ui.button("➕ StepSeq").clicked() {
            actions
                .modulation_actions
                .push(ModulationAction::AddStepSequencer {
                    num_steps: 8,
                    rate: 2.0,
                });
        }
    });

    if data.modulation_sources.is_empty() {
        ui.label(egui::RichText::new("No modulation sources").small().weak());
    } else {
        egui::ScrollArea::vertical().id_salt("mod_sources_vscroll").show(ui, |ui| {
            for (idx, entry) in data.modulation_sources.iter().enumerate() {
                let mod_color = modulator_color(idx);
                let dim_color = egui::Color32::from_rgba_premultiplied(
                    mod_color.r() / 4, mod_color.g() / 4, mod_color.b() / 4, 40
                );
                let sid = &entry.uuid;
                let header_label = match &entry.source {
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
                    ModSourceUI::Analyzer { analyzer_type, .. } => {
                        format!("Analyzer {} {}", analyzer_type, idx + 1)
                    }
                };
                // Show current value in header if available
                let value_text = data.modulation_current_values.get(sid)
                    .map(|v| format!(" ({:.2})", v))
                    .unwrap_or_default();

                egui::Frame::default()
                    .inner_margin(4.0)
                    .corner_radius(4.0)
                    .fill(dim_color)
                    .stroke(egui::Stroke::new(1.0_f32, mod_color))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(160.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}{}", header_label, value_text)).strong().color(mod_color));
                            if ui.small_button("x").clicked() {
                                actions.modulation_actions.push(ModulationAction::RemoveSource { source_id: sid.clone() });
                            }
                        });
                        match &entry.source {
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
                                                    actions.modulation_actions.push(ModulationAction::UpdateLFOWaveform { source_id: sid.clone(), waveform: new_wf });
                                                }
                                            }
                                        });
                                });
                                let mut freq = *frequency;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Freq:").small());
                                    let path = format!("mod/{}/frequency", idx);
                                    if render_mod_learn_slider(ui, &mut freq, 0.01..=10.0, |s| s.logarithmic(true).show_value(true).suffix("Hz"), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOFrequency { source_id: sid.clone(), frequency: freq });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "frequency");
                                });
                                let mut amp = *amplitude;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Amp:").small());
                                    let path = format!("mod/{}/amplitude", idx);
                                    if render_mod_learn_slider(ui, &mut amp, 0.0..=1.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOAmplitude { source_id: sid.clone(), amplitude: amp });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "amplitude");
                                });
                                let mut ph = *phase;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Phase:").small());
                                    let path = format!("mod/{}/phase", idx);
                                    if render_mod_learn_slider(ui, &mut ph, 0.0..=1.0, |s| s.show_value(false), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateLFOPhase { source_id: sid.clone(), phase: ph });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "phase");
                                });
                                let mut bp = *bipolar;
                                if ui.checkbox(&mut bp, egui::RichText::new("Bipolar (-1 to 1)").small()).changed() {
                                    actions.modulation_actions.push(ModulationAction::UpdateLFOBipolar { source_id: sid.clone(), bipolar: bp });
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
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5_f32, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(sid) {
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
                                                    actions.modulation_actions.push(ModulationAction::UpdateAudioMode { source_id: sid.clone(), mode: m });
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
                                                    actions.modulation_actions.push(ModulationAction::UpdateAudioPreset { source_id: sid.clone(), preset });
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
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioFreqLow { source_id: sid.clone(), freq_low: fl });
                                    }
                                });
                                let mut fh = *freq_high;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Hi:").small());
                                    let path = format!("mod/{}/freq_high", idx);
                                    if render_mod_learn_slider(ui, &mut fh, 20.0..=20000.0, |s| s.logarithmic(true).suffix("Hz"), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioFreqHigh { source_id: sid.clone(), freq_high: fh });
                                    }
                                });
                                // Gain slider
                                let mut g = *gain;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Gain:").small());
                                    let path = format!("mod/{}/gain", idx);
                                    if render_mod_learn_slider(ui, &mut g, 0.0..=4.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioGain { source_id: sid.clone(), gain: g });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "gain");
                                });
                                // Smoothing slider
                                let mut sm = *smoothing;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Smooth:").small());
                                    let path = format!("mod/{}/smoothing", idx);
                                    if render_mod_learn_slider(ui, &mut sm, 0.0..=0.99, |s| s.show_value(false), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioSmoothing { source_id: sid.clone(), smoothing: sm });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "smoothing");
                                });
                                // Noise gate slider
                                let mut ng = *noise_gate;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Gate:").small());
                                    if ui.add(egui::Slider::new(&mut ng, 0.0..=0.5).show_value(true)).changed() {
                                        actions.modulation_actions.push(ModulationAction::UpdateAudioNoiseGate { source_id: sid.clone(), noise_gate: ng });
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
                                                    actions.modulation_actions.push(ModulationAction::UpdateAudioSource { source_id: sid.clone(), source_id_audio: None });
                                                }
                                                for dev in &data.audio.devices {
                                                    if ui.selectable_label(*source_id == Some(dev.id), &dev.name).clicked() {
                                                        actions.modulation_actions.push(ModulationAction::UpdateAudioSource { source_id: sid.clone(), source_id_audio: Some(dev.id) });
                                                    }
                                                }
                                            });
                                    });
                                }
                                // Audio level bar
                                if let Some(&cur_val) = data.modulation_current_values.get(sid) {
                                    ui.add(egui::ProgressBar::new(cur_val).desired_width(140.0).fill(mod_color));
                                }
                            }
                            ModSourceUI::ADSR { attack, decay, sustain, release, stage: _ } => {
                                // Combined gate button: press → trigger, release → gate off
                                let gate_path = format!("mod/{}/gate", idx);
                                let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                                let gate_id = ui.id().with(("adsr_gate", idx));
                                let was_held = ui.memory(|m| m.data.get_temp::<bool>(gate_id).unwrap_or(false));
                                let label = if was_held { "⏹ Gate (held)" } else { "▶ Gate" };
                                let btn_color = if was_held { egui::Color32::from_rgb(80, 200, 80) } else { egui::Color32::from_rgb(180, 180, 180) };
                                let (gate_rect, gate_resp) = ui.allocate_exact_size(
                                    egui::vec2(80.0, 18.0),
                                    egui::Sense::click_and_drag(),
                                );
                                // Draw button-like appearance
                                let visuals = if was_held {
                                    ui.visuals().widgets.active
                                } else if gate_resp.hovered() {
                                    ui.visuals().widgets.hovered
                                } else {
                                    ui.visuals().widgets.inactive
                                };
                                ui.painter().rect(gate_rect, visuals.corner_radius, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Outside);
                                ui.painter().text(
                                    gate_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    label,
                                    egui::FontId::proportional(11.0),
                                    btn_color,
                                );
                                // Detect pointer press and release on this widget
                                if !any_learn {
                                    let pressed_now = gate_resp.is_pointer_button_down_on();
                                    if pressed_now && !was_held {
                                        ui.memory_mut(|m| m.data.insert_temp(gate_id, true));
                                        actions.modulation_actions.push(ModulationAction::TriggerADSR { source_id: sid.clone() });
                                    } else if !pressed_now && was_held {
                                        ui.memory_mut(|m| m.data.insert_temp(gate_id, false));
                                        actions.modulation_actions.push(ModulationAction::ReleaseADSR { source_id: sid.clone() });
                                    }
                                }
                                // MIDI learn overlay
                                if data.midi_learn_active {
                                    let is_target = data.midi_learn_target.as_deref() == Some(gate_path.as_str());
                                    if is_target {
                                        widgets::draw_midi_learn_selected(ui, gate_rect);
                                    } else {
                                        widgets::draw_midi_learn_glow(ui, gate_rect);
                                    }
                                    let click_id = ui.id().with(("midi_learn_gate", idx));
                                    if ui.interact(gate_rect, click_id, egui::Sense::click()).clicked() {
                                        actions.midi_learn_select = Some(gate_path.clone());
                                    }
                                }
                                // Keyboard learn overlay
                                if data.keyboard_learn_active {
                                    let is_target = data.keyboard_learn_target.as_deref() == Some(gate_path.as_str());
                                    if is_target {
                                        widgets::draw_keyboard_learn_selected(ui, gate_rect);
                                    } else {
                                        widgets::draw_keyboard_learn_glow(ui, gate_rect);
                                    }
                                    let click_id = ui.id().with(("kb_learn_gate", idx));
                                    if ui.interact(gate_rect, click_id, egui::Sense::click()).clicked() {
                                        actions.keyboard_learn_select = Some(crate::keymap::KeyTarget::ParamPath(gate_path.clone()));
                                    }
                                }
                                let mut a = *attack;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("A:").small());
                                    let path = format!("mod/{}/attack", idx);
                                    if render_mod_learn_slider(ui, &mut a, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRAttack { source_id: sid.clone(), attack: a });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "attack");
                                });
                                let mut d = *decay;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("D:").small());
                                    let path = format!("mod/{}/decay", idx);
                                    if render_mod_learn_slider(ui, &mut d, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRDecay { source_id: sid.clone(), decay: d });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "decay");
                                });
                                let mut s = *sustain;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("S:").small());
                                    let path = format!("mod/{}/sustain", idx);
                                    if render_mod_learn_slider(ui, &mut s, 0.0..=1.0, |s| s.show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRSustain { source_id: sid.clone(), sustain: s });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "sustain");
                                });
                                let mut r = *release;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("R:").small());
                                    let path = format!("mod/{}/release", idx);
                                    if render_mod_learn_slider(ui, &mut r, 0.001..=5.0, |s| s.logarithmic(true).suffix("s").show_value(true), &path, data, actions) {
                                        actions.modulation_actions.push(ModulationAction::UpdateADSRRelease { source_id: sid.clone(), release: r });
                                    }
                                    render_mod_on_mod_dropdown(ui, data, actions, sid, "release");
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
                                painter.add(egui::Shape::line(points, egui::Stroke::new(1.5_f32, mod_color)));
                                // Current value indicator
                                if let Some(&cur_val) = data.modulation_current_values.get(sid) {
                                    let y = bot - cur_val * (bot - top);
                                    painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, mod_color);
                                }
                            }
                            ModSourceUI::StepSequencer { steps, rate, interpolation, bipolar } => {
                                render_step_sequencer_controls(ui, idx, sid, steps, *rate, interpolation, *bipolar, mod_color, data, actions);
                            }
                            ModSourceUI::Analyzer { deck_id, analyzer_type, output_name, smoothing } => {
                                ui.label(format!("Deck: {}", deck_id));
                                ui.label(format!("Type: {}", analyzer_type));
                                ui.label(format!("Output: {}", output_name));
                                ui.label(format!("Smoothing: {:.2}", smoothing));
                            }
                        }
                    });
                ui.add_space(4.0);
            }
        });
    }
}

/// Render step sequencer controls: rate, interpolation, bipolar, step count, and painted bar grid.
// UI render fn taking many independent egui state/handle args; no shared invariant to bundle.
#[allow(clippy::too_many_arguments)]
fn render_step_sequencer_controls(
    ui: &mut egui::Ui,
    idx: usize,
    sid: &str,
    steps: &[f32],
    rate: f32,
    interpolation: &StepInterpolation,
    bipolar: bool,
    mod_color: egui::Color32,
    data: &UIData,
    actions: &mut UIActions,
) {
    // Controls row: Rate + Interp + Bipolar + Step count
    let mut r = rate;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Rate:").small());
        let path = format!("mod/{}/rate", idx);
        if render_mod_learn_slider(
            ui,
            &mut r,
            0.1..=20.0,
            |s| s.logarithmic(true).suffix("Hz").show_value(true),
            &path,
            data,
            actions,
        ) {
            actions
                .modulation_actions
                .push(ModulationAction::UpdateStepRate {
                    source_id: sid.to_string(),
                    rate: r,
                });
        }
        render_mod_on_mod_dropdown(ui, data, actions, sid, "rate");
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Interp:").small());
        let interp_names = ["None", "Linear", "Smooth"];
        let current_interp = match interpolation {
            StepInterpolation::None => 0,
            StepInterpolation::Linear => 1,
            StepInterpolation::Smooth => 2,
        };
        let mut selected_interp = current_interp;
        egui::ComboBox::from_id_salt(format!("step_interp_{}", idx))
            .selected_text(interp_names[selected_interp])
            .width(60.0)
            .show_ui(ui, |ui| {
                for (i, name) in interp_names.iter().enumerate() {
                    if ui
                        .selectable_value(&mut selected_interp, i, *name)
                        .changed()
                    {
                        let new_interp = match i {
                            0 => StepInterpolation::None,
                            1 => StepInterpolation::Linear,
                            _ => StepInterpolation::Smooth,
                        };
                        actions.modulation_actions.push(
                            ModulationAction::UpdateStepInterpolation {
                                source_id: sid.to_string(),
                                interpolation: new_interp,
                            },
                        );
                    }
                }
            });
        ui.add_space(4.0);
        let mut bp = bipolar;
        if ui
            .checkbox(&mut bp, egui::RichText::new("Bipolar").small())
            .changed()
        {
            actions
                .modulation_actions
                .push(ModulationAction::UpdateStepBipolar {
                    source_id: sid.to_string(),
                    bipolar: bp,
                });
        }
    });
    // Step count controls
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Steps: {}", steps.len())).small());
        if ui.small_button("−").clicked() && steps.len() > 2 {
            actions
                .modulation_actions
                .push(ModulationAction::SetStepCount {
                    source_id: sid.to_string(),
                    count: steps.len() - 1,
                });
        }
        if ui.small_button("+").clicked() && steps.len() < 64 {
            actions
                .modulation_actions
                .push(ModulationAction::SetStepCount {
                    source_id: sid.to_string(),
                    count: steps.len() + 1,
                });
        }
        for &preset in &[4, 8, 16, 32] {
            if steps.len() != preset {
                if ui
                    .small_button(egui::RichText::new(format!("{}", preset)).small())
                    .clicked()
                {
                    actions
                        .modulation_actions
                        .push(ModulationAction::SetStepCount {
                            source_id: sid.to_string(),
                            count: preset,
                        });
                }
            } else {
                ui.label(
                    egui::RichText::new(format!("[{}]", preset))
                        .small()
                        .strong()
                        .color(mod_color),
                );
            }
        }
    });

    // Painted step grid
    let grid_width = ui.available_width().clamp(120.0, 260.0);
    let grid_height = 80.0_f32;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(grid_width, grid_height),
        egui::Sense::click_and_drag(),
    );
    let rect = response.rect;

    // Background
    painter.rect_filled(rect, 3.0, egui::Color32::from_gray(18));
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(40)),
        egui::StrokeKind::Outside,
    );

    let num_steps = steps.len();
    if num_steps == 0 {
        return;
    }
    let step_w = rect.width() / num_steps as f32;
    let bar_pad = 1.0_f32;

    // Grid lines (horizontal at 25%, 50%, 75%)
    for frac in &[0.25_f32, 0.5, 0.75] {
        let y = rect.top() + rect.height() * (1.0 - frac);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.5_f32, egui::Color32::from_gray(35)),
        );
    }
    // Vertical step dividers
    for i in 1..num_steps {
        let x = rect.left() + i as f32 * step_w;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(0.5_f32, egui::Color32::from_gray(30)),
        );
    }

    // Draw bars
    let bar_color = mod_color.linear_multiply(0.7);
    let bar_hover_color = mod_color;
    let hover_step = if response.hovered() || response.dragged() {
        response
            .hover_pos()
            .or_else(|| response.interact_pointer_pos())
            .and_then(|pos| {
                let local_x = pos.x - rect.left();
                let s = (local_x / step_w).floor() as usize;
                if s < num_steps {
                    Some(s)
                } else {
                    None
                }
            })
    } else {
        None
    };

    for (i, &val) in steps.iter().enumerate() {
        let x0 = rect.left() + i as f32 * step_w + bar_pad;
        let x1 = rect.left() + (i + 1) as f32 * step_w - bar_pad;
        let bar_h = val.clamp(0.0, 1.0) * rect.height();
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x0, rect.bottom() - bar_h),
            egui::pos2(x1, rect.bottom()),
        );
        let color = if hover_step == Some(i) {
            bar_hover_color
        } else {
            bar_color
        };
        painter.rect_filled(bar_rect, 1.0, color);
    }

    // Current value indicator line
    if let Some(&cur_val) = data.modulation_current_values.get(sid) {
        let display_val = if bipolar {
            (cur_val + 1.0) / 2.0
        } else {
            cur_val
        };
        let y = rect.bottom() - display_val.clamp(0.0, 1.0) * rect.height();
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.5_f32, egui::Color32::WHITE.linear_multiply(0.6)),
        );
    }

    // Click/drag to set step values
    let any_learn = data.midi_learn_active || data.keyboard_learn_active;
    if !any_learn && (response.clicked() || response.dragged()) {
        if let (Some(pos), Some(step_idx)) = (
            response
                .hover_pos()
                .or_else(|| response.interact_pointer_pos()),
            hover_step,
        ) {
            let val = 1.0 - ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            actions
                .modulation_actions
                .push(ModulationAction::UpdateStepValue {
                    source_id: sid.to_string(),
                    step_idx,
                    value: val,
                });
        }
    }

    // MIDI/keyboard learn overlays on step bars
    if any_learn {
        for (step_idx, _) in steps.iter().enumerate() {
            let x0 = rect.left() + step_idx as f32 * step_w;
            let step_rect = egui::Rect::from_min_size(
                egui::pos2(x0, rect.top()),
                egui::vec2(step_w, rect.height()),
            );
            let step_path = format!("mod/{}/step/{}", idx, step_idx);
            if data.midi_learn_active {
                let is_target = data.midi_learn_target.as_deref() == Some(step_path.as_str());
                if is_target {
                    widgets::draw_midi_learn_selected(ui, step_rect);
                } else {
                    widgets::draw_midi_learn_glow(ui, step_rect);
                }
                let click_id = ui.id().with(("midi_learn_step", idx, step_idx));
                if ui
                    .interact(step_rect, click_id, egui::Sense::click())
                    .clicked()
                {
                    actions.midi_learn_select = Some(step_path.clone());
                }
            }
            if data.keyboard_learn_active {
                let is_target = data.keyboard_learn_target.as_deref() == Some(step_path.as_str());
                if is_target {
                    widgets::draw_keyboard_learn_selected(ui, step_rect);
                } else {
                    widgets::draw_keyboard_learn_glow(ui, step_rect);
                }
                let click_id = ui.id().with(("kb_learn_step", idx, step_idx));
                if ui
                    .interact(step_rect, click_id, egui::Sense::click())
                    .clicked()
                {
                    actions.keyboard_learn_select =
                        Some(crate::keymap::KeyTarget::ParamPath(step_path));
                }
            }
        }
    }
}

/// Render a modulation source slider with MIDI/keyboard learn support.
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
    let any_learn = data.midi_learn_active || data.keyboard_learn_active;
    if any_learn {
        let inner = ui.scope(|ui| {
            ui.disable();
            ui.add(slider_opts(egui::Slider::new(value, range)))
        });
        let slider_rect = inner.inner.rect;
        if data.midi_learn_active {
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
        }
        if data.keyboard_learn_active {
            let is_target = data.keyboard_learn_target.as_deref() == Some(midi_path);
            if is_target {
                widgets::draw_keyboard_learn_selected(ui, slider_rect);
            } else {
                widgets::draw_keyboard_learn_glow(ui, slider_rect);
            }
            let click_id = ui.id().with(("kb_learn_mod", midi_path));
            let click_resp = ui.interact(slider_rect, click_id, egui::Sense::click());
            if click_resp.clicked() {
                actions.keyboard_learn_select =
                    Some(crate::keymap::KeyTarget::ParamPath(midi_path.to_string()));
            }
        }
    } else if ui
        .add(slider_opts(egui::Slider::new(value, range)))
        .changed()
    {
        changed = true;
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
    target_uuid: &str,
    param_name: &str,
) {
    let key = format!("mod:{}:{}", target_uuid, param_name);
    let has_assignment = data
        .modulation_assignments
        .get(&key)
        .is_some_and(|v| !v.is_empty());
    let btn_text = "〰";
    let btn_color = if has_assignment {
        let source_id = data
            .modulation_assignments
            .get(&key)
            .and_then(|v| v.first())
            .map(|a| &a.source_id);
        let color_idx = source_id
            .and_then(|sid| data.modulation_sources.iter().position(|e| &e.uuid == sid))
            .unwrap_or(0);
        modulator_color(color_idx)
    } else {
        egui::Color32::GRAY
    };

    egui::ComboBox::from_id_salt(format!("mom_{}_{}", target_uuid, param_name))
        .selected_text(egui::RichText::new(btn_text).color(btn_color).small())
        .width(30.0)
        .show_ui(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("Modulate {}", param_name))
                    .small()
                    .strong(),
            );
            for (src_idx, entry) in data.modulation_sources.iter().enumerate() {
                if entry.uuid == target_uuid {
                    continue;
                } // can't modulate yourself
                let color = modulator_color(src_idx);
                let src_name = match &entry.source {
                    ModSourceUI::LFO { .. } => format!("LFO {}", src_idx + 1),
                    ModSourceUI::Audio {
                        freq_low,
                        freq_high,
                        ..
                    } => format!("Audio {:.0}-{:.0}Hz", freq_low, freq_high),
                    ModSourceUI::ADSR { .. } => format!("ADSR {}", src_idx + 1),
                    ModSourceUI::StepSequencer { .. } => format!("StepSeq {}", src_idx + 1),
                    ModSourceUI::Analyzer { analyzer_type, .. } => {
                        format!("Analyzer {} {}", analyzer_type, src_idx + 1)
                    }
                };
                if ui
                    .button(
                        egui::RichText::new(format!("+ {}", src_name))
                            .color(color)
                            .small(),
                    )
                    .clicked()
                {
                    actions
                        .modulation_actions
                        .push(ModulationAction::AssignModOnMod {
                            target_source_id: target_uuid.to_string(),
                            param_name: param_name.to_string(),
                            modulator_id: entry.uuid.clone(),
                            amount: 1.0,
                        });
                }
            }
            if has_assignment {
                ui.separator();
                if ui
                    .button(
                        egui::RichText::new("x Remove")
                            .small()
                            .color(egui::Color32::from_rgb(255, 100, 100)),
                    )
                    .clicked()
                {
                    actions
                        .modulation_actions
                        .push(ModulationAction::RemoveModOnMod {
                            target_source_id: target_uuid.to_string(),
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
