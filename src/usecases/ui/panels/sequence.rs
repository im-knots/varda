//! Transition sequence builder: compact read-only timeline strip for mixer area,
//! full interactive editor for the bottom bar.

use super::super::{SequenceStepKindUI, SequenceUIData, UIActions, UIData};
use super::utils::channel_color;
use crate::channel::DurationUnit;

/// Max drag-value range per duration unit.
pub(super) fn duration_drag_max(unit: DurationUnit) -> f64 {
    match unit {
        DurationUnit::Seconds => 120.0,
        DurationUnit::Minutes => 60.0,
        DurationUnit::Hours => 24.0,
        DurationUnit::Beats => 128.0,
    }
}

/// Get step duration in seconds, converting from the step's native unit.
/// Uses the provided BPM for beat-based durations (falls back to 120 BPM if None).
fn step_duration_secs(kind: &SequenceStepKindUI, bpm: Option<f32>) -> f64 {
    match kind {
        SequenceStepKindUI::Fade {
            duration_val,
            duration_unit,
            ..
        }
        | SequenceStepKindUI::Wait {
            duration_val,
            duration_unit,
        } => {
            let val = *duration_val;
            match duration_unit {
                DurationUnit::Seconds => val,
                DurationUnit::Minutes => val * 60.0,
                DurationUnit::Hours => val * 3600.0,
                DurationUnit::Beats => {
                    let bpm_val = bpm.unwrap_or(120.0) as f64;
                    val * 60.0 / bpm_val
                }
            }
        }
        SequenceStepKindUI::GoTo { .. } => 0.0,
    }
}

/// Render compact, read-only timeline strips for all sequences in the mixer area.
/// Clicking a sequence card selects it and opens the bottom bar editor.
pub(super) fn render_sequence_builder(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    use super::super::SequenceAction;

    for (seq_idx, seq) in data.sequences.iter().enumerate() {
        ui.push_id(format!("seq_{}", seq_idx), |ui| {
            let is_selected = data.selected_sequence == Some(seq_idx);
            let border_color = if seq.playing {
                egui::Color32::from_rgb(80, 200, 80)
            } else if is_selected {
                egui::Color32::from_rgb(200, 200, 255)
            } else {
                egui::Color32::from_rgb(50, 50, 70)
            };
            let border_width = if is_selected || seq.playing { 1.5 } else { 1.0 };
            egui::Frame::default()
                .inner_margin(4.0)
                .corner_radius(4.0)
                .fill(egui::Color32::from_rgb(18, 18, 28))
                .stroke(egui::Stroke::new(border_width, border_color))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    // Header: name (clickable to select) | On/Off | Play/Stop | Delete
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;

                        // Name label doubles as click-to-select target
                        let name_resp = ui.add(
                            egui::Label::new(egui::RichText::new(&seq.name).strong().size(11.0))
                                .sense(egui::Sense::click()),
                        );
                        if name_resp.clicked() {
                            actions.select_sequence = Some(seq_idx);
                        }

                        let (en_label, en_color) = if seq.enabled {
                            ("On", egui::Color32::from_rgb(80, 200, 80))
                        } else {
                            ("Off", egui::Color32::from_rgb(120, 120, 120))
                        };
                        if ui
                            .small_button(egui::RichText::new(en_label).color(en_color))
                            .on_hover_text("Toggle enabled")
                            .clicked()
                        {
                            actions
                                .sequence_actions
                                .push(SequenceAction::ToggleEnabled(seq_idx));
                        }

                        if seq.playing {
                            if ui
                                .small_button("Stop")
                                .on_hover_text("Stop playback")
                                .clicked()
                            {
                                actions.sequence_actions.push(SequenceAction::Stop(seq_idx));
                            }
                        } else if seq.enabled && !seq.steps.is_empty() {
                            if ui
                                .small_button("Play")
                                .on_hover_text("Start playback")
                                .clicked()
                            {
                                actions.sequence_actions.push(SequenceAction::Play(seq_idx));
                            }
                        }

                        if ui
                            .small_button("x")
                            .on_hover_text("Delete sequence")
                            .clicked()
                        {
                            actions
                                .sequence_actions
                                .push(SequenceAction::Delete(seq_idx));
                        }
                    });

                    // Read-only timeline strip (click to select)
                    if seq.steps.is_empty() {
                        let empty_resp = ui.add(
                            egui::Label::new(
                                egui::RichText::new("Empty — click to edit").small().weak(),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if empty_resp.clicked() {
                            actions.select_sequence = Some(seq_idx);
                        }
                    } else {
                        let (_step, strip_clicked) = render_timeline_strip(
                            ui,
                            seq,
                            &data.channel_names,
                            false,
                            None,
                            data.clock_bpm,
                        );
                        if strip_clicked {
                            actions.select_sequence = Some(seq_idx);
                        }
                    }
                });

            // Animate playhead during playback
            if seq.playing {
                ui.ctx().request_repaint();
            }

            ui.add_space(2.0);
        });
    }
}

