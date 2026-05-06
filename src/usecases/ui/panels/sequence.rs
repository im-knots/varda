//! Transition sequence builder and step editor.

use super::super::{UIData, UIActions};

pub(super) fn render_sequence_builder(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    use super::super::SequenceAction;

    for (seq_idx, seq) in data.sequences.iter().enumerate() {
        ui.push_id(format!("seq_{}", seq_idx), |ui| {
            let border_color = if seq.playing {
                egui::Color32::from_rgb(80, 160, 80)
            } else {
                egui::Color32::from_rgb(50, 50, 70)
            };
            egui::Frame::default()
                .inner_margin(4.0)
                .corner_radius(4.0)
                .fill(egui::Color32::from_rgb(18, 18, 28))
                .stroke(egui::Stroke::new(1.0, border_color))
                .show(ui, |ui| {
                    // Header: name | enable | play/stop | delete
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(egui::RichText::new(&seq.name).strong().size(11.0));

                        let (en_label, en_color) = if seq.enabled {
                            ("On", egui::Color32::from_rgb(80, 200, 80))
                        } else {
                            ("Off", egui::Color32::from_rgb(120, 120, 120))
                        };
                        if ui.small_button(egui::RichText::new(en_label).color(en_color)).on_hover_text("Toggle enabled").clicked() {
                            actions.sequence_actions.push(SequenceAction::ToggleEnabled(seq_idx));
                        }

                        if seq.playing {
                            if ui.small_button("Stop").on_hover_text("Stop playback").clicked() {
                                actions.sequence_actions.push(SequenceAction::Stop(seq_idx));
                            }
                        } else if seq.enabled && !seq.steps.is_empty() {
                            if ui.small_button("Play").on_hover_text("Start playback").clicked() {
                                actions.sequence_actions.push(SequenceAction::Play(seq_idx));
                            }
                        }

                        if ui.small_button("x").on_hover_text("Delete sequence").clicked() {
                            actions.sequence_actions.push(SequenceAction::Delete(seq_idx));
                        }
                    });

                    // Step list
                    let mut step_to_remove: Option<usize> = None;
                    if seq.steps.is_empty() {
                        ui.label(egui::RichText::new("No steps").small().weak());
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt(format!("seq_scroll_{}", seq_idx))
                            .max_height(120.0)
                            .show(ui, |ui| {
                            for (i, step) in seq.steps.iter().enumerate() {
                                let is_current = seq.playing && i == seq.current_step;
                                let bg = if is_current {
                                    egui::Color32::from_rgba_premultiplied(30, 70, 30, 220)
                                } else if i % 2 == 0 {
                                    egui::Color32::from_rgb(22, 22, 32)
                                } else {
                                    egui::Color32::from_rgb(26, 26, 36)
                                };

                                egui::Frame::default()
                                    .fill(bg)
                                    .inner_margin(egui::Margin::symmetric(2, 1))
                                    .corner_radius(2.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            let idx_color = if is_current {
                                                egui::Color32::from_rgb(100, 220, 100)
                                            } else {
                                                egui::Color32::from_rgb(80, 80, 100)
                                            };
                                            ui.label(egui::RichText::new(format!("{}", i + 1)).monospace().small().color(idx_color));
                                            render_sequence_step_editor(ui, seq_idx, i, step, data, actions);
                                            if ui.small_button("x").on_hover_text("Remove step").clicked() {
                                                step_to_remove = Some(i);
                                            }
                                        });
                                    });
                            }
                        });
                    }

                    if let Some(idx) = step_to_remove {
                        actions.sequence_actions.push(SequenceAction::RemoveStep { seq_idx, step_idx: idx });
                    }

                    ui.add_space(2.0);
                    // Add step buttons
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let from = 0.min(data.channel_count.saturating_sub(1));
                        let to = 1.min(data.channel_count.saturating_sub(1));
                        if ui.small_button("+Fade").clicked() {
                            actions.sequence_actions.push(SequenceAction::AddFade { seq_idx, from_ch: from, to_ch: to });
                        }
                        if ui.small_button("+Wait").clicked() {
                            actions.sequence_actions.push(SequenceAction::AddWait(seq_idx));
                        }
                        if ui.small_button("+Loop").clicked() {
                            actions.sequence_actions.push(SequenceAction::AddGoTo { seq_idx, step_index: 0 });
                        }
                    });
                });
            ui.add_space(2.0);
        });
    }

    // Add sequence button
    if ui.small_button("+ Sequence").on_hover_text("Create a new transition sequence").clicked() {
        actions.sequence_actions.push(SequenceAction::Create);
    }
}

