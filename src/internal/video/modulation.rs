//! Playback modulation: the resolved values the render thread hands the decode
//! thread, and the pure arithmetic that produces them.
//!
//! Everything here is deliberately free of `Deck`, `Mixer`, and GPU types so the
//! ranges, the play gate, and the seek-versus-walk decision can be unit tested
//! without a video file or an adapter.
//!
//! See /spec/video-playback-modulation.md.

use crate::modulation::ResolvedModulation;

/// Reserved parameter names on a deck's modulation prefix.
///
/// These live in the same `deck_<uuid>:<name>` namespace as ISF generator
/// inputs, so they take a `video_` prefix rather than the bare control name.
/// `deck_<uuid>:speed` already belongs to any shader with an input called
/// `speed`, which is common, and the collision would silently drive a clip's
/// playback rate from a shader's animation rate on the same deck.
/// `tests/shader_pipeline_guard.rs` enforces the reservation.
pub const SPEED: &str = "video_speed";
pub const POSITION: &str = "video_position";
pub const PLAY: &str = "video_play";
pub const LOOP_MODE: &str = "video_loop_mode";
/// Source scaling applies to every deck, not just video ones, so it carries no
/// `video_` prefix. It is reserved by the same guard test.
pub const SCALING_MODE: &str = "scaling_mode";

/// Speed multiplier bounds. Match `param_router::scale_speed` and the UI slider
/// so a MIDI knob, an LFO, and the mouse move the parameter over one interval.
pub const SPEED_MIN: f64 = 0.1;
pub const SPEED_MAX: f64 = 4.0;

/// Walking further than this to satisfy a forward offset costs more in decode
/// than a seek does. Shared with the chase servo so both paths agree on where
/// "far" begins.
pub const WALK_LIMIT_SECS: f64 = super::chase::SEEK_THRESHOLD_SECS;

/// Gate thresholds for [`play_gate`]. The band between them is what stops a
/// modulator resting near the middle from flipping play and pause every frame.
const GATE_ON: f32 = 0.55;
const GATE_OFF: f32 = 0.45;

/// What modulation asks of the playhead this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PositionTarget {
    /// Nothing is assigned. The clip advances on its own terms.
    #[default]
    Free,
    /// Seconds to sit away from the playhead the clip would have reached.
    Offset(f64),
    /// An absolute clip time, from a source that replaces rather than nudges.
    Absolute(f64),
}

/// One frame of resolved playback modulation, published to a decode thread.
///
/// Only the continuous targets travel this way. Play, loop mode, and scaling
/// mode are discrete, change rarely, and go through the ordinary command path so
/// a settled modulator produces no cross-thread traffic at all.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlaybackModulation {
    /// Effective speed, or `None` when nothing is assigned and the stored speed
    /// stands.
    pub speed: Option<f64>,
    pub position: PositionTarget,
}

impl PlaybackModulation {
    /// Whether this carries anything the decode thread has to act on.
    pub fn is_inert(&self) -> bool {
        self.speed.is_none() && self.position == PositionTarget::Free
    }
}

/// Render thread publishes, decode thread consumes, newest value wins.
///
/// Deliberately not a `VideoCommand` over the existing mpsc channel: only the
/// newest value matters, and a queue grows without bound whenever the render
/// thread outruns the decode thread, which is the normal case (a 30fps clip
/// under a 120fps renderer would strand four stale values per decode tick,
/// forever). Overwriting a cell drops staleness by construction.
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

/// Effective playback speed. Additive sources ride on top of the stored speed,
/// which stays the performer's set point; an absolute source replaces it.
pub fn effective_speed(base: f64, resolved: &ResolvedModulation) -> f64 {
    let range = SPEED_MAX - SPEED_MIN;
    let base = resolved
        .absolute
        .map_or(base, |v| SPEED_MIN + f64::from(v) * range);
    (base + f64::from(resolved.additive) * range).clamp(SPEED_MIN, SPEED_MAX)
}

/// Where modulation wants the playhead.
///
/// The two kinds of source measure against different spans, because they are
/// answering different questions.
///
/// An **offset** is scaled against the active loop region, so the same patch
/// stays proportionate to the loop the performer set up. On a four-bar loop an
/// LFO wobbles within those four bars; scaling by full duration would swing
/// minutes on a long clip and be unusable.
///
/// An **absolute** source is stating where the playhead *is*, and every other
/// way of saying that (the scrub bar, a MIDI seek on `deck/<uuid>/video/seek`)
/// addresses the whole clip. A drawn curve reading half-way therefore lands
/// half-way through the clip, not half-way through the loop region. Scoping it
/// to the region instead would also have put the recorded value for a live seek
/// in a different space from the curve it overrides, which is a silent
/// off-by-the-in-point rather than a design choice.
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

