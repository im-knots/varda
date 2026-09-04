//! Transition sequence builder: compact read-only timeline strip for mixer area,
//! full interactive editor for the bottom bar.

use super::super::{ChannelUIInfo, SequenceStepKindUI, SequenceUIData, UIActions, UIData};
use super::utils::{channel_color, resolve_channel};
use crate::channel::DurationUnit;

/// Max drag-value range per duration unit.
fn duration_drag_max(unit: DurationUnit) -> f64 {
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
                    let bpm_val = f64::from(bpm.unwrap_or(120.0));
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
    use crate::engine::EngineCommand;

    for (seq_idx, seq) in data.sequences.iter().enumerate() {
        ui.push_id(format!("seq_{seq_idx}"), |ui| {
            let is_selected = data.selected_sequence == Some(seq_idx);
            let border_color = if seq.playing {
                egui::Color32::from_rgb(80, 200, 80)
            } else if is_selected {
                egui::Color32::from_rgb(200, 200, 255)
            } else {
                egui::Color32::from_rgb(50, 50, 70)
            };
            let border_width = if is_selected || seq.playing {
                1.5_f32
            } else {
                1.0_f32
            };
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
                            actions.session.select_sequence = Some(seq_idx);
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
                            actions.commands.push(EngineCommand::ToggleSequence {
                                sequence_uuid: seq.uuid.clone(),
                            });
                        }

                        if seq.playing {
                            if ui
                                .small_button("Stop")
                                .on_hover_text("Stop playback")
                                .clicked()
                            {
                                actions.commands.push(EngineCommand::StopSequence {
                                    sequence_uuid: seq.uuid.clone(),
                                });
                            }
                        } else if seq.enabled
                            && !seq.steps.is_empty()
                            && ui
                                .small_button("Play")
                                .on_hover_text("Start playback")
                                .clicked()
                        {
                            actions.commands.push(EngineCommand::PlaySequence {
                                sequence_uuid: seq.uuid.clone(),
                            });
                        }

                        if ui
                            .small_button("x")
                            .on_hover_text("Delete sequence")
                            .clicked()
                        {
                            actions.commands.push(EngineCommand::DeleteSequence {
                                sequence_uuid: seq.uuid.clone(),
                            });
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
                            actions.session.select_sequence = Some(seq_idx);
                        }
                    } else {
                        let (_step, strip_clicked) = render_timeline_strip(
                            ui,
                            seq,
                            &data.channels,
                            false,
                            None,
                            data.clock_bpm,
                        );
                        if strip_clicked {
                            actions.session.select_sequence = Some(seq_idx);
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

/// Constant width for `GoTo` blocks in the timeline strip.
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
fn render_timeline_strip(
    ui: &mut egui::Ui,
    seq: &SequenceUIData,
    channels: &[ChannelUIInfo],
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
        egui::Sense::click(),
    );

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(12, 12, 20));

    let mut x = rect.left();
    let mut clicked_step = None;

    for (i, step) in seq.steps.iter().enumerate() {
        let block_w = if let SequenceStepKindUI::GoTo { .. } = &step.kind {
            GOTO_BLOCK_WIDTH
        } else {
            let dur = step_duration_secs(&step.kind, bpm).max(0.5);
            let frac = dur / total_duration;
            (frac as f32 * timed_width).max(MIN_BLOCK_WIDTH)
        };
        let block_rect =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(block_w, strip_height))
                .intersect(rect);

        // Block color
        let (fill, label) = match &step.kind {
            SequenceStepKindUI::Fade { from_ch, to_ch, .. } => {
                let from = resolve_channel(channels, from_ch);
                let to = resolve_channel(channels, to_ch);
                let from_color = darken(channel_color(from.as_ref().map_or(0, |(i, _)| *i)), 0.5);
                let to_color = darken(channel_color(to.as_ref().map_or(0, |(i, _)| *i)), 0.5);
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
                let short_from = from.map_or_else(
                    || "?".to_string(),
                    |(_, name)| name.chars().take(3).collect::<String>(),
                );
                let short_to = to.map_or_else(
                    || "?".to_string(),
                    |(_, name)| name.chars().take(3).collect::<String>(),
                );
                (None, format!("{short_from}→{short_to}"))
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
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 200, 80)),
                egui::StrokeKind::Outside,
            );
        }

        // Current step indicator (playback)
        let is_current = seq.playing && i == seq.current_step;
        if is_current && !interactive {
            painter.rect_stroke(
                block_rect,
                0.0,
                egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(100, 255, 100)),
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
            egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(60, 60, 80)),
            egui::StrokeKind::Outside,
        );

        // Click to select step
        if interactive
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && block_rect.contains(pos)
        {
            clicked_step = Some(i);
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
        (f32::from(c.r()) * factor) as u8,
        (f32::from(c.g()) * factor) as u8,
        (f32::from(c.b()) * factor) as u8,
    )
}

