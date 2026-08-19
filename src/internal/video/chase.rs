//! Video-deck chase servo. See /spec/timecode.md § Consumer 2.

use crate::engine::value::video::DeckTransportSync;

/// Result of one chase step: the clip position to take, and whether the
/// decoder must seek rather than walk sequentially.
#[derive(Debug, Clone, Copy)]
pub struct ChaseStep {
    pub position: f64,
    pub needs_seek: bool,
    pub frames_to_decode: u32,
}

/// One-frame deadband, in clip frames.
pub const DEADBAND_FRAMES: f64 = 1.0;
/// Seek rather than trim once error reaches this many seconds.
pub const SEEK_THRESHOLD_SECS: f64 = 0.5;
/// Maximum relative trim around `base` (±20%).
pub const TRIM_CLAMP: f64 = 0.2;
/// P-gain (per second) so a 0.5 s error sits on the clamp.
pub const GAIN: f64 = 0.4;

/// Transport snapshot written once per render frame and read by decode threads.
#[derive(Debug, Clone, Copy)]
pub struct VideoChaseBroadcast {
    pub position: f64,
    pub running: bool,
    pub fps: f64,
}

impl Default for VideoChaseBroadcast {
    fn default() -> Self {
        Self {
            position: 0.0,
            running: false,
            fps: 30.0,
        }
    }
}

/// Per-tick transport facts consumed by [`PlaybackState::advance_frame`].
#[derive(Debug, Clone, Copy)]
pub struct ChaseTransport {
    pub position: f64,
    pub running: bool,
    pub discontinuity: bool,
    pub fps: f64,
}

/// Shared inbox: render thread publishes, decode thread consumes.
/// Discontinuity is sticky so a one-frame locate cannot be missed.
pub struct ChaseInbox {
    sample: std::sync::Mutex<VideoChaseBroadcast>,
    discontinuity: std::sync::atomic::AtomicBool,
}

