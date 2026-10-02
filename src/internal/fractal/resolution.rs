//! Dynamic resolution: the render scale that holds a target frame rate.

/// The lowest render scale the governor goes to.
pub const FLOOR: f64 = 0.35;

/// Holds a render scale that keeps frames near a target time.
#[derive(Debug, Clone, Default)]
pub struct Governor {
    live: Option<f64>,
    smoothed: Option<f64>,
}

/// Weight of the newest frame in the smoothed frame time.
const SMOOTHING: f64 = 0.1;
/// How far from the target the smoothed time may be before the scale moves.
const BAND: f64 = 0.05;
/// The most the scale changes in one frame, as a fraction.
const MAX_STEP: f64 = 0.03;

impl Governor {
    /// Take the time the last frame took and return the scale for the next.
    /// `target_fps` 0 turns the governor off: the scale is the ceiling.
    pub fn step(&mut self, frame_seconds: f64, target_fps: f64, ceiling: f64) -> f64 {
        let ceiling = ceiling.clamp(FLOOR, 1.0);
        let live = self.live.unwrap_or(ceiling).min(ceiling);
        if target_fps <= 0.0 {
            self.smoothed = None;
            self.live = Some(ceiling);
            return ceiling;
        }
        if !(frame_seconds.is_finite() && frame_seconds > 0.0) {
            self.live = Some(live);
            return live;
        }
        let smoothed = self
            .smoothed
            .map_or(frame_seconds, |s| s + (frame_seconds - s) * SMOOTHING);
        self.smoothed = Some(smoothed);
        let target = 1.0 / target_fps;
        let mut next = live;
        if (smoothed - target).abs() > BAND * target {
            // Frame time follows the pixel count, the square of the scale.
            let wanted = live * (target / smoothed).sqrt();
            next = wanted.clamp(live * (1.0 - MAX_STEP), live * (1.0 + MAX_STEP));
        }
        let next = next.clamp(FLOOR, ceiling);
        self.live = Some(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames whose time is `cost` seconds at scale 1 and falls with the
    /// pixel count. Returns the scale after `frames` frames.
    fn run(
        governor: &mut Governor,
        cost: f64,
        target_fps: f64,
        ceiling: f64,
        frames: usize,
    ) -> f64 {
        let mut live = governor.step(1.0 / 60.0, target_fps, ceiling);
        for _ in 0..frames {
            live = governor.step(cost * live * live, target_fps, ceiling);
        }
        live
    }

    #[test]
    fn off_or_under_budget_holds_the_ceiling() {
        assert_eq!(run(&mut Governor::default(), 0.08, 0.0, 1.0, 300), 1.0);
        assert_eq!(run(&mut Governor::default(), 0.01, 50.0, 0.8, 300), 0.8);
    }

    #[test]
    fn converges_to_the_scale_that_meets_the_target() {
        // 80 ms at full scale against 20 ms: a quarter of the pixels.
        let live = run(&mut Governor::default(), 0.08, 50.0, 1.0, 400);
        assert!((live - 0.5).abs() < 0.03, "{live}");
    }

    #[test]
    fn stays_between_the_floor_and_the_ceiling() {
        assert_eq!(run(&mut Governor::default(), 10.0, 50.0, 1.0, 400), FLOOR);
        let mut governor = Governor::default();
        run(&mut governor, 0.01, 50.0, 1.0, 50);
        assert_eq!(
            governor.step(0.001, 50.0, 0.6),
            0.6,
            "a lowered ceiling applies at once"
        );
    }

    #[test]
    fn holds_steady_once_settled() {
        let mut governor = Governor::default();
        let settled = run(&mut governor, 0.08, 50.0, 1.0, 400);
        let mut live = settled;
        for _ in 0..200 {
            live = governor.step(0.08 * live * live, 50.0, 1.0);
        }
        assert_eq!(settled, live);
    }

    #[test]
    fn moves_at_most_three_percent_a_frame() {
        let mut governor = Governor::default();
        let mut live = governor.step(1.0 / 60.0, 50.0, 1.0);
        for _ in 0..100 {
            let next = governor.step(1.0, 50.0, 1.0);
            assert!(next >= live * 0.97 - 1e-12, "{live} -> {next}");
            live = next;
        }
    }
}
