//! Linear Timecode: SMPTE as an audio signal.
//!
//! Eighty bits per frame, biphase-mark coded: every bit period starts with a
//! transition, and a `1` has a second one in the middle. The signal is
//! self-clocking and polarity-independent. No maintained crate decodes it, so
//! the decoder and a test encoder live here.

use super::TimecodeFrame;
use crate::transport::TimecodeRate;

/// Sync word: the low sixteen bits of the shift register after a frame's last
/// bit. `0011111111111101` in transmission order.
const SYNC: u128 = 0x3FFD;

const FRAME_BITS: u32 = 80;

/// Transitions further apart than this many bit periods are a gap (signal
/// stopped, or not timecode).
const GAP_BIT_PERIODS: f64 = 2.5;

/// Smoothing on the bit-period estimate. Low, since the period only changes
/// with the master's rate.
const PERIOD_ALPHA: f64 = 0.1;

/// Crossing threshold as a fraction of the running peak. Rejects hum and
/// quantization noise at both line and mic level.
const CROSSING_FRACTION: f32 = 0.25;

/// Peak decay per sample, so the threshold follows an attenuated signal down.
const PEAK_DECAY: f32 = 0.9999;

/// Decodes samples into frames. Use one per input channel and feed it that
/// channel's samples in order; it yields a frame at each sync word.
#[derive(Debug)]
pub struct LtcDecoder {
    sample_rate: f64,
    /// Rate to report, or `None` to infer it from the signal's cadence.
    rate_override: Option<TimecodeRate>,
    /// Sign of the last committed level (Schmitt trigger).
    high: bool,
    peak: f32,
    samples_since_transition: f64,
    /// Estimate of one bit period, in samples. `None` until the first interval.
    bit_period: Option<f64>,
    /// A half period is waiting for its partner; together they make a `1`.
    half_pending: bool,
    bits: u128,
    /// Bits since the last reset, so a partial frame is not decoded.
    filled: u32,
    /// Samples since the last sync word, and the smoothed distance between the
    /// last two. Averaged over eighty bits, a much better rate estimate than
    /// the bit period.
    samples_since_sync: f64,
    frame_period: Option<f64>,
    /// Whether that count started at a real sync word, not at the first sample
    /// or after a gap.
    synced: bool,
}

