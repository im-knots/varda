//! Absolute show position.
//!
//! The transport is the one position that arrangement regions, automation
//! envelopes, video chase, and the show runner all read. It either advances on
//! its own internal clock or chases incoming timecode, and because it can run
//! internally every position-locked feature works with no external hardware.
//!
//! This is deliberately *not* the tempo clock ([`crate::clock`]), which resolves
//! BPM and beat phase. Both can be active at once: an arrangement can run
//! against the transport while beat-synced modulators follow a DJ's MIDI clock.
//!
//! See /spec/transport.md.

use serde::{Deserialize, Serialize};

/// Frame rate used to display and quantise timecode positions.
///
/// Defined here rather than with the timecode receiver because the arrangement
/// ruler needs to render `HH:MM:SS:FF` before any timecode exists.
/// See /spec/arrangement.md § `TimecodeRate` ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub enum TimecodeRate {
    Fps24,
    Fps25,
    /// 29.97 non-drop. Frame numbers run 0–29 and drift against wall time.
    Fps2997,
    /// 29.97 drop-frame, the broadcast default: frame numbers skip to keep
    /// long-run agreement with wall time.
    #[default]
    Fps2997Drop,
    Fps30,
}

impl TimecodeRate {
    /// Frames per second as a rate, for converting positions to frame counts.
    pub fn fps(self) -> f64 {
        match self {
            TimecodeRate::Fps24 => 24.0,
            TimecodeRate::Fps25 => 25.0,
            TimecodeRate::Fps2997 | TimecodeRate::Fps2997Drop => 30000.0 / 1001.0,
            TimecodeRate::Fps30 => 30.0,
        }
    }

    /// Whether frame numbers are dropped to track wall time.
    pub fn is_drop_frame(self) -> bool {
        matches!(self, TimecodeRate::Fps2997Drop)
    }

    pub fn label(self) -> &'static str {
        match self {
            TimecodeRate::Fps24 => "24",
            TimecodeRate::Fps25 => "25",
            TimecodeRate::Fps2997 => "29.97",
            TimecodeRate::Fps2997Drop => "29.97 DF",
            TimecodeRate::Fps30 => "30",
        }
    }

    /// Format an absolute position as `HH:MM:SS:FF`.
    ///
    /// Drop-frame renumbers so the label tracks wall time, and is written with
    /// a `;` before the frames, which is the convention desks and players use
    /// to signal the distinction. Negative positions clamp to zero.
    pub fn format(self, position: f64) -> String {
        let (hours, minutes, seconds, frames) = self.label_parts(position);
        let sep = if self.is_drop_frame() { ';' } else { ':' };
        format!("{hours:02}:{minutes:02}:{seconds:02}{sep}{frames:02}")
    }

    /// The `HH`, `MM`, `SS`, `FF` a position is labelled with.
    ///
    /// Split out of [`Self::format`] because the timecode receiver needs the
    /// same arithmetic to *write* frames, and a decoder that disagreed with the
    /// display about drop-frame would be a bug nobody could see.
    /// See /spec/timecode.md § Data Model.
    pub fn label_parts(self, position: f64) -> (u8, u8, u8, u8) {
        let elapsed_frames = (position.max(0.0) * self.fps()).floor() as u64;
        let counted = if self.is_drop_frame() {
            Self::renumber_drop_frame(elapsed_frames)
        } else {
            elapsed_frames
        };

        // Label rate: 29.97 counts to 30 and lets the label drift against wall
        // time, which is exactly what non-drop means.
        let fps = self.nominal_fps();
        let (frames, total_seconds) = (counted % fps, counted / fps);
        (
            (total_seconds / 3600) as u8,
            ((total_seconds / 60) % 60) as u8,
            (total_seconds % 60) as u8,
            frames as u8,
        )
    }

    /// Frames per second as labels are counted: 30 for both 29.97 variants.
    pub fn nominal_fps(self) -> u64 {
        self.fps().round() as u64
    }

    /// SMPTE 12M drop-frame: skip frame numbers 0 and 1 at the start of every
    /// minute except every tenth, so the label stays within a frame of wall
    /// time despite counting at 30 while running at 29.97.
    fn renumber_drop_frame(elapsed_frames: u64) -> u64 {
        /// Frames actually elapsed in ten minutes at 29.97.
        const PER_TEN_MINUTES: u64 = 17_982;
        /// Frames actually elapsed in one minute at 29.97, after the first.
        const PER_MINUTE: u64 = 1_798;

        let ten_minute_blocks = elapsed_frames / PER_TEN_MINUTES;
        let within_block = elapsed_frames % PER_TEN_MINUTES;
        // The first two frames of a block are inside the un-dropped tenth
        // minute, so they contribute nothing.
        let dropped_in_block = within_block.saturating_sub(2) / PER_MINUTE * 2;
        elapsed_frames + 18 * ten_minute_blocks + dropped_in_block
    }

    pub const ALL: [TimecodeRate; 5] = [
        TimecodeRate::Fps24,
        TimecodeRate::Fps25,
        TimecodeRate::Fps2997,
        TimecodeRate::Fps2997Drop,
        TimecodeRate::Fps30,
    ];
}

