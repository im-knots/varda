//! Editing and running transition sequences, addressed by UUID.
//!
//! Sequences are addressed by UUID; steps are positional within their sequence,
//! so `step_idx` is an ordinal rather than an address. See
//! [`/spec/api-addressing.md`].

use anyhow::Result;

use super::{CrossfadeEasing, Mixer, StepKind, TransitionSequence, TransitionStep};
use crate::channel::{DurationSpec, DurationUnit};
use crate::engine::value::entity::Resolved;

/// A step ordinal past the end of its sequence. Reported as "not found", the
/// same as an unknown sequence.
#[derive(Debug, thiserror::Error)]
#[error("Sequence '{sequence}' has no step {step}")]
pub struct NoSuchStep {
    pub sequence: String,
    pub step: usize,
}

impl Mixer {
    fn sequence_mut(&mut self, sequence_uuid: &str) -> Resolved<&mut TransitionSequence> {
        let idx = self.resolve_sequence(sequence_uuid)?;
        Ok(&mut self.transition_sequences_mut()[idx])
    }

    fn step_mut(&mut self, sequence_uuid: &str, step_idx: usize) -> Result<&mut TransitionStep> {
        self.sequence_mut(sequence_uuid)?
            .steps
            .get_mut(step_idx)
            .ok_or_else(|| {
                NoSuchStep {
                    sequence: sequence_uuid.to_string(),
                    step: step_idx,
                }
                .into()
            })
    }

    /// The duration of a Fade or Wait step. `GoTo` steps have none.
    fn step_duration_mut(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
    ) -> Result<Option<&mut DurationSpec>> {
        Ok(match &mut self.step_mut(sequence_uuid, step_idx)?.kind {
            StepKind::Fade { duration, .. } | StepKind::Wait { duration } => Some(duration),
            StepKind::GoTo { .. } => None,
        })
    }