impl LtcDecoder {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate,
            rate_override: None,
            high: false,
            peak: 0.0,
            samples_since_transition: 0.0,
            bit_period: None,
            half_pending: false,
            bits: 0,
            filled: 0,
            samples_since_sync: 0.0,
            frame_period: None,
            synced: false,
        }
    }

    /// Reports frames at this rate instead of inferring one. 29.97 non-drop and
    /// 30 differ by 0.1% in the signal and not at all in the labels, but by 3.6 s
    /// per hour in position; no decoder can tell them apart.
    pub fn set_rate_override(&mut self, rate: Option<TimecodeRate>) {
        self.rate_override = rate;
    }

    /// Feeds one channel's samples, calling `on_frame` for each decoded frame.
    /// A callback, not a `Vec`, because this is on the frame path and most
    /// buffers yield nothing.
    pub fn feed(&mut self, samples: &[f32], mut on_frame: impl FnMut(TimecodeFrame)) {
        for &sample in samples {
            // Treat non-finite samples as silence. An infinity would pin the
            // peak (and threshold) at infinity for good.
            let sample = if sample.is_finite() { sample } else { 0.0 };
            self.peak = (self.peak * PEAK_DECAY).max(sample.abs());
            self.samples_since_transition += 1.0;
            self.samples_since_sync += 1.0;

            let threshold = self.peak * CROSSING_FRACTION;
            let crossed = if self.high {
                sample < -threshold
            } else {
                sample > threshold
            };
            if !crossed {
                continue;
            }
            self.high = !self.high;
            let interval = std::mem::take(&mut self.samples_since_transition);
            if let Some(bit) = self.bit_from(interval)
                && let Some(frame) = self.push(bit)
            {
                on_frame(frame);
            }
        }
    }

    /// Classifies the interval between two transitions. A full period is a `0`;
    /// two half periods are a `1`, so the first half yields nothing.
    fn bit_from(&mut self, interval: f64) -> Option<bool> {
        let Some(period) = self.bit_period else {
            // No estimate yet: assume the first interval is a whole bit. A wrong
            // guess costs the frames before the next sync.
            self.bit_period = Some(interval);
            return None;
        };

        if interval > period * GAP_BIT_PERIODS {
            // Silence, or not timecode. Start again.
            self.reset();
            self.bit_period = Some(interval);
            return None;
        }

        if interval > period * 0.75 {
            self.bit_period = Some(smooth(period, interval));
            self.half_pending = false;
            Some(false)
        } else if self.half_pending {
            self.bit_period = Some(smooth(period, interval * 2.0));
            self.half_pending = false;
            Some(true)
        } else {
            self.half_pending = true;
            None
        }
    }

    /// Shifts a bit in and decodes when the sync word lands.
    fn push(&mut self, bit: bool) -> Option<TimecodeFrame> {
        self.bits = (self.bits << 1) | u128::from(bit);
        self.filled = (self.filled + 1).min(FRAME_BITS);
        if self.filled < FRAME_BITS || self.bits & 0xFFFF != SYNC {
            return None;
        }
        // A frame ended; don't decode across the boundary.
        self.filled = 0;
        // Sync to sync is one frame, but only if the previous sync was real.
        let elapsed = std::mem::take(&mut self.samples_since_sync);
        let measured = self.synced.then_some(elapsed);
        self.synced = true;
        self.frame_period = match (self.frame_period, measured) {
            (Some(previous), Some(m)) if m < previous * 1.5 => Some(smooth(previous, m)),
            (_, Some(m)) => Some(m),
            (previous, None) => previous,
        };

        // Without two sync words there is no cadence to read the rate from, and
        // a guessed rate gives a wrong position. Locking costs one frame, which
        // the freewheel covers.
        if measured.is_none() && self.rate_override.is_none() {
            return None;
        }
        decode(
            self.bits,
            self.rate_override.or_else(|| self.inferred_rate()),
        )
        // A frame's word spans that frame, with the sync word at its end, so
        // the address just read is a frame old. Add one (SMPTE 12M LTC timing;
        // MTC adds two for the same reason).
        .map(|frame| frame.plus_frames(1))
    }

    /// The rate implied by how often the sync word arrives. The signal only
    /// states drop-frame, so the rate comes from cadence, measured frame to
    /// frame to average over eighty bits.
    ///
    /// 29.97 non-drop is 0.1% from 30 with identical labels, which clock drift
    /// hides; that case needs the override. A wrong choice shows as the
    /// playhead jumping once a second.
    fn inferred_rate(&self) -> Option<TimecodeRate> {
        let period = self.frame_period?;
        if period <= 0.0 {
            return None;
        }
        let fps = self.sample_rate / period;
        if (self.bits >> (79 - 10)) & 1 == 1 {
            return Some(TimecodeRate::Fps2997Drop);
        }
        Some(match fps {
            f if f < 24.5 => TimecodeRate::Fps24,
            f if f < 27.5 => TimecodeRate::Fps25,
            _ => TimecodeRate::Fps30,
        })
    }

    fn reset(&mut self) {
        self.filled = 0;
        self.half_pending = false;
        // The next sync closes an interval spanning the gap; don't measure it.
        self.synced = false;
    }
}

fn smooth(previous: f64, measured: f64) -> f64 {
    PERIOD_ALPHA * measured + (1.0 - PERIOD_ALPHA) * previous
}

