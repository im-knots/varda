//! Transition sequence state mutations.
//!
//! Sequences are addressed by UUID; steps are positional within their sequence,
//! so `step_idx` is an ordinal rather than an address. See
//! [`/spec/api-addressing.md`].

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};
use crate::mixer::{CrossfadeEasing, StepKind, TransitionSequence, TransitionStep};

use crate::channel::{DurationSpec, DurationUnit};

impl VardaApp {
    /// Resolve `sequence_uuid` and hand the sequence to `f`.
    fn with_sequence(
        &mut self,
        sequence_uuid: &str,
        f: impl FnOnce(&mut TransitionSequence),
    ) -> CommandResult {
        let idx = match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => idx,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                };
            }
        };
        f(&mut self.mixer.transition_sequences_mut()[idx]);
        CommandResult::Ok
    }

    /// Resolve `sequence_uuid` plus the step at `step_idx` and hand the step to
    /// `f`. An out-of-range ordinal is `NotFound`, same as an unknown sequence.
    fn with_step(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        f: impl FnOnce(&mut TransitionStep),
    ) -> CommandResult {
        let idx = match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => idx,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                };
            }
        };
        match self.mixer.transition_sequences_mut()[idx]
            .steps
            .get_mut(step_idx)
        {
            Some(step) => {
                f(step);
                CommandResult::Ok
            }
            None => CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Sequence '{sequence_uuid}' has no step {step_idx}"),
            },
        }
    }

    /// Mutate the duration of a Fade or Wait step. `GoTo` steps have no duration.
    fn with_step_duration(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        f: impl FnOnce(&mut DurationSpec),
    ) -> CommandResult {
        self.with_step(sequence_uuid, step_idx, |step| match &mut step.kind {
            StepKind::Fade { duration, .. } | StepKind::Wait { duration } => f(duration),
            StepKind::GoTo { .. } => {}
        })
    }

    pub(crate) fn cmd_create_sequence(&mut self) -> CommandResult {
        let n = self.mixer.transition_sequences().len() + 1;
        let seq = TransitionSequence::new(format!("Sequence {n}"));
        let uuid = seq.uuid.clone();
        self.mixer.transition_sequences_mut().push(seq);
        CommandResult::OkWithId { uuid }
    }

    pub(crate) fn cmd_delete_sequence(&mut self, sequence_uuid: &str) -> CommandResult {
        match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => {
                self.mixer.transition_sequences_mut().remove(idx);
                CommandResult::Ok
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::NotFound,
                message: e.to_string(),
            },
        }
    }

    pub(crate) fn cmd_play_sequence(&mut self, sequence_uuid: &str) -> CommandResult {
        // Rejected with a reason rather than silently ignored: a Fade step
        // targets a pair of channels, so a free-running sequence and the
        // arrangement's crossfader automation would overwrite each other every
        // frame. The arrangement may still fire a sequence as a one-shot cue.
        // See /spec/transport.md § Relationship to Performance Mode Sequencers.
        if self.arrangement_authority().is_engaged() {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "the arrangement holds authority; stop the transport to \
                          run a transition sequence freely"
                    .to_string(),
            };
        }
        match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => {
                self.mixer.start_sequence(idx);
                CommandResult::Ok
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::NotFound,
                message: e.to_string(),
            },
        }
    }

    pub(crate) fn cmd_stop_sequence(&mut self, sequence_uuid: &str) -> CommandResult {
        match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => {
                self.mixer.stop_sequence(idx);
                CommandResult::Ok
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::NotFound,
                message: e.to_string(),
            },
        }
    }

    pub(crate) fn cmd_toggle_sequence(&mut self, sequence_uuid: &str) -> CommandResult {
        self.with_sequence(sequence_uuid, |seq| {
            seq.enabled = !seq.enabled;
            if !seq.enabled {
                seq.state.reset();
            }
        })
    }

    pub(crate) fn cmd_add_fade_step(
        &mut self,
        sequence_uuid: &str,
        from_channel_uuid: &str,
        to_channel_uuid: &str,
    ) -> CommandResult {
        // Validate both channels up front so a step can never be created
        // pointing at a channel that does not exist.
        for uuid in [from_channel_uuid, to_channel_uuid] {
            if let Err(e) = self.resolve_channel(uuid) {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                };
            }
        }
        let from_ch = from_channel_uuid.to_string();
        let to_ch = to_channel_uuid.to_string();
        self.with_sequence(sequence_uuid, |seq| {
            seq.steps.push(TransitionStep {
                kind: StepKind::Fade {
                    from_ch,
                    to_ch,
                    duration: DurationSpec::Seconds(2.0),
                    easing: CrossfadeEasing::EaseInOut,
                    transition_shader: None,
                    target_amount: 1.0,
                },
            });
        })
    }

    pub(crate) fn cmd_add_wait_step(&mut self, sequence_uuid: &str) -> CommandResult {
        self.with_sequence(sequence_uuid, |seq| {
            seq.steps.push(TransitionStep {
                kind: StepKind::Wait {
                    duration: DurationSpec::Seconds(2.0),
                },
            });
        })
    }

    pub(crate) fn cmd_add_goto_step(
        &mut self,
        sequence_uuid: &str,
        step_index: usize,
    ) -> CommandResult {
        self.with_sequence(sequence_uuid, |seq| {
            seq.steps.push(TransitionStep {
                kind: StepKind::GoTo { step_index },
            });
        })
    }

    pub(crate) fn cmd_remove_step(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
    ) -> CommandResult {
        let idx = match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => idx,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                };
            }
        };
        let steps = &mut self.mixer.transition_sequences_mut()[idx].steps;
        if step_idx >= steps.len() {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Sequence '{sequence_uuid}' has no step {step_idx}"),
            };
        }
        steps.remove(step_idx);
        CommandResult::Ok
    }

    pub(crate) fn cmd_set_step_duration(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        value: f64,
        unit: DurationUnit,
    ) -> CommandResult {
        self.with_step_duration(sequence_uuid, step_idx, |d| {
            *d = DurationSpec::from_value_unit(value, unit);
        })
    }

    pub(crate) fn cmd_set_step_easing(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        easing: &str,
    ) -> CommandResult {
        self.with_step(sequence_uuid, step_idx, |step| {
            if let StepKind::Fade { easing: e, .. } = &mut step.kind {
                *e = match easing {
                    "Linear" => CrossfadeEasing::Linear,
                    "EaseIn" => CrossfadeEasing::EaseIn,
                    "EaseOut" => CrossfadeEasing::EaseOut,
                    _ => CrossfadeEasing::EaseInOut,
                };
            }
        })
    }

    pub(crate) fn cmd_set_step_transition_shader(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        shader_name: Option<String>,
    ) -> CommandResult {
        self.with_step(sequence_uuid, step_idx, |step| {
            if let StepKind::Fade {
                transition_shader, ..
            } = &mut step.kind
            {
                *transition_shader = shader_name;
            }
        })
    }

    pub(crate) fn cmd_move_step(
        &mut self,
        sequence_uuid: &str,
        from: usize,
        to: usize,
    ) -> CommandResult {
        let idx = match self.resolve_sequence(sequence_uuid) {
            Ok(idx) => idx,
            Err(e) => {
                return CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                };
            }
        };
        let steps = &mut self.mixer.transition_sequences_mut()[idx].steps;
        if from >= steps.len() || to >= steps.len() {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!(
                    "move_step: ordinals {from}->{to} out of range for {} steps",
                    steps.len()
                ),
            };
        }
        if from != to {
            let step = steps.remove(from);
            steps.insert(to, step);
        }
        CommandResult::Ok
    }

    pub(crate) fn cmd_set_step_duration_unit(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        unit: DurationUnit,
    ) -> CommandResult {
        self.with_step_duration(sequence_uuid, step_idx, |d| {
            *d = DurationSpec::from_value_unit(d.value(), unit);
        })
    }

    pub(crate) fn cmd_toggle_step_duration_unit(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
    ) -> CommandResult {
        self.with_step_duration(sequence_uuid, step_idx, |d| {
            let next_unit = d.unit().next();
            *d = DurationSpec::from_value_unit(d.value(), next_unit);
        })
    }

    pub(crate) fn cmd_set_step_duration_value(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        value: f64,
    ) -> CommandResult {
        self.with_step_duration(sequence_uuid, step_idx, |d| d.set_value(value))
    }

    pub(crate) fn cmd_set_step_from_ch(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        channel_uuid: String,
    ) -> CommandResult {
        if let Err(e) = self.resolve_channel(&channel_uuid) {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: e.to_string(),
            };
        }
        self.with_step(sequence_uuid, step_idx, |step| {
            if let StepKind::Fade { from_ch, .. } = &mut step.kind {
                *from_ch = channel_uuid;
            }
        })
    }

    pub(crate) fn cmd_set_step_to_ch(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        channel_uuid: String,
    ) -> CommandResult {
        if let Err(e) = self.resolve_channel(&channel_uuid) {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: e.to_string(),
            };
        }
        self.with_step(sequence_uuid, step_idx, |step| {
            if let StepKind::Fade { to_ch, .. } = &mut step.kind {
                *to_ch = channel_uuid;
            }
        })
    }

    pub(crate) fn cmd_set_goto_target(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        target: usize,
    ) -> CommandResult {
        self.with_step(sequence_uuid, step_idx, |step| {
            if let StepKind::GoTo { step_index } = &mut step.kind {
                *step_index = target;
            }
        })
    }

    pub(crate) fn cmd_set_step_target_amount(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        amount: f32,
    ) -> CommandResult {
        self.with_step(sequence_uuid, step_idx, |step| {
            if let StepKind::Fade { target_amount, .. } = &mut step.kind {
                *target_amount = amount.clamp(0.0, 1.0);
            }
        })
    }
}
