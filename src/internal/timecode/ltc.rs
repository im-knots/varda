//! Linear Timecode: SMPTE as an audio signal.
//!
//! Eighty bits per frame, biphase-mark coded: every bit period begins with a
//! transition, and a `1` has a second one in the middle. That makes the signal
//! self-clocking and immune to a flipped cable, which is why it survives being
//! run down a mic line and through a mixer.
//!
//! Decoded here rather than by a crate: no maintained one exists, the test plan
//! needs the encoder below either way, and the jitter tolerance of a
//! show-critical input is worth owning. See /spec/timecode.md § LTC.

use super::TimecodeFrame;
use crate::transport::TimecodeRate;

/// Sync word, as the low sixteen bits of the shift register once the frame's
/// last bit has landed. `0011111111111101` in transmission order.
const SYNC: u128 = 0x3FFD;

/// Eighty bits, one frame.
const FRAME_BITS: u32 = 80;

/// A transition further apart than this many bit periods is a gap, not a bit:
/// the signal stopped, or something else is on the channel.
const GAP_BIT_PERIODS: f64 = 2.5;

/// Smoothing on the bit-period estimate. Low, because the period only moves
/// when the master changes rate.
const PERIOD_ALPHA: f64 = 0.1;

/// Where the level must cross, as a fraction of the running peak, before a
/// transition is called. Keeps hum and quantisation noise from clocking bits
/// while still following a signal that arrives at line level or at mic level.
const CROSSING_FRACTION: f32 = 0.25;

/// Peak decay per sample, so an attenuated signal is followed down rather than
/// measured against a level it had a minute ago.
const PEAK_DECAY: f32 = 0.9999;

/// Turns samples into frames.
///
/// One per input channel. Feed it the samples of that channel in order; it
/// yields a frame every time the sync word lands.
#[derive(Debug)]
pub struct LtcDecoder {
    sample_rate: f64,
    /// Rate to report, or `None` to infer it from the signal's cadence.
    rate_override: Option<TimecodeRate>,
    /// Sign of the last committed level, as a Schmitt trigger.
    high: bool,
    peak: f32,
    samples_since_transition: f64,
    /// Estimate of one bit period, in samples. `None` until the first interval.
    bit_period: Option<f64>,
    /// A half period is waiting for its partner, which together make a `1`.
    half_pending: bool,
    bits: u128,
    /// Bits seen since the last reset, so a partial frame is not decoded.
    filled: u32,
    /// Samples since the last sync word, and the smoothed distance between the
    /// last two. Eighty bits of averaging makes this a far better rate estimate
    /// than the bit period, which is what the decision below needs.
    samples_since_sync: f64,
    frame_period: Option<f64>,
    /// Whether the count above started at a sync word rather than at the first
    /// sample the decoder was ever handed, or at the far side of a gap.
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

    /// Report frames at this rate instead of inferring one.
    ///
    /// Worth exposing because 29.97 non-drop and 30 differ by one part in a
    /// thousand in the signal and not at all in the labels, while their
    /// positions differ by 3.6 seconds an hour. No decoder can tell them apart;
    /// the person who patched the cable can.
    pub fn set_rate_override(&mut self, rate: Option<TimecodeRate>) {
        self.rate_override = rate;
    }

    /// Feed one channel's samples, calling `on_frame` for each frame decoded.
    ///
    /// Takes a callback rather than returning a `Vec` because this runs on the
    /// frame path and a buffer usually yields nothing at all.
    pub fn feed(&mut self, samples: &[f32], mut on_frame: impl FnMut(TimecodeFrame)) {
        for &sample in samples {
            // A stuck converter or a half-dead driver hands over samples that
            // are not numbers. Read as silence rather than as level: an
            // infinity would set the crossing threshold to infinity, which
            // decays to infinity, and the reader would never hear a signal
            // again for the rest of the show.
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
            if let Some(bit) = self.bit_from(interval) {
                if let Some(frame) = self.push(bit) {
                    on_frame(frame);
                }
            }
        }
    }

