//! Modulation spread across a group.
//!
//! A sine chasing left to right across a truss is the single most used lighting effect in
//! existence, and it is the one thing Varda's modulation engine cannot currently express: a
//! modulator produces one value per frame, not one per fixture.
//!
//! Spread fans a modulator's phase across the fixtures of a group in patch order, which is what
//! a console's effects engine does. See /spec/lighting-routing.md § Lighting Deck Source Types.

use serde::{Deserialize, Serialize};

/// How phase is distributed across a group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpreadMode {
    /// Even ramp from the first member to the last. The plain chase.
    #[default]
    Linear,
    /// Ramps out from the centre to both ends, so the effect opens and closes symmetrically
    /// rather than sweeping. What a designer reaches for on a symmetric rig.
    Symmetric,
    /// Fixed pseudo-random offsets. Stable across frames and across runs, because an effect
    /// that reshuffles every frame is noise rather than a look.
    Random,
}

/// Phase offset in turns for one member of a group.
///
/// `index` is the member's position and `count` the group size. The result is in `0.0..1.0`
/// and is added to the modulator's own phase.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn offset(mode: SpreadMode, spread: f32, index: usize, count: usize) -> f32 {
    if count <= 1 || spread == 0.0 {
        return 0.0;
    }
    let last = (count - 1) as f32;
    let position = index as f32 / last; // 0.0 at the first member, 1.0 at the last
    let raw = match mode {
        SpreadMode::Linear => position,
        // Fold the ramp about the centre: ends together, middle opposite.
        SpreadMode::Symmetric => 1.0 - (position * 2.0 - 1.0).abs(),
        SpreadMode::Random => hash_unit(index),
    };
    (raw * spread).rem_euclid(1.0)
}

/// A stable pseudo-random value in `0.0..1.0` for a member index.
///
/// Deterministic on purpose: the same rig produces the same scatter every night, so a look an
/// operator liked in rehearsal is the look they get in the show.
#[allow(clippy::cast_precision_loss)]
fn hash_unit(index: usize) -> f32 {
    // A small integer hash (splitmix-style finalizer), then scaled into the unit interval.
    let mut x = index as u64 ^ 0x9E37_79B9_7F4A_7C15;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    // Keep 24 bits: plenty of distinct offsets, and exactly representable in f32.
    ((x & 0x00FF_FFFF) as f32) / (0x0100_0000_u32 as f32)
}

/// Spread configuration carried by a lighting deck.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    /// Turns of phase distributed across the group. 0 disables spread entirely; 1 fans a full
    /// cycle across the group, so the first and last members are back in phase.
    #[serde(default)]
    pub amount: f32,
    #[serde(default)]
    pub mode: SpreadMode,
}

impl Spread {
    #[must_use]
    pub fn offset_for(self, index: usize, count: usize) -> f32 {
        offset(self.mode, self.amount, index, count)
    }

    #[must_use]
    pub fn is_active(self) -> bool {
        self.amount != 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_spread_means_every_member_is_in_phase() {
        for i in 0..8 {
            assert_eq!(offset(SpreadMode::Linear, 0.0, i, 8), 0.0);
        }
    }

    #[test]
    fn a_single_member_group_has_no_offset() {
        assert_eq!(offset(SpreadMode::Linear, 1.0, 0, 1), 0.0);
    }

    #[test]
    fn an_empty_group_does_not_divide_by_zero() {
        assert_eq!(offset(SpreadMode::Linear, 1.0, 0, 0), 0.0);
    }

    #[test]
    fn linear_ramps_evenly_from_first_to_last() {
        let offsets: Vec<f32> = (0..5)
            .map(|i| offset(SpreadMode::Linear, 0.5, i, 5))
            .collect();
        assert!((offsets[0] - 0.0).abs() < 1e-6);
        assert!((offsets[4] - 0.5).abs() < 1e-6, "got {}", offsets[4]);
        for pair in offsets.windows(2) {
            assert!(pair[1] > pair[0], "must ramp monotonically: {offsets:?}");
        }
    }

    #[test]
    fn a_full_turn_of_spread_brings_the_ends_back_into_phase() {
        let first = offset(SpreadMode::Linear, 1.0, 0, 5);
        let last = offset(SpreadMode::Linear, 1.0, 4, 5);
        assert!(
            (first - last).abs() < 1e-6,
            "a full cycle across the group should wrap: {first} vs {last}"
        );
    }

    #[test]
    fn symmetric_matches_at_both_ends_and_peaks_in_the_middle() {
        let offsets: Vec<f32> = (0..5)
            .map(|i| offset(SpreadMode::Symmetric, 0.5, i, 5))
            .collect();
        assert!(
            (offsets[0] - offsets[4]).abs() < 1e-6,
            "the ends must move together: {offsets:?}"
        );
        assert!(
            offsets[2] > offsets[0] && offsets[2] > offsets[4],
            "the centre must be the extreme: {offsets:?}"
        );
    }

    /// An effect that reshuffles every frame is noise, not a look.
    #[test]
    fn random_is_stable_across_calls() {
        let first: Vec<f32> = (0..8)
            .map(|i| offset(SpreadMode::Random, 1.0, i, 8))
            .collect();
        for _ in 0..5 {
            let again: Vec<f32> = (0..8)
                .map(|i| offset(SpreadMode::Random, 1.0, i, 8))
                .collect();
            assert_eq!(first, again);
        }
    }

    #[test]
    fn random_actually_scatters() {
        let offsets: Vec<f32> = (0..16)
            .map(|i| offset(SpreadMode::Random, 1.0, i, 16))
            .collect();
        let mut sorted = offsets.clone();
        sorted.sort_by(f32::total_cmp);
        sorted.dedup();
        assert!(
            sorted.len() >= 14,
            "offsets should be well distributed, got {offsets:?}"
        );
        assert_ne!(offsets, {
            let mut ramp = offsets.clone();
            ramp.sort_by(f32::total_cmp);
            ramp
        });
    }

    #[test]
    fn every_offset_stays_inside_one_turn() {
        for mode in [
            SpreadMode::Linear,
            SpreadMode::Symmetric,
            SpreadMode::Random,
        ] {
            for amount in [0.25, 1.0, 3.0, 7.5] {
                for i in 0..12 {
                    let o = offset(mode, amount, i, 12);
                    assert!(
                        (0.0..1.0).contains(&o),
                        "{mode:?} amount {amount} index {i} gave {o}"
                    );
                }
            }
        }
    }

    #[test]
    fn spread_struct_delegates_and_reports_activity() {
        let inactive = Spread::default();
        assert!(!inactive.is_active());
        assert_eq!(inactive.offset_for(3, 8), 0.0);

        let active = Spread {
            amount: 0.5,
            mode: SpreadMode::Linear,
        };
        assert!(active.is_active());
        assert!((active.offset_for(8, 9) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn spread_round_trips_through_json() {
        let s = Spread {
            amount: 0.75,
            mode: SpreadMode::Symmetric,
        };
        let back: Spread = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn an_absent_spread_field_defaults_to_inactive() {
        let s: Spread = serde_json::from_str("{}").unwrap();
        assert!(!s.is_active());
        assert_eq!(s.mode, SpreadMode::Linear);
    }
}