    /// Create an empty sequence, named by its position. Returns its UUID.
    pub fn create_sequence(&mut self) -> String {
        let n = self.transition_sequences().len() + 1;
        let seq = TransitionSequence::new(format!("Sequence {n}"));
        let uuid = seq.uuid.clone();
        self.transition_sequences_mut().push(seq);
        uuid
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no sequence.
    pub fn delete_sequence(&mut self, sequence_uuid: &str) -> Resolved<()> {
        let idx = self.resolve_sequence(sequence_uuid)?;
        self.transition_sequences_mut().remove(idx);
        Ok(())
    }

    /// Start a sequence from its first step. An empty sequence does nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no sequence.
    pub fn start_sequence(&mut self, sequence_uuid: &str) -> Resolved<()> {
        let idx = self.resolve_sequence(sequence_uuid)?;
        self.start_sequence_at(idx);
        Ok(())
    }

    /// Stop a sequence, leaving its channels where they are.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no sequence.
    pub fn stop_sequence(&mut self, sequence_uuid: &str) -> Resolved<()> {
        let idx = self.resolve_sequence(sequence_uuid)?;
        self.stop_sequence_at(idx);
        Ok(())
    }

    /// Enable or disable a sequence; disabling resets its playhead.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no sequence.
    pub fn toggle_sequence(&mut self, sequence_uuid: &str) -> Resolved<()> {
        let seq = self.sequence_mut(sequence_uuid)?;
        seq.enabled = !seq.enabled;
        if !seq.enabled {
            seq.state.reset();
        }
        Ok(())
    }

    fn push_step(&mut self, sequence_uuid: &str, kind: StepKind) -> Resolved<()> {
        self.sequence_mut(sequence_uuid)?
            .steps
            .push(TransitionStep { kind });
        Ok(())
    }

    /// Append a two-second ease-in-out fade between two channels.
    ///
    /// # Errors
    ///
    /// Returns an error if any UUID names nothing; the step is then not added.
    pub fn add_fade_step(
        &mut self,
        sequence_uuid: &str,
        from_channel_uuid: &str,
        to_channel_uuid: &str,
    ) -> Resolved<()> {
        // Validate both channels up front so a step can never be created
        // pointing at a channel that does not exist.
        self.resolve_channel(from_channel_uuid)?;
        self.resolve_channel(to_channel_uuid)?;
        self.push_step(
            sequence_uuid,
            StepKind::Fade {
                from_ch: from_channel_uuid.to_string(),
                to_ch: to_channel_uuid.to_string(),
                duration: DurationSpec::Seconds(2.0),
                easing: CrossfadeEasing::EaseInOut,
                transition_shader: None,
                target_amount: 1.0,
            },
        )
    }

    /// Append a two-second wait.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no sequence.
    pub fn add_wait_step(&mut self, sequence_uuid: &str) -> Resolved<()> {
        self.push_step(
            sequence_uuid,
            StepKind::Wait {
                duration: DurationSpec::Seconds(2.0),
            },
        )
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no sequence.
    pub fn add_goto_step(&mut self, sequence_uuid: &str, step_index: usize) -> Resolved<()> {
        self.push_step(sequence_uuid, StepKind::GoTo { step_index })
    }

    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn remove_step(&mut self, sequence_uuid: &str, step_idx: usize) -> Result<()> {
        let steps = &mut self.sequence_mut(sequence_uuid)?.steps;
        if step_idx >= steps.len() {
            return Err(NoSuchStep {
                sequence: sequence_uuid.to_string(),
                step: step_idx,
            }
            .into());
        }
        steps.remove(step_idx);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the sequence names nothing or either ordinal is out
    /// of range.
    pub fn move_step(&mut self, sequence_uuid: &str, from: usize, to: usize) -> Result<()> {
        let steps = &mut self.sequence_mut(sequence_uuid)?.steps;
        if from >= steps.len() || to >= steps.len() {
            anyhow::bail!(
                "move_step: ordinals {from}->{to} out of range for {} steps",
                steps.len()
            );
        }
        if from != to {
            let step = steps.remove(from);
            steps.insert(to, step);
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_step_duration(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        value: f64,
        unit: DurationUnit,
    ) -> Result<()> {
        if let Some(d) = self.step_duration_mut(sequence_uuid, step_idx)? {
            *d = DurationSpec::from_value_unit(value, unit);
        }
        Ok(())
    }

    /// Change a duration's unit, keeping its number.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_step_duration_unit(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        unit: DurationUnit,
    ) -> Result<()> {
        if let Some(d) = self.step_duration_mut(sequence_uuid, step_idx)? {
            *d = DurationSpec::from_value_unit(d.value(), unit);
        }
        Ok(())
    }

    /// Cycle a duration to the next unit, keeping its number.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn toggle_step_duration_unit(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
    ) -> Result<()> {
        if let Some(d) = self.step_duration_mut(sequence_uuid, step_idx)? {
            *d = DurationSpec::from_value_unit(d.value(), d.unit().next());
        }
        Ok(())
    }

    /// Change a duration's number, keeping its unit.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_step_duration_value(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        value: f64,
    ) -> Result<()> {
        if let Some(d) = self.step_duration_mut(sequence_uuid, step_idx)? {
            d.set_value(value);
        }
        Ok(())
    }

    /// Set a fade's easing by name; an unknown name means ease-in-out.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_step_easing(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        easing: &str,
    ) -> Result<()> {
        if let StepKind::Fade { easing: e, .. } = &mut self.step_mut(sequence_uuid, step_idx)?.kind
        {
            *e = match easing {
                "Linear" => CrossfadeEasing::Linear,
                "EaseIn" => CrossfadeEasing::EaseIn,
                "EaseOut" => CrossfadeEasing::EaseOut,
                _ => CrossfadeEasing::EaseInOut,
            };
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_step_transition_shader(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        shader_name: Option<String>,
    ) -> Result<()> {
        if let StepKind::Fade {
            transition_shader, ..
        } = &mut self.step_mut(sequence_uuid, step_idx)?.kind
        {
            *transition_shader = shader_name;
        }
        Ok(())
    }

    /// Point a fade at a different source channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel, sequence, or step does not exist.
    pub fn set_step_from_channel(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        channel_uuid: String,
    ) -> Result<()> {
        self.resolve_channel(&channel_uuid)?;
        if let StepKind::Fade { from_ch, .. } = &mut self.step_mut(sequence_uuid, step_idx)?.kind {
            *from_ch = channel_uuid;
        }
        Ok(())
    }

    /// Point a fade at a different destination channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel, sequence, or step does not exist.
    pub fn set_step_to_channel(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        channel_uuid: String,
    ) -> Result<()> {
        self.resolve_channel(&channel_uuid)?;
        if let StepKind::Fade { to_ch, .. } = &mut self.step_mut(sequence_uuid, step_idx)?.kind {
            *to_ch = channel_uuid;
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_goto_target(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        target: usize,
    ) -> Result<()> {
        if let StepKind::GoTo { step_index } = &mut self.step_mut(sequence_uuid, step_idx)?.kind {
            *step_index = target;
        }
        Ok(())
    }

    /// Set how far a fade travels, clamped to 0–1.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence or the step does not exist.
    pub fn set_step_target_amount(
        &mut self,
        sequence_uuid: &str,
        step_idx: usize,
        amount: f32,
    ) -> Result<()> {
        if let StepKind::Fade { target_amount, .. } =
            &mut self.step_mut(sequence_uuid, step_idx)?.kind
        {
            *target_amount = amount.clamp(0.0, 1.0);
        }
        Ok(())
    }
}