/// Decodes the time address from a complete 80-bit frame. Bit `i` in
/// transmission order arrived `79 - i` shifts ago; fields are little-endian BCD.
fn decode(bits: u128, rate: Option<TimecodeRate>) -> Option<TimecodeFrame> {
    let field = |start: u32, width: u32| -> u8 {
        (0..width).fold(0u8, |acc, i| {
            let bit = (bits >> (79 - (start + i))) & 1;
            acc | ((bit as u8) << i)
        })
    };

    let frames = field(0, 4) + field(8, 2) * 10;
    let seconds = field(16, 4) + field(24, 3) * 10;
    let minutes = field(32, 4) + field(40, 3) * 10;
    let hours = field(48, 4) + field(56, 2) * 10;
    let drop = field(10, 1) == 1;

    let rate = rate.unwrap_or(if drop {
        TimecodeRate::Fps2997Drop
    } else {
        TimecodeRate::Fps30
    });
    // If the rate disagrees with the drop flag, trust the flag: it is the only
    // rate information the signal carries.
    let rate = match (drop, rate.is_drop_frame()) {
        (true, false) => TimecodeRate::Fps2997Drop,
        (false, true) => TimecodeRate::Fps30,
        _ => rate,
    };

    (hours < 24 && minutes < 60 && seconds < 60 && u64::from(frames) < rate.nominal_fps())
        .then(|| TimecodeFrame::new(hours, minutes, seconds, frames, rate))
}

#[cfg(test)]
pub(crate) mod encode {
    //! A biphase-mark encoder for tests, built from the standard so the decoder
    //! isn't tested against its own assumptions.

    use super::{FRAME_BITS, SYNC};
    use crate::timecode::TimecodeFrame;

    /// The eighty bits of one frame, in transmission order.
    pub(super) fn frame_bits(frame: TimecodeFrame) -> Vec<bool> {
        let mut bits = vec![false; FRAME_BITS as usize];
        let mut put = |start: usize, width: usize, value: u8| {
            for i in 0..width {
                bits[start + i] = (value >> i) & 1 == 1;
            }
        };
        put(0, 4, frame.frames % 10);
        put(8, 2, frame.frames / 10);
        put(10, 1, u8::from(frame.rate.is_drop_frame()));
        put(16, 4, frame.seconds % 10);
        put(24, 3, frame.seconds / 10);
        put(32, 4, frame.minutes % 10);
        put(40, 3, frame.minutes / 10);
        put(48, 4, frame.hours % 10);
        put(56, 2, frame.hours / 10);
        for i in 0..16 {
            bits[64 + i] = (SYNC >> (15 - i)) & 1 == 1;
        }

        // Polarity correction: keep an even number of zeroes so every frame
        // starts on the same edge. Bit 27 at every rate except 25 fps, where
        // it is bit 59. The decoder ignores it; it is set so the fixture
        // matches the standard.
        let slot = if frame.rate == crate::transport::TimecodeRate::Fps25 {
            59
        } else {
            27
        };
        if bits.iter().filter(|bit| !**bit).count() % 2 == 1 {
            bits[slot] = true;
        }
        bits
    }

    /// One frame of LTC audio at `sample_rate`, starting from `level`.
    /// `amplitude` and `sample_rate` vary in tests to cover mic level and 44.1 kHz.
    pub fn frame(
        tc: TimecodeFrame,
        sample_rate: f64,
        amplitude: f32,
        level: &mut bool,
    ) -> Vec<f32> {
        let samples_per_bit = sample_rate / (tc.rate.fps() * f64::from(FRAME_BITS));
        let mut out = Vec::with_capacity(samples_per_bit as usize * FRAME_BITS as usize + 1);
        let mut written = 0.0_f64;

        for (index, bit) in frame_bits(tc).into_iter().enumerate() {
            // Every bit period starts with a transition; a one has another in
            // the middle.
            *level = !*level;
            let start = f64::from(index as u32) * samples_per_bit;
            let middle = start + samples_per_bit / 2.0;
            let end = start + samples_per_bit;

            while written < middle {
                out.push(if *level { amplitude } else { -amplitude });
                written += 1.0;
            }
            if bit {
                *level = !*level;
            }
            while written < end {
                out.push(if *level { amplitude } else { -amplitude });
                written += 1.0;
            }
        }
        out
    }