/// Where the transport's position comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub enum TransportSource {
    /// Position advances locally on play/stop/locate. Scrubbing allowed.
    #[default]
    Internal,
    /// Position chases incoming timecode. Position is read-only.
    Timecode,
}

/// A range of show positions, in seconds, that internal playback wraps within.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoopRegion {
    pub start: f64,
    pub end: f64,
}

impl LoopRegion {
    /// # Errors
    ///
    /// Returns [`TransportError::EmptyLoopRegion`] if the range is empty or
    /// inverted.
    pub fn new(start: f64, end: f64) -> Result<Self, TransportError> {
        if end <= start {
            return Err(TransportError::EmptyLoopRegion);
        }
        Ok(Self { start, end })
    }

    pub fn span(self) -> f64 {
        self.end - self.start
    }
}

/// One read of an external timecode master, as the transport sees it.
///
/// A plain struct rather than the receiver's own state so the transport does
/// not depend on the timecode module: the transport is what everything reads,
/// and it should not know which protocol, if any, is behind the position.
/// See /spec/timecode.md § Consumer 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chase {
    pub position: f64,
    /// Frames are arriving, or the freewheel is still coasting.
    pub running: bool,
    /// The master jumped rather than played on.
    pub discontinuity: bool,
    /// Coasting through a dropout.
    pub freewheeling: bool,
    /// Measured against wall time; 1.0 while a master plays forwards.
    pub speed: f64,
}

/// Why the transport is or is not moving.
///
/// Idle and broken look identical on the output (both are "nothing is
/// happening"), so the reason has to be legible rather than inferred.
/// See /spec/transport.md § Legibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub enum TransportStatus {
    /// Internal, never started this session. The saved scene renders as-is.
    Idle,
    /// Armed to chase, but no timecode has arrived.
    WaitingForSignal,
    /// Position is advancing.
    Running,
    /// Coasting through a timecode dropout. Still running, but on the reader's
    /// extrapolation rather than on frames that arrived.
    Freewheeling,
    /// Ran and stopped. Position holds, so envelopes freeze.
    Stopped,
}

impl TransportStatus {
    pub fn label(self) -> &'static str {
        match self {
            TransportStatus::Idle => "Idle",
            TransportStatus::WaitingForSignal => "Waiting for signal",
            TransportStatus::Running => "Running",
            TransportStatus::Freewheeling => "Freewheeling",
            TransportStatus::Stopped => "Stopped",
        }
    }
}

/// Rejected transport operations, so a caller learns why rather than watching
/// nothing happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// Position is owned by the incoming timecode master.
    PositionIsReadOnly,
    /// A loop region must have a positive length.
    EmptyLoopRegion,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::PositionIsReadOnly => {
                write!(f, "transport is chasing timecode; position is read-only")
            }
            TransportError::EmptyLoopRegion => {
                write!(f, "loop region must end after it starts")
            }
        }
    }
}

impl std::error::Error for TransportError {}