/// Render duration value + unit selector (s | m | h | b as side-by-side buttons).
fn render_duration_editor(
    ui: &mut egui::Ui,
    sequence_uuid: &str,
    step_idx: usize,
    duration_val: f64,
    duration_unit: DurationUnit,
    actions: &mut UIActions,
) {
    use crate::engine::EngineCommand;
    let mut dur = duration_val;
    let max_val = duration_drag_max(duration_unit);
    let drag = egui::DragValue::new(&mut dur)
        .range(0.1..=max_val)
        .speed(0.1)
        .max_decimals(1);
    if ui.add(drag).changed() {
        actions.commands.push(EngineCommand::SetStepDurationValue {
            sequence_uuid: sequence_uuid.to_string(),
            step_idx,
            value: dur,
        });
    }
    // Slider for duration (visual scrub)
    let slider = egui::Slider::new(&mut dur, 0.1..=max_val)
        .max_decimals(1)
        .show_value(false);
    if ui.add_sized([80.0, 16.0], slider).changed() {
        actions.commands.push(EngineCommand::SetStepDurationValue {
            sequence_uuid: sequence_uuid.to_string(),
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
        let is_active = duration_unit == *unit;
        let text = if is_active {
            egui::RichText::new(*label)
                .small()
                .strong()
                .color(egui::Color32::WHITE)
        } else {
            egui::RichText::new(*label).small().weak()
        };
        if ui.selectable_label(is_active, text).clicked() && !is_active {
            actions.commands.push(EngineCommand::SetStepDurationUnit {
                sequence_uuid: sequence_uuid.to_string(),
                step_idx,
                unit: *unit,
            });
        }
    }
}

/// Render the full inline step editor for the bottom bar.
fn render_sequence_step_editor(
    ui: &mut egui::Ui,
    seq: &SequenceUIData,
    step_idx: usize,
    step: &super::super::SequenceStepUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    use crate::engine::EngineCommand;
    let seq_uuid = seq.uuid.as_str();

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
                let from_label = resolve_channel(&data.channels, from_ch)
                    .map_or_else(|| "?".to_string(), |(_, name)| name);
                egui::ComboBox::from_id_salt(format!("seq{seq_uuid}_from_{step_idx}"))
                    .selected_text(egui::RichText::new(from_label).small())
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for ch in &data.channels {
                            if ui.selectable_label(ch.uuid == *from_ch, &ch.name).clicked() {
                                actions.commands.push(EngineCommand::SetStepFromCh {
                                    sequence_uuid: seq_uuid.to_string(),
                                    step_idx,
                                    channel_uuid: ch.uuid.clone(),
                                });
                            }
                        }
                    });
                ui.label(egui::RichText::new("→").small());
                let to_label = resolve_channel(&data.channels, to_ch)
                    .map_or_else(|| "?".to_string(), |(_, name)| name);
                egui::ComboBox::from_id_salt(format!("seq{seq_uuid}_to_{step_idx}"))
                    .selected_text(egui::RichText::new(to_label).small())
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for ch in &data.channels {
                            if ui.selectable_label(ch.uuid == *to_ch, &ch.name).clicked() {
                                actions.commands.push(EngineCommand::SetStepToCh {
                                    sequence_uuid: seq_uuid.to_string(),
                                    step_idx,
                                    channel_uuid: ch.uuid.clone(),
                                });
                            }
                        }
                    });
                render_duration_editor(
                    ui,
                    seq_uuid,
                    step_idx,
                    *duration_val,
                    *duration_unit,
                    actions,
                );
                ui.separator();
                egui::ComboBox::from_id_salt(format!("seq{seq_uuid}_ease_{step_idx}"))
                    .selected_text(egui::RichText::new(easing.as_str()).small())
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for e in &["Linear", "EaseInOut", "EaseIn", "EaseOut"] {
                            if ui.selectable_label(*e == easing.as_str(), *e).clicked() {
                                actions.commands.push(EngineCommand::SetStepEasing {
                                    sequence_uuid: seq_uuid.to_string(),
                                    step_idx,
                                    easing: e.to_string(),
                                });
                            }
                        }
                    });
                let shader_label = transition_shader.as_deref().unwrap_or("Opacity");
                egui::ComboBox::from_id_salt(format!("seq{seq_uuid}_shader_{step_idx}"))
                    .selected_text(egui::RichText::new(shader_label).small())
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        let is_opacity = transition_shader.is_none();
                        if ui.selectable_label(is_opacity, "Opacity").clicked() {
                            actions
                                .commands
                                .push(EngineCommand::SetStepTransitionShader {
                                    sequence_uuid: seq_uuid.to_string(),
                                    step_idx,
                                    shader_name: None,
                                });
                        }
                        for name in &data.transition_names {
                            let selected = transition_shader.as_ref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() {
                                actions
                                    .commands
                                    .push(EngineCommand::SetStepTransitionShader {
                                        sequence_uuid: seq_uuid.to_string(),
                                        step_idx,
                                        shader_name: Some(name.clone()),
                                    });
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
                    actions.commands.push(EngineCommand::SetStepTargetAmount {
                        sequence_uuid: seq_uuid.to_string(),
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
                    seq_uuid,
                    step_idx,
                    *duration_val,
                    *duration_unit,
                    actions,
                );
            });
        }
        SequenceStepKindUI::GoTo { step_index } => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(egui::RichText::new("GoTo").small().strong());
                ui.label(egui::RichText::new("Step:").small());
                let mut target = i32::try_from(*step_index).unwrap_or(i32::MAX);
                let max = i32::try_from(seq.steps.len().saturating_sub(1)).unwrap_or(i32::MAX);
                if ui
                    .add(egui::DragValue::new(&mut target).range(0..=max).speed(0.1))
                    .changed()
                {
                    actions.commands.push(EngineCommand::SetGoToTarget {
                        sequence_uuid: seq_uuid.to_string(),
                        step_idx,
                        target: target.max(0) as usize,
                    });
                }
            });
        }
    }
}

