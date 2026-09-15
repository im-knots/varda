//! Universe assembly: slot writes into 512-byte frames.
//!
//! The patcher emits **slot writes**, not fixtures. Fixtures are one producer of those writes;
//! a pixel map (Phase 61) is another. Nothing below this layer learns which produced them.
//! See /spec/dmx-output.md § Pixel Mapping Readiness.

use std::collections::BTreeMap;

/// Slots in one DMX512 universe. Fixed by the standard.
pub const UNIVERSE_SIZE: usize = 512;

/// Universe identifier. The sACN range is 1..=63999; Art-Net's 15-bit Port-Address maps onto it.
pub type UniverseId = u16;

/// Lowest legal universe number.
pub const UNIVERSE_MIN: UniverseId = 1;
/// Highest legal universe number (ANSI E1.31 reserves 64000 and above).
pub const UNIVERSE_MAX: UniverseId = 63_999;

/// How many slots a channel occupies and where its fine byte lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// One slot, 0..=255.
    Eight,
    /// Two slots: this channel is the coarse byte, `fine_offset` slots away is the fine byte.
    ///
    /// The offset is relative to the coarse slot and is almost always positive, but Open Fixture
    /// Library mode orderings do not guarantee the fine channel follows the coarse one, so it is
    /// signed.
    SixteenCoarse { fine_offset: i16 },
}

/// Quantize a normalized value to 8 bits.
///
/// Clamps rather than wrapping: a modulator overshooting 1.0 must not wrap a dimmer to black.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn quantize_8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Quantize a normalized value to 16 bits.
///
/// Quantizing to `u8` and scaling back up is a bug, not an optimization: it discards the fine
/// byte entirely and makes a 16-bit pan as steppy as an 8-bit one. See /spec/dmx-output.md § C4.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn quantize_16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// A pending write of one normalized value to one channel of one universe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotWrite {
    pub universe: UniverseId,
    /// Zero-based slot index of the coarse byte.
    pub slot: u16,
    pub resolution: Resolution,
    pub value: f32,
}

/// A sparse set of universes, each a full 512-slot frame.
///
/// Sparse because only universes with at least one patched fixture are assembled or
/// transmitted: a show with fixtures in universes 1 and 40 must not emit 40 universes of zeroes.
/// `BTreeMap` rather than `HashMap` so iteration order is deterministic, which keeps transmit
/// order stable and makes the tests reproducible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UniverseSet {
    frames: BTreeMap<UniverseId, [u8; UNIVERSE_SIZE]>,
}

