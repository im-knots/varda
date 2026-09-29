//! Absolute show position.
//!
//! Arrangement regions, automation envelopes, video chase, and the show runner
//! all read this position. It advances on an internal clock or chases incoming
//! timecode, so position-locked features work without external hardware.
//!
//! Separate from the tempo clock ([`crate::clock`]), which resolves BPM and
//! beat phase. Both can run at once, e.g. an arrangement on the transport while
//! modulators follow a DJ's MIDI clock.

use serde::{Deserialize, Serialize};

/// Frame rate for displaying and quantizing timecode positions. Lives here
/// because the arrangement ruler shows `HH:MM:SS:FF` without any timecode input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub enum TimecodeRate {
    Fps24,
    Fps25,
    /// 29.97 non-drop. Frame numbers run 0–29 and drift against wall time.
    Fps2997,
    /// 29.97 drop-frame, the broadcast default: frame numbers skip to stay in
    /// step with wall time.
    #[default]
    Fps2997Drop,
    Fps30,
}

impl TimecodeRate {
    /// Frames per second, for converting positions to frame counts.
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

    /// Formats a position as `HH:MM:SS:FF`. Drop-frame renumbers to track wall
    /// time and uses `;` before the frames, as desks and players do. Negative
    /// positions clamp to zero.
    pub fn format(self, position: f64) -> String {
        let (hours, minutes, seconds, frames) = self.label_parts(position);
        let sep = if self.is_drop_frame() { ';' } else { ':' };
        format!("{hours:02}:{minutes:02}:{seconds:02}{sep}{frames:02}")
    }

    /// The `HH`, `MM`, `SS`, `FF` label for a position. Shared with the timecode
    /// receiver so encoding and display agree on drop-frame.
    pub fn label_parts(self, position: f64) -> (u8, u8, u8, u8) {
        let elapsed_frames = (position.max(0.0) * self.fps()).floor() as u64;
        let counted = if self.is_drop_frame() {
            Self::renumber_drop_frame(elapsed_frames)
        } else {
            elapsed_frames
        };

        // Labels count at 30 for 29.97, so non-drop drifts against wall time.
        let fps = self.nominal_fps();
        let (frames, total_seconds) = (counted % fps, counted / fps);
        (
            (total_seconds / 3600) as u8,
            ((total_seconds / 60) % 60) as u8,
            (total_seconds % 60) as u8,
            frames as u8,
        )
    }

    /// Label frames per second: 30 for both 29.97 variants.
    pub fn nominal_fps(self) -> u64 {
        self.fps().round() as u64
    }

    /// SMPTE 12M drop-frame: skip frame numbers 0 and 1 at the start of every
    /// minute except every tenth, keeping the label within a frame of wall time.
    fn renumber_drop_frame(elapsed_frames: u64) -> u64 {
        /// Real frames in ten minutes at 29.97.
        const PER_TEN_MINUTES: u64 = 17_982;
        /// Real frames in each minute after the first, at 29.97.
        const PER_MINUTE: u64 = 1_798;

        let ten_minute_blocks = elapsed_frames / PER_TEN_MINUTES;
        let within_block = elapsed_frames % PER_TEN_MINUTES;
        // A block's first two frames are in the undropped tenth minute.
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
    /// Position chases incoming timecode and is read-only.
    Timecode,
}

/// Show positions in seconds that internal playback loops within.
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

/// One read of an external timecode master. A plain struct so the transport
/// doesn't depend on the timecode module or know which protocol is behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chase {
    pub position: f64,
    /// Frames are arriving, or the freewheel is still coasting.
    pub running: bool,
    /// The master jumped instead of playing on.
    pub discontinuity: bool,
    /// Coasting through a dropout.
    pub freewheeling: bool,
    /// Measured against wall time; 1.0 while a master plays forwards.
    pub speed: f64,
}

/// Why the transport is or isn't moving. Idle and broken look the same on the
/// output, so the state is reported explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub enum TransportStatus {
    /// Internal, never started this session. The saved scene renders as-is.
    Idle,
    /// Armed to chase, but no timecode has arrived.
    WaitingForSignal,
    /// Position is advancing.
    Running,
    /// Coasting through a timecode dropout on the reader's extrapolation.
    /// Still counts as running.
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