/// The engine-owned show position.
// Four flags, but they are independent axes (advancing, ever-advanced, jumped,
// jump-pending) rather than a state that wants an enum: `running` and `has_run`
// disagree for exactly the window a play sits before its first tick, which is
// the distinction engagement depends on.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Transport {
    position: f64,
    running: bool,
    has_run: bool,
    rate: f64,
    discontinuity: bool,
    /// A jump raised since the last tick. Held separately because a locate
    /// arrives during command processing, which is before the frame's tick;
    /// clearing on tick alone would drop it before anything read it.
    pending_discontinuity: bool,
    timecode_rate: TimecodeRate,
    source: TransportSource,
    /// Coasting through a timecode dropout. Only ever true while chasing.
    freewheeling: bool,
    loop_region: Option<LoopRegion>,
    /// Frame time of the previous [`Transport::update`], for `tick`'s dt.
    last_update: Option<std::time::Instant>,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            position: 0.0,
            running: false,
            has_run: false,
            rate: 1.0,
            discontinuity: false,
            pending_discontinuity: false,
            timecode_rate: TimecodeRate::default(),
            source: TransportSource::default(),
            freewheeling: false,
            loop_region: None,
            last_update: None,
        }
    }
}

impl Transport {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Reads ───────────────────────────────────────────────────

    /// Absolute position in seconds. `f64` because shows conventionally start
    /// at hour 1, where `f32` would quantise to about 0.4 ms.
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// Whether the transport has advanced at least once this session.
    ///
    /// This is the entire basis of arrangement engagement: until it is true, the
    /// arrangement stays inert and the saved scene renders live, so a missing
    /// timecode cable cannot black the output on a cold start.
    pub fn has_run(&self) -> bool {
        self.has_run
    }

    /// Playback rate multiplier. 1.0 internally; derived from timecode cadence
    /// when chasing.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// True from a position jump until the end of the frame that publishes it,
    /// so consumers that integrate can react.
    pub fn discontinuity(&self) -> bool {
        self.discontinuity || self.pending_discontinuity
    }

    pub fn source(&self) -> TransportSource {
        self.source
    }

    /// Frame rate positions are displayed and quantised at. Display only while
    /// running internally; it becomes load-bearing once timecode arrives.
    pub fn timecode_rate(&self) -> TimecodeRate {
        self.timecode_rate
    }

    pub fn set_timecode_rate(&mut self, rate: TimecodeRate) {
        self.timecode_rate = rate;
    }

    /// The current position as `HH:MM:SS:FF` at the transport's rate.
    pub fn formatted_position(&self) -> String {
        self.timecode_rate.format(self.position)
    }

    pub fn loop_region(&self) -> Option<LoopRegion> {
        self.loop_region
    }

    /// This frame's position for the timebase resolver, or `None` until the
    /// transport has run.
    ///
    /// `None` is what freezes every transport-locked consumer on a cold start;
    /// see [`Self::has_run`].
    pub fn sample(&self) -> Option<crate::timebase::TransportSample> {
        self.has_run.then_some(crate::timebase::TransportSample {
            position: self.position,
            running: self.running,
            discontinuity: self.discontinuity(),
            fps: self.timecode_rate.fps(),
        })
    }

    pub fn status(&self) -> TransportStatus {
        if self.running {
            if self.freewheeling {
                TransportStatus::Freewheeling
            } else {
                TransportStatus::Running
            }
        } else if self.source == TransportSource::Timecode && !self.has_run {
            TransportStatus::WaitingForSignal
        } else if self.has_run {
            TransportStatus::Stopped
        } else {
            TransportStatus::Idle
        }
    }

    // ── Control ─────────────────────────────────────────────────

    /// Choose where position comes from.
    ///
    /// Switching to `Timecode` stops local playback: the master owns position
    /// from that point, and leaving a local play running would race it.
    pub fn set_source(&mut self, source: TransportSource) {
        if source == self.source {
            return;
        }
        self.source = source;
        self.freewheeling = false;
        if source == TransportSource::Timecode {
            self.running = false;
            self.rate = 1.0;
        }
    }

