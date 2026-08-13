//! MIDI Time Code: eight quarter-frame messages, or one system-exclusive
//! locate.
//!
//! Quarter frames (`0xF1`) each carry one nibble of the time address, so a
//! position takes eight of them and arrives over two frames of show. The rate
//! is carried in the last one, which is why MTC needs no rate configuration
//! while LTC does.
//!
//! See /spec/timecode.md § MTC.

use super::TimecodeFrame;
use crate::transport::TimecodeRate;

/// Rebuilds a position from quarter-frame nibbles.
///
/// One of these per device: two masters on two ports are two conversations,
/// and interleaving their nibbles would assemble a time neither of them sent.
#[derive(Debug, Default, Clone)]
pub struct QuarterFrameAssembler {
    nibbles: [u8; 8],
    /// Which of the eight pieces have arrived since the last assembly.
    seen: u8,
    last_piece: Option<u8>,
}

impl QuarterFrameAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the data byte of an `F1` message, returning a position once eight
    /// consecutive pieces have made one.
    ///
    /// The sequence takes two frames to send, so a master running forwards is
    /// two frames further on by the time the last nibble lands, and the reader
    /// adds them back. Running backwards the sequence arrives in reverse and
    /// the assembled address is already where the master is.
    pub fn feed(&mut self, data: u8) -> Option<TimecodeFrame> {
        let piece = (data >> 4) & 0x07;
        let value = data & 0x0F;

        let ascending = match self.last_piece {
            Some(previous) if piece == (previous + 1) % 8 => true,
            Some(previous) if previous == (piece + 1) % 8 => false,
            // Any other step means pieces were lost or a locate cut in, so the
            // half-built address is abandoned rather than half-updated.
            Some(_) => {
                self.seen = 0;
                self.nibbles = [0; 8];
                self.last_piece = Some(piece);
                self.nibbles[piece as usize] = value;
                self.seen |= 1 << piece;
                return None;
            }
            None => true,
        };

        self.nibbles[piece as usize] = value;
        self.seen |= 1 << piece;
        self.last_piece = Some(piece);

        let complete =
            self.seen == 0xFF && ((ascending && piece == 7) || (!ascending && piece == 0));
        if !complete {
            return None;
        }
        self.seen = 0;

        let frame = self.assemble()?;
        if ascending {
            // Two frames of show elapsed while the eight pieces were sent.
            Some(frame.plus_frames(2))
        } else {
            Some(frame)
        }
    }

    fn assemble(&self) -> Option<TimecodeFrame> {
        let n = &self.nibbles;
        let frames = n[0] | (n[1] << 4);
        let seconds = n[2] | (n[3] << 4);
        let minutes = n[4] | (n[5] << 4);
        // The last piece is `0rrh`: two rate bits above the top hours bit.
        let hours = n[6] | ((n[7] & 0x01) << 4);
        let rate = rate_from_bits((n[7] >> 1) & 0x03);
        valid(hours, minutes, seconds, frames, rate)
    }
}

/// The `hh mm ss ff` of a full-frame locate, lifted out of its
/// system-exclusive wrapper by [`crate::midi`].
///
/// Masters send these when they jump, because eight quarter frames would take
/// two frames to say where they went.
pub fn full_frame(payload: [u8; 4]) -> Option<TimecodeFrame> {
    let [hours_byte, minutes, seconds, frames] = payload;
    valid(
        hours_byte & 0x1F,
        minutes,
        seconds,
        frames,
        rate_from_bits((hours_byte >> 5) & 0x03),
    )
}

/// Bits 5 and 6 of the hours byte, in both quarter-frame and full-frame form.
fn rate_from_bits(bits: u8) -> TimecodeRate {
    match bits {
        0 => TimecodeRate::Fps24,
        1 => TimecodeRate::Fps25,
        2 => TimecodeRate::Fps2997Drop,
        _ => TimecodeRate::Fps30,
    }
}

/// The rate bits a master sends for a rate, for tests and for a future sender.
pub fn rate_bits(rate: TimecodeRate) -> u8 {
    match rate {
        TimecodeRate::Fps24 => 0,
        TimecodeRate::Fps25 => 1,
        TimecodeRate::Fps2997Drop => 2,
        // MTC has no code for 29.97 non-drop; masters send it as 30.
        TimecodeRate::Fps2997 | TimecodeRate::Fps30 => 3,
    }
}