    /// Classify the gap between two transitions.
    ///
    /// A full period is a `0`. Two half periods are a `1`, so the first half
    /// yields nothing and the second completes the bit.
    fn bit_from(&mut self, interval: f64) -> Option<bool> {
        let Some(period) = self.bit_period else {
            // Nothing to compare against yet: assume the first interval was a
            // whole bit. A wrong guess costs the frames before the next sync.
            self.bit_period = Some(interval);
            return None;
        };

        if interval > period * GAP_BIT_PERIODS {
            // Silence, or something that is not timecode. Start again.
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

    /// Shift a bit in, and decode when the sync word lands.
    fn push(&mut self, bit: bool) -> Option<TimecodeFrame> {
        self.bits = (self.bits << 1) | u128::from(bit);
        self.filled = (self.filled + 1).min(FRAME_BITS);
        if self.filled < FRAME_BITS || self.bits & 0xFFFF != SYNC {
            return None;
        }
        // A frame ended here, so the next one starts from nothing rather than
        // decoding across the boundary.
        self.filled = 0;
        // Sync to sync is a frame, but only if the previous sync was real: the
        // count since the decoder was handed its first sample is not one.
        let elapsed = std::mem::take(&mut self.samples_since_sync);
        let measured = self.synced.then_some(elapsed);
        self.synced = true;
        self.frame_period = match (self.frame_period, measured) {
            (Some(previous), Some(m)) if m < previous * 1.5 => Some(smooth(previous, m)),
            (_, Some(m)) => Some(m),
            (previous, None) => previous,
        };

        // Until two sync words have been seen there is no cadence to read the
        // rate from, and a frame at a guessed rate is a wrong position rather
        // than a late one. Locking costs one frame, which the freewheel covers.
        if measured.is_none() && self.rate_override.is_none() {
            return None;
        }
        decode(
            self.bits,
            self.rate_override.or_else(|| self.inferred_rate()),
        )
        // The word for a frame is transmitted *across* that frame: bit 0 lands
        // at its start and the sync word at its end, which is where we are
        // standing now. So the address just read is where the master was a
        // frame ago, and reporting it unadjusted would chase a frame behind.
        // Every reader corrects for this; SMPTE 12M § LTC timing, and see the
        // MTC assembler, which owes two frames for the same reason.
        .map(|frame| frame.plus_frames(1))
    }

    /// The rate implied by how often the sync word arrives.
    ///
    /// The signal states whether it is drop-frame but never what rate it runs
    /// at, so the rate comes from cadence. Measuring it frame to frame rather
    /// than bit to bit averages over eighty bits, which is the difference
    /// between telling 24 from 25 reliably and guessing.
    ///
    /// 29.97 non-drop is one part in a thousand away from 30 and carries
    /// identical labels, which no decoder can resolve through a master's own
    /// clock drift. That one is what the override is for; a wrong choice shows
    /// as the playhead jumping once a second, which is the standard diagnostic.
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
        // The next sync closes an interval that spans the gap, which is not a
        // frame period and must not be measured as one.
        self.synced = false;
    }
}

fn smooth(previous: f64, measured: f64) -> f64 {
    PERIOD_ALPHA * measured + (1.0 - PERIOD_ALPHA) * previous
}

/// Pull the time address out of a complete 80-bit frame.
///
/// Bit `i` in transmission order was received `79 - i` shifts ago, and every
/// field is little-endian BCD.
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
    // A rate that disagrees with the drop flag means the inference lost, and
    // the flag is the only thing the signal actually states.
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
    //! A biphase-mark encoder, for tests.
    //!
    //! Writing this was always required by the test plan: a decoder can only be
    //! trusted against a signal built from the standard rather than from the
    //! decoder's own assumptions.

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

        // Polarity correction: the word carries an even number of zeroes, so
        // every frame begins on the same edge and words can be spliced. Bit 27
        // holds it at every rate but 25 fps, where the slot is a group flag and
        // bit 59 does the job instead. Nothing here reads it; it is set because
        // a fixture that skipped it would not be the signal the standard
        // describes, and the point of owning the encoder is testing against the
        // standard rather than against our own decoder.
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
    ///
    /// `amplitude` and a slow sample rate are both worth testing: a decoder that
    /// only works at 48 kHz line level is a decoder that fails in the room.
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
            // the middle. That is the whole of biphase mark.
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

    /// 29.97 non-drop and 30 send identical labels one part in a thousand
    /// apart, which no reader can tell apart from a master's own clock drift.
    /// Guessing wrong costs 3.6 seconds an hour, so the patch can say which it
    /// is, and saying so has to actually change what the address means.
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
        // Told the rate, the reader locks a frame sooner, so the runs are
        // lined up at their ends before the labels are compared.
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