    /// A run of consecutive frames starting at `first`.
    pub fn run(first: TimecodeFrame, count: usize, sample_rate: f64, amplitude: f32) -> Vec<f32> {
        let mut level = false;
        let mut out = Vec::new();
        for i in 0..i64::try_from(count).unwrap_or(i64::MAX) {
            let tc = first.plus_frames(i);
            out.extend(frame(tc, sample_rate, amplitude, &mut level));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(samples: &[f32], sample_rate: f64) -> Vec<TimecodeFrame> {
        let mut decoder = LtcDecoder::new(sample_rate);
        let mut out = Vec::new();
        decoder.feed(samples, |frame| out.push(frame));
        out
    }

    fn decoded_told(
        samples: &[f32],
        sample_rate: f64,
        rate: Option<TimecodeRate>,
    ) -> Vec<TimecodeFrame> {
        let mut decoder = LtcDecoder::new(sample_rate);
        decoder.set_rate_override(rate);
        let mut out = Vec::new();
        decoder.feed(samples, |frame| out.push(frame));
        out
    }

    /// 29.97 non-drop and 30 send identical labels, so the patch can set the
    /// rate, and that setting must change what the address means.
    #[test]
    fn telling_the_reader_the_rate_changes_what_the_same_address_means() {
        let sent = TimecodeFrame::new(0, 59, 59, 0, TimecodeRate::Fps2997);
        let audio = encode::run(sent, 10, 48_000.0, 0.5);

        let guessed = decoded(&audio, 48_000.0);
        let told = decoded_told(&audio, 48_000.0, Some(TimecodeRate::Fps2997));
        assert!(!guessed.is_empty() && !told.is_empty(), "both decoded");

        assert!(
            guessed.iter().all(|f| f.rate == TimecodeRate::Fps30),
            "unaided, the cadence reads as 30: {guessed:?}"
        );
        assert!(
            told.iter().all(|f| f.rate == TimecodeRate::Fps2997),
            "told the rate, it keeps it: {told:?}"
        );
        // A known rate locks a frame sooner, so align the runs at their ends
        // before comparing labels.
        let labels = |frames: &[TimecodeFrame]| {
            frames
                .iter()
                .map(|f| (f.hours, f.minutes, f.seconds, f.frames))
                .collect::<Vec<_>>()
        };
        let (guessed_labels, told_labels) = (labels(&guessed), labels(&told));
        let shared = guessed_labels.len().min(told_labels.len());
        assert_eq!(
            guessed_labels[guessed_labels.len() - shared..],
            told_labels[told_labels.len() - shared..],
            "the labels are identical, which is the whole problem"
        );

        // An hour of labels is an hour plus 3.6 s at the slower rate.
        let last = |frames: &[TimecodeFrame]| frames[frames.len() - 1].position();
        let drift = last(&told) - last(&guessed);
        assert!(
            (drift - 3.6).abs() < 0.05,
            "an hour in, the two readings should be 3.6s apart, got {drift}"
        );
    }

    /// Non-timecode audio (mic, click track, half a jack) must not decode to an
    /// address.
    #[test]
    fn noise_is_never_read_as_a_position() {
        use proptest::prelude::*;

        proptest!(|(samples in proptest::collection::vec(-2.0f32..2.0, 0..6000))| {
            let mut decoder = LtcDecoder::new(48_000.0);
            let mut frames = Vec::new();
            decoder.feed(&samples, |frame| frames.push(frame));

            for frame in frames {
                prop_assert!(
                    frame.hours < 24 && frame.minutes < 60 && frame.seconds < 60,
                    "invented {frame:?}"
                );
                prop_assert!(u64::from(frame.frames) < frame.rate.nominal_fps());
                let at = frame.position();
                prop_assert!(at.is_finite() && at >= 0.0, "invented position {at}");
            }
        });
    }

    /// Non-finite samples (stuck converter, pulled jack) must not leave the
    /// reader deaf once the signal returns.
    #[test]
    fn a_poisoned_buffer_does_not_deafen_the_reader() {
        for poison in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut decoder = LtcDecoder::new(48_000.0);
            decoder.feed(&[poison; 1024], |_| {});

            let sent = TimecodeFrame::new(1, 0, 0, 0, TimecodeRate::Fps25);
            let mut frames = Vec::new();
            decoder.feed(&encode::run(sent, 12, 48_000.0, 0.5), |frame| {
                frames.push(frame);
            });

            assert!(
                !frames.is_empty(),
                "{poison} left the reader deaf to a signal that came back"
            );
        }
    }

    /// Clipped and near-silent signals both still decode.
    #[test]
    fn a_signal_that_clips_or_barely_registers_still_reads() {
        let sent = TimecodeFrame::new(1, 0, 0, 0, TimecodeRate::Fps25);
        for amplitude in [0.001, 0.01, 1.0, 40.0] {
            let frames = decoded(&encode::run(sent, 12, 48_000.0, amplitude), 48_000.0);
            assert!(
                frames.len() >= 6,
                "at amplitude {amplitude} only {} frames read",
                frames.len()
            );
        }
    }

    /// Where cadence and the drop-frame flag disagree, the flag wins; an hour
    /// of drop-frame read as 30 is 3.6 s out.
    #[test]
    fn the_drop_flag_in_the_signal_overrules_a_guessed_rate() {
        let word = |frame: TimecodeFrame| {
            encode::frame_bits(frame)
                .into_iter()
                .enumerate()
                .fold(0u128, |acc, (i, bit)| acc | (u128::from(bit) << (79 - i)))
        };

        let dropped = TimecodeFrame::new(1, 2, 3, 4, TimecodeRate::Fps2997Drop);
        assert_eq!(
            decode(word(dropped), Some(TimecodeRate::Fps30)).map(|f| f.rate),
            Some(TimecodeRate::Fps2997Drop),
            "the flag is set, whatever the cadence looked like"
        );

        let straight = TimecodeFrame::new(1, 2, 3, 4, TimecodeRate::Fps30);
        assert_eq!(
            decode(word(straight), Some(TimecodeRate::Fps2997Drop)).map(|f| f.rate),
            Some(TimecodeRate::Fps30),
            "and an unset flag is just as much a statement"
        );
    }

    /// Round trip at every rate, including drop-frame.
    #[test]
    fn a_generated_signal_decodes_to_the_frames_it_was_built_from() {
        for rate in [
            TimecodeRate::Fps24,
            TimecodeRate::Fps25,
            TimecodeRate::Fps2997Drop,
            TimecodeRate::Fps30,
        ] {
            let first = TimecodeFrame::new(1, 22, 33, 4, rate);
            let sent: Vec<TimecodeFrame> = (0..10).map(|i| first.plus_frames(i)).collect();
            let frames = decoded(&encode::run(first, sent.len(), 48_000.0, 0.5), 48_000.0);

            // The reader reports one frame past the address in the bits.
            let expected: Vec<TimecodeFrame> = sent.iter().map(|f| f.plus_frames(1)).collect();

            // Three frames are lost at the edges: two to acquiring the bit
            // period and cadence, and the last has no transition to close its
            // final bit.
            assert!(
                frames.len() >= 6,
                "{rate:?}: expected most of ten frames, got {}",
                frames.len()
            );
            let at = expected
                .iter()
                .position(|f| *f == frames[0])
                .unwrap_or_else(|| panic!("{rate:?}: decoded {frames:?}, sent {sent:?}"));
            assert_eq!(
                frames,
                expected[at..at + frames.len()],
                "{rate:?}: consecutive frames, in the order they were sent"
            );
        }
    }

    /// A frame's word spans the frame, so a reader reporting the address it
    /// just read runs a frame (33 ms) behind the master.
    #[test]
    fn a_word_read_at_its_end_reports_the_frame_that_started_there() {
        let first = TimecodeFrame::new(10, 0, 0, 0, TimecodeRate::Fps25);
        let frames = decoded(&encode::run(first, 6, 48_000.0, 0.5), 48_000.0);
        let read = *frames.first().expect("a frame");

        // The decoder reports the frame after the word it finished reading.
        let word = (0..6)
            .map(|i| first.plus_frames(i))
            .find(|sent| sent.plus_frames(1) == read)
            .unwrap_or_else(|| panic!("{read:?} is not one frame past any word sent"));
        assert_eq!(read, word.plus_frames(1));
    }

    /// LTC is transitions, not levels, so inverted polarity decodes the same.
    #[test]
    fn an_inverted_signal_decodes_the_same() {
        let first = TimecodeFrame::new(0, 1, 2, 3, TimecodeRate::Fps25);
        let audio = encode::run(first, 5, 48_000.0, 0.5);
        let inverted: Vec<f32> = audio.iter().map(|s| -s).collect();

        assert_eq!(decoded(&audio, 48_000.0), decoded(&inverted, 48_000.0));
    }

    /// Quiet signals decode, since the trigger is relative to the running peak.
    #[test]
    fn a_quiet_signal_still_decodes() {
        let first = TimecodeFrame::new(2, 0, 0, 0, TimecodeRate::Fps25);
        let audio = encode::run(first, 10, 48_000.0, 0.02);
        let frames = decoded(&audio, 48_000.0);

        assert!(
            frames.len() >= 6,
            "a quiet cable is still a cable, got {} frames",
            frames.len()
        );
    }

    /// 44.1 kHz is not a whole number of samples per bit at any frame rate.
    #[test]
    fn a_sample_rate_that_does_not_divide_evenly_still_decodes() {
        let first = TimecodeFrame::new(0, 0, 10, 0, TimecodeRate::Fps2997Drop);
        let audio = encode::run(first, 8, 44_100.0, 0.5);
        let frames = decoded(&audio, 44_100.0);

        assert!(frames.len() >= 5, "got {} frames", frames.len());
        assert_eq!(frames[frames.len() - 1].rate, TimecodeRate::Fps2997Drop);
    }

    /// A gap (silence, a re-patched cable) doesn't decode as a position; the
    /// reader resumes at the next clean frame.
    #[test]
    fn a_gap_in_the_signal_does_not_invent_a_frame() {
        let first = TimecodeFrame::new(1, 0, 0, 0, TimecodeRate::Fps25);
        let mut audio = encode::run(first, 6, 48_000.0, 0.5);
        audio.extend(std::iter::repeat_n(0.0_f32, 4_000));
        let resumed = TimecodeFrame::new(1, 0, 30, 0, TimecodeRate::Fps25);
        audio.extend(encode::run(resumed, 6, 48_000.0, 0.5));

        let frames = decoded(&audio, 48_000.0);
        assert!(
            frames.iter().all(|f| f.seconds == 0 || f.seconds >= 30),
            "no frame was invented across the gap: {frames:?}"
        );
        let last = frames.last().copied().expect("a frame after the gap");
        assert_eq!(last.seconds, 30, "and the reader picked up again");
    }

    /// The encoder keeps an even zero count, so every word starts on the same
    /// edge. The decoder doesn't read this, so it is checked here.
    #[test]
    fn a_generated_word_carries_the_polarity_the_standard_requires() {
        for rate in TimecodeRate::ALL {
            for frames in 0..8 {
                let tc = TimecodeFrame::new(3, 14, 15, frames, rate);
                let zeroes = encode::frame_bits(tc).iter().filter(|b| !**b).count();
                assert_eq!(zeroes % 2, 0, "{rate:?} at frame {frames}: {zeroes} zeroes");
            }
        }
    }

    /// A frame with impossible fields is dropped.
    #[test]
    fn a_corrupted_frame_is_dropped_rather_than_believed() {
        // Hours field of 39.
        let mut bits: u128 = 0;
        for i in 0..64 {
            bits = (bits << 1) | u128::from(matches!(i, 48 | 49 | 50 | 56 | 57));
        }
        bits = (bits << 16) | SYNC;
        assert_eq!(decode(bits, Some(TimecodeRate::Fps25)), None);
    }
}