impl UniverseSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-create `universe` as all zeroes so it is transmitted even when nothing writes to it.
    ///
    /// Needed because a patched universe whose fixtures are all dark must keep transmitting a
    /// dark frame rather than disappearing from the set: a receiver that stops hearing a
    /// universe holds its last value until its data-loss timeout expires.
    pub fn ensure(&mut self, universe: UniverseId) {
        self.frames.entry(universe).or_insert([0u8; UNIVERSE_SIZE]);
    }

    /// Apply one slot write, creating the universe if needed.
    ///
    /// Writes that fall outside the universe are dropped rather than panicking. Patch validation
    /// ([`super::patch`]) rejects out-of-range addresses up front and is the place that reports
    /// them; this is the defensive floor beneath it.
    pub fn apply(&mut self, write: SlotWrite) {
        let frame = self
            .frames
            .entry(write.universe)
            .or_insert([0u8; UNIVERSE_SIZE]);
        let slot = write.slot as usize;
        match write.resolution {
            Resolution::Eight => {
                if slot < UNIVERSE_SIZE {
                    frame[slot] = quantize_8(write.value);
                }
            }
            Resolution::SixteenCoarse { fine_offset } => {
                let v = quantize_16(write.value);
                if slot < UNIVERSE_SIZE {
                    frame[slot] = (v >> 8) as u8;
                }
                // Checked rather than cast through i64: a fine offset can legitimately be
                // negative, and the sum must land inside the universe on any pointer width.
                if let Some(fine) = slot.checked_add_signed(isize::from(fine_offset))
                    && fine < UNIVERSE_SIZE
                {
                    frame[fine] = (v & 0xFF) as u8;
                }
            }
        }
    }

    pub fn apply_all<I: IntoIterator<Item = SlotWrite>>(&mut self, writes: I) {
        for w in writes {
            self.apply(w);
        }
    }

    #[must_use]
    pub fn get(&self, universe: UniverseId) -> Option<&[u8; UNIVERSE_SIZE]> {
        self.frames.get(&universe)
    }

    pub fn iter(&self) -> impl Iterator<Item = (UniverseId, &[u8; UNIVERSE_SIZE])> {
        self.frames.iter().map(|(k, v)| (*k, v))
    }

    #[must_use]
    pub fn ids(&self) -> Vec<UniverseId> {
        self.frames.keys().copied().collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Zero every slot of every universe already present, keeping the universe set intact.
    ///
    /// This is the blackout and shutdown frame: the universes must still be transmitted, they
    /// must just carry zeroes. See /spec/dmx-output.md § C6.
    pub fn blackout(&mut self) {
        for frame in self.frames.values_mut() {
            *frame = [0u8; UNIVERSE_SIZE];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(slot: u16, value: f32) -> SlotWrite {
        SlotWrite {
            universe: 1,
            slot,
            resolution: Resolution::Eight,
            value,
        }
    }

    #[test]
    fn quantize_8_covers_the_full_range() {
        assert_eq!(quantize_8(0.0), 0);
        assert_eq!(quantize_8(1.0), 255);
        assert_eq!(quantize_8(0.5), 128);
    }

    #[test]
    fn quantize_clamps_rather_than_wrapping() {
        // A modulator overshooting must not wrap a dimmer from full to black.
        assert_eq!(quantize_8(1.5), 255);
        assert_eq!(quantize_8(-0.5), 0);
        assert_eq!(quantize_16(2.0), 65535);
        assert_eq!(quantize_16(-1.0), 0);
    }

    #[test]
    fn quantize_16_covers_the_full_range() {
        assert_eq!(quantize_16(0.0), 0);
        assert_eq!(quantize_16(1.0), 65535);
    }

    #[test]
    fn eight_bit_write_lands_in_one_slot() {
        let mut set = UniverseSet::new();
        set.apply(w(0, 1.0));
        let f = set.get(1).unwrap();
        assert_eq!(f[0], 255);
        assert_eq!(f[1], 0);
    }

    #[test]
    fn sixteen_bit_write_splits_coarse_and_fine() {
        let mut set = UniverseSet::new();
        set.apply(SlotWrite {
            universe: 1,
            slot: 10,
            resolution: Resolution::SixteenCoarse { fine_offset: 1 },
            value: 1.0,
        });
        let f = set.get(1).unwrap();
        assert_eq!(f[10], 0xFF, "coarse");
        assert_eq!(f[11], 0xFF, "fine");
    }

    #[test]
    fn sixteen_bit_honours_a_non_adjacent_fine_channel() {
        // OFL mode orderings do not guarantee the fine channel follows the coarse one.
        let mut set = UniverseSet::new();
        set.apply(SlotWrite {
            universe: 1,
            slot: 4,
            resolution: Resolution::SixteenCoarse { fine_offset: 6 },
            value: 0.5,
        });
        let f = set.get(1).unwrap();
        let expect = quantize_16(0.5);
        assert_eq!(f[4], (expect >> 8) as u8);
        assert_eq!(f[10], (expect & 0xFF) as u8);
        assert_eq!(f[5], 0, "slot between coarse and fine is untouched");
    }

    /// Contract C4. A smooth ramp across a 16-bit channel must be monotonic in `u16`, and the
    /// coarse byte must never dwell for more than 256 fine steps. Quantizing through `u8` would
    /// give 256 flat plateaus here.
    #[test]
    fn sixteen_bit_ramp_is_monotonic_with_no_long_coarse_dwell() {
        const STEPS: u32 = 4096;
        let mut prev = 0u16;
        let mut dwell = 0u32;
        let mut max_dwell = 0u32;
        let mut prev_coarse = 0u8;
        let mut distinct = std::collections::BTreeSet::new();

        for i in 0..=STEPS {
            #[allow(clippy::cast_precision_loss)]
            let v = quantize_16(i as f32 / STEPS as f32);
            assert!(v >= prev, "ramp must be monotonic at step {i}");
            prev = v;
            distinct.insert(v);

            let coarse = (v >> 8) as u8;
            if coarse == prev_coarse {
                dwell += 1;
                max_dwell = max_dwell.max(dwell);
            } else {
                dwell = 0;
                prev_coarse = coarse;
            }
        }
        assert!(
            distinct.len() > 4000,
            "a 16-bit ramp must resolve far more than 256 distinct values, got {}",
            distinct.len()
        );
        assert!(
            max_dwell <= 256,
            "coarse byte dwelled {max_dwell} steps, which means the fine byte is not moving"
        );
    }

    #[test]
    fn out_of_range_slot_is_dropped_not_panicking() {
        let mut set = UniverseSet::new();
        set.apply(w(511, 1.0));
        set.apply(w(512, 1.0));
        set.apply(w(60_000, 1.0));
        let f = set.get(1).unwrap();
        assert_eq!(f[511], 255);
    }

    #[test]
    fn sixteen_bit_fine_landing_outside_the_universe_still_writes_coarse() {
        let mut set = UniverseSet::new();
        set.apply(SlotWrite {
            universe: 1,
            slot: 511,
            resolution: Resolution::SixteenCoarse { fine_offset: 1 },
            value: 1.0,
        });
        assert_eq!(set.get(1).unwrap()[511], 0xFF);
    }

    #[test]
    fn negative_fine_offset_writes_below_the_coarse_slot() {
        let mut set = UniverseSet::new();
        set.apply(SlotWrite {
            universe: 1,
            slot: 5,
            resolution: Resolution::SixteenCoarse { fine_offset: -1 },
            value: 1.0,
        });
        let f = set.get(1).unwrap();
        assert_eq!(f[5], 0xFF);
        assert_eq!(f[4], 0xFF);
    }

    #[test]
    fn universes_are_sparse_and_ordered() {
        let mut set = UniverseSet::new();
        set.apply(SlotWrite {
            universe: 40,
            slot: 0,
            resolution: Resolution::Eight,
            value: 1.0,
        });
        set.apply(w(0, 1.0));
        assert_eq!(set.ids(), vec![1, 40], "sparse and sorted");
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn ensure_creates_a_dark_universe_that_still_transmits() {
        let mut set = UniverseSet::new();
        set.ensure(7);
        assert_eq!(set.len(), 1);
        assert_eq!(set.get(7).unwrap()[0], 0);
    }

    #[test]
    fn ensure_does_not_clobber_existing_data() {
        let mut set = UniverseSet::new();
        set.apply(w(0, 1.0));
        set.ensure(1);
        assert_eq!(set.get(1).unwrap()[0], 255);
    }

    #[test]
    fn blackout_zeroes_values_but_keeps_universes() {
        let mut set = UniverseSet::new();
        set.apply(w(0, 1.0));
        set.apply(SlotWrite {
            universe: 5,
            slot: 3,
            resolution: Resolution::Eight,
            value: 1.0,
        });
        set.blackout();
        assert_eq!(set.len(), 2, "universes must survive a blackout");
        assert!(set.iter().all(|(_, f)| f.iter().all(|&b| b == 0)));
    }

    #[test]
    fn later_write_to_the_same_slot_wins() {
        let mut set = UniverseSet::new();
        set.apply_all([w(0, 1.0), w(0, 0.0)]);
        assert_eq!(set.get(1).unwrap()[0], 0);
    }
}