        // An hour of labels stands for an hour and 3.6 seconds of wall time at
        // the slower rate, which is the drift the override exists to avoid.
        let last = |frames: &[TimecodeFrame]| frames[frames.len() - 1].position();
        let drift = last(&told) - last(&guessed);
        assert!(
            (drift - 3.6).abs() < 0.05,
            "an hour in, the two readings should be 3.6s apart, got {drift}"
        );
    }

    /// An audio input carries whatever is plugged into it: a mic, a click
    /// track, half a jack. None of it is timecode, and an address invented out
    /// of noise is a show jumping somewhere nobody asked for.
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

    /// A stuck converter, a jack pulled mid-buffer or a driver handing over
    /// garbage all arrive as samples that are not numbers. Timecode has to come
    /// back when the signal does, rather than the reader staying deaf for the
    /// rest of the show.
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

    /// Amplitude is not ours to choose: a line output into a mic input clips,
    /// and a mic input into a line stage is nearly silent. Neither is a reason
    /// to stop reading timecode.
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

    /// The one thing an LTC word states about its own rate is whether it is
    /// drop-frame. Cadence has to guess the rest, so where the two disagree the
    /// signal wins: an hour of drop-frame read as 30 is 3.6 seconds out.
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

    /// The core of the feature. Drop-frame is in here deliberately: it is the
    /// easiest part to get wrong and the hardest to eyeball.
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

            // A word is read at its end, by which time the master has moved on
            // by the frame the word occupied, so the reader reports one frame
            // later than the address in the bits.
            let expected: Vec<TimecodeFrame> = sent.iter().map(|f| f.plus_frames(1)).collect();

            // Three frames are lost at the edges by construction: two to
            // acquiring the bit period and the frame cadence, and the last to
            // having no following transition to close its final bit with. A
            // signal that is actually running has neither edge.
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

    /// The word for a frame occupies that whole frame: bit 0 at its start, the
    /// sync word at its end. A reader that reports the address it just read,
    /// unadjusted, therefore runs a frame behind the master. Hardware readers
    /// all correct for this, and so must we, or every LTC show sits 33 ms late.
    #[test]
    fn a_word_read_at_its_end_reports_the_frame_that_started_there() {
        let first = TimecodeFrame::new(10, 0, 0, 0, TimecodeRate::Fps25);
        let frames = decoded(&encode::run(first, 6, 48_000.0, 0.5), 48_000.0);
        let read = *frames.first().expect("a frame");

        // Whichever word the decoder locked onto, it reported the frame after
        // it rather than the one whose bits it just finished reading.
        let word = (0..6)
            .map(|i| first.plus_frames(i))
            .find(|sent| sent.plus_frames(1) == read)
            .unwrap_or_else(|| panic!("{read:?} is not one frame past any word sent"));
        assert_eq!(read, word.plus_frames(1));
    }

    /// LTC is transitions, not levels, so a swapped pair on the cable decodes
    /// identically. Worth asserting: it is the reason the format survives being
    /// patched through anything.
    #[test]
    fn an_inverted_signal_decodes_the_same() {
        let first = TimecodeFrame::new(0, 1, 2, 3, TimecodeRate::Fps25);
        let audio = encode::run(first, 5, 48_000.0, 0.5);
        let inverted: Vec<f32> = audio.iter().map(|s| -s).collect();

        assert_eq!(decoded(&audio, 48_000.0), decoded(&inverted, 48_000.0));
    }

    /// A signal off a mic input, or one that has been through a mixer, arrives
    /// quiet. The trigger is relative to the running peak for this reason.
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

    /// 44.1 kHz is what half the interfaces in the world are set to, and it is
    /// not a whole number of samples per bit at any frame rate.
    #[test]
    fn a_sample_rate_that_does_not_divide_evenly_still_decodes() {
        let first = TimecodeFrame::new(0, 0, 10, 0, TimecodeRate::Fps2997Drop);
        let audio = encode::run(first, 8, 44_100.0, 0.5);
        let frames = decoded(&audio, 44_100.0);

        assert!(frames.len() >= 5, "got {} frames", frames.len());
        assert_eq!(frames[frames.len() - 1].rate, TimecodeRate::Fps2997Drop);
    }

    /// Silence between takes, or a cable pulled and replaced, must not decode
    /// as a position: the reader picks up again at the next clean frame.
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

    /// The generated signal is checked against the standard rather than against
    /// the decoder: an even zero count is what keeps every word starting on the
    /// same edge, and it is the one property of the format the decoder does not
    /// read and so could not catch.
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

    /// Noise on a long cable flips the odd sample. A frame whose fields are
    /// impossible is dropped rather than jumping the show.
    #[test]
    fn a_corrupted_frame_is_dropped_rather_than_believed() {
        // Hours field claims 39, which no clock reaches.
        let mut bits: u128 = 0;
        for i in 0..64 {
            bits = (bits << 1) | u128::from(matches!(i, 48 | 49 | 50 | 56 | 57));
        }
        bits = (bits << 16) | SYNC;
        assert_eq!(decode(bits, Some(TimecodeRate::Fps25)), None);
    }
}