/// Constant width for GoTo blocks in the timeline strip.
const GOTO_BLOCK_WIDTH: f32 = 24.0;
/// Minimum width for timed blocks so labels remain visible.
const MIN_BLOCK_WIDTH: f32 = 30.0;

/// Paint a horizontal timeline strip showing sequence steps as colored blocks.
///
/// * `interactive` — if true, blocks are clickable and return the clicked step index.
/// * `selected_step` — optional step index to highlight in the interactive version.
///
/// Returns `(clicked_step, strip_clicked)`:
/// - `clicked_step`: index of the clicked step (interactive mode only)
/// - `strip_clicked`: true if the strip itself was clicked (any mode)
pub(super) fn render_timeline_strip(
    ui: &mut egui::Ui,
    seq: &SequenceUIData,
    channel_names: &[String],
    interactive: bool,
    selected_step: Option<usize>,
    bpm: Option<f32>,
) -> (Option<usize>, bool) {
    let strip_height = if interactive { 28.0 } else { 20.0 };
    let available_width = ui.available_width().max(60.0);

    // Compute total duration for proportional widths (proper unit conversion)
    let total_duration: f64 = seq
        .steps
        .iter()
        .map(|s| step_duration_secs(&s.kind, bpm).max(0.5))
        .sum();
    let goto_count = seq
        .steps
        .iter()
        .filter(|s| matches!(s.kind, SequenceStepKindUI::GoTo { .. }))
        .count();
    let goto_total_width = goto_count as f32 * GOTO_BLOCK_WIDTH;
    let timed_width = (available_width - goto_total_width).max(60.0);

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available_width, strip_height),
        if interactive {
            egui::Sense::click()
        } else {
            egui::Sense::click()
        },
    );

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(12, 12, 20));

    let mut x = rect.left();
    let mut clicked_step = None;

    for (i, step) in seq.steps.iter().enumerate() {
        let block_w = match &step.kind {
            SequenceStepKindUI::GoTo { .. } => GOTO_BLOCK_WIDTH,
            _ => {
                let dur = step_duration_secs(&step.kind, bpm).max(0.5);
                let frac = dur / total_duration;
                (frac as f32 * timed_width).max(MIN_BLOCK_WIDTH)
            }
        };
        let block_rect =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(block_w, strip_height))
                .intersect(rect);

        // Block color
        let (fill, label) = match &step.kind {
            SequenceStepKindUI::Fade { from_ch, to_ch, .. } => {
                let from_color = darken(channel_color(*from_ch), 0.5);
                let to_color = darken(channel_color(*to_ch), 0.5);
                // Diagonal split: from_color top-left triangle, to_color bottom-right
                let tl = block_rect.left_top();
                let tr = block_rect.right_top();
                let bl = block_rect.left_bottom();
                let br = block_rect.right_bottom();
                painter.add(egui::Shape::convex_polygon(
                    vec![tl, tr, bl],
                    from_color,
                    egui::Stroke::NONE,
                ));
                painter.add(egui::Shape::convex_polygon(
                    vec![tr, br, bl],
                    to_color,
                    egui::Stroke::NONE,
                ));
                let from_name = channel_names
                    .get(*from_ch)
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                let to_name = channel_names.get(*to_ch).map(|s| s.as_str()).unwrap_or("?");
                let short_from = from_name.chars().take(3).collect::<String>();
                let short_to = to_name.chars().take(3).collect::<String>();
                (None, format!("{}→{}", short_from, short_to))
            }
            SequenceStepKindUI::Wait {
                duration_val,
                duration_unit,
            } => {
                let fill = egui::Color32::from_rgb(40, 40, 50);
                painter.rect_filled(block_rect, 0.0, fill);
                (
                    Some(fill),
                    format!("{:.0}{}", duration_val, duration_unit.label()),
                )
            }
            SequenceStepKindUI::GoTo { .. } => {
                let fill = egui::Color32::from_rgb(60, 50, 70);
                painter.rect_filled(block_rect, 0.0, fill);
                (Some(fill), "↺".to_string())
            }
        };

        // Selection highlight
        if interactive && selected_step == Some(i) {
            painter.rect_stroke(
                block_rect,
                0.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 80)),
                egui::StrokeKind::Outside,
            );
        }

        // Current step indicator (playback)
        let is_current = seq.playing && i == seq.current_step;
        if is_current && !interactive {
            painter.rect_stroke(
                block_rect,
                0.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 255, 100)),
                egui::StrokeKind::Outside,
            );
        }

        let _ = fill;
        let font = egui::FontId::proportional(if interactive { 10.0 } else { 9.0 });
        painter.text(
            block_rect.center(),
            egui::Align2::CENTER_CENTER,
            &label,
            font,
            egui::Color32::from_rgb(220, 220, 230),
        );
        painter.rect_stroke(
            block_rect,
            0.0,
            egui::Stroke::new(0.5, egui::Color32::from_rgb(60, 60, 80)),
            egui::StrokeKind::Outside,
        );

        // Click to select step
        if interactive && response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if block_rect.contains(pos) {
                    clicked_step = Some(i);
                }
            }
        }

        x += block_w;
    }

    // Playhead: thin vertical line at the current playback position
    if seq.playing && !seq.steps.is_empty() {
        let playhead_x = compute_playhead_x(seq, rect.left(), available_width, bpm);
        let playhead_rect = egui::Rect::from_min_size(
            egui::pos2(playhead_x, rect.top()),
            egui::vec2(2.0, strip_height),
        );
        painter.rect_filled(playhead_rect, 0.0, egui::Color32::from_rgb(255, 255, 255));
    }

    let strip_clicked = response.clicked();
    (clicked_step, strip_clicked)
}