impl ChaseInbox {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sample: std::sync::Mutex::new(VideoChaseBroadcast::default()),
            discontinuity: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn publish(&self, sample: VideoChaseBroadcast, discontinuity: bool) {
        if let Ok(mut slot) = self.sample.lock() {
            *slot = sample;
        }
        if discontinuity {
            self.discontinuity
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn take(&self, extra_discontinuity: bool) -> ChaseTransport {
        let sample = self.sample.lock().map(|g| *g).unwrap_or_default();
        let discontinuity = self
            .discontinuity
            .swap(false, std::sync::atomic::Ordering::AcqRel)
            || extra_discontinuity;
        ChaseTransport {
            position: sample.position,
            running: sample.running,
            discontinuity,
            fps: sample.fps,
        }
    }
}

impl Default for ChaseInbox {
    fn default() -> Self {
        Self::new()
    }
}

/// Inputs for one chase step. `transport_dt` is the change in transport
/// position since the previous chase tick (zero while stopped).
#[derive(Debug, Clone, Copy)]
pub struct ChaseInput {
    pub position: f64,
    pub in_point: f64,
    pub out_point: f64,
    pub frame_rate: f64,
    pub base_speed: f64,
    pub transport_position: f64,
    pub transport_dt: f64,
    pub transport_fps: f64,
    pub discontinuity: bool,
    pub sync: DeckTransportSync,
}

/// Mapped clip time for a transport instant, before clamping to `[in, out]`.
#[must_use]
pub fn desired_position(
    transport_position: f64,
    in_point: f64,
    base_speed: f64,
    offset: f64,
    delay_frames: i32,
    transport_fps: f64,
) -> f64 {
    let in_point = finite_non_negative(in_point, 0.0);
    let transport_position = finite_or(transport_position, 0.0);
    let base_speed = finite_or(base_speed, 1.0);
    let offset = finite_or(offset, 0.0);
    let fps = positive_rate_or_default(transport_fps);
    let delay = f64::from(delay_frames) / fps;
    let elapsed = transport_position - offset - delay;
    let mapped = if base_speed == 0.0 {
        in_point
    } else {
        in_point + elapsed * base_speed
    };
    if mapped.is_nan() {
        in_point
    } else {
        mapped
    }
}

/// One servo step. Loop mode is not consulted: hold the in/out bounds.
#[must_use]
pub fn step_chase(input: ChaseInput) -> ChaseStep {
    let ChaseInput {
        position,
        in_point,
        out_point,
        frame_rate,
        base_speed,
        transport_position,
        transport_dt,
        transport_fps,
        discontinuity,
        sync,
    } = input;

    let in_point = finite_non_negative(in_point, 0.0);
    let held_position = finite_non_negative(position, in_point).max(in_point);
    let out_pt = if out_point.is_finite() {
        out_point.max(in_point)
    } else {
        held_position
    };
    let position = held_position.clamp(in_point, out_pt);
    let frame_time = 1.0 / positive_rate_or_default(frame_rate);
    let base_speed = finite_or(base_speed, 1.0);

    // A transport position that is not a place cannot move the clip. The
    // transport rejects this upstream too, but the decode thread is a
    // show-critical boundary and must remain safe if called directly.
    if !transport_position.is_finite() {
        return ChaseStep {
            needs_seek: false,
            frames_to_decode: 0,
            position,
        };
    }

    let desired = desired_position(
        transport_position,
        in_point,
        base_speed,
        sync.offset,
        sync.delay_frames,
        transport_fps,
    );
    let target = desired.clamp(in_point, out_pt);
    let error = target - position;

    if discontinuity || error.abs() >= SEEK_THRESHOLD_SECS {
        return ChaseStep {
            needs_seek: true,
            frames_to_decode: 1,
            position: target,
        };
    }

    if error.abs() < frame_time * DEADBAND_FRAMES {
        return ChaseStep {
            needs_seek: false,
            frames_to_decode: 0,
            position: target,
        };
    }

    let factor = (1.0 + GAIN * error).clamp(1.0 - TRIM_CLAMP, 1.0 + TRIM_CLAMP);
    let dt = finite_or(transport_dt, 0.0).clamp(-0.1, 0.1);
    let mut next = position + dt * base_speed * factor;
    if error > 0.0 {
        next = next.min(target);
    } else {
        next = next.max(target);
    }
    next = next.clamp(in_point, out_pt);
    ChaseStep {
        needs_seek: false,
        frames_to_decode: 1,
        position: next,
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn finite_non_negative(value: f64, fallback: f64) -> f64 {
    finite_or(value, fallback).max(0.0)
}

fn positive_rate_or_default(rate: f64) -> f64 {
    if rate.is_finite() && rate > f64::EPSILON {
        rate
    } else {
        30.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::value::video::TransportSyncMode;

    fn input(position: f64, transport: f64) -> ChaseInput {
        ChaseInput {
            position,
            in_point: 0.0,
            out_point: 10.0,
            frame_rate: 30.0,
            base_speed: 1.0,
            transport_position: transport,
            transport_dt: 1.0 / 30.0,
            transport_fps: 30.0,
            discontinuity: false,
            sync: DeckTransportSync::default(),
        }
    }

    #[test]
    fn offset_places_in_point_on_the_transport() {
        let desired = desired_position(3600.0, 2.0, 1.0, 3600.0, 0, 30.0);
        assert!((desired - 2.0).abs() < 1e-9);
    }

    #[test]
    fn delay_frames_use_transport_fps_not_clip_fps() {
        // 30 transport frames = 1 s, clip in-point stays put relative to that.
        let with_delay = desired_position(10.0, 0.0, 1.0, 0.0, 30, 30.0);
        assert!((with_delay - 9.0).abs() < 1e-9);
    }

    #[test]
    fn base_speed_scales_mapped_time() {
        let desired = desired_position(2.0, 1.0, 2.0, 0.0, 0, 30.0);
        assert!((desired - 5.0).abs() < 1e-9);
    }

    #[test]
    fn outside_range_holds_the_boundary() {
        let mut i = input(0.0, 50.0);
        i.out_point = 10.0;
        let out = step_chase(i);
        assert!(out.needs_seek);
        assert!((out.position - 10.0).abs() < 1e-9);
    }

    #[test]
    fn before_offset_holds_in_point() {
        let mut i = input(2.0, 0.0);
        i.in_point = 2.0;
        i.sync.offset = 8.0;
        let out = step_chase(i);
        assert!((out.position - 2.0).abs() < 1e-9);
    }

    #[test]
    fn deadband_does_not_seek() {
        let frame = 1.0 / 30.0;
        let out = step_chase(input(1.0, 1.0 + frame * 0.25));
        assert!(!out.needs_seek);
        assert_eq!(out.frames_to_decode, 0);
    }

    #[test]
    fn half_second_error_seeks() {
        let out = step_chase(input(0.0, 0.6));
        assert!(out.needs_seek);
        assert!((out.position - 0.6).abs() < 1e-9);
    }

    #[test]
    fn discontinuity_always_seeks() {
        let mut i = input(1.0, 1.05);
        i.discontinuity = true;
        let out = step_chase(i);
        assert!(out.needs_seek);
        assert!((out.position - 1.05).abs() < 1e-9);
    }

    #[test]
    fn trim_band_does_not_seek() {
        let out = step_chase(input(0.0, 0.2));
        assert!(!out.needs_seek);
        assert!(out.position > 0.0);
        assert!(out.position < 0.2 + 1e-9);
    }

    #[test]
    fn scripted_series_converges_without_seeking() {
        let frame = 1.0 / 30.0;
        let mut position = 0.0;
        let mut transport = 0.2;

        for _ in 0..120 {
            let mut tick = input(position, transport);
            tick.transport_dt = frame;
            let out = step_chase(tick);
            assert!(!out.needs_seek, "ordinary drift correction must not seek");
            position = out.position;
            transport += frame;
        }

        assert!(
            (transport - frame - position).abs() < frame,
            "servo did not converge: transport={}, clip={position}",
            transport - frame
        );
    }

    #[test]
    fn trim_is_clamped_to_twenty_percent_of_base() {
        let mut tick = input(2.50, 1.49);
        tick.transport_dt = 0.1;
        tick.base_speed = 2.0;
        // desired = 2.98, error = 0.48, and the gain produces a 1.192 factor.
        let out = step_chase(tick);
        assert!(!out.needs_seek);
        assert!((out.position - 2.7384).abs() < 1e-9);
    }

    #[test]
    fn threshold_is_inclusive() {
        let out = step_chase(input(0.0, SEEK_THRESHOLD_SECS));
        assert!(out.needs_seek);
    }

    #[test]
    fn signed_delay_moves_the_target_both_ways() {
        let early = desired_position(10.0, 0.0, 1.0, 0.0, 30, 30.0);
        let late = desired_position(10.0, 0.0, 1.0, 0.0, -30, 30.0);
        assert!((early - 9.0).abs() < 1e-9);
        assert!((late - 11.0).abs() < 1e-9);
    }

    #[test]
    fn zero_speed_holds_the_in_point_even_at_extreme_transport_time() {
        let desired = desired_position(f64::MAX, 2.0, 0.0, 0.0, i32::MIN, 24.0);
        assert_eq!(desired, 2.0);
    }

    #[test]
    fn inbox_keeps_latest_sample_and_consumes_discontinuity_once() {
        let inbox = ChaseInbox::new();
        inbox.publish(
            VideoChaseBroadcast {
                position: 1.0,
                running: true,
                fps: 25.0,
            },
            true,
        );
        inbox.publish(
            VideoChaseBroadcast {
                position: 2.0,
                running: false,
                fps: 30.0,
            },
            false,
        );

        let first = inbox.take(false);
        assert_eq!(first.position, 2.0);
        assert!(!first.running);
        assert_eq!(first.fps, 30.0);
        assert!(first.discontinuity);
        assert!(!inbox.take(false).discontinuity);
    }

    #[test]
    fn residency_wake_forces_a_discontinuity() {
        let inbox = ChaseInbox::new();
        assert!(inbox.take(true).discontinuity);
    }

    #[test]
    fn auto_is_default() {
        assert_eq!(DeckTransportSync::default().mode, TransportSyncMode::Auto);
        assert!(TransportSyncMode::Auto.is_chasing(true));
        assert!(!TransportSyncMode::Auto.is_chasing(false));
        assert!(TransportSyncMode::Always.is_chasing(false));
        assert!(!TransportSyncMode::Never.is_chasing(true));
    }
}