/// Rejected transport operations, so the caller learns why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// Position belongs to the incoming timecode master.
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
// The four flags are independent: `running` and `has_run` differ between a
// play and its first tick, which engagement depends on.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Transport {
    position: f64,
    running: bool,
    has_run: bool,
    rate: f64,
    discontinuity: bool,
    /// A jump since the last tick. Separate because a locate arrives during
    /// command processing, before the tick; clearing on tick alone would drop
    /// it unread.
    pending_discontinuity: bool,
    timecode_rate: TimecodeRate,
    source: TransportSource,
    /// Coasting through a timecode dropout. Only true while chasing.
    freewheeling: bool,
    loop_region: Option<LoopRegion>,
    /// Previous [`Transport::update`] time, for `tick`'s dt.
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

    /// Absolute position in seconds. `f64` because shows usually start at hour
    /// 1, where `f32` quantizes to about 0.4 ms.
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// Whether the transport has advanced this session. Until it has, the
    /// arrangement stays inert and the saved scene renders, so a missing
    /// timecode cable can't black out a cold start.
    pub fn has_run(&self) -> bool {
        self.has_run
    }

    /// Playback rate multiplier: 1.0 internally, from timecode cadence when
    /// chasing.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// True from a jump until the end of the frame that publishes it, so
    /// integrating consumers can react.
    pub fn discontinuity(&self) -> bool {
        self.discontinuity || self.pending_discontinuity
    }

    pub fn source(&self) -> TransportSource {
        self.source
    }

    /// Frame rate for display and quantization. Only affects position once
    /// timecode arrives.
    pub fn timecode_rate(&self) -> TimecodeRate {
        self.timecode_rate
    }

    pub fn set_timecode_rate(&mut self, rate: TimecodeRate) {
        self.timecode_rate = rate;
    }

    /// Position as `HH:MM:SS:FF` at the transport's rate.
    pub fn formatted_position(&self) -> String {
        self.timecode_rate.format(self.position)
    }

    pub fn loop_region(&self) -> Option<LoopRegion> {
        self.loop_region
    }

    /// This frame's position for the timebase resolver, or `None` until the
    /// transport has run, which freezes transport-locked consumers on a cold
    /// start.
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

    /// Sets where position comes from. Switching to `Timecode` stops local
    /// playback so it can't race the master.
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

    /// Starts advancing. `has_run` is set on the first advanced frame, not
    /// here, so play then immediately stop leaves a cold start cold.
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

    /// Stops advancing; if already stopped, returns to zero.
    ///
    /// The first stop holds position so readers freeze on the last look. The
    /// second returns to zero. `has_run` stays set, so the arrangement keeps
    /// control instead of handing the output back to Performance mode.
    pub fn stop(&mut self) {
        if self.running {
            self.running = false;
        } else if self.source == TransportSource::Internal {
            self.position = 0.0;
            self.pending_discontinuity = true;
        }
    }

    /// Jumps to an absolute position.
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

    /// Takes this frame's position from an external timecode master: the only
    /// write allowed while the source is `Timecode`. Ignored when the source is
    /// `Internal`, so a cable left patched can't move the playhead. The loop
    /// region is not applied; the master owns position.
    pub fn chase(&mut self, chase: Chase) {
        if self.source != TransportSource::Timecode {
            return;
        }
        if chase.discontinuity {
            self.pending_discontinuity = true;
        }
        // Keep the last position if the master sends a non-finite one; a NaN
        // position would stop rendering.
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
        // Any movement engages the arrangement, chased or played.
        if chase.running {
            self.has_run = true;
        }
    }

    /// Sets or clears the loop range for internal playback. Kept but inert
    /// while chasing, so switching back to internal restores it.
    pub fn set_loop_region(&mut self, region: Option<LoopRegion>) {
        self.loop_region = region;
    }

    // ── Per-frame ───────────────────────────────────────────────

    /// Advances one frame by wall-clock time. Call once per frame, before
    /// anything reads position.
    pub fn update(&mut self) {
        self.update_at(std::time::Instant::now());
    }

    /// [`Self::update`] with an injected frame time, for tests.
    pub fn update_at(&mut self, now: std::time::Instant) {
        let dt = self.last_update.map_or(0.0, |prev| {
            now.saturating_duration_since(prev).as_secs_f64()
        });
        self.last_update = Some(now);
        self.tick(dt);
    }

    /// Advances by `dt` wall-clock seconds. Does nothing unless running
    /// internally, so a cold start stays at zero.
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

    /// The transport must not free-run: setting `has_run` on launch would engage
    /// the arrangement at zero and black out a cold start.
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

    /// Stop holds position so envelopes freeze.
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

    /// A second stop returns to zero.
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

    /// While chasing, stop clears local running state and nothing else.
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

    /// A locate arrives before the frame's tick, so the flag survives that tick
    /// and clears on the next.
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

    /// Position depends only on the operations applied, not the path.
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

    /// An incoming master moves the show and engages the arrangement.
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

    /// Timecode is ignored while the source is internal.
    #[test]
    fn a_master_is_ignored_while_running_internally() {
        let mut t = Transport::new();
        t.play().expect("play");
        t.tick(FRAME);
        let position = t.position();

        t.chase(arrived(3600.0));

        assert!((t.position() - position).abs() < 1e-9);
    }

    /// Freewheeling counts as running but is reported separately.
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

    /// A stopped master holds the show where it stopped, like a local stop.
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

    /// A master's locate reaches integrating consumers, so a chasing video deck
    /// jumps instead of varispeeding.
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

    /// A stopped master is not reported as freewheeling.
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

    /// A master counting down before zero doesn't take the show negative.
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

    /// Switching back to internal clears the chase state.
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

    /// Surfaces resend state they already sent (periodic pushes, redraws), so
    /// setting the current source must not disturb the show.
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

    /// Play is a state, not an edge: repeating it doesn't restart anything.
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

    /// Inert, not cleared, so switching back to internal restores it.
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

    /// Over one hour, non-drop 29.97 drifts 3 seconds and 18 frames from wall
    /// time; drop-frame doesn't.
    #[test]
    fn drop_frame_tracks_wall_time_and_non_drop_does_not() {
        assert_eq!(TimecodeRate::Fps2997Drop.format(3600.0), "01:00:00;00");
        assert_eq!(TimecodeRate::Fps2997.format(3600.0), "00:59:56:12");
    }

    /// Drop-frame corrects at minute boundaries, so within a minute it lags by
    /// up to two frames. The tenth minute drops nothing and lands exactly.
    #[test]
    fn drop_frame_corrects_at_minute_boundaries() {
        assert_eq!(TimecodeRate::Fps2997Drop.format(60.0), "00:00:59;28");
        assert_eq!(TimecodeRate::Fps2997Drop.format(600.0), "00:10:00;00");
        // The skipped numbers resume two frames past the minute.
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