    /// Start advancing.
    ///
    /// Does not set `has_run`; that happens on the first frame actually
    /// advanced, so a play immediately followed by a stop leaves a cold start
    /// cold.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::PositionIsReadOnly`] while chasing timecode.
    pub fn play(&mut self) -> Result<(), TransportError> {
        if self.source == TransportSource::Timecode {
            return Err(TransportError::PositionIsReadOnly);
        }
        self.running = true;
        Ok(())
    }

    /// Stop advancing, and return to zero when already stopped.
    ///
    /// The first stop holds position, so anything reading it freezes rather
    /// than releasing: a tripped cable should keep the last look, not cut it.
    /// The second is the way home, since the arrangement's return-to-zero arrow
    /// is now the cue back arrow. See /spec/transport.md § Stop Twice to Return.
    ///
    /// `has_run` survives the return, so the arrangement keeps authority rather
    /// than handing the output back to Performance mode mid-show.
    pub fn stop(&mut self) {
        if self.running {
            self.running = false;
        } else if self.source == TransportSource::Internal {
            self.position = 0.0;
            self.pending_discontinuity = true;
        }
    }

    /// Jump to an absolute position.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::PositionIsReadOnly`] while chasing timecode.
    pub fn locate(&mut self, position: f64) -> Result<(), TransportError> {
        if self.source == TransportSource::Timecode {
            return Err(TransportError::PositionIsReadOnly);
        }
        self.position = position.max(0.0);
        self.pending_discontinuity = true;
        Ok(())
    }

    /// Take this frame's position from an external timecode master.
    ///
    /// The one way position may be written while the source is `Timecode`, and
    /// the reason [`Self::play`] and [`Self::locate`] can refuse everyone else
    /// outright. Ignored when the source is `Internal`, so a cable left patched
    /// during a rehearsal cannot drag the playhead.
    ///
    /// A loop is not applied here: the master owns position, and wrapping it
    /// locally would put the show somewhere the master says it is not.
    /// See /spec/timecode.md § Consumer 1.
    pub fn chase(&mut self, chase: Chase) {
        if self.source != TransportSource::Timecode {
            return;
        }
        if chase.discontinuity {
            self.pending_discontinuity = true;
        }
        // A master that reports something that is not a place keeps the last
        // one it did. This is what the whole renderer reads: a position that is
        // not a number stops the show rendering, where holding the last look is
        // survivable and legible.
        if chase.position.is_finite() {
            self.position = chase.position.max(0.0);
        }
        self.running = chase.running;
        self.freewheeling = chase.running && chase.freewheeling;
        self.rate = if chase.running && chase.speed.is_finite() {
            chase.speed
        } else {
            1.0
        };
        // Engagement is "the show has moved", however it moved: a chased
        // arrangement takes authority exactly as an internally played one does.
        if chase.running {
            self.has_run = true;
        }
    }

    /// Set or clear the loop range honoured during internal playback.
    ///
    /// Retained but inert while chasing timecode, so switching back to internal
    /// restores it.
    pub fn set_loop_region(&mut self, region: Option<LoopRegion>) {
        self.loop_region = region;
    }

    // ── Per-frame ───────────────────────────────────────────────

    /// Advance one frame using wall-clock elapsed time. Call once per frame,
    /// before anything reads position.
    pub fn update(&mut self) {
        self.update_at(std::time::Instant::now());
    }

    /// [`Self::update`] with an injectable frame time, so advancement can be
    /// tested without sleeping.
    pub fn update_at(&mut self, now: std::time::Instant) {
        let dt = self.last_update.map_or(0.0, |prev| {
            now.saturating_duration_since(prev).as_secs_f64()
        });
        self.last_update = Some(now);
        self.tick(dt);
    }

