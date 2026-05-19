//! Transition sequence state mutations.

use crate::engine::{CommandResult, ErrorCode};
use super::super::VardaApp;

impl VardaApp {
    pub(crate) fn cmd_create_sequence(&mut self) -> CommandResult {
        let n = self.mixer.transition_sequences().len() + 1;
        self.mixer.transition_sequences_mut().push(
            crate::mixer::TransitionSequence::new(format!("Sequence {}", n))
        );
        CommandResult::Ok
    }

    pub(crate) fn cmd_delete_sequence(&mut self, idx: usize) -> CommandResult {
        if idx < self.mixer.transition_sequences().len() {
            self.mixer.transition_sequences_mut().remove(idx);
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_play_sequence(&mut self, idx: usize) -> CommandResult {
        self.mixer.start_sequence(idx);
        CommandResult::Ok
    }

    pub(crate) fn cmd_stop_sequence(&mut self, idx: usize) -> CommandResult {
        self.mixer.stop_sequence(idx);
        CommandResult::Ok
    }

    pub(crate) fn cmd_toggle_sequence(&mut self, idx: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(idx) {
            seq.enabled = !seq.enabled;
            if !seq.enabled { seq.state.reset(); }
        }
        CommandResult::Ok
    }

    pub(crate) fn cmd_add_fade_step(&mut self, seq_idx: usize, from_ch: usize, to_ch: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            seq.steps.push(crate::mixer::TransitionStep { kind: crate::mixer::StepKind::Fade {
                from_ch, to_ch,
                duration: crate::channel::DurationSpec::Seconds(2.0),
                easing: crate::mixer::CrossfadeEasing::EaseInOut,
                transition_shader: None,
                target_amount: 1.0,
            }});
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_add_wait_step(&mut self, seq_idx: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            seq.steps.push(crate::mixer::TransitionStep { kind: crate::mixer::StepKind::Wait {
                duration: crate::channel::DurationSpec::Seconds(2.0),
            }});
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_add_goto_step(&mut self, seq_idx: usize, step_index: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            seq.steps.push(crate::mixer::TransitionStep { kind: crate::mixer::StepKind::GoTo { step_index } });
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_remove_step(&mut self, seq_idx: usize, step_idx: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if step_idx < seq.steps.len() {
                seq.steps.remove(step_idx);
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_duration(&mut self, seq_idx: usize, step_idx: usize, value: f64, unit: crate::channel::DurationUnit) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                match &mut step.kind {
                    crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                        *duration = crate::channel::DurationSpec::from_value_unit(value, unit);
                    }
                    _ => {}
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_easing(&mut self, seq_idx: usize, step_idx: usize, easing: String) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                if let crate::mixer::StepKind::Fade { easing: e, .. } = &mut step.kind {
                    *e = match easing.as_str() {
                        "Linear" => crate::mixer::CrossfadeEasing::Linear,
                        "EaseIn" => crate::mixer::CrossfadeEasing::EaseIn,
                        "EaseOut" => crate::mixer::CrossfadeEasing::EaseOut,
                        _ => crate::mixer::CrossfadeEasing::EaseInOut,
                    };
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_transition_shader(&mut self, seq_idx: usize, step_idx: usize, shader_name: Option<String>) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                if let crate::mixer::StepKind::Fade { transition_shader, .. } = &mut step.kind {
                    *transition_shader = shader_name;
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_move_step(&mut self, seq_idx: usize, from: usize, to: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if from < seq.steps.len() && to < seq.steps.len() && from != to {
                let step = seq.steps.remove(from);
                seq.steps.insert(to, step);
            }
            CommandResult::Ok
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_duration_unit(&mut self, seq_idx: usize, step_idx: usize, unit: crate::channel::DurationUnit) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                match &mut step.kind {
                    crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                        *duration = crate::channel::DurationSpec::from_value_unit(duration.value(), unit);
                    }
                    _ => {}
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_toggle_step_duration_unit(&mut self, seq_idx: usize, step_idx: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                match &mut step.kind {
                    crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                        let next_unit = duration.unit().next();
                        *duration = crate::channel::DurationSpec::from_value_unit(duration.value(), next_unit);
                    }
                    _ => {}
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_duration_value(&mut self, seq_idx: usize, step_idx: usize, value: f64) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                match &mut step.kind {
                    crate::mixer::StepKind::Fade { duration, .. } | crate::mixer::StepKind::Wait { duration } => {
                        duration.set_value(value);
                    }
                    _ => {}
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_from_ch(&mut self, seq_idx: usize, step_idx: usize, ch: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                if let crate::mixer::StepKind::Fade { from_ch, .. } = &mut step.kind { *from_ch = ch; }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_to_ch(&mut self, seq_idx: usize, step_idx: usize, ch: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                if let crate::mixer::StepKind::Fade { to_ch, .. } = &mut step.kind { *to_ch = ch; }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_goto_target(&mut self, seq_idx: usize, step_idx: usize, target: usize) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                if let crate::mixer::StepKind::GoTo { step_index } = &mut step.kind { *step_index = target; }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }

    pub(crate) fn cmd_set_step_target_amount(&mut self, seq_idx: usize, step_idx: usize, amount: f32) -> CommandResult {
        if let Some(seq) = self.mixer.transition_sequences_mut().get_mut(seq_idx) {
            if let Some(step) = seq.steps.get_mut(step_idx) {
                if let crate::mixer::StepKind::Fade { target_amount, .. } = &mut step.kind {
                    *target_amount = amount.clamp(0.0, 1.0);
                }
                CommandResult::Ok
            } else {
                CommandResult::Err { code: ErrorCode::NotFound, message: "Step not found".into() }
            }
        } else {
            CommandResult::Err { code: ErrorCode::NotFound, message: "Sequence not found".into() }
        }
    }
}