/// Bottom bar: full sequence editor when a sequence is selected.
pub(super) fn render_sequence_detail(
    ui: &mut egui::Ui,
    seq_idx: usize,
    data: &UIData,
    actions: &mut UIActions,
) {
    use super::super::SequenceStepDrag;
    use crate::engine::EngineCommand;

    let Some(seq) = data.sequences.get(seq_idx) else {
        ui.label(egui::RichText::new("Sequence not found").weak());
        return;
    };

    // Header: name, enable, play/stop, delete
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            egui::RichText::new(format!("🎬 {}", seq.name))
                .strong()
                .size(14.0),
        );

        let (en_label, en_color) = if seq.enabled {
            ("On", egui::Color32::from_rgb(80, 200, 80))
        } else {
            ("Off", egui::Color32::from_rgb(120, 120, 120))
        };
        if ui
            .button(egui::RichText::new(en_label).color(en_color))
            .on_hover_text("Toggle enabled")
            .clicked()
        {
            actions.commands.push(EngineCommand::ToggleSequence {
                sequence_uuid: seq.uuid.clone(),
            });
        }

        if seq.playing {
            if ui.button("⏹ Stop").on_hover_text("Stop playback").clicked() {
                actions.commands.push(EngineCommand::StopSequence {
                    sequence_uuid: seq.uuid.clone(),
                });
            }
        } else if seq.enabled
            && !seq.steps.is_empty()
            && ui
                .button("▶ Play")
                .on_hover_text("Start playback")
                .clicked()
        {
            actions.commands.push(EngineCommand::PlaySequence {
                sequence_uuid: seq.uuid.clone(),
            });
        }

        if ui
            .button("🗑 Delete")
            .on_hover_text("Delete sequence")
            .clicked()
        {
            actions.commands.push(EngineCommand::DeleteSequence {
                sequence_uuid: seq.uuid.clone(),
            });
        }
    });

    ui.add_space(4.0);

    // Interactive timeline strip (larger, clickable)
    let selected_step_idx = data
        .selected_sequence_step
        .filter(|(si, _)| *si == seq_idx)
        .map(|(_, step)| step);

    if seq.steps.is_empty() {
        ui.label(egui::RichText::new("No steps yet — add steps below").weak());
    } else {
        let (clicked_step, _) = render_timeline_strip(
            ui,
            seq,
            &data.channels,
            true,
            selected_step_idx,
            data.clock_bpm,
        );
        if let Some(clicked) = clicked_step {
            actions.session.select_sequence_step = Some((seq_idx, clicked));
        }
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);

    // Two-column layout: step list (left) | step editor (right)
    let target_id = egui::Id::new("__seq_step_dnd_target");
    ui.horizontal_top(|ui| {
        // ── Left column: stacked step list + add buttons ──
        let list_width = 280.0;
        ui.vertical(|ui| {
            ui.set_width(list_width);

            // Scrollable step list with visual-gap drag-and-drop
            egui::ScrollArea::vertical()
                .id_salt("seq_step_list")
                .max_height(ui.available_height() - 30.0)
                .show(ui, |ui| {
                    let src_id = egui::Id::new("__seq_step_dnd_src");
                    let is_dragging =
                        egui::DragAndDrop::has_payload_of_type::<SequenceStepDrag>(ui.ctx());
                    let drag_src: Option<SequenceStepDrag> = if is_dragging {
                        ui.ctx().memory(|mem| mem.data.get_temp(src_id))
                    } else {
                        None
                    };
                    let dragged_idx = drag_src.map(|d| d.step_idx);

                    // Compute drop target from pointer position BEFORE rendering,
                    // using fixed row heights to avoid oscillation from gap insertion.
                    let row_height = 22.0;
                    let gap_height = row_height;
                    let step_count = seq.steps.len();
                    let list_top = ui.cursor().top();

                    let drop_target: Option<usize> = match (is_dragging, dragged_idx) {
                        (true, Some(src)) => {
                            if let Some(pos) = ui.ctx().input(|inp| inp.pointer.hover_pos()) {
                                // Compute pointer offset from list top, in terms of
                                // the *logical* list (source item removed).
                                let rel_y = pos.y - list_top;
                                if rel_y >= 0.0 {
                                    // Visible items = all except the dragged one
                                    let visible_count = step_count - 1;
                                    // Which slot the pointer is over (0-based)
                                    let slot = ((rel_y / row_height) as usize).min(visible_count);
                                    // Map slot back to original index, re-inserting the gap for the source
                                    let target = if slot < src { slot } else { slot + 1 };
                                    Some(target.min(step_count))
                                } else {
                                    Some(0)
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    // Store the computed target in memory for the deferred handler
                    if let Some(t) = drop_target {
                        ui.ctx().memory_mut(|mem| {
                            mem.data.insert_temp::<usize>(target_id, t);
                        });
                    }

                    for (i, step) in seq.steps.iter().enumerate() {
                        // Hide the step being dragged from its original position
                        if dragged_idx == Some(i) {
                            continue;
                        }

                        // Insert gap BEFORE this item if it's the drop target
                        if drop_target == Some(i) {
                            let (gap_rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), gap_height),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                gap_rect,
                                2.0,
                                egui::Color32::from_rgba_premultiplied(255, 200, 80, 30),
                            );
                            ui.painter().rect_stroke(
                                gap_rect,
                                2.0,
                                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(255, 200, 80)),
                                egui::StrokeKind::Outside,
                            );
                        }

                        let is_selected = selected_step_idx == Some(i);
                        let is_current = seq.playing && i == seq.current_step;

                        let (icon, summary) = match &step.kind {
                            SequenceStepKindUI::Fade {
                                from_ch,
                                to_ch,
                                duration_val,
                                duration_unit,
                                ..
                            } => {
                                let from_name = resolve_channel(&data.channels, from_ch)
                                    .map_or_else(|| "?".to_string(), |(_, name)| name);
                                let to_name = resolve_channel(&data.channels, to_ch)
                                    .map_or_else(|| "?".to_string(), |(_, name)| name);
                                (
                                    "🔀",
                                    format!(
                                        "{} → {}  {:.1}{}",
                                        from_name,
                                        to_name,
                                        duration_val,
                                        duration_unit.label()
                                    ),
                                )
                            }
                            SequenceStepKindUI::Wait {
                                duration_val,
                                duration_unit,
                            } => ("⏸", format!("{:.1}{}", duration_val, duration_unit.label())),
                            SequenceStepKindUI::GoTo { step_index } => {
                                ("↺", format!("→ Step {}", step_index + 1))
                            }
                        };

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;

                            // Drag handle (grip dots)
                            let handle_size = egui::vec2(12.0, 16.0);
                            let (handle_rect, handle_resp) =
                                ui.allocate_exact_size(handle_size, egui::Sense::drag());
                            let grip_color = if handle_resp.dragged() || handle_resp.hovered() {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            let cx = handle_rect.center().x;
                            let cy = handle_rect.center().y;
                            for row in -1..=1 {
                                for col in [-1.0_f32, 1.0] {
                                    ui.painter().circle_filled(
                                        egui::pos2(cx + col * 3.0, cy + row as f32 * 4.0),
                                        1.5,
                                        grip_color,
                                    );
                                }
                            }
                            if handle_resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            if handle_resp.dragged() {
                                let drag = SequenceStepDrag {
                                    sequence_uuid: seq.uuid.clone(),
                                    step_idx: i,
                                };
                                egui::DragAndDrop::set_payload(ui.ctx(), drag.clone());
                                ui.ctx().memory_mut(|mem| {
                                    mem.data
                                        .insert_temp(egui::Id::new("__seq_step_dnd_src"), drag);
                                });
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            }

                            // Clickable label
                            let label_text =
                                format!("{} {}. {} {}", icon, i + 1, step.label, summary);
                            let text = if is_current {
                                egui::RichText::new(&label_text)
                                    .color(egui::Color32::from_rgb(80, 200, 80))
                            } else if is_selected {
                                egui::RichText::new(&label_text).strong()
                            } else {
                                egui::RichText::new(&label_text)
                            };

                            if ui.selectable_label(is_selected, text).clicked() {
                                actions.session.select_sequence_step = Some((seq_idx, i));
                            }
                        });
                    }

                    // Gap at the end of the list (drop after last item)
                    if drop_target == Some(step_count) {
                        let (gap_rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), gap_height),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(
                            gap_rect,
                            2.0,
                            egui::Color32::from_rgba_premultiplied(255, 200, 80, 30),
                        );
                        ui.painter().rect_stroke(
                            gap_rect,
                            2.0,
                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(255, 200, 80)),
                            egui::StrokeKind::Outside,
                        );
                    }
                });

            // Add step buttons at bottom of list
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // A fade needs two channels to name; with one channel it fades
                // onto itself, which is what the previous index clamp did too.
                let from_uuid = data.channels.first().map(|c| c.uuid.clone());
                let to_uuid = data
                    .channels
                    .get(1)
                    .or_else(|| data.channels.first())
                    .map(|c| c.uuid.clone());
                if let (Some(from_ch), Some(to_ch)) = (from_uuid, to_uuid)
                    && ui.small_button("+Fade").clicked()
                {
                    actions.commands.push(EngineCommand::AddFadeStep {
                        sequence_uuid: seq.uuid.clone(),
                        from_channel_uuid: from_ch,
                        to_channel_uuid: to_ch,
                    });
                }
                if ui.small_button("+Wait").clicked() {
                    actions.commands.push(EngineCommand::AddWaitStep {
                        sequence_uuid: seq.uuid.clone(),
                    });
                }
                if ui.small_button("+Loop").clicked() {
                    actions.commands.push(EngineCommand::AddGoToStep {
                        sequence_uuid: seq.uuid.clone(),
                        step_index: 0,
                    });
                }
            });
        });

        ui.separator();

        // ── Right column: selected step editor ──
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            if let Some(step_idx) = selected_step_idx {
                if let Some(step) = seq.steps.get(step_idx) {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("Step {} — {}", step_idx + 1, step.label))
                                .strong(),
                        );
                        if ui
                            .small_button("🗑 Remove")
                            .on_hover_text("Remove this step")
                            .clicked()
                        {
                            actions.commands.push(EngineCommand::RemoveStep {
                                sequence_uuid: seq.uuid.clone(),
                                step_idx,
                            });
                        }
                    });
                    ui.add_space(4.0);
                    render_sequence_step_editor(ui, seq, step_idx, step, data, actions);
                } else {
                    ui.label(egui::RichText::new("Step not found").weak());
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("← Select a step to edit").weak());
                });
            }
        });
    });

    // Animate playhead
    if seq.playing {
        ui.ctx().request_repaint();
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
            uuid: "seq00001".to_string(),
            name: "Test Seq".to_string(),
            enabled: true,
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
            steps: vec![
                SequenceStepUI {
                    label: "Fade".into(),
                    kind: SequenceStepKindUI::Fade {
                        from_ch: "ca000001".to_string(),
                        to_ch: "cb000001".to_string(),
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
            render_timeline_strip(ui, seq, &data.channels, false, None, None);
        });
    }

    #[test]
    fn render_timeline_strip_interactive() {
        let data = fixture_with_sequence();
        let seq = &data.sequences[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_timeline_strip(ui, seq, &data.channels, true, Some(1), Some(120.0));
        });
    }

    #[test]
    fn render_timeline_strip_playing() {
        let mut data = fixture_with_sequence();
        data.sequences[0].playing = true;
        data.sequences[0].step_elapsed = 2.5;
        let seq = &data.sequences[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_timeline_strip(ui, seq, &data.channels, false, None, None);
        });
    }

    #[test]
    fn render_step_editor_fade() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let seq = &data.sequences[0];
        let step = &seq.steps[0];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_step_editor(ui, seq, 0, step, &data, &mut actions);
        });
    }

    #[test]
    fn render_step_editor_wait() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let seq = &data.sequences[0];
        let step = &seq.steps[1];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_step_editor(ui, seq, 1, step, &data, &mut actions);
        });
    }

    #[test]
    fn render_step_editor_goto() {
        let data = fixture_with_sequence();
        let mut actions = UIActions::new();
        let seq = &data.sequences[0];
        let step = &seq.steps[2];
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_sequence_step_editor(ui, seq, 2, step, &data, &mut actions);
        });
    }
}
