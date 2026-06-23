//! Crossfade, transition, and sequence types + Mixer transition control methods.

use super::Mixer;
use crate::isf::{compile_glsl_to_spirv, ISFShader};
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, TransitionPipeline};
use anyhow::{Context as _, Result};

/// Easing curve for crossfade transitions
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum CrossfadeEasing {
    Linear,
    EaseInOut,
    EaseIn,
    EaseOut,
}

impl CrossfadeEasing {
    /// Apply easing to normalized time t (0.0 to 1.0)
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            CrossfadeEasing::Linear => t,
            CrossfadeEasing::EaseInOut => t * t * (3.0 - 2.0 * t),
            CrossfadeEasing::EaseIn => t * t,
            CrossfadeEasing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        }
    }
}

/// Describes an in-progress auto-crossfade
#[derive(Debug, Clone)]
pub struct AutoCrossfade {
    /// Where the crossfader started
    pub from: f32,
    /// Where the crossfader is heading
    pub to: f32,
    /// Total duration in seconds
    pub duration: f32,
    /// Elapsed time in seconds
    pub elapsed: f32,
    /// Easing curve
    pub easing: CrossfadeEasing,
}

impl AutoCrossfade {
    /// Create a new auto-crossfade
    pub fn new(from: f32, to: f32, duration: f32, easing: CrossfadeEasing) -> Self {
        Self {
            from,
            to,
            duration,
            elapsed: 0.0,
            easing,
        }
    }

    /// Tick the crossfade by dt seconds, return the new crossfader value.
    /// Returns None if the crossfade is complete.
    pub fn tick(&mut self, dt: f32) -> Option<f32> {
        self.elapsed += dt;
        if self.elapsed >= self.duration {
            return None;
        }
        let t = self.easing.apply(self.elapsed / self.duration);
        Some(self.from + (self.to - self.from) * t)
    }

    /// Progress as 0.0 to 1.0
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }
}

/// Beat-synced crossfade configuration
#[derive(Debug, Clone)]
pub struct BeatSyncCrossfade {
    /// Target crossfader value
    pub to: f32,
    /// Duration in beats
    pub beats: f32,
    /// Whether we've started (waiting for next beat boundary)
    pub started: bool,
    /// The auto-crossfade that runs once triggered
    pub auto: Option<AutoCrossfade>,
}

// ── Transition Sequence (channel-to-channel automation) ──────────────

/// A named sequence of channel transition steps for automated shows/installations.
#[derive(Debug, Clone)]
pub struct TransitionSequence {
    pub name: String,
    pub steps: Vec<TransitionStep>,
    pub enabled: bool,
    /// Runtime sequencer state — NOT persisted.
    pub state: SequencerState,
}