/// Whether the clip should be playing, given what a modulator is asking and
/// where play currently stands.
///
/// An assignment here takes the play state over rather than nudging it: "an
/// audio band gates play" only means anything if the band decides. `current` is
/// the hold value inside the hysteresis band, not a value to add to.
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

/// The normalized value a discrete target's modulation is pointing at, ready to
/// hand to the router's own bucketing.
///
/// Discrete targets are owned by the modulator when assigned, for the same
/// reason as the play gate: there is no meaningful sum of "Ping-Pong" and an
/// offset. The caller feeds this to `param_router::loop_mode_from_value` or
/// `scaling_mode_from_value` rather than bucketing here, so a fader and an LFO
/// land on the same option by construction rather than by agreement.
pub fn discrete_value(resolved: &ResolvedModulation) -> f32 {
    resolved
        .absolute
        .unwrap_or(resolved.additive)
        .clamp(0.0, 1.0)
}

/// How the decoder must react to a step in the modulated playhead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetStep {
    /// The decoder cannot walk here; flush and seek.
    pub needs_seek: bool,
    /// Forward clip time to walk through, which the frame accumulator turns
    /// into whole frames to decode.
    pub walk_secs: f64,
}

/// Decide how to move the playhead by `delta` seconds.
///
/// This is the one place the codec's shape shows through. ffmpeg cannot walk
/// backward, so a backward step is a seek; a forward step is just extra frames
/// decoded in order, until walking costs more than seeking would.
///
/// `walk_secs` carries sub-frame amounts through rather than discarding them,
/// because the caller's frame accumulator is already the frame quantizer. The
/// one-frame deadband guards only the *seek* decision, which is where a stray
/// decision is expensive: a backward nudge smaller than a frame would land on
/// the picture already on screen, so flushing the decoder to fetch it is waste.
///
/// A P-controller like the chase servo would be the wrong instrument here. That
/// servo trims speed by at most ±20% because it is correcting drift measured in
/// single frames. Modulation-scale offsets are orders of magnitude larger, and a
/// ±20% trim could never reach them: at 1 Hz it would cap travel at roughly
/// 30 ms before falling behind and seeking anyway.
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
        // range is 3.9, so +0.1 of it is +0.39. Tolerance is f32-scale because
        // resolved modulation arrives as f32.
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
        // No resolved value can push the multiplier through zero, so a modulator
        // cannot reverse a clip. Reverse belongs to ping-pong.
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
        // Same assignment, same depth, different loop: the offset follows the
        // region, which is what keeps a patch musical across clips.
        let whole = position_target(&additive(0.5), 0.0, 100.0, 100.0);
        let loop_region = position_target(&additive(0.5), 10.0, 14.0, 100.0);
        assert_eq!(whole, PositionTarget::Offset(50.0));
        assert_eq!(loop_region, PositionTarget::Offset(2.0));
    }

    #[test]
    fn absolute_position_addresses_the_whole_clip_not_the_region() {
        // A drawn curve states where the playhead is, and the scrub bar and a
        // MIDI seek both say that against the whole clip. In and out points at
        // 4..8 of a 20 s clip do not move what half-way means.
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
        // The two have to agree, because a live seek records its normalized
        // value as the override for the curve it takes over. Mirrors
        // `param_router::scale_to_duration`.
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
        // The band is the whole point: a modulator resting near the middle must
        // not flip play and pause every frame, because each round trip shows.
        assert!(play_gate(&additive(0.5), true));
        assert!(!play_gate(&additive(0.5), false));
    }

    #[test]
    fn gate_does_not_chatter_across_a_slow_sweep_through_the_band() {
        let mut state = false;
        let mut flips = 0;
        // Up through the band and back down: one flip each way, not one per step.
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
        // It would land on the picture already on screen, so flushing the
        // decoder to fetch it is pure waste.
        let back = offset_step(-FRAME * 0.4, FRAME);
        assert!(!back.needs_seek);
        assert_eq!(back.walk_secs, 0.0);
    }

    #[test]
    fn a_sub_frame_forward_step_is_carried_not_discarded() {
        // The caller's frame accumulator quantizes into whole frames, so
        // dropping sub-frame motion here would let the playhead drift ahead of
        // what has actually been decoded. This is what makes a slow LFO on the
        // playhead move smoothly instead of stepping.
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
