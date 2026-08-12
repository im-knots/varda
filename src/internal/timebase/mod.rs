//! Which notion of "now" a consumer follows.
//!
//! Varda runs several independent clocks: wall time since engine start, musical
//! time from [`crate::clock::ClockManager`], and (from the transport onward)
//! absolute show position. A [`Timebase`] names one of them, a [`TimeContext`]
//! is one of them resolved for the current frame, and a [`TimebaseSet`] is all
//! of them resolved together so a consumer can pick per item without re-reading
//! any source.
//!
//! See /spec/timebase.md.

use serde::{Deserialize, Serialize};

/// Which notion of time a consumer follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub enum Timebase {
    /// Wall clock since engine start. The default, and the behaviour every
    /// modulation source had before timebases existed.
    #[default]
    FreeRun,
    /// Musical time in beats, derived from the resolved clock. An LFO at
    /// `frequency = 1.0` completes one cycle per beat.
    Beat,
    /// Absolute show position in seconds, read from the transport. Frozen until
    /// the transport has run, which is what keeps a cold start honest.
    Transport,
}

impl Timebase {
    /// Label used by the UI and by API responses.
    pub fn label(self) -> &'static str {
        match self {
            Timebase::FreeRun => "Free",
            Timebase::Beat => "Beat",
            Timebase::Transport => "Show",
        }
    }

    /// Every variant, in the order the UI presents them.
    pub const ALL: [Timebase; 3] = [Timebase::FreeRun, Timebase::Beat, Timebase::Transport];
}

/// The transport sampled for one frame.
///
/// `None` in [`TimebaseInput`] means the transport is not yet a usable position
/// source (it has never run), which freezes every transport-locked consumer.
#[derive(Debug, Clone, Copy)]
pub struct TransportSample {
    /// Absolute position in seconds, at full precision.
    pub position: f64,
    pub running: bool,
    pub discontinuity: bool,
}

/// One timebase resolved for the current frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeContext {
    /// Position in the timebase's natural unit: seconds for `FreeRun`, beats
    /// for `Beat`.
    pub time: f32,
    /// Delta since the previous frame in the same unit. Zero when stopped.
    pub dt: f32,
    /// False when the underlying source is stopped or unavailable.
    pub running: bool,
    /// True for one frame after a jump, so consumers that integrate can react.
    pub discontinuity: bool,
}

impl Default for TimeContext {
    fn default() -> Self {
        Self {
            time: 0.0,
            dt: 0.0,
            running: false,
            discontinuity: false,
        }
    }
}

/// All timebases resolved once per frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimebaseSet {
    free_run: TimeContext,
    beat: TimeContext,
    transport: TimeContext,
    transport_position: f64,
}

impl TimebaseSet {
    /// Build a set directly. Used by tests and by callers that already hold
    /// resolved contexts; the engine goes through [`TimebaseResolver`].
    pub fn new(free_run: TimeContext, beat: TimeContext, transport: TimeContext) -> Self {
        Self {
            free_run,
            beat,
            transport,
            transport_position: f64::from(transport.time),
        }
    }

    /// A set where every timebase reports the same free-running seconds. Used
    /// by call sites that have no clock (benchmarks, headless tests).
    pub fn free_running(time: f32, dt: f32) -> Self {
        let ctx = TimeContext {
            time,
            dt,
            running: true,
            discontinuity: false,
        };
        Self {
            free_run: ctx,
            beat: ctx,
            transport: ctx,
            transport_position: f64::from(time),
        }
    }

    pub fn get(&self, tb: Timebase) -> &TimeContext {
        match tb {
            Timebase::FreeRun => &self.free_run,
            Timebase::Beat => &self.beat,
            Timebase::Transport => &self.transport,
        }
    }

    /// Show position at full precision.
    ///
    /// `TimeContext::time` narrows to `f32`, which is ample for an oscillator
    /// but not for resolving an automation breakpoint an hour into a show. Kept
    /// off `TimeContext` deliberately: that struct is copied per source in the
    /// modulation loop, and widening it would cost the hot path for a value
    /// almost nothing reads. See /spec/timebase.md § Precision.
    pub fn transport_position(&self) -> f64 {
        self.transport_position
    }