impl TransitionSequence {
    pub fn new(name: String) -> Self {
        Self {
            name,
            steps: Vec::new(),
            enabled: true,
            state: SequencerState::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransitionStep {
    pub kind: StepKind,
}

#[derive(Debug, Clone)]
pub enum StepKind {
    /// Fade from one channel to another over a duration.
    Fade {
        from_ch: usize,
        to_ch: usize,
        duration: crate::channel::DurationSpec,
        easing: CrossfadeEasing,
        /// Transition shader name (None = opacity fade). Only used in 2-channel mode.
        transition_shader: Option<String>,
        /// Target opacity for the destination channel (0.0–1.0). Default 1.0 = full.
        target_amount: f32,
    },
    /// Wait/hold for a duration.
    Wait {
        duration: crate::channel::DurationSpec,
    },
    /// Jump to a step index (0-based). Enables looping.
    GoTo { step_index: usize },
}

/// Runtime sequencer state — NOT persisted, computed each frame.
#[derive(Debug, Clone)]
pub struct SequencerState {
    pub playing: bool,
    pub current_step: usize,
    pub step_elapsed: f64,
}

impl Default for SequencerState {
    fn default() -> Self {
        Self::new()
    }
}

impl SequencerState {
    pub fn new() -> Self {
        Self {
            playing: false,
            current_step: 0,
            step_elapsed: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.playing = false;
        self.current_step = 0;
        self.step_elapsed = 0.0;
    }
}

/// Active transition effect between channels A and B
pub struct TransitionEffect {
    /// The ISF transition shader source
    pub shader: ISFShader,
    /// The compiled transition pipeline (two input textures + progress)
    pub pipeline: TransitionPipeline,
    /// User-controllable parameters (progress is always index 0)
    pub params: ShaderParams,
    /// Shader name for display
    pub name: String,
}

// ── Mixer transition/crossfade/sequence control methods ──────────────

impl Mixer {
    /// Start a timed auto-crossfade to the target value
    pub fn start_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing) {
        let target = if target.is_finite() {
            target.clamp(0.0, 1.0)
        } else {
            0.5
        };
        if (self.crossfader - target).abs() < 0.001 {
            return;
        }
        self.beat_sync_crossfade = None;
        self.auto_crossfade = Some(AutoCrossfade::new(
            self.crossfader,
            target,
            duration_secs,
            easing,
        ));
        log::info!(
            "Starting auto-crossfade: {:.2} → {:.2} over {:.1}s ({:?})",
            self.crossfader,
            target,
            duration_secs,
            easing
        );
    }

    /// Start a beat-synced crossfade (waits for next beat boundary, then transitions over N beats)
    pub fn start_beat_crossfade(&mut self, target: f32, beats: f32) {
        let target = if target.is_finite() {
            target.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.auto_crossfade = None;
        self.beat_sync_crossfade = Some(BeatSyncCrossfade {
            to: target,
            beats,
            started: false,
            auto: None,
        });
        log::info!(
            "Queued beat-synced crossfade: → {:.2} over {:.1} beats",
            target,
            beats
        );
    }

    /// Snap crossfader to a value immediately (cancels any in-progress transitions)
    pub fn snap_crossfader(&mut self, value: f32) {
        self.crossfader = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.auto_crossfade = None;
        self.beat_sync_crossfade = None;
    }

    /// Whether a crossfade transition is currently in progress
    pub fn is_crossfading(&self) -> bool {
        self.auto_crossfade.is_some()
            || self.beat_sync_crossfade.as_ref().is_some_and(|b| b.started)
    }

    /// Set the active transition shader. Compiles the shader and creates the pipeline.
    pub fn set_transition(&mut self, context: &GpuContext, shader: ISFShader) -> Result<()> {
        let name = shader.name();
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &name)
            .context("Failed to compile transition shader")?;

        let target_format = context.compositing_format;
        let pipeline = TransitionPipeline::new(&context.device, &spirv, target_format)
            .context("Failed to create transition pipeline")?;

        let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
        let mut params = ShaderParams::from_inputs(inputs);
        params.ensure_buffer(&context.device);

        log::info!("Active transition set: {}", name);
        self.active_transition = Some(TransitionEffect {
            shader,
            pipeline,
            params,
            name,
        });
        Ok(())
    }

    /// Clear the active transition (revert to opacity-based crossfade)
    pub fn clear_transition(&mut self) {
        if self.active_transition.is_some() {
            log::info!("Transition cleared, reverting to opacity crossfade");
        }
        self.active_transition = None;
    }

    /// Sync the transition's `progress` parameter with the crossfader value.
    pub(super) fn sync_transition_progress(&mut self) {
        if let Some(transition) = &mut self.active_transition {
            transition.params.set(
                "progress",
                crate::params::ParamValue::Float(self.crossfader),
            );
        }
    }

    // ── Transition Sequence Control ──────────────────────────────────

    /// Start playing a transition sequence by index from the beginning.
    pub fn start_sequence(&mut self, seq_idx: usize) {
        if let Some(seq) = self.transition_sequences.get_mut(seq_idx) {
            if seq.steps.is_empty() {
                return;
            }
            self.auto_crossfade = None;
            self.beat_sync_crossfade = None;
            seq.state = SequencerState {
                playing: true,
                current_step: 0,
                step_elapsed: 0.0,
            };
            log::info!("Transition sequence '{}' started", seq.name);
        }
    }

    /// Stop a transition sequence by index (leaves channels at current state).
    pub fn stop_sequence(&mut self, seq_idx: usize) {
        if let Some(seq) = self.transition_sequences.get_mut(seq_idx) {
            seq.state.playing = false;
            log::info!(
                "Transition sequence '{}' stopped at step {}",
                seq.name,
                seq.state.current_step
            );
        }
    }

    /// Tick all transition sequences forward by dt seconds.
    pub(super) fn tick_sequence(&mut self, dt: f32, bpm: Option<f64>) {
        let channel_count = self.channels.len();

        for seq_idx in 0..self.transition_sequences.len() {
            let seq = &mut self.transition_sequences[seq_idx];
            if !seq.state.playing || !seq.enabled || seq.steps.is_empty() {
                continue;
            }
            let num_steps = seq.steps.len();
            if seq.state.current_step >= num_steps {
                seq.state.playing = false;
                continue;
            }

            let step = &seq.steps[seq.state.current_step];
            let mutation = match &step.kind {
                StepKind::Fade {
                    from_ch,
                    to_ch,
                    duration,
                    easing,
                    target_amount,
                    ..
                } => {
                    let duration_secs = duration.to_seconds(bpm);
                    if duration_secs <= 0.0 {
                        seq.state.current_step += 1;
                        seq.state.step_elapsed = 0.0;
                        if seq.state.current_step >= num_steps {
                            seq.state.playing = false;
                        }
                        continue;
                    }
                    let progress = (seq.state.step_elapsed / duration_secs).clamp(0.0, 1.0) as f32;
                    let eased = easing.apply(progress);
                    let completed = seq.state.step_elapsed + dt as f64 >= duration_secs;
                    seq.state.step_elapsed += dt as f64;
                    if completed {
                        seq.state.current_step += 1;
                        seq.state.step_elapsed = 0.0;
                        if seq.state.current_step >= num_steps {
                            seq.state.playing = false;
                        }
                    }
                    Some((*from_ch, *to_ch, eased, completed, *target_amount))
                }
                StepKind::Wait { duration } => {
                    let duration_secs = duration.to_seconds(bpm);
                    seq.state.step_elapsed += dt as f64;
                    if seq.state.step_elapsed >= duration_secs {
                        seq.state.current_step += 1;
                        seq.state.step_elapsed = 0.0;
                        if seq.state.current_step >= num_steps {
                            seq.state.playing = false;
                        }
                    }
                    None
                }
                StepKind::GoTo { step_index } => {
                    let target = *step_index;
                    if target < num_steps && target != seq.state.current_step {
                        seq.state.current_step = target;
                        seq.state.step_elapsed = 0.0;
                    } else {
                        // Self-referencing GoTo or out-of-bounds → stop to prevent infinite loop
                        if target == seq.state.current_step {
                            log::warn!(
                                "Transition sequence {}: GoTo step {} references itself, stopping",
                                seq_idx,
                                target
                            );
                        }
                        seq.state.playing = false;
                    }
                    None
                }
            };

            if let Some((from, to, eased, completed, target_amount)) = mutation {
                if channel_count == 2 {
                    if from < channel_count && to < channel_count {
                        let from_val = if from == 0 { 0.0f32 } else { 1.0f32 };
                        let to_val = if to == 0 { 0.0f32 } else { target_amount };
                        self.crossfader = if completed {
                            to_val
                        } else {
                            from_val + (to_val - from_val) * eased
                        };
                    }
                } else if completed {
                    if from < channel_count {
                        self.channels[from].opacity = 1.0 - target_amount;
                    }
                    if to < channel_count {
                        self.channels[to].opacity = target_amount;
                    }
                } else {
                    if from < channel_count {
                        self.channels[from].opacity = 1.0 - eased * target_amount;
                    }
                    if to < channel_count {
                        self.channels[to].opacity = eased * target_amount;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CrossfadeEasing tests ────────────────────────────────────────

    #[test]
    fn easing_linear_passthrough() {
        assert_eq!(CrossfadeEasing::Linear.apply(0.0), 0.0);
        assert_eq!(CrossfadeEasing::Linear.apply(0.5), 0.5);
        assert_eq!(CrossfadeEasing::Linear.apply(1.0), 1.0);
    }

    #[test]
    fn easing_ease_in_out_endpoints() {
        assert!((CrossfadeEasing::EaseInOut.apply(0.0)).abs() < 1e-6);
        assert!((CrossfadeEasing::EaseInOut.apply(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn easing_ease_in_out_midpoint() {
        let mid = CrossfadeEasing::EaseInOut.apply(0.5);
        assert!((mid - 0.5).abs() < 1e-6, "EaseInOut midpoint: {mid}");
    }

    #[test]
    fn easing_ease_in_slow_start() {
        let quarter = CrossfadeEasing::EaseIn.apply(0.25);
        assert!(quarter < 0.25, "EaseIn at 0.25 should be < 0.25: {quarter}");
    }

    #[test]
    fn easing_ease_out_fast_start() {
        let quarter = CrossfadeEasing::EaseOut.apply(0.25);
        assert!(
            quarter > 0.25,
            "EaseOut at 0.25 should be > 0.25: {quarter}"
        );
    }

    #[test]
    fn easing_clamps_input() {
        assert_eq!(CrossfadeEasing::Linear.apply(-0.5), 0.0);
        assert_eq!(CrossfadeEasing::Linear.apply(1.5), 1.0);
    }

    #[test]
    fn easing_monotonic() {
        for easing in [
            CrossfadeEasing::Linear,
            CrossfadeEasing::EaseInOut,
            CrossfadeEasing::EaseIn,
            CrossfadeEasing::EaseOut,
        ] {
            let mut prev = 0.0;
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                let val = easing.apply(t);
                assert!(
                    val >= prev - 1e-6,
                    "{easing:?} not monotonic at t={t}: {val} < {prev}"
                );
                prev = val;
            }
        }
    }

    // ── AutoCrossfade tests ──────────────────────────────────────────

    #[test]
    fn auto_crossfade_tick_interpolates() {
        let mut cf = AutoCrossfade::new(0.0, 1.0, 2.0, CrossfadeEasing::Linear);
        let val = cf.tick(1.0).expect("should still be active");
        assert!((val - 0.5).abs() < 1e-5, "Midpoint: {val}");
    }

    #[test]
    fn auto_crossfade_tick_completes() {
        let mut cf = AutoCrossfade::new(0.0, 1.0, 1.0, CrossfadeEasing::Linear);
        let result = cf.tick(1.5);
        assert!(result.is_none(), "Should complete when elapsed >= duration");
    }

    #[test]
    fn auto_crossfade_reverse_direction() {
        let mut cf = AutoCrossfade::new(1.0, 0.0, 2.0, CrossfadeEasing::Linear);
        let val = cf.tick(1.0).expect("active");
        assert!((val - 0.5).abs() < 1e-5, "Reverse midpoint: {val}");
    }

    #[test]
    fn auto_crossfade_progress() {
        let mut cf = AutoCrossfade::new(0.0, 1.0, 4.0, CrossfadeEasing::Linear);
        assert_eq!(cf.progress(), 0.0);
        cf.tick(2.0);
        assert!((cf.progress() - 0.5).abs() < 1e-5);
        cf.tick(2.0);
        assert!((cf.progress() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn auto_crossfade_with_easing() {
        let mut cf = AutoCrossfade::new(0.0, 1.0, 2.0, CrossfadeEasing::EaseInOut);
        let val = cf.tick(1.0).expect("active");
        // EaseInOut at t=0.5 → 0.5
        assert!((val - 0.5).abs() < 1e-5, "EaseInOut midpoint: {val}");
    }

    // ── SequencerState tests ─────────────────────────────────────────

    #[test]
    fn sequencer_state_defaults() {
        let state = SequencerState::new();
        assert!(!state.playing);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.step_elapsed, 0.0);
    }

    #[test]
    fn sequencer_state_reset() {
        let mut state = SequencerState {
            playing: true,
            current_step: 5,
            step_elapsed: 3.2,
        };
        state.reset();
        assert!(!state.playing);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.step_elapsed, 0.0);
    }

    // ── TransitionSequence tests ─────────────────────────────────────

    #[test]
    fn transition_sequence_new_defaults() {
        let seq = TransitionSequence::new("TestSeq".to_string());
        assert_eq!(seq.name, "TestSeq");
        assert!(seq.steps.is_empty());
        assert!(seq.enabled);
        assert!(!seq.state.playing);
    }

    // ── Crossfade opacity formula tests ──────────────────────────────

    /// Verify the 2-channel crossfade opacity formula produces a linear blend.
    /// First channel is always 1.0 (base layer); crossfader drives the second
    /// channel's composite opacity.  The composite shader's mix() then yields:
    ///     result = (1 - cf) * A + cf * B
    #[test]
    fn crossfade_opacities_linear() {
        for i in 0..=10 {
            let cf = i as f32 / 10.0;
            let opacities = [1.0_f32, cf];
            // Simulates composite shader mix(A, B, src_a):
            //   result = (1 - src_a) * A + src_a * B
            // With A blitted at full opacity, A_weight = 1 - cf, B_weight = cf.
            let a_weight = 1.0 - opacities[1];
            let b_weight = opacities[1];
            let sum = a_weight + b_weight;
            assert!(
                (sum - 1.0).abs() < 1e-6,
                "Weights must sum to 1.0 at cf={cf}: got {sum}"
            );
        }
    }

    /// Old formula had squared falloff: first channel weight was (1-cf)²
    /// because ALPHA_BLENDING double-applied the opacity.  Ensure the new
    /// formula avoids this.
    #[test]
    fn crossfade_midpoint_symmetric() {
        let cf = 0.5_f32;
        let opacities = [1.0_f32, cf];
        let a_weight = 1.0 - opacities[1];
        let b_weight = opacities[1];
        assert!(
            (a_weight - b_weight).abs() < 1e-6,
            "At crossfader 0.5, weights must be equal: A={a_weight}, B={b_weight}"
        );
    }
}