/// Compute the x position of the playhead based on sequence progress.
fn compute_playhead_x(
    seq: &SequenceUIData,
    strip_left: f32,
    strip_width: f32,
    bpm: Option<f32>,
) -> f32 {
    let total_duration: f64 = seq
        .steps
        .iter()
        .map(|s| step_duration_secs(&s.kind, bpm).max(0.5))
        .sum();
    if total_duration <= 0.0 {
        return strip_left;
    }

    // Sum durations of completed steps + elapsed in current step
    let mut elapsed = 0.0_f64;
    for (i, step) in seq.steps.iter().enumerate() {
        if i < seq.current_step {
            elapsed += step_duration_secs(&step.kind, bpm).max(0.5);
        } else if i == seq.current_step {
            elapsed += seq
                .step_elapsed
                .min(step_duration_secs(&step.kind, bpm).max(0.5));
            break;
        }
    }

    let frac = (elapsed / total_duration).clamp(0.0, 1.0) as f32;
    strip_left + frac * strip_width
}

/// Darken a color by multiplying RGB by a factor.
fn darken(c: egui::Color32, factor: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c.r() as f32 * factor) as u8,
        (c.g() as f32 * factor) as u8,
        (c.b() as f32 * factor) as u8,
    )
}

/// Render duration value + unit selector (s | m | h | b as side-by-side buttons).
fn render_duration_editor(
    ui: &mut egui::Ui,
    seq_idx: usize,
    step_idx: usize,
    duration_val: f64,
    duration_unit: &DurationUnit,
    actions: &mut UIActions,
) {
    use super::super::SequenceAction;
    let mut dur = duration_val;
    let max_val = duration_drag_max(*duration_unit);
    let drag = egui::DragValue::new(&mut dur)
        .range(0.1..=max_val)
        .speed(0.1)
        .max_decimals(1);
    if ui.add(drag).changed() {
        actions
            .sequence_actions
            .push(SequenceAction::SetStepDuration {
                seq_idx,
                step_idx,
                value: dur,
            });
    }
    // Slider for duration (visual scrub)
    let slider = egui::Slider::new(&mut dur, 0.1..=max_val)
        .max_decimals(1)
        .show_value(false);
    if ui.add_sized([80.0, 16.0], slider).changed() {
        actions
            .sequence_actions
            .push(SequenceAction::SetStepDuration {
                seq_idx,
                step_idx,
                value: dur,
            });
    }
    // Unit selector: side-by-side buttons
    let units = [
        (DurationUnit::Seconds, "s"),
        (DurationUnit::Minutes, "m"),
        (DurationUnit::Hours, "h"),
        (DurationUnit::Beats, "b"),
    ];
    for (unit, label) in &units {
        let is_active = duration_unit == unit;
        let text = if is_active {
            egui::RichText::new(*label)
                .small()
                .strong()
                .color(egui::Color32::WHITE)
        } else {
            egui::RichText::new(*label).small().weak()
        };
        if ui.selectable_label(is_active, text).clicked() && !is_active {
            actions
                .sequence_actions
                .push(SequenceAction::SetStepDurationUnit {
                    seq_idx,
                    step_idx,
                    unit: *unit,
                });
        }
    }
}