    /// The free-run context, which is always available. Sources that ignore
    /// their timebase (envelope followers, ADSR) read this regardless of what
    /// they are set to.
    pub fn free_run(&self) -> &TimeContext {
        &self.free_run
    }
}

/// Per-frame input to [`TimebaseResolver::resolve`].
#[derive(Debug, Clone, Copy)]
pub struct TimebaseInput {
    /// Wall-clock seconds since engine start. Always available.
    pub free_run_time: f32,
    /// Monotonic beat count from the resolved clock, or `None` when no clock
    /// source is active.
    pub beat_time: Option<f64>,
    /// The transport, or `None` until it has run. See [`TransportSample`].
    pub transport: Option<TransportSample>,
}

/// Turns raw per-frame samples into a [`TimebaseSet`], carrying the last known
/// position of each source so an unavailable timebase freezes rather than
/// snapping to zero.
#[derive(Debug, Default)]
pub struct TimebaseResolver {
    free_run: TimeContext,
    beat: TimeContext,
    transport: TimeContext,
    transport_position: f64,
}

impl TimebaseResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve every timebase for this frame. Call once per frame, before any
    /// consumer reads time.
    pub fn resolve(&mut self, input: TimebaseInput) -> TimebaseSet {
        self.free_run = TimeContext {
            time: input.free_run_time,
            dt: input.free_run_time - self.free_run.time,
            running: true,
            discontinuity: false,
        };

        self.beat = match input.beat_time {
            Some(beats) => {
                // Beats are accumulated as f64 so a long show does not lose
                // resolution; the delta is small and narrows safely.
                let time = beats as f32;
                TimeContext {
                    time,
                    dt: if self.beat.running {
                        time - self.beat.time
                    } else {
                        0.0
                    },
                    running: true,
                    // A clock coming back after a dropout is a jump, not a
                    // smooth advance, so integrating consumers are told.
                    discontinuity: !self.beat.running,
                }
            }
            // Hold the last beat position. Freezing is honest: a modulator
            // locked to a clock that has gone away holds its look instead of
            // silently reverting to free-run.
            None => TimeContext {
                dt: 0.0,
                running: false,
                discontinuity: false,
                ..self.beat
            },
        };

        self.transport = match input.transport {
            Some(sample) => {
                let time = sample.position as f32;
                let ctx = TimeContext {
                    time,
                    // Derived from the f64 positions rather than the narrowed
                    // values, so a frame delta stays exact deep into a show.
                    dt: if self.transport.running {
                        (sample.position - self.transport_position) as f32
                    } else {
                        0.0
                    },
                    running: sample.running,
                    discontinuity: sample.discontinuity || !self.transport.running,
                };
                self.transport_position = sample.position;
                ctx
            }
            // The transport has never run. Freezing is what keeps a cold start
            // honest: a missing timecode cable holds the saved look rather than
            // driving everything to a pre-show value.
            None => TimeContext {
                dt: 0.0,
                running: false,
                discontinuity: false,
                ..self.transport
            },
        };

        TimebaseSet {
            free_run: self.free_run,
            beat: self.beat,
            transport: self.transport,
            transport_position: self.transport_position,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(r: &mut TimebaseResolver, t: f32, beats: Option<f64>) -> TimebaseSet {
        r.resolve(TimebaseInput {
            free_run_time: t,
            beat_time: beats,
            transport: None,
        })
    }

    fn resolve_transport(
        r: &mut TimebaseResolver,
        t: f32,
        transport: Option<TransportSample>,
    ) -> TimebaseSet {
        r.resolve(TimebaseInput {
            free_run_time: t,
            beat_time: None,
            transport,
        })
    }

    fn running_at(position: f64) -> TransportSample {
        TransportSample {
            position,
            running: true,
            discontinuity: false,
        }
    }

    #[test]
    fn free_run_always_runs_and_tracks_wall_time() {
        let mut r = TimebaseResolver::new();
        let set = resolve(&mut r, 1.5, None);
        let ctx = set.get(Timebase::FreeRun);
        assert!(ctx.running);
        assert!((ctx.time - 1.5).abs() < 1e-6);
        assert!((ctx.dt - 1.5).abs() < 1e-6);
    }

    #[test]
    fn free_run_dt_is_the_frame_delta() {
        let mut r = TimebaseResolver::new();
        resolve(&mut r, 1.0, None);
        let set = resolve(&mut r, 1.25, None);
        assert!((set.get(Timebase::FreeRun).dt - 0.25).abs() < 1e-6);
    }

    #[test]
    fn beat_tracks_the_clock_when_active() {
        let mut r = TimebaseResolver::new();
        resolve(&mut r, 0.0, Some(0.0));
        let set = resolve(&mut r, 0.5, Some(1.0));
        let ctx = set.get(Timebase::Beat);
        assert!(ctx.running);
        assert!((ctx.time - 1.0).abs() < 1e-6);
        assert!((ctx.dt - 1.0).abs() < 1e-6);
    }

    #[test]
    fn beat_freezes_when_the_clock_goes_away() {
        let mut r = TimebaseResolver::new();
        resolve(&mut r, 0.0, Some(0.0));
        resolve(&mut r, 0.5, Some(2.0));

        let set = resolve(&mut r, 1.0, None);
        let ctx = set.get(Timebase::Beat);
        assert!(!ctx.running, "an absent clock must not report running");
        assert!(
            (ctx.time - 2.0).abs() < 1e-6,
            "position must hold, not snap to zero"
        );
        assert!(
            (ctx.dt - 0.0).abs() < 1e-6,
            "a frozen timebase advances by 0"
        );
    }

    #[test]
    fn beat_never_falls_back_to_free_run() {
        let mut r = TimebaseResolver::new();
        let set = resolve(&mut r, 10.0, None);
        assert!((set.get(Timebase::Beat).time - 0.0).abs() < 1e-6);
        assert!(!set.get(Timebase::Beat).running);
        assert!(set.get(Timebase::FreeRun).running);
    }

    #[test]
    fn first_beat_frame_reports_a_discontinuity() {
        let mut r = TimebaseResolver::new();
        let set = resolve(&mut r, 0.0, Some(4.0));
        let ctx = set.get(Timebase::Beat);
        assert!(ctx.discontinuity);
        assert!(
            (ctx.dt - 0.0).abs() < 1e-6,
            "no delta is meaningful across the gap"
        );
    }

    #[test]
    fn clock_returning_after_a_dropout_is_a_discontinuity() {
        let mut r = TimebaseResolver::new();
        resolve(&mut r, 0.0, Some(0.0));
        resolve(&mut r, 0.1, Some(1.0));
        assert!(
            !resolve(&mut r, 0.2, Some(2.0))
                .get(Timebase::Beat)
                .discontinuity
        );

        resolve(&mut r, 0.3, None);
        let set = resolve(&mut r, 0.4, Some(9.0));
        assert!(set.get(Timebase::Beat).discontinuity);
    }

    #[test]
    fn free_running_helper_reports_every_timebase_as_running() {
        let set = TimebaseSet::free_running(2.0, 0.016);
        for tb in Timebase::ALL {
            assert!(set.get(tb).running);
            assert!((set.get(tb).time - 2.0).abs() < 1e-6);
        }
    }

    #[test]
    fn default_timebase_is_free_run() {
        assert_eq!(Timebase::default(), Timebase::FreeRun);
    }

    #[test]
    fn every_variant_is_offered_and_labelled() {
        assert_eq!(Timebase::ALL.len(), 3);
        for tb in Timebase::ALL {
            assert!(!tb.label().is_empty());
        }
        let mut labels: Vec<&str> = Timebase::ALL.iter().map(|t| t.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "labels must be distinguishable");
    }

    // ── Transport timebase ──────────────────────────────────────

    #[test]
    fn transport_tracks_show_position() {
        let mut r = TimebaseResolver::new();
        resolve_transport(&mut r, 0.0, Some(running_at(3600.0)));
        let set = resolve_transport(&mut r, 0.1, Some(running_at(3600.5)));

        let ctx = set.get(Timebase::Transport);
        assert!(ctx.running);
        assert!((ctx.time - 3600.5).abs() < 0.01);
        assert!((ctx.dt - 0.5).abs() < 1e-4);
    }

    /// A cold start with no transport must hold, not drive everything to a
    /// pre-show value. See /spec/transport.md § Engagement.
    #[test]
    fn transport_freezes_until_it_has_run() {
        let mut r = TimebaseResolver::new();
        let set = resolve_transport(&mut r, 5.0, None);
        let ctx = set.get(Timebase::Transport);
        assert!(!ctx.running);
        assert_eq!(ctx.time, 0.0);
        assert_eq!(ctx.dt, 0.0);
    }

    #[test]
    fn stopped_transport_holds_position() {
        let mut r = TimebaseResolver::new();
        resolve_transport(&mut r, 0.0, Some(running_at(10.0)));
        let set = resolve_transport(
            &mut r,
            0.1,
            Some(TransportSample {
                position: 10.0,
                running: false,
                discontinuity: false,
            }),
        );
        let ctx = set.get(Timebase::Transport);
        assert!(!ctx.running);
        assert!((ctx.time - 10.0).abs() < 1e-4);
    }

    #[test]
    fn transport_never_falls_back_to_wall_time() {
        let mut r = TimebaseResolver::new();
        let set = resolve_transport(&mut r, 42.0, None);
        assert_eq!(set.get(Timebase::Transport).time, 0.0);
        assert!(set.get(Timebase::FreeRun).running);
    }

    #[test]
    fn transport_locate_is_reported_as_a_discontinuity() {
        let mut r = TimebaseResolver::new();
        resolve_transport(&mut r, 0.0, Some(running_at(10.0)));
        assert!(
            !resolve_transport(&mut r, 0.1, Some(running_at(10.1)))
                .get(Timebase::Transport)
                .discontinuity
        );

        let set = resolve_transport(
            &mut r,
            0.2,
            Some(TransportSample {
                position: 900.0,
                running: true,
                discontinuity: true,
            }),
        );
        assert!(set.get(Timebase::Transport).discontinuity);
    }

    /// An hour into a show, `f32` seconds quantise to about 0.25 ms, which is
    /// fine for an oscillator and not fine for an automation breakpoint.
    #[test]
    fn full_precision_position_survives_a_long_show() {
        let mut r = TimebaseResolver::new();
        let deep = 36_000.123_456_789;
        let set = resolve_transport(&mut r, 0.0, Some(running_at(deep)));

        assert!((set.transport_position() - deep).abs() < 1e-9);
        assert!(
            (f64::from(set.get(Timebase::Transport).time) - deep).abs() > 1e-9,
            "the narrowed value is expected to lose precision; that is why the f64 exists"
        );
    }

    #[test]
    fn free_run_accessor_matches_get() {
        let mut r = TimebaseResolver::new();
        let set = resolve(&mut r, 3.0, Some(1.0));
        assert_eq!(*set.free_run(), *set.get(Timebase::FreeRun));
    }

    /// Scenes store the timebase by name, so renaming a variant would silently
    /// reinterpret saved shows.
    #[test]
    fn variants_serialize_by_name() {
        assert_eq!(
            serde_json::to_string(&Timebase::FreeRun).unwrap(),
            "\"FreeRun\""
        );
        assert_eq!(serde_json::to_string(&Timebase::Beat).unwrap(), "\"Beat\"");
        assert_eq!(
            serde_json::to_string(&Timebase::Transport).unwrap(),
            "\"Transport\""
        );
    }

    /// The names are a persisted format: a scene written by any build has to
    /// keep meaning the same thing, so a rename is a scene migration.
    #[test]
    fn every_variant_round_trips_through_its_name() {
        for timebase in Timebase::ALL {
            let json = serde_json::to_string(&timebase).unwrap();
            let back: Timebase = serde_json::from_str(&json).unwrap();
            assert_eq!(back, timebase, "{json}");
        }
    }

    /// A scene written before timebases existed has no field at all, and must
    /// open free-running rather than failing to parse.
    #[test]
    fn an_absent_timebase_reads_as_free_run() {
        #[derive(serde::Deserialize)]
        struct Holder {
            #[serde(default)]
            timebase: Timebase,
        }
        let holder: Holder = serde_json::from_str("{}").unwrap();
        assert_eq!(holder.timebase, Timebase::FreeRun);
    }
}