    /// Advance by an explicit delta.
    ///
    /// `dt` is wall-clock seconds. Does nothing unless internally running, which
    /// is what keeps a cold start at zero: the transport never free-runs.
    pub fn tick(&mut self, dt: f64) {
        self.discontinuity = std::mem::take(&mut self.pending_discontinuity);

        if !self.running || self.source != TransportSource::Internal {
            return;
        }

        self.position += dt * self.rate;
        self.has_run = true;

        if let Some(region) = self.loop_region
            && self.position >= region.end
        {
            let overshoot = (self.position - region.end) % region.span();
            self.position = region.start + overshoot;
            self.discontinuity = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f64 = 1.0 / 60.0;

    fn loop_region(start: f64, end: f64) -> LoopRegion {
        LoopRegion::new(start, end).expect("valid range")
    }

    #[test]
    fn starts_stopped_at_zero() {
        let t = Transport::new();
        assert_eq!(t.position(), 0.0);
        assert!(!t.running());
        assert!(!t.has_run());
        assert_eq!(t.status(), TransportStatus::Idle);
    }

    /// The transport must not free-run. `has_run` is the basis of arrangement
    /// engagement, and a transport that sets it on launch would engage
    /// authority at position zero and black a cold start.
    #[test]
    fn does_not_advance_until_played() {
        let mut t = Transport::new();
        for _ in 0..600 {
            t.tick(FRAME);
        }
        assert_eq!(t.position(), 0.0);
        assert!(!t.has_run());
    }

    #[test]
    fn advances_while_playing() {
        let mut t = Transport::new();
        t.play().expect("internal play");
        for _ in 0..60 {
            t.tick(FRAME);
        }
        assert!((t.position() - 1.0).abs() < 1e-9);
        assert!(t.has_run());
        assert_eq!(t.status(), TransportStatus::Running);
    }

    #[test]
    fn play_alone_does_not_count_as_having_run() {
        let mut t = Transport::new();
        t.play().expect("internal play");
        assert!(!t.has_run(), "no frame has advanced yet");
        t.stop();
        assert_eq!(t.status(), TransportStatus::Idle);
    }

    /// Stopping holds position so envelopes freeze. Releasing instead would cut
    /// the look the instant someone trips over a cable.
    #[test]
    fn stop_holds_position() {
        let mut t = Transport::new();
        t.play().expect("internal play");
        for _ in 0..30 {
            t.tick(FRAME);
        }
        let held = t.position();
        t.stop();
        for _ in 0..600 {
            t.tick(FRAME);
        }
        assert_eq!(t.position(), held);
        assert_eq!(t.status(), TransportStatus::Stopped);
    }

    /// The second press is the way home, now that the arrangement's
    /// return-to-zero arrow walks cues instead.
    #[test]
    fn stopping_twice_returns_to_zero() {
        let mut t = Transport::new();
        t.play().expect("internal play");
        for _ in 0..30 {
            t.tick(FRAME);
        }
        t.stop();
        assert!(t.position() > 0.0, "the first stop holds where it stopped");

        t.stop();
        assert_eq!(t.position(), 0.0);
        assert!(t.discontinuity(), "a return is a jump, not a rewind");
        assert!(
            t.has_run(),
            "the show has run, so the arrangement keeps authority"
        );
    }

    /// Position is read-only while chasing, so a stop clears local running state
    /// and touches nothing else.
    #[test]
    fn stopping_twice_while_chasing_does_not_move_the_playhead() {
        let mut t = Transport::new();
        t.locate(42.0).expect("internal locate");
        t.set_source(TransportSource::Timecode);

        t.stop();
        t.stop();

        assert!((t.position() - 42.0).abs() < 1e-9);
    }

    #[test]
    fn locate_jumps_and_reports_a_discontinuity() {
        let mut t = Transport::new();
        t.locate(3600.0).expect("internal locate");
        assert!((t.position() - 3600.0).abs() < 1e-9);
        assert!(t.discontinuity());
    }

    /// A locate arrives during command processing, before the frame's tick, so
    /// the flag has to survive that tick and clear on the next one.
    #[test]
    fn discontinuity_survives_the_tick_that_publishes_it() {
        let mut t = Transport::new();
        t.locate(10.0).expect("internal locate");
        assert!(t.discontinuity());
        t.tick(FRAME);
        assert!(t.discontinuity(), "the frame that publishes the jump");
        t.tick(FRAME);
        assert!(!t.discontinuity());
    }

    #[test]
    fn a_locate_is_never_silently_dropped() {
        let mut t = Transport::new();
        t.play().expect("internal play");
        t.tick(FRAME);
        t.locate(500.0).expect("internal locate");
        t.tick(FRAME);

        let sample = t.sample().expect("has run");
        assert!(sample.discontinuity);
        assert!(sample.position > 500.0);
    }

    #[test]
    fn locate_does_not_engage_a_cold_start() {
        let mut t = Transport::new();
        t.locate(3600.0).expect("internal locate");
        assert!(
            !t.has_run(),
            "locating is not advancing; the arrangement must stay inert"
        );
        assert_eq!(t.status(), TransportStatus::Idle);
    }

    #[test]
    fn locate_clamps_negative_positions() {
        let mut t = Transport::new();
        t.locate(-5.0).expect("internal locate");
        assert_eq!(t.position(), 0.0);
    }

    /// Position is deterministic from the operations applied, not from the path
    /// taken to get there.
    #[test]
    fn locate_is_deterministic_regardless_of_path() {
        let mut direct = Transport::new();
        direct.locate(120.0).expect("locate");

        let mut wandered = Transport::new();
        wandered.play().expect("play");
        for _ in 0..120 {
            wandered.tick(FRAME);
        }
        wandered.stop();
        wandered.locate(500.0).expect("locate");
        wandered.locate(120.0).expect("locate");

        assert!((direct.position() - wandered.position()).abs() < 1e-9);
    }

    // ── Loop region ─────────────────────────────────────────────

    #[test]
    fn loop_region_wraps_playback() {
        let mut t = Transport::new();
        t.set_loop_region(Some(loop_region(10.0, 12.0)));
        t.locate(11.0).expect("locate");
        t.play().expect("play");

        for _ in 0..120 {
            t.tick(FRAME);
        }
        assert!(
            (10.0..12.0).contains(&t.position()),
            "position {} escaped the loop",
            t.position()
        );
    }

    #[test]
    fn loop_wrap_preserves_the_overshoot() {
        let mut t = Transport::new();
        t.set_loop_region(Some(loop_region(0.0, 1.0)));
        t.locate(0.9).expect("locate");
        t.play().expect("play");
        t.tick(0.25);
        // 0.9 + 0.25 = 1.15, which is 0.15 past the loop end.
        assert!((t.position() - 0.15).abs() < 1e-9);
        assert!(t.discontinuity());
    }

    #[test]
    fn loop_region_rejects_empty_and_inverted_ranges() {
        assert_eq!(
            LoopRegion::new(5.0, 5.0),
            Err(TransportError::EmptyLoopRegion)
        );
        assert_eq!(
            LoopRegion::new(9.0, 2.0),
            Err(TransportError::EmptyLoopRegion)
        );
        assert_eq!(loop_region(1.0, 4.0).span(), 3.0);
    }

    // ── Timecode source ─────────────────────────────────────────

    #[test]
    fn chasing_timecode_makes_position_read_only() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        assert_eq!(t.play(), Err(TransportError::PositionIsReadOnly));
        assert_eq!(t.locate(60.0), Err(TransportError::PositionIsReadOnly));
        assert_eq!(t.position(), 0.0);
    }

    #[test]
    fn armed_to_chase_with_no_signal_is_legible() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        assert_eq!(t.status(), TransportStatus::WaitingForSignal);
    }

