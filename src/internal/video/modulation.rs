//! Playback modulation: the values the render thread passes to the decode
//! thread, and the arithmetic that produces them.
//!
//! Free of `Deck`, `Mixer` and GPU types so the ranges, play gate and
//! seek-or-walk decision are testable without a video file or an adapter.

use crate::modulation::ResolvedModulation;

/// Reserved parameter names under a deck's modulation prefix, relative to
/// `deck/<uuid>/`. Shader inputs live under `deck/<uuid>/param/`, so these
/// cannot collide with them.
pub const SPEED: &str = "video/speed";
pub const POSITION: &str = "video/position";
pub const PLAY: &str = "video/play";
pub const LOOP_MODE: &str = "video/loop_mode";
/// Source scaling applies to every deck, not only video.
pub const SCALING_MODE: &str = "scaling_mode";

/// Speed multiplier bounds. Match `param_router::scale_speed` and the UI slider
/// so MIDI, LFOs and the mouse cover the same range.
pub const SPEED_MIN: f64 = 0.1;
pub const SPEED_MAX: f64 = 4.0;

/// Beyond this forward offset, decoding forward costs more than a seek.
/// Shared with the chase servo.
pub const WALK_LIMIT_SECS: f64 = super::chase::SEEK_THRESHOLD_SECS;

/// Thresholds for [`play_gate`]. The band between them stops a modulator
/// resting near the middle from toggling play every frame.
const GATE_ON: f32 = 0.55;
const GATE_OFF: f32 = 0.45;

/// What modulation asks of the playhead this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PositionTarget {
    /// Nothing is assigned; the clip plays normally.
    #[default]
    Free,
    /// Seconds away from where the clip would otherwise be.
    Offset(f64),
    /// An absolute clip time.
    Absolute(f64),
}

/// One frame of resolved playback modulation, published to a decode thread.
///
/// Only continuous targets travel this way. Play, loop mode and scaling mode
/// are discrete and use the command channel, so a settled modulator sends
/// nothing across threads.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlaybackModulation {
    /// Effective speed, or `None` when nothing is assigned.
    pub speed: Option<f64>,
    pub position: PositionTarget,
}

impl PlaybackModulation {
    /// Whether the decode thread has anything to act on.
    pub fn is_inert(&self) -> bool {
        self.speed.is_none() && self.position == PositionTarget::Free
    }
}

/// Latest-value cell: the render thread writes, the decode thread reads.
///
/// Not a `VideoCommand` on the mpsc channel because only the newest value
/// matters, and a queue grows whenever the render thread outruns the decode
/// thread (e.g. a 30fps clip under a 120fps renderer).
#[derive(Debug, Default)]
pub struct PlaybackModulationInbox {
    slot: std::sync::Mutex<PlaybackModulation>,
}

impl PlaybackModulationInbox {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, value: PlaybackModulation) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = value;
        }
    }

    pub fn take(&self) -> PlaybackModulation {
        self.slot.lock().map(|g| *g).unwrap_or_default()
    }
}

/// Effective playback speed. Additive sources add to the stored speed; an
/// absolute source replaces it.
pub fn effective_speed(base: f64, resolved: &ResolvedModulation) -> f64 {
    let range = SPEED_MAX - SPEED_MIN;
    let base = resolved
        .absolute
        .map_or(base, |v| SPEED_MIN + f64::from(v) * range);
    (base + f64::from(resolved.additive) * range).clamp(SPEED_MIN, SPEED_MAX)
}

/// Where modulation wants the playhead.
///
/// An offset scales against the active loop region, so a patch stays
/// proportional to the loop; scaling by the full duration would swing minutes
/// on a long clip.
///
/// An absolute value scales against the whole clip, like the scrub bar and a
/// MIDI seek on `deck/<uuid>/video/seek`. A live seek records its value as the
/// override for the curve it replaces, so both must use the same span.
pub fn position_target(
    resolved: &ResolvedModulation,
    in_point: f64,
    effective_out: f64,
    duration: f64,
) -> PositionTarget {
    if let Some(v) = resolved.absolute {
        let clip = duration.max(0.0);
        return PositionTarget::Absolute((f64::from(v) * clip).clamp(0.0, clip));
    }
    if resolved.additive == 0.0 {
        return PositionTarget::Free;
    }
    let region = (effective_out - in_point).max(0.0);
    PositionTarget::Offset(f64::from(resolved.additive) * region)
}

