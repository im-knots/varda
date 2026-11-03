//! The show's frame rate, as the FPS display and the API report it.

use std::collections::VecDeque;

/// Frames the rate is counted over.
const WINDOW: usize = 60;

/// Frames per second over the last [`WINDOW`] frames: the frame count divided
/// by the time those frames took. Averaging each frame's `1 / dt` instead
/// overstates an uneven rate, since short frames weigh as much as long ones.
#[derive(Debug, Default)]
pub(crate) struct FrameRate {
    intervals: VecDeque<f32>,
}

impl FrameRate {
    /// Record one frame that took `dt` seconds. Non-positive intervals are
    /// ignored.
    pub(crate) fn record(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        if self.intervals.len() == WINDOW {
            self.intervals.pop_front();
        }
        self.intervals.push_back(dt);
    }

    /// Frames per second, or zero before any frame.
    pub(crate) fn fps(&self) -> f32 {
        let total: f32 = self.intervals.iter().sum();
        if total > 0.0 {
            self.intervals.len() as f32 / total
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(intervals: impl IntoIterator<Item = f32>) -> f32 {
        let mut rate = FrameRate::default();
        for dt in intervals {
            rate.record(dt);
        }
        rate.fps()
    }

    #[test]
    fn a_steady_rate_reads_as_itself() {
        assert!((rate(std::iter::repeat_n(1.0 / 60.0, 60)) - 60.0).abs() < 0.01);
    }

    /// Frames of 10 and 30 ms are 50 frames a second. Averaging `1 / dt`
    /// would read 66.7.
    #[test]
    fn an_uneven_rate_counts_frames_over_time() {
        let frames = [0.010, 0.030].into_iter().cycle().take(60);
        assert!((rate(frames) - 50.0).abs() < 0.01);
    }

    #[test]
    fn only_the_last_window_of_frames_counts() {
        let slow = std::iter::repeat_n(1.0 / 30.0, WINDOW);
        let fast = std::iter::repeat_n(1.0 / 60.0, WINDOW);
        assert!((rate(slow.chain(fast)) - 60.0).abs() < 0.01);
    }

    #[test]
    fn no_frames_read_zero_and_empty_intervals_are_ignored() {
        assert!(rate([]).abs() < f32::EPSILON);
        assert!((rate([0.0, -1.0, 0.020]) - 50.0).abs() < 0.01);
    }
}