    fn arrived(position: f64) -> Chase {
        Chase {
            position,
            running: true,
            discontinuity: false,
            freewheeling: false,
            speed: 1.0,
        }
    }

    /// The whole point of the source: an incoming master moves the show, and
    /// moving the show is what engages the arrangement.
    #[test]
    fn an_arriving_master_drives_the_position() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);

        t.chase(arrived(3600.0));

        assert!((t.position() - 3600.0).abs() < 1e-9);
        assert!(t.running());
        assert!(t.has_run(), "a chased show is a running show");
        assert_eq!(t.status(), TransportStatus::Running);
    }

    /// A cable left patched from yesterday's rehearsal must not drag the
    /// playhead of a show being run by hand.
    #[test]
    fn a_master_is_ignored_while_running_internally() {
        let mut t = Transport::new();
        t.play().expect("play");
        t.tick(FRAME);
        let position = t.position();

        t.chase(arrived(3600.0));

        assert!((t.position() - position).abs() < 1e-9);
    }

    /// Coasting is still running (the show must not stutter on one bad frame)
    /// but it is a different thing to be told about.
    #[test]
    fn coasting_through_a_dropout_reads_as_freewheeling() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        t.chase(Chase {
            freewheeling: true,
            ..arrived(10.0)
        });

        assert!(t.running());
        assert_eq!(t.status(), TransportStatus::Freewheeling);

        t.chase(arrived(10.1));
        assert_eq!(t.status(), TransportStatus::Running, "and it clears");
    }

    /// A master that stops holds the show where it stopped, exactly as a local
    /// stop does: a dropped signal should keep the last look, not cut it.
    #[test]
    fn a_master_that_stops_holds_the_position() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        t.chase(arrived(42.0));
        t.chase(Chase {
            running: false,
            ..arrived(42.0)
        });

        assert!(!t.running());
        assert_eq!(t.status(), TransportStatus::Stopped);
        assert!((t.position() - 42.0).abs() < 1e-9);
        assert!(
            (t.rate() - 1.0).abs() < 1e-9,
            "a stopped master has no speed to report"
        );
    }

    /// A master's locate has to reach the consumers that integrate, or a video
    /// deck chasing it would varispeed its way across an hour.
    #[test]
    fn a_master_locate_is_published_as_a_jump() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        t.chase(Chase {
            discontinuity: true,
            ..arrived(600.0)
        });
        t.tick(FRAME);

        assert!(t.discontinuity());
        t.chase(arrived(600.04));
        t.tick(FRAME);
        assert!(!t.discontinuity(), "and playing on is not a jump");
    }

    /// Freewheeling is a claim that the show is still moving on an educated
    /// guess. A master that has stopped is not moving at all, and saying
    /// otherwise sends a performer looking for a cable fault that is not there.
    #[test]
    fn a_stopped_master_is_never_reported_as_coasting() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        t.chase(arrived(10.0));
        t.chase(Chase {
            running: false,
            freewheeling: true,
            ..arrived(10.0)
        });

        assert_eq!(t.status(), TransportStatus::Stopped);
    }

    /// Nothing downstream expects a negative show position: an arrangement
    /// starts at zero and a region lookup before it has nothing to return. A
    /// master counting down to its start must not take the show there.
    #[test]
    fn a_master_counting_down_cannot_push_the_show_before_zero() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        t.chase(arrived(-5.0));

        assert!(
            t.position() >= 0.0,
            "the show cannot start before it starts, got {}",
            t.position()
        );
    }

    /// Switching back to internal must not leave the show reading as if a
    /// master were still coasting it along.
    #[test]
    fn taking_the_show_back_clears_the_chase() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        t.chase(Chase {
            freewheeling: true,
            speed: 0.5,
            ..arrived(10.0)
        });

        t.set_source(TransportSource::Internal);

        assert_ne!(t.status(), TransportStatus::Freewheeling);
        assert!((t.position() - 10.0).abs() < 1e-9, "position is kept");
    }

    #[test]
    fn switching_to_timecode_stops_local_playback() {
        let mut t = Transport::new();
        t.play().expect("play");
        t.tick(FRAME);
        t.set_source(TransportSource::Timecode);
        assert!(!t.running(), "a local play must not race the master");
    }

    /// Surfaces resend state they already sent (a controller's periodic push, a
    /// UI redraw), so setting the source it already has must not disturb the
    /// show. Re-asserting Internal mid-flight used to be the risk here.
    #[test]
    fn setting_the_source_it_already_has_changes_nothing() {
        let mut t = Transport::new();
        t.play().expect("play");
        t.tick(FRAME);
        let position = t.position();

        t.set_source(TransportSource::Internal);

        assert!(t.running(), "the show kept rolling");
        assert!(
            (t.position() - position).abs() < 1e-9,
            "and stayed where it was"
        );
    }

    /// Play is a state, not an edge: a held pad or a repeated API call must not
    /// restart or jog anything.
    #[test]
    fn playing_while_already_playing_is_the_same_as_playing() {
        let mut t = Transport::new();
        t.play().expect("play");
        t.tick(FRAME);
        t.play().expect("play again");
        t.tick(FRAME);

        assert!(
            (t.position() - FRAME * 2.0).abs() < 1e-9,
            "two frames of playback, not a rewind between them"
        );
    }

    #[test]
    fn timecode_source_does_not_advance_locally() {
        let mut t = Transport::new();
        t.set_source(TransportSource::Timecode);
        for _ in 0..600 {
            t.tick(FRAME);
        }
        assert_eq!(t.position(), 0.0);
    }

    /// Inert rather than cleared, so switching back to internal restores it.
    #[test]
    fn loop_region_survives_a_trip_through_timecode() {
        let mut t = Transport::new();
        t.set_loop_region(Some(loop_region(1.0, 2.0)));
        t.set_source(TransportSource::Timecode);
        assert_eq!(t.loop_region(), Some(loop_region(1.0, 2.0)));
        t.set_source(TransportSource::Internal);
        assert_eq!(t.loop_region(), Some(loop_region(1.0, 2.0)));
    }

    // ── Timecode rate ───────────────────────────────────────────

    #[test]
    fn timecode_rates_report_their_fps() {
        assert!((TimecodeRate::Fps24.fps() - 24.0).abs() < 1e-9);
        assert!((TimecodeRate::Fps25.fps() - 25.0).abs() < 1e-9);
        assert!((TimecodeRate::Fps30.fps() - 30.0).abs() < 1e-9);
        assert!((TimecodeRate::Fps2997.fps() - 29.97).abs() < 0.001);
        assert!((TimecodeRate::Fps2997Drop.fps() - 29.97).abs() < 0.001);
    }

    #[test]
    fn only_2997_drop_is_drop_frame() {
        for rate in TimecodeRate::ALL {
            assert_eq!(rate.is_drop_frame(), rate == TimecodeRate::Fps2997Drop);
        }
    }

    #[test]
    fn formats_positions_as_timecode() {
        assert_eq!(TimecodeRate::Fps25.format(0.0), "00:00:00:00");
        assert_eq!(TimecodeRate::Fps25.format(0.5), "00:00:00:12");
        assert_eq!(TimecodeRate::Fps24.format(3600.0), "01:00:00:00");
        assert_eq!(TimecodeRate::Fps30.format(3661.5), "01:01:01:15");
    }

    #[test]
    fn negative_positions_clamp_rather_than_wrapping() {
        assert_eq!(TimecodeRate::Fps30.format(-5.0), "00:00:00:00");
    }

    /// Non-drop 29.97 counts to 30 and drifts against wall time; drop-frame
    /// renumbers so it does not. One hour of real time is the classic check:
    /// the difference is the famous 3 seconds and 18 frames.
    #[test]
    fn drop_frame_tracks_wall_time_and_non_drop_does_not() {
        assert_eq!(TimecodeRate::Fps2997Drop.format(3600.0), "01:00:00;00");
        assert_eq!(TimecodeRate::Fps2997.format(3600.0), "00:59:56:12");
    }

    /// Drop-frame corrects at minute boundaries rather than continuously, so
    /// within a minute it lags wall time by up to two frames. The tenth minute
    /// drops nothing and lands exactly.
    #[test]
    fn drop_frame_corrects_at_minute_boundaries() {
        assert_eq!(TimecodeRate::Fps2997Drop.format(60.0), "00:00:59;28");
        assert_eq!(TimecodeRate::Fps2997Drop.format(600.0), "00:10:00;00");
        // Two frames past the minute is where the skipped numbers resume.
        assert_eq!(
            TimecodeRate::Fps2997Drop.format(1800.0 / 29.97),
            "00:01:00;02"
        );
    }

    #[test]
    fn drop_frame_is_the_only_rate_that_marks_the_separator() {
        for rate in TimecodeRate::ALL {
            let formatted = rate.format(1.0);
            assert_eq!(
                formatted.contains(';'),
                rate.is_drop_frame(),
                "{formatted} for {}",
                rate.label()
            );
        }
    }

    #[test]
    fn every_timecode_rate_is_labelled_distinctly() {
        let mut labels: Vec<&str> = TimecodeRate::ALL.iter().map(|r| r.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "labels must be distinguishable");
    }
}