/// Render the full inline step editor for the bottom bar.
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
        SequenceStepKindUI::Fade {
            from_ch,
            to_ch,
            duration_val,
            duration_unit,
            easing,
            transition_shader,
            target_amount,
        } => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let from_label = channel_names
                    .get(*from_ch)
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                egui::ComboBox::from_id_salt(format!("seq{}_from_{}", seq_idx, step_idx))
                    .selected_text(egui::RichText::new(from_label).small())
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for i in 0..channel_count {
                            let name = channel_names.get(i).map(|s| s.as_str()).unwrap_or("?");
                            if ui.selectable_label(i == *from_ch, name).clicked() {
                                actions
                                    .sequence_actions
                                    .push(SequenceAction::SetStepFromCh {
                                        seq_idx,
                                        step_idx,
                                        ch: i,
                                    });
                            }
                        }
                    });
                ui.label(egui::RichText::new("→").small());
                let to_label = channel_names.get(*to_ch).map(|s| s.as_str()).unwrap_or("?");
                egui::ComboBox::from_id_salt(format!("seq{}_to_{}", seq_idx, step_idx))
                    .selected_text(egui::RichText::new(to_label).small())
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for i in 0..channel_count {
                            let name = channel_names.get(i).map(|s| s.as_str()).unwrap_or("?");
                            if ui.selectable_label(i == *to_ch, name).clicked() {
                                actions.sequence_actions.push(SequenceAction::SetStepToCh {
                                    seq_idx,
                                    step_idx,
                                    ch: i,
                                });
                            }
                        }
                    });
                render_duration_editor(
                    ui,
                    seq_idx,
                    step_idx,
                    *duration_val,
                    duration_unit,
                    actions,
                );
                ui.separator();
                egui::ComboBox::from_id_salt(format!("seq{}_ease_{}", seq_idx, step_idx))
                    .selected_text(egui::RichText::new(easing.as_str()).small())
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for e in &["Linear", "EaseInOut", "EaseIn", "EaseOut"] {
                            if ui.selectable_label(*e == easing.as_str(), *e).clicked() {
                                actions
                                    .sequence_actions
                                    .push(SequenceAction::SetStepEasing {
                                        seq_idx,
                                        step_idx,
                                        easing: e.to_string(),
                                    });
                            }
                        }
                    });
                let shader_label = transition_shader.as_deref().unwrap_or("Opacity");
                egui::ComboBox::from_id_salt(format!("seq{}_shader_{}", seq_idx, step_idx))
                    .selected_text(egui::RichText::new(shader_label).small())
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        let is_opacity = transition_shader.is_none();
                        if ui.selectable_label(is_opacity, "Opacity").clicked() {
                            actions.sequence_actions.push(
                                SequenceAction::SetStepTransitionShader {
                                    seq_idx,
                                    step_idx,
                                    shader: None,
                                },
                            );
                        }
                        for name in &data.transition_names {
                            let selected = transition_shader.as_ref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() {
                                actions.sequence_actions.push(
                                    SequenceAction::SetStepTransitionShader {
                                        seq_idx,
                                        step_idx,
                                        shader: Some(name.clone()),
                                    },
                                );
                            }
                        }
                    });
                ui.separator();
                // Target amount slider (0–100%)
                ui.label(egui::RichText::new("Target:").small());
                let mut amt = *target_amount;
                let slider = egui::Slider::new(&mut amt, 0.0..=1.0)
                    .max_decimals(2)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
                if ui.add_sized([70.0, 16.0], slider).changed() {
                    actions
                        .sequence_actions
                        .push(SequenceAction::SetStepTargetAmount {
                            seq_idx,
                            step_idx,
                            amount: amt,
                        });
                }
            });
        }
        SequenceStepKindUI::Wait {
            duration_val,
            duration_unit,
        } => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(egui::RichText::new("Wait").small().strong());
                ui.label(egui::RichText::new("Duration:").small());
                render_duration_editor(
                    ui,
                    seq_idx,
                    step_idx,
                    *duration_val,
                    duration_unit,
                    actions,
                );
            });
        }
        SequenceStepKindUI::GoTo { step_index } => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(egui::RichText::new("GoTo").small().strong());
                ui.label(egui::RichText::new("Step:").small());
                let mut target = *step_index as i32;
                let max = seq.steps.len().saturating_sub(1) as i32;
                if ui
                    .add(egui::DragValue::new(&mut target).range(0..=max).speed(0.1))
                    .changed()
                {
                    actions
                        .sequence_actions
                        .push(SequenceAction::SetGoToTarget {
                            seq_idx,
                            step_idx,
                            target: target.max(0) as usize,
                        });
                }
            });
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

    fn fixture_with_sequence() -> UIData {
        use super::super::super::{SequenceStepKindUI, SequenceStepUI, SequenceUIData};
        use crate::channel::DurationUnit;
        let mut data = UIData::test_fixture();
        data.sequences.push(SequenceUIData {
            name: "Test Seq".to_string(),
            enabled: true,
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
            steps: vec![
                SequenceStepUI {
                    label: "Fade".into(),
                    kind: SequenceStepKindUI::Fade {
                        from_ch: 0,
                        to_ch: 1,
                        duration_val: 5.0,
                        duration_unit: DurationUnit::Seconds,
                        easing: "Linear".into(),
                        transition_shader: None,
                        target_amount: 1.0,
                    },
                },
                SequenceStepUI {
                    label: "Wait".into(),
                    kind: SequenceStepKindUI::Wait {
                        duration_val: 2.0,
                        duration_unit: DurationUnit::Seconds,
                    },
                },
                SequenceStepUI {
                    label: "GoTo".into(),
                    kind: SequenceStepKindUI::GoTo { step_index: 0 },
                },
            ],
        });
        data
    }

    #[test]
    fn render_sequence_builder_with_steps() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_builder(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_timeline_strip_smoke() {
        let data = fixture_with_sequence();
        let seq = &data.sequences[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_timeline_strip(ui, seq, &data.channel_names, false, None, None);
        });
    }

    #[test]
    fn render_timeline_strip_interactive() {
        let data = fixture_with_sequence();
        let seq = &data.sequences[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_timeline_strip(ui, seq, &data.channel_names, true, Some(1), Some(120.0));
        });
    }

    #[test]
    fn render_timeline_strip_playing() {
        let mut data = fixture_with_sequence();
        data.sequences[0].playing = true;
        data.sequences[0].step_elapsed = 2.5;
        let seq = &data.sequences[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_timeline_strip(ui, seq, &data.channel_names, false, None, None);
        });
    }

    #[test]
    fn render_step_editor_fade() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let step = &data.sequences[0].steps[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_step_editor(ui, 0, 0, step, &data, &mut actions);
        });
    }

    #[test]
    fn render_step_editor_wait() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let step = &data.sequences[0].steps[1];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_step_editor(ui, 0, 1, step, &data, &mut actions);
        });
    }

    #[test]
    fn render_step_editor_goto() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let step = &data.sequences[0].steps[2];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_step_editor(ui, 0, 2, step, &data, &mut actions);
        });
    }
}