/// Render inline editor for a single sequence step
pub(super) fn render_sequence_step_editor(
    ui: &mut egui::Ui,
    seq_idx: usize,
    step_idx: usize,
    step: &super::super::SequenceStepUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    use super::super::SequenceAction;
    let channel_count = data.channel_count;
    let channel_names = &data.channel_names;
    let seq = &data.sequences[seq_idx];

    match &step.kind {
        super::super::SequenceStepKindUI::Fade { from_ch, to_ch, duration_val, is_beats, easing, transition_shader } => {
            // From channel selector
            let from_label = channel_names.get(*from_ch).map(|s| s.as_str()).unwrap_or("?");
            egui::ComboBox::from_id_salt(format!("seq{}_from_{}", seq_idx, step_idx))
                .selected_text(egui::RichText::new(from_label).small())
                .width(45.0)
                .show_ui(ui, |ui| {
                    for i in 0..channel_count {
                        let name = channel_names.get(i).map(|s| s.as_str()).unwrap_or("?");
                        if ui.selectable_label(i == *from_ch, name).clicked() {
                            actions.sequence_actions.push(SequenceAction::SetStepFromCh { seq_idx, step_idx, ch: i });
                        }
                    }
                });
            ui.label(egui::RichText::new("->").small());
            // To channel selector
            let to_label = channel_names.get(*to_ch).map(|s| s.as_str()).unwrap_or("?");
            egui::ComboBox::from_id_salt(format!("seq{}_to_{}", seq_idx, step_idx))
                .selected_text(egui::RichText::new(to_label).small())
                .width(45.0)
                .show_ui(ui, |ui| {
                    for i in 0..channel_count {
                        let name = channel_names.get(i).map(|s| s.as_str()).unwrap_or("?");
                        if ui.selectable_label(i == *to_ch, name).clicked() {
                            actions.sequence_actions.push(SequenceAction::SetStepToCh { seq_idx, step_idx, ch: i });
                        }
                    }
                });

            // Duration
            let mut dur = *duration_val;
            let unit_label = if *is_beats { "b" } else { "s" };
            let slider = egui::Slider::new(&mut dur, 0.1..=60.0)
                .logarithmic(true)
                .suffix(unit_label)
                .max_decimals(1);
            if ui.add_sized([70.0, 16.0], slider).changed() {
                actions.sequence_actions.push(SequenceAction::SetStepDuration { seq_idx, step_idx, value: dur });
            }
            if ui.small_button(unit_label).on_hover_text("Toggle beats/seconds").clicked() {
                actions.sequence_actions.push(SequenceAction::ToggleStepDurationUnit { seq_idx, step_idx });
            }

            // Easing
            egui::ComboBox::from_id_salt(format!("seq{}_ease_{}", seq_idx, step_idx))
                .selected_text(egui::RichText::new(easing.as_str()).small())
                .width(55.0)
                .show_ui(ui, |ui| {
                    for e in &["Linear", "EaseInOut", "EaseIn", "EaseOut"] {
                        if ui.selectable_label(*e == easing.as_str(), *e).clicked() {
                            actions.sequence_actions.push(SequenceAction::SetStepEasing {
                                seq_idx, step_idx, easing: e.to_string(),
                            });
                        }
                    }
                });

            // Transition shader selector (truncated label + tooltip)
            let shader_full = transition_shader.as_deref().unwrap_or("Opacity");
            let max_chars = 8;
            let shader_short = if shader_full.len() > max_chars {
                format!("{}...", &shader_full[..max_chars])
            } else {
                shader_full.to_string()
            };
            let combo = egui::ComboBox::from_id_salt(format!("seq{}_shader_{}", seq_idx, step_idx))
                .selected_text(egui::RichText::new(&shader_short).small())
                .width(60.0)
                .show_ui(ui, |ui| {
                    let is_opacity = transition_shader.is_none();
                    if ui.selectable_label(is_opacity, "Opacity").clicked() {
                        actions.sequence_actions.push(SequenceAction::SetStepTransitionShader {
                            seq_idx, step_idx, shader: None,
                        });
                    }
                    for name in &data.transition_names {
                        let selected = transition_shader.as_ref() == Some(name);
                        if ui.selectable_label(selected, name).clicked() {
                            actions.sequence_actions.push(SequenceAction::SetStepTransitionShader {
                                seq_idx, step_idx, shader: Some(name.clone()),
                            });
                        }
                    }
                });
            combo.response.on_hover_text(shader_full);
        }
        super::super::SequenceStepKindUI::Wait { duration_val, is_beats } => {
            ui.label(egui::RichText::new("Wait").small());
            let mut dur = *duration_val;
            let unit_label = if *is_beats { "b" } else { "s" };
            let slider = egui::Slider::new(&mut dur, 0.1..=60.0)
                .logarithmic(true)
                .suffix(unit_label)
                .max_decimals(1);
            if ui.add_sized([90.0, 16.0], slider).changed() {
                actions.sequence_actions.push(SequenceAction::SetStepDuration { seq_idx, step_idx, value: dur });
            }
            if ui.small_button(unit_label).on_hover_text("Toggle beats/seconds").clicked() {
                actions.sequence_actions.push(SequenceAction::ToggleStepDurationUnit { seq_idx, step_idx });
            }
        }
        super::super::SequenceStepKindUI::GoTo { step_index } => {
            ui.label(egui::RichText::new("GoTo").small());
            let mut target = *step_index as i32;
            let max = seq.steps.len().saturating_sub(1) as i32;
            if ui.add(egui::DragValue::new(&mut target).range(0..=max).speed(0.1)).changed() {
                actions.sequence_actions.push(SequenceAction::SetGoToTarget {
                    seq_idx, step_idx, target: target.max(0) as usize,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_sequence_builder_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_builder(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_sequence_builder_smoke_empty() {
        let mut data = UIData::test_fixture();
        data.sequences.clear();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_builder(ui, &data, &mut actions);
        });
    }
}