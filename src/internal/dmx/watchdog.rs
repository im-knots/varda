//! Dark and stuck detection, per fixture.
//!
//! Catches the three things an operator actually troubleshoots at load-in: a dead fixture, a
//! wrong address, and a wedged modulator. Suppressed during blackout and other states where
//! stillness is correct, because a watchdog that cries during a deliberate blackout is one an
//! operator learns to ignore. See /spec/dmx-output.md § Observability.

use super::role::Role;
use super::values::RoleValues;
use std::time::{Duration, Instant};

/// A fixture at or near zero for longer than this is reported dark.
pub const DARK_AFTER: Duration = Duration::from_secs(6);
/// A fixture whose values have not moved for longer than this is reported stuck.
pub const STUCK_AFTER: Duration = Duration::from_secs(12);
/// Movement below this, in normalized units, does not count as a change. Roughly one 8-bit step.
const MOVEMENT_EPSILON: f32 = 1.5 / 255.0;
/// Total emitted light below this counts as dark.
const DARK_EPSILON: f32 = 2.0 / 255.0;

/// What the watchdog noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchdogKind {
    /// Emitting nothing for a long time, outside any state where that is expected.
    Dark,
    /// Not moving at all for a long time.
    Stuck,
}

/// One finding, naming the fixture by index into the resolved rig.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WatchdogFlag {
    pub fixture: usize,
    pub name: String,
    pub kind: WatchdogKind,
    pub age_secs: f32,
}

#[derive(Debug, Clone)]
struct FixtureHistory {
    last_lit: Instant,
    last_moved: Instant,
    previous: RoleValues,
}

/// Per-fixture history for dark and stuck detection.
#[derive(Debug, Default)]
pub struct FixtureWatchdog {
    history: Vec<FixtureHistory>,
}

impl FixtureWatchdog {
    #[must_use]
    pub fn new(fixture_count: usize) -> Self {
        let now = Instant::now();
        Self {
            history: vec![
                FixtureHistory {
                    last_lit: now,
                    last_moved: now,
                    previous: RoleValues::new(),
                };
                fixture_count
            ],
        }
    }

    /// Resize for a repatched rig, treating new fixtures as freshly seen.
    pub fn resize(&mut self, fixture_count: usize) {
        let now = Instant::now();
        self.history.resize(
            fixture_count,
            FixtureHistory {
                last_lit: now,
                last_moved: now,
                previous: RoleValues::new(),
            },
        );
    }

    /// Update history and report findings.
    ///
    /// `suppress` covers blackout and any other state where stillness is the intent. History
    /// still advances while suppressed, so releasing a blackout does not immediately trip every
    /// fixture at once.
    pub fn tick(
        &mut self,
        fixtures: &[(String, RoleValues)],
        now: Instant,
        suppress: bool,
    ) -> Vec<WatchdogFlag> {
        if self.history.len() != fixtures.len() {
            self.resize(fixtures.len());
        }

        for (i, (_, values)) in fixtures.iter().enumerate() {
            let h = &mut self.history[i];
            if emitted_light(values) > DARK_EPSILON {
                h.last_lit = now;
            }
            if moved(&h.previous, values) {
                h.last_moved = now;
                h.previous = *values;
            }
            if suppress {
                // Keep the clocks fresh so releasing a blackout does not instantly flag the rig.
                h.last_lit = now;
                h.last_moved = now;
            }
        }

        if suppress {
            return Vec::new();
        }

        let mut flags = Vec::new();
        for (i, (name, _)) in fixtures.iter().enumerate() {
            let h = &self.history[i];
            let dark_age = now.duration_since(h.last_lit);
            if dark_age > DARK_AFTER {
                flags.push(WatchdogFlag {
                    fixture: i,
                    name: name.clone(),
                    kind: WatchdogKind::Dark,
                    age_secs: dark_age.as_secs_f32(),
                });
            }
            let still_age = now.duration_since(h.last_moved);
            if still_age > STUCK_AFTER {
                flags.push(WatchdogFlag {
                    fixture: i,
                    name: name.clone(),
                    kind: WatchdogKind::Stuck,
                    age_secs: still_age.as_secs_f32(),
                });
            }
        }
        flags
    }
}

/// Total additive light a fixture is emitting, ignoring position and selection channels.
fn emitted_light(values: &RoleValues) -> f32 {
    let dimmer = values.get(Role::Dimmer).unwrap_or(1.0);
    let color: f32 = values
        .iter()
        .filter(|(r, _)| r.is_additive_color())
        .map(|(_, v)| v)
        .sum();
    // A fixture with a dimmer at zero is dark regardless of its color channels, which is how
    // a par with a separate master actually behaves.
    dimmer * color
}

fn moved(previous: &RoleValues, current: &RoleValues) -> bool {
    for (role, value) in current.iter() {
        match previous.get(role) {
            None => return true,
            Some(prev) => {
                if (value - prev).abs() > MOVEMENT_EPSILON {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit() -> RoleValues {
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 1.0);
        v.set(Role::Red, 1.0);
        v
    }

    fn dark() -> RoleValues {
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 0.0);
        v.set(Role::Red, 0.0);
        v
    }

    fn rig(values: RoleValues) -> Vec<(String, RoleValues)> {
        vec![("par-1".to_string(), values)]
    }

    #[test]
    fn a_lit_fixture_raises_nothing() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        assert!(w.tick(&rig(lit()), t0, false).is_empty());
    }

    #[test]
    fn a_dark_fixture_is_flagged_after_the_threshold() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let flags = w.tick(
            &rig(dark()),
            t0 + DARK_AFTER + Duration::from_secs(1),
            false,
        );
        assert!(
            flags.iter().any(|f| f.kind == WatchdogKind::Dark),
            "got {flags:?}"
        );
        assert_eq!(flags[0].name, "par-1", "a flag must name the fixture");
    }

    #[test]
    fn a_dark_fixture_is_not_flagged_before_the_threshold() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let flags = w.tick(&rig(dark()), t0 + Duration::from_secs(2), false);
        assert!(flags.is_empty(), "got {flags:?}");
    }

    #[test]
    fn a_motionless_fixture_is_flagged_stuck() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let flags = w.tick(
            &rig(lit()),
            t0 + STUCK_AFTER + Duration::from_secs(1),
            false,
        );
        assert!(
            flags.iter().any(|f| f.kind == WatchdogKind::Stuck),
            "got {flags:?}"
        );
    }

    #[test]
    fn movement_resets_the_stuck_clock() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let mut moved_values = lit();
        moved_values.set(Role::Red, 0.5);
        w.tick(
            &rig(moved_values),
            t0 + STUCK_AFTER.saturating_sub(Duration::from_secs(1)),
            false,
        );
        let flags = w.tick(
            &rig(lit()),
            t0 + STUCK_AFTER + Duration::from_secs(1),
            false,
        );
        assert!(
            !flags.iter().any(|f| f.kind == WatchdogKind::Stuck),
            "movement should have reset the clock: {flags:?}"
        );
    }

    #[test]
    fn sub_lsb_dither_does_not_count_as_movement() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let mut jittered = lit();
        jittered.set(Role::Red, 1.0 - 0.001);
        w.tick(&rig(jittered), t0 + Duration::from_secs(1), false);
        let flags = w.tick(
            &rig(lit()),
            t0 + STUCK_AFTER + Duration::from_secs(1),
            false,
        );
        assert!(
            flags.iter().any(|f| f.kind == WatchdogKind::Stuck),
            "a sub-LSB wobble is not movement: {flags:?}"
        );
    }

    /// A watchdog that cries during a deliberate blackout is one an operator learns to ignore.
    #[test]
    fn blackout_suppresses_flags() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let flags = w.tick(&rig(dark()), t0 + DARK_AFTER * 3, true);
        assert!(flags.is_empty(), "got {flags:?}");
    }

    /// Releasing a blackout must not instantly flag the whole rig as dark for the duration it
    /// was deliberately off.
    #[test]
    fn releasing_a_blackout_does_not_immediately_flag_everything() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        w.tick(&rig(lit()), t0, false);
        let during = t0 + DARK_AFTER * 3;
        w.tick(&rig(dark()), during, true);
        let flags = w.tick(&rig(dark()), during + Duration::from_secs(1), false);
        assert!(
            flags.is_empty(),
            "the dark clock must have been held during suppression: {flags:?}"
        );
    }

    #[test]
    fn a_dimmer_at_zero_reads_as_dark_despite_lit_color() {
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 0.0);
        v.set(Role::Red, 1.0);
        assert!(emitted_light(&v) <= DARK_EPSILON);
    }

    #[test]
    fn a_fixture_with_no_dimmer_channel_uses_its_color() {
        let mut v = RoleValues::new();
        v.set(Role::Red, 1.0);
        assert!(emitted_light(&v) > DARK_EPSILON);
    }

    #[test]
    fn position_and_wheel_channels_do_not_count_as_light() {
        let mut v = RoleValues::new();
        v.set(Role::Dimmer, 1.0);
        v.set(Role::Pan, 1.0);
        v.set(Role::ColorWheel, 1.0);
        assert!(
            emitted_light(&v) <= DARK_EPSILON,
            "a head pointed somewhere with its wheel set is still dark"
        );
    }

    #[test]
    fn resize_tracks_a_repatched_rig() {
        let mut w = FixtureWatchdog::new(1);
        let t0 = Instant::now();
        let two = vec![("a".to_string(), lit()), ("b".to_string(), lit())];
        let flags = w.tick(&two, t0, false);
        assert!(flags.is_empty());
        assert_eq!(w.history.len(), 2);
    }
}