/// Whether the clip should play, given the modulator value and the current
/// play state.
///
/// An assigned modulator decides play outright. `current` is the value held
/// inside the hysteresis band.
pub fn play_gate(resolved: &ResolvedModulation, current: bool) -> bool {
    let value = resolved.absolute.unwrap_or(resolved.additive);
    if value >= GATE_ON {
        true
    } else if value <= GATE_OFF {
        false
    } else {
        current
    }
}

/// The normalized value for a discrete target, for the router's own
/// bucketing.
///
/// An assigned modulator sets discrete targets outright, since an offset from
/// "Ping-Pong" means nothing. The caller passes this to `LoopMode::from_value`
/// or `ScalingMode::from_value` so faders and LFOs pick options the same way.
pub fn discrete_value(resolved: &ResolvedModulation) -> f32 {
    resolved
        .absolute
        .unwrap_or(resolved.additive)
        .clamp(0.0, 1.0)
}

/// How the decoder handles a step in the modulated playhead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetStep {
    /// Flush and seek.
    pub needs_seek: bool,
    /// Forward clip time to decode through; the frame accumulator turns it into
    /// whole frames.
    pub walk_secs: f64,
}

/// Decides how to move the playhead by `delta` seconds.
///
/// ffmpeg cannot decode backward, so a backward step seeks. A forward step
/// decodes extra frames until that costs more than a seek.
///
/// `walk_secs` keeps sub-frame amounts because the caller's frame accumulator
/// quantizes them. The one-frame deadband applies only to seeks: a backward
/// step under a frame would land on the frame already shown.
///
/// A P-controller like the chase servo does not fit: its ±20% speed trim
/// cannot reach modulation-scale offsets (at 1 Hz it covers about 30 ms).
pub fn offset_step(delta: f64, frame_time: f64) -> OffsetStep {
    let still = OffsetStep {
        needs_seek: false,
        walk_secs: 0.0,
    };
    if !delta.is_finite() || delta == 0.0 {
        return still;
    }
    if delta > WALK_LIMIT_SECS || delta < -frame_time {
        return OffsetStep {
            needs_seek: true,
            walk_secs: 0.0,
        };
    }
    if delta < 0.0 {
        return still;
    }
    OffsetStep {
        needs_seek: false,
        walk_secs: delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn additive(v: f32) -> ResolvedModulation {
        ResolvedModulation {
            additive: v,
            absolute: None,
        }
    }

    fn absolute(v: f32) -> ResolvedModulation {
        ResolvedModulation {
            additive: 0.0,
            absolute: Some(v),
        }
    }

    // ── speed ──────────────────────────────────────────────────────────

    #[test]
    fn unassigned_speed_is_the_stored_speed() {
        assert!((effective_speed(1.5, &additive(0.0)) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn additive_speed_rides_on_the_stored_base() {
        // range is 3.9, so +0.1 of it is +0.39. f32 tolerance because resolved
        // modulation is f32.
        assert!((effective_speed(1.0, &additive(0.1)) - 1.39).abs() < 1e-6);
    }

    #[test]
    fn absolute_speed_replaces_the_stored_base() {
        assert!((effective_speed(4.0, &absolute(0.0)) - SPEED_MIN).abs() < 1e-9);
        assert!((effective_speed(0.1, &absolute(1.0)) - SPEED_MAX).abs() < 1e-9);
    }

    #[test]
    fn speed_clamps_to_the_slider_range_at_both_ends() {
        assert!((effective_speed(4.0, &additive(1.0)) - SPEED_MAX).abs() < 1e-9);
        assert!((effective_speed(0.1, &additive(-1.0)) - SPEED_MIN).abs() < 1e-9);
    }

    #[test]
    fn speed_never_reverses() {
        // No value can push the multiplier through zero, so a modulator cannot
        // reverse a clip. Reverse is ping-pong's job.
        for a in [-10.0, -1.0, -0.5, 0.0, 0.5, 10.0] {
            assert!(effective_speed(1.0, &additive(a)) > 0.0);
        }
    }

    // ── position ───────────────────────────────────────────────────────

    #[test]
    fn no_assignment_leaves_the_playhead_free() {
        assert_eq!(
            position_target(&additive(0.0), 0.0, 10.0, 10.0),
            PositionTarget::Free
        );
    }

    #[test]
    fn offset_scales_against_the_loop_region_not_the_clip() {
        // Same assignment and depth, different loop: the offset follows the region.
        let whole = position_target(&additive(0.5), 0.0, 100.0, 100.0);
        let loop_region = position_target(&additive(0.5), 10.0, 14.0, 100.0);
        assert_eq!(whole, PositionTarget::Offset(50.0));
        assert_eq!(loop_region, PositionTarget::Offset(2.0));
    }

    #[test]
    fn absolute_position_addresses_the_whole_clip_not_the_region() {
        // Absolute values scale against the whole clip. In and out points at
        // 4..8 of a 20 s clip do not change what half-way means.
        assert_eq!(
            position_target(&absolute(0.0), 4.0, 8.0, 20.0),
            PositionTarget::Absolute(0.0)
        );
        assert_eq!(
            position_target(&absolute(0.5), 4.0, 8.0, 20.0),
            PositionTarget::Absolute(10.0)
        );
        assert_eq!(
            position_target(&absolute(1.0), 4.0, 8.0, 20.0),
            PositionTarget::Absolute(20.0)
        );
    }

    #[test]
    fn absolute_position_matches_a_live_seek_on_the_same_value() {
        // Must agree because a live seek records its normalized value as the
        // curve's override. Mirrors `param_router::scale_to_duration`.
        for v in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            let seek = f64::from(v) * 30.0;
            assert_eq!(
                position_target(&absolute(v), 5.0, 12.0, 30.0),
                PositionTarget::Absolute(seek)
            );
        }
    }

    #[test]
    fn absolute_position_stays_inside_the_clip() {
        assert_eq!(
            position_target(&absolute(4.0), 0.0, 10.0, 10.0),
            PositionTarget::Absolute(10.0)
        );
        assert_eq!(
            position_target(&absolute(0.5), 0.0, 0.0, -3.0),
            PositionTarget::Absolute(0.0)
        );
    }

    #[test]
    fn an_inverted_region_does_not_produce_a_negative_offset_scale() {
        assert_eq!(
            position_target(&additive(1.0), 10.0, 2.0, 20.0),
            PositionTarget::Offset(0.0)
        );
    }

    // ── play gate ──────────────────────────────────────────────────────

    #[test]
    fn gate_opens_above_the_upper_threshold_and_closes_below_the_lower() {
        assert!(play_gate(&additive(0.9), false));
        assert!(!play_gate(&additive(0.1), true));
    }

    #[test]
    fn gate_holds_inside_the_hysteresis_band() {
        // A modulator resting near the middle must not toggle play every frame.
        assert!(play_gate(&additive(0.5), true));
        assert!(!play_gate(&additive(0.5), false));
    }

    #[test]
    fn gate_does_not_chatter_across_a_slow_sweep_through_the_band() {
        let mut state = false;
        let mut flips = 0;
        // Up through the band and back down: one flip each way.
        for i in 0_u8..=40 {
            let v = f32::from(i) / 40.0;
            let next = play_gate(&additive(v), state);
            if next != state {
                flips += 1;
            }
            state = next;
        }
        for i in (0_u8..=40).rev() {
            let v = f32::from(i) / 40.0;
            let next = play_gate(&additive(v), state);
            if next != state {
                flips += 1;
            }
            state = next;
        }
        assert_eq!(flips, 2, "gate chattered");
    }

    // ── discrete targets ───────────────────────────────────────────────

    #[test]
    fn discrete_value_clamps_into_fader_range() {
        assert_eq!(discrete_value(&additive(-5.0)), 0.0);
        assert_eq!(discrete_value(&additive(5.0)), 1.0);
        assert!((discrete_value(&additive(0.4)) - 0.4).abs() < 1e-9);
    }

    #[test]
    fn an_absolute_source_owns_a_discrete_target() {
        let mut r = absolute(0.25);
        r.additive = 0.9;
        assert!((discrete_value(&r) - 0.25).abs() < 1e-9);
    }

    // ── offset step ────────────────────────────────────────────────────

    const FRAME: f64 = 1.0 / 30.0;

    #[test]
    fn a_sub_frame_backward_step_neither_seeks_nor_decodes() {
        // It would land on the frame already shown, so a seek is wasted.
        let back = offset_step(-FRAME * 0.4, FRAME);
        assert!(!back.needs_seek);
        assert_eq!(back.walk_secs, 0.0);
    }

    #[test]
    fn a_sub_frame_forward_step_is_carried_not_discarded() {
        // The frame accumulator quantizes to whole frames; dropping sub-frame
        // motion would let the playhead drift from what was decoded, and a slow
        // LFO would step instead of moving smoothly.
        let step = offset_step(FRAME * 0.4, FRAME);
        assert!(!step.needs_seek);
        assert!((step.walk_secs - FRAME * 0.4).abs() < 1e-12);
    }

    #[test]
    fn a_still_playhead_does_nothing() {
        let step = offset_step(0.0, FRAME);
        assert!(!step.needs_seek);
        assert_eq!(step.walk_secs, 0.0);
    }

    #[test]
    fn a_forward_step_walks_instead_of_seeking() {
        let step = offset_step(FRAME * 3.0, FRAME);
        assert!(!step.needs_seek);
        assert!((step.walk_secs - FRAME * 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_backward_step_must_seek() {
        // ffmpeg cannot walk backward through a stream.
        assert!(offset_step(-FRAME * 3.0, FRAME).needs_seek);
    }

    #[test]
    fn a_long_forward_step_seeks_rather_than_decoding_the_gap() {
        let step = offset_step(WALK_LIMIT_SECS + 0.1, FRAME);
        assert!(step.needs_seek);
        assert_eq!(step.walk_secs, 0.0);
    }

    #[test]
    fn a_non_finite_step_is_ignored() {
        for d in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let step = offset_step(d, FRAME);
            assert!(!step.needs_seek);
            assert_eq!(step.walk_secs, 0.0);
        }
    }

    // ── inbox ──────────────────────────────────────────────────────────

    #[test]
    fn inbox_keeps_only_the_newest_value() {
        let inbox = PlaybackModulationInbox::new();
        inbox.publish(PlaybackModulation {
            speed: Some(2.0),
            position: PositionTarget::Offset(1.0),
        });
        inbox.publish(PlaybackModulation {
            speed: Some(3.0),
            position: PositionTarget::Free,
        });
        let taken = inbox.take();
        assert_eq!(taken.speed, Some(3.0));
        assert_eq!(taken.position, PositionTarget::Free);
    }

    #[test]
    fn inbox_repeats_the_last_value_because_it_is_a_level_not_an_event() {
        let inbox = PlaybackModulationInbox::new();
        inbox.publish(PlaybackModulation {
            speed: Some(2.0),
            position: PositionTarget::Free,
        });
        assert_eq!(inbox.take().speed, Some(2.0));
        assert_eq!(inbox.take().speed, Some(2.0));
    }

    #[test]
    fn a_default_inbox_is_inert() {
        assert!(PlaybackModulationInbox::new().take().is_inert());
    }
}