/// Reject an address that cannot exist, so a corrupted nibble is dropped rather
/// than jumping the show to hour 19.
fn valid(
    hours: u8,
    minutes: u8,
    seconds: u8,
    frames: u8,
    rate: TimecodeRate,
) -> Option<TimecodeFrame> {
    let sane = hours < 24 && minutes < 60 && seconds < 60 && u64::from(frames) < rate.nominal_fps();
    sane.then(|| TimecodeFrame::new(hours, minutes, seconds, frames, rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eight messages a master sends for one address, in order.
    fn quarter_frames(frame: TimecodeFrame) -> Vec<u8> {
        let hours = frame.hours | (rate_bits(frame.rate) << 5);
        let fields = [
            frame.frames & 0x0F,
            (frame.frames >> 4) & 0x0F,
            frame.seconds & 0x0F,
            (frame.seconds >> 4) & 0x0F,
            frame.minutes & 0x0F,
            (frame.minutes >> 4) & 0x0F,
            hours & 0x0F,
            (hours >> 4) & 0x0F,
        ];
        fields
            .iter()
            .enumerate()
            .map(|(piece, value)| ((piece as u8) << 4) | value)
            .collect()
    }

    fn feed_all(assembler: &mut QuarterFrameAssembler, messages: &[u8]) -> Option<TimecodeFrame> {
        let mut last = None;
        for byte in messages {
            last = assembler.feed(*byte).or(last);
        }
        last
    }

    /// The whole point: eight nibbles in, one position out, at the rate the
    /// master named.
    #[test]
    fn eight_nibbles_assemble_the_address_that_was_sent() {
        for rate in [
            TimecodeRate::Fps24,
            TimecodeRate::Fps25,
            TimecodeRate::Fps2997Drop,
            TimecodeRate::Fps30,
        ] {
            let sent = TimecodeFrame::new(1, 22, 33, 4, rate);
            let mut assembler = QuarterFrameAssembler::new();
            let got = feed_all(&mut assembler, &quarter_frames(sent)).expect("a position");

            // Two frames later than the address, because that is how long the
            // eight messages took to arrive.
            assert_eq!(got, sent.plus_frames(2), "{rate:?}");
        }
    }

    /// A master scrubbing backwards sends the sequence in reverse, and the
    /// address it completes on is where it already is.
    #[test]
    fn a_reversed_sequence_reads_as_the_address_it_completes_on() {
        let sent = TimecodeFrame::new(0, 10, 0, 12, TimecodeRate::Fps25);
        let mut messages = quarter_frames(sent);
        messages.reverse();

        let mut assembler = QuarterFrameAssembler::new();
        assert_eq!(feed_all(&mut assembler, &messages), Some(sent));
    }

    /// Nibbles lost to a busy bus must not assemble half of one address and
    /// half of the next: that would be a position the master never sent.
    #[test]
    fn a_gap_in_the_sequence_abandons_the_half_built_address() {
        let sent = TimecodeFrame::new(2, 0, 0, 0, TimecodeRate::Fps25);
        let messages = quarter_frames(sent);

        let mut assembler = QuarterFrameAssembler::new();
        // Pieces 0 and 1, then a jump to 5: the run is broken.
        assert_eq!(assembler.feed(messages[0]), None);
        assert_eq!(assembler.feed(messages[1]), None);
        assert_eq!(assembler.feed(messages[5]), None);
        assert_eq!(assembler.feed(messages[6]), None);
        assert_eq!(assembler.feed(messages[7]), None, "nothing is assembled");

        // A clean run afterwards still works.
        assert!(feed_all(&mut assembler, &messages).is_some());
    }

    /// A locate cannot wait two frames for eight nibbles, so masters send the
    /// whole address at once.
    #[test]
    fn a_locate_carries_the_whole_address() {
        // 0x41 = 0b010_00001: rate bits 10 (29.97 drop), hour 1.
        assert_eq!(
            full_frame([0x41, 0x02, 0x03, 0x04]),
            Some(TimecodeFrame::new(1, 2, 3, 4, TimecodeRate::Fps2997Drop))
        );
    }

    /// A flipped bit on the wire must not throw the show to an impossible
    /// address; dropping the frame costs one frame of freewheel instead.
    #[test]
    fn an_impossible_address_is_dropped() {
        assert_eq!(
            full_frame([0x01, 0x63, 0x03, 0x04]),
            None,
            "99 minutes is not a time"
        );
        assert_eq!(
            full_frame([0x21, 0x02, 0x03, 0x1D]),
            None,
            "frame 29 does not exist at 25 fps"
        );
        assert_eq!(
            full_frame([0x18, 0x02, 0x03, 0x04]),
            None,
            "the day ends at hour 23"
        );
        assert_eq!(
            full_frame([0x01, 0x02, 0x3C, 0x04]),
            None,
            "60 seconds is the next minute, not a second"
        );
    }

    /// The rate rides in the same two bits as the hour, and a locate read at
    /// the wrong rate lands the show at the wrong second. 29.97 non-drop has no
    /// code of its own, which is why the LTC patch carries an override.
    #[test]
    fn a_locate_carries_the_rate_it_was_sent_at() {
        for (rate, expected) in [
            (TimecodeRate::Fps24, TimecodeRate::Fps24),
            (TimecodeRate::Fps25, TimecodeRate::Fps25),
            (TimecodeRate::Fps2997Drop, TimecodeRate::Fps2997Drop),
            (TimecodeRate::Fps30, TimecodeRate::Fps30),
            (TimecodeRate::Fps2997, TimecodeRate::Fps30),
        ] {
            let hours_byte = 1 | (rate_bits(rate) << 5);
            assert_eq!(
                full_frame([hours_byte, 2, 3, 4]),
                Some(TimecodeFrame::new(1, 2, 3, 4, expected)),
                "{rate:?} on the wire"
            );
        }
    }
}
