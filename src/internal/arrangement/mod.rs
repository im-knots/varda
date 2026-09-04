//! Arrangement mode: deck activity and parameter values positioned against
//! transport time rather than performed live.
//!
//! The arrangement is the mixer rotated ninety degrees. A lane *is* a deck, a
//! group *is* a channel, and a region compiles to breakpoints on that deck's
//! opacity envelope, so almost nothing new enters the engine: evaluation,
//! stacking, and persistence all come from the modulation graph that Phase 33
//! already built.
//!
//! See /spec/arrangement.md.

pub mod authority;
pub mod residency;

use crate::modulation::{Breakpoint, CurveKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use authority::{Authority, DEFAULT_REARM_SECONDS};
pub use residency::SourceDemand;

/// How close to the playhead a cue counts as the one already reached.
const CUE_EPSILON: f64 = 1e-3;

/// What renders while the arrangement is not driving anything.
///
/// "Run this loop until the schedule starts" is a normal installation
/// requirement, so the pre-show state needs something to *be* rather than
/// something to fail at. See /spec/transport.md § Idle behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub enum IdleBehaviour {
    /// Performance mode holds; the arrangement stays inert.
    #[default]
    HoldPerformance,
    /// A designated deck is shown until the transport reaches the arranged range.
    ShowDeck { deck_uuid: String },
}

/// A span during which a deck is visible.
///
/// Not a container for content: the deck exists in the scene whether or not any
/// region covers it, and a region only says *when*.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegionConfig {
    /// Transport position, in seconds, where the region begins.
    pub start: f64,
    /// Transport position, in seconds, where the region ends.
    pub end: f64,
    /// Fade-in duration in seconds, measured from `start`.
    #[serde(default)]
    pub fade_in: f64,
    /// Fade-out duration in seconds, measured back from `end`.
    #[serde(default)]
    pub fade_out: f64,
}

impl RegionConfig {
    pub fn new(start: f64, end: f64) -> Self {
        Self {
            start,
            end,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }

    #[must_use]
    pub fn with_fades(mut self, fade_in: f64, fade_out: f64) -> Self {
        self.fade_in = fade_in;
        self.fade_out = fade_out;
        self
    }

    pub fn span(self) -> f64 {
        self.end - self.start
    }

    /// Whether the region carries any time at all. Zero-length and inverted
    /// regions are dropped at compile time rather than rejected at edit time,
    /// so a drag that collapses on itself is harmless.
    pub fn is_valid(self) -> bool {
        self.span() > 0.0 && self.start.is_finite() && self.end.is_finite()
    }

    /// Fade durations clamped so they cannot overlap each other.
    ///
    /// Two fades longer than the region together become a triangle rather than
    /// crossing over, which is what every DAW does when you drag handles past
    /// each other.
    pub fn clamped_fades(self) -> (f64, f64) {
        let span = self.span();
        let fade_in = self.fade_in.max(0.0);
        let fade_out = self.fade_out.max(0.0);
        let total = fade_in + fade_out;
        if total > span && total > 0.0 {
            let scale = span / total;
            (fade_in * scale, fade_out * scale)
        } else {
            (fade_in, fade_out)
        }
    }
}

/// One deck's row in the arrangement.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LaneConfig {
    /// The deck this lane *is*. Not a copy, and not a reference to a copy.
    pub deck_uuid: String,
    /// Visibility spans, compiled to the deck's opacity envelope.
    #[serde(default)]
    pub regions: Vec<RegionConfig>,
    /// Envelope source UUIDs, keyed by the modulation engine's parameter key.
    #[serde(default)]
    pub envelopes: HashMap<String, String>,
    #[serde(default = "default_lane_height")]
    pub height: f32,
    #[serde(default)]
    pub collapsed: bool,
}

fn default_lane_height() -> f32 {
    48.0
}

impl LaneConfig {
    pub fn new(deck_uuid: impl Into<String>) -> Self {
        Self {
            deck_uuid: deck_uuid.into(),
            regions: Vec::new(),
            envelopes: HashMap::new(),
            height: default_lane_height(),
            collapsed: false,
        }
    }

    /// Whether this lane has anything for the arrangement to drive.
    ///
    /// A lane with neither regions nor envelopes is a row in the UI and nothing
    /// more, so it must not take authority away from Performance mode.
    pub fn drives_anything(&self) -> bool {
        self.regions.iter().any(|r| r.is_valid()) || !self.envelopes.is_empty()
    }

    /// Every envelope source UUID this lane owns, including the compiled
    /// opacity envelope. This is the set suspended by a live override.
    pub fn envelope_uuids(&self) -> impl Iterator<Item = &str> {
        self.envelopes.values().map(String::as_str)
    }
}

/// A named instant on the timeline, and the thing the transport's arrows walk.
///
/// Marks only. The show runner adds a command to this same struct rather than a
/// second list, so a cue dropped to navigate by can later be given something to
/// fire. See /spec/arrangement.md § Cue points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Cue {
    pub uuid: String,
    pub name: String,
    /// Absolute seconds, the axis regions and envelopes are already on.
    pub at: f64,
}

/// The arrangement's own data. Envelopes live in the modulation graph; lanes
/// reference them by UUID.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ArrangementConfig {
    #[serde(default)]
    pub lanes: Vec<LaneConfig>,
    #[serde(default)]
    pub idle: IdleBehaviour,
    /// Sorted by position, so navigation is a scan rather than a sort per press.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
}

impl ArrangementConfig {
    pub fn lane(&self, deck_uuid: &str) -> Option<&LaneConfig> {
        self.lanes.iter().find(|l| l.deck_uuid == deck_uuid)
    }

    pub fn lane_mut(&mut self, deck_uuid: &str) -> Option<&mut LaneConfig> {
        self.lanes.iter_mut().find(|l| l.deck_uuid == deck_uuid)
    }

    /// Insert a cue, keeping the list in position order.
    pub fn add_cue(&mut self, cue: Cue) {
        self.cues.push(cue);
        self.sort_cues();
    }

    pub fn cue_mut(&mut self, uuid: &str) -> Option<&mut Cue> {
        self.cues.iter_mut().find(|c| c.uuid == uuid)
    }

    /// Restore position order after a cue has been moved.
    pub fn sort_cues(&mut self) {
        self.cues.sort_by(|a, b| a.at.total_cmp(&b.at));
    }

    /// The first cue after `position`, for the transport's forward arrow.
    ///
    /// A cue level with the playhead is skipped, or holding the arrow would
    /// stick on it rather than walking the list. The tolerance is a millisecond
    /// because positions are continuous and a cue authored at a snapped instant
    /// is rarely bit-identical to the position a locate produced.
    pub fn cue_after(&self, position: f64) -> Option<&Cue> {
        self.cues.iter().find(|c| c.at > position + CUE_EPSILON)
    }

    /// Where an arrow press steps from, given where the last press landed.
    ///
    /// The live position is the wrong answer while the show is running:
    /// playback carries the playhead off the cue between presses, so every
    /// press back would return to the same cue. A press therefore steps from
    /// where the last one landed, for as long as the playhead is still inside
    /// the stretch that jump reached. Once playback crosses into the next cue's
    /// stretch the anchor is stale, and the position is honest again.
    pub fn cue_walk_origin(&self, anchor: Option<f64>, position: f64) -> f64 {
        let Some(anchor) = anchor else {
            return position;
        };
        let next = self.cue_after(anchor).map_or(f64::INFINITY, |cue| cue.at);
        if position >= anchor && position < next {
            anchor
        } else {
            position
        }
    }

    /// The last cue before `position`, for the transport's back arrow.
    pub fn cue_before(&self, position: f64) -> Option<&Cue> {
        self.cues
            .iter()
            .rev()
            .find(|c| c.at < position - CUE_EPSILON)
    }

    /// Whether any lane has something to drive. An arrangement of empty lanes
    /// must not engage, or adding a row would black the decks it names.
    pub fn drives_anything(&self) -> bool {
        self.lanes.iter().any(LaneConfig::drives_anything)
    }

    /// The span from the earliest region start to the latest region end, or
    /// `None` when nothing is authored.
    ///
    /// A gap *inside* this span is authored silence and stays dark. Outside it
    /// the arrangement has said nothing at all, which is what idle behaviour is
    /// for. See /spec/transport.md § Idle behaviour.
    pub fn range(&self) -> Option<(f64, f64)> {
        let mut regions = self
            .lanes
            .iter()
            .flat_map(|l| l.regions.iter())
            .filter(|r| r.is_valid())
            .peekable();
        regions.peek()?;
        regions.fold(None, |acc: Option<(f64, f64)>, r| {
            Some(acc.map_or((r.start, r.end), |(s, e)| (s.min(r.start), e.max(r.end))))
        })
    }

    /// Whether the arranged range has anything to say about this position.
    pub fn within_range(&self, position: f64) -> bool {
        self.range()
            .is_some_and(|(start, end)| position >= start && position < end)
    }

    /// Latest position covered by any region, for the ruler's default extent.
    pub fn duration(&self) -> f64 {
        self.lanes
            .iter()
            .flat_map(|l| l.regions.iter())
            .filter(|r| r.is_valid())
            .map(|r| r.end)
            .fold(0.0, f64::max)
    }
}

/// The parameter key the modulation engine addresses a deck's opacity by.
pub fn opacity_param_key(deck_uuid: &str) -> String {
    let mut key = String::with_capacity(deck_uuid.len() + 13);
    write_opacity_param_key(&mut key, deck_uuid);
    key
}

/// [`opacity_param_key`] into a reused buffer, for the per-frame path.
pub fn write_opacity_param_key(buf: &mut String, deck_uuid: &str) {
    buf.clear();
    buf.push_str("deck_");
    buf.push_str(deck_uuid);
    buf.push_str(":opacity");
}

/// The parameter key the modulation engine addresses a channel's fader by.
///
/// A channel has no lane of its own, so unlike a deck's opacity this key is
/// never compiled from regions: it carries only curves someone drew or recorded.
/// See /spec/automation-recording.md § What can be recorded.
pub fn channel_opacity_param_key(channel_uuid: &str) -> String {
    let mut key = String::with_capacity(channel_uuid.len() + 11);
    key.push_str("ch_");
    key.push_str(channel_uuid);
    key.push_str(":opacity");
    key
}

/// Compile a lane's regions into breakpoints on its opacity envelope.
///
/// This is the whole region mechanism. Because the result is an ordinary
/// envelope, a region survives locates and loop wraps for free: opacity becomes
/// a pure function of position with no accumulated state to resync.
///
/// Overlapping regions in one lane are clipped rather than rejected, since
/// breakpoints must come out strictly ordered for evaluation to bracket them.
/// Overlap *between sibling lanes* is untouched and is how a crossfade is
/// expressed.
pub fn compile_regions(regions: &[RegionConfig]) -> Vec<Breakpoint> {
    let mut ordered: Vec<RegionConfig> = regions.iter().copied().filter(|r| r.is_valid()).collect();
    ordered.sort_by(|a, b| a.start.total_cmp(&b.start));

    let mut out: Vec<Breakpoint> = Vec::with_capacity(ordered.len() * 4 + 1);
    let mut previous_end = f64::NEG_INFINITY;

    for region in ordered {
        let start = region.start.max(previous_end);
        if start >= region.end {
            continue;
        }
        let (fade_in, fade_out) = RegionConfig { start, ..region }.clamped_fades();

        let full_from = start + fade_in;
        let full_until = region.end - fade_out;

        if fade_in > 0.0 {
            out.push(Breakpoint::new(start, 0.0));
            out.push(Breakpoint::new(full_from, 1.0));
        } else {
            out.push(Breakpoint::new(start, 1.0).with_curve(CurveKind::Step));
        }

        if full_until > full_from {
            // Holds the plateau flat regardless of the fade-in's shape.
            if let Some(last) = out.last_mut() {
                last.curve = CurveKind::Step;
            }
            out.push(Breakpoint::new(full_until, 1.0));
        }

        if fade_out > 0.0 {
            // The point above is the ramp's start; give it a slope to fall on.
            if let Some(last) = out.last_mut() {
                last.curve = CurveKind::default();
            }
        }
        out.push(Breakpoint::new(region.end, 0.0).with_curve(CurveKind::Step));

        previous_end = region.end;
    }

    // An envelope holds its first value backwards forever, so a lane whose
    // first region starts hard-on (no fade) would show the deck from the
    // beginning of time without an explicit zero in front of it.
    if let Some(first) = out.first()
        && first.value > 0.0
        && first.position > 0.0
    {
        out.insert(0, Breakpoint::new(0.0, 0.0).with_curve(CurveKind::Step));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulation::evaluate_envelope;

    fn value_at(bps: &[Breakpoint], position: f64) -> f32 {
        let mut cursor = 0;
        evaluate_envelope(bps, position, &mut cursor)
    }

    fn assert_close(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "{what}: expected {expected}, got {actual}"
        );
    }

    #[test]
    fn an_empty_lane_compiles_to_an_inert_envelope() {
        assert!(compile_regions(&[]).is_empty());
    }

    #[test]
    fn a_hard_region_is_off_before_on_during_and_off_after() {
        let bps = compile_regions(&[RegionConfig::new(10.0, 20.0)]);

        assert_close(value_at(&bps, 0.0), 0.0, "before the show");
        assert_close(value_at(&bps, 9.9), 0.0, "just before the region");
        assert_close(value_at(&bps, 10.0), 1.0, "on the in-point");
        assert_close(value_at(&bps, 15.0), 1.0, "mid region");
        assert_close(value_at(&bps, 19.9), 1.0, "just before the out-point");
        assert_close(value_at(&bps, 20.0), 0.0, "on the out-point");
        assert_close(value_at(&bps, 1000.0), 0.0, "long after");
    }

    /// The failure this guards is specific: an envelope holds its first value
    /// backwards, so without a leading zero a deck whose first region starts at
    /// hour one would be visible from a cold start at position zero.
    #[test]
    fn a_late_first_region_does_not_leak_backwards() {
        let bps = compile_regions(&[RegionConfig::new(3600.0, 3700.0)]);
        assert_close(value_at(&bps, 0.0), 0.0, "position zero");
        assert_close(value_at(&bps, 1800.0), 0.0, "half an hour in");
        assert_close(value_at(&bps, 3600.0), 1.0, "the in-point");
    }

    #[test]
    fn fades_ramp_between_the_handles() {
        let bps = compile_regions(&[RegionConfig::new(0.0, 10.0).with_fades(2.0, 4.0)]);

        assert_close(value_at(&bps, 0.0), 0.0, "fade-in start");
        assert_close(value_at(&bps, 1.0), 0.5, "halfway up");
        assert_close(value_at(&bps, 2.0), 1.0, "fade-in complete");
        assert_close(value_at(&bps, 4.0), 1.0, "plateau");
        assert_close(value_at(&bps, 6.0), 1.0, "fade-out start");
        assert_close(value_at(&bps, 8.0), 0.5, "halfway down");
        assert_close(value_at(&bps, 10.0), 0.0, "out-point");
    }

    #[test]
    fn a_plateau_stays_flat_between_asymmetric_fades() {
        let bps = compile_regions(&[RegionConfig::new(0.0, 100.0).with_fades(1.0, 50.0)]);
        assert_close(value_at(&bps, 25.0), 1.0, "still on the plateau");
        assert_close(value_at(&bps, 50.0), 1.0, "the last plateau instant");
        assert_close(value_at(&bps, 75.0), 0.5, "halfway down the long fade");
    }

    /// Dragging both handles past each other should give a triangle, not a
    /// crossed-over envelope that reads above full or below zero.
    #[test]
    fn fades_longer_than_the_region_collapse_to_a_triangle() {
        let bps = compile_regions(&[RegionConfig::new(0.0, 10.0).with_fades(30.0, 30.0)]);

        assert_close(value_at(&bps, 0.0), 0.0, "start");
        assert_close(value_at(&bps, 5.0), 1.0, "the peak");
        assert_close(value_at(&bps, 10.0), 0.0, "end");
        for probe in [1.0, 2.5, 7.5, 9.0] {
            let v = value_at(&bps, probe);
            assert!((0.0..=1.0).contains(&v), "value {v} at {probe} left 0..1");
        }
    }

    #[test]
    fn consecutive_regions_stay_dark_in_the_gap() {
        let bps = compile_regions(&[RegionConfig::new(0.0, 10.0), RegionConfig::new(20.0, 30.0)]);

        assert_close(value_at(&bps, 5.0), 1.0, "first region");
        assert_close(value_at(&bps, 10.0), 0.0, "gap starts");
        assert_close(value_at(&bps, 15.0), 0.0, "mid gap");
        assert_close(value_at(&bps, 25.0), 1.0, "second region");
        assert_close(value_at(&bps, 30.0), 0.0, "after");
    }

    #[test]
    fn regions_compile_in_position_order_however_they_are_listed() {
        let forwards =
            compile_regions(&[RegionConfig::new(0.0, 10.0), RegionConfig::new(20.0, 30.0)]);
        let backwards =
            compile_regions(&[RegionConfig::new(20.0, 30.0), RegionConfig::new(0.0, 10.0)]);
        assert_eq!(forwards, backwards);
    }

    /// Evaluation brackets breakpoints by binary search, so a non-monotonic
    /// list would silently return wrong values rather than failing loudly.
    #[test]
    fn compiled_breakpoints_are_always_ordered() {
        let bps = compile_regions(&[
            RegionConfig::new(5.0, 25.0).with_fades(3.0, 3.0),
            RegionConfig::new(0.0, 10.0),
            RegionConfig::new(20.0, 30.0).with_fades(8.0, 0.0),
            RegionConfig::new(-5.0, 2.0),
        ]);

        assert!(
            bps.windows(2).all(|w| w[0].position <= w[1].position),
            "positions went backwards: {:?}",
            bps.iter().map(|b| b.position).collect::<Vec<_>>()
        );
    }

    #[test]
    fn overlapping_regions_in_one_lane_are_clipped_not_stacked() {
        let bps = compile_regions(&[RegionConfig::new(0.0, 20.0), RegionConfig::new(10.0, 30.0)]);

        for probe in [0.0, 5.0, 15.0, 25.0] {
            assert_close(value_at(&bps, probe), 1.0, "continuously covered");
        }
        assert_close(value_at(&bps, 30.0), 0.0, "after both");
    }

    #[test]
    fn degenerate_regions_are_dropped() {
        assert!(compile_regions(&[RegionConfig::new(5.0, 5.0)]).is_empty());
        assert!(compile_regions(&[RegionConfig::new(9.0, 1.0)]).is_empty());
        assert!(compile_regions(&[RegionConfig::new(f64::NAN, 1.0)]).is_empty());
    }

    /// The property automation is built on: how you reached a position cannot
    /// change what is on screen there.
    #[test]
    fn opacity_is_a_pure_function_of_position() {
        let bps = compile_regions(&[
            RegionConfig::new(0.0, 10.0).with_fades(2.0, 2.0),
            RegionConfig::new(15.0, 25.0),
        ]);

        let mut played = 0;
        let mut swept = 0.0;
        while swept < 18.0 {
            evaluate_envelope(&bps, swept, &mut played);
            swept += 1.0 / 60.0;
        }
        let after_playing = evaluate_envelope(&bps, 18.0, &mut played);

        let mut located = 0;
        let after_locate = evaluate_envelope(&bps, 18.0, &mut located);

        assert_close(after_playing, after_locate, "play and locate disagree");
    }

    #[test]
    fn a_lane_without_regions_or_envelopes_drives_nothing() {
        let mut lane = LaneConfig::new("deck-1");
        assert!(!lane.drives_anything());

        lane.regions.push(RegionConfig::new(0.0, 0.0));
        assert!(!lane.drives_anything(), "a collapsed region is not content");

        lane.regions.push(RegionConfig::new(0.0, 5.0));
        assert!(lane.drives_anything());
    }

    #[test]
    fn an_arrangement_of_empty_lanes_does_not_engage() {
        let arrangement = ArrangementConfig {
            lanes: vec![LaneConfig::new("a"), LaneConfig::new("b")],
            ..ArrangementConfig::default()
        };
        assert!(!arrangement.drives_anything());
        assert_close(arrangement.duration() as f32, 0.0, "duration");
    }

    #[test]
    fn duration_is_the_last_covered_position() {
        let mut a = LaneConfig::new("a");
        a.regions.push(RegionConfig::new(0.0, 30.0));
        let mut b = LaneConfig::new("b");
        b.regions.push(RegionConfig::new(10.0, 90.0));

        let arrangement = ArrangementConfig {
            lanes: vec![a, b],
            ..ArrangementConfig::default()
        };
        assert_close(arrangement.duration() as f32, 90.0, "duration");
    }

    /// The range spans every lane, so a lane that starts late does not shorten
    /// it and a lane that ends late does extend it.
    #[test]
    fn the_range_spans_every_lane() {
        let mut a = LaneConfig::new("a");
        a.regions.push(RegionConfig::new(30.0, 40.0));
        let mut b = LaneConfig::new("b");
        b.regions.push(RegionConfig::new(10.0, 20.0));

        let arrangement = ArrangementConfig {
            lanes: vec![a, b],
            ..ArrangementConfig::default()
        };
        let (start, end) = arrangement.range().expect("range");
        assert_close(start as f32, 10.0, "range start");
        assert_close(end as f32, 40.0, "range end");
    }

    /// The distinction the idle rule turns on: a gap between regions is inside
    /// the range and therefore authored, while the stretches on either side of
    /// everything are not.
    #[test]
    fn a_gap_between_regions_is_inside_the_range() {
        let mut lane = LaneConfig::new("a");
        lane.regions.push(RegionConfig::new(10.0, 20.0));
        lane.regions.push(RegionConfig::new(40.0, 50.0));
        let arrangement = ArrangementConfig {
            lanes: vec![lane],
            ..ArrangementConfig::default()
        };

        assert!(
            arrangement.within_range(25.0),
            "the gap is authored silence"
        );
        assert!(!arrangement.within_range(5.0), "before the show");
        assert!(!arrangement.within_range(60.0), "after the show");
    }

    #[test]
    fn an_arrangement_with_nothing_authored_has_no_range() {
        let arrangement = ArrangementConfig {
            lanes: vec![LaneConfig::new("a")],
            ..ArrangementConfig::default()
        };
        assert!(arrangement.range().is_none());
        assert!(!arrangement.within_range(0.0));
    }

    #[test]
    fn idle_behaviour_defaults_to_holding_performance() {
        assert_eq!(IdleBehaviour::default(), IdleBehaviour::HoldPerformance);
    }

    #[test]
    fn the_opacity_key_matches_the_modulation_engine_convention() {
        assert_eq!(opacity_param_key("abc123"), "deck_abc123:opacity");
    }

    fn cued(positions: &[f64]) -> ArrangementConfig {
        let mut arrangement = ArrangementConfig::default();
        for (i, at) in positions.iter().enumerate() {
            arrangement.add_cue(Cue {
                uuid: format!("cue{i}"),
                name: format!("Cue {i}"),
                at: *at,
            });
        }
        arrangement
    }

    /// The rule that makes an arrow walk the list while the show runs, rather
    /// than returning to the cue playback just carried the playhead off.
    #[test]
    fn a_walk_steps_from_where_the_last_press_landed() {
        let arrangement = cued(&[10.0, 20.0, 30.0]);

        assert_close(
            arrangement.cue_walk_origin(None, 24.0) as f32,
            24.0,
            "with no press behind it, the position is all there is",
        );
        assert_close(
            arrangement.cue_walk_origin(Some(20.0), 24.0) as f32,
            20.0,
            "playback moved on, but the walk is still in that stretch",
        );
    }

    /// Letting it play into the next stretch ends the walk, or the forward
    /// arrow would send the playhead backwards to a cue already passed.
    #[test]
    fn a_walk_ends_when_playback_leaves_its_stretch() {
        let arrangement = cued(&[10.0, 20.0, 30.0]);

        assert_close(
            arrangement.cue_walk_origin(Some(20.0), 31.5) as f32,
            31.5,
            "playback crossed the next cue, so the anchor is stale",
        );
        assert_close(
            arrangement.cue_walk_origin(Some(20.0), 5.0) as f32,
            5.0,
            "something moved the playhead behind the anchor",
        );
    }

    /// Nothing follows the last cue, so a walk that reaches it stays anchored
    /// however long the show runs on.
    #[test]
    fn a_walk_anchored_on_the_last_cue_does_not_expire() {
        let arrangement = cued(&[10.0, 20.0]);
        assert_close(
            arrangement.cue_walk_origin(Some(20.0), 900.0) as f32,
            20.0,
            "there is no next stretch to cross into",
        );
    }

    /// A press from a standing start on a cue has to move, or the arrows do
    /// nothing at the one position a performer is most likely to press them
    /// from: the cue they just landed on.
    #[test]
    fn the_cue_under_the_playhead_is_skipped_in_both_directions() {
        let arrangement = cued(&[10.0, 20.0, 30.0]);

        assert_eq!(
            arrangement.cue_after(20.0).map(|c| c.uuid.as_str()),
            Some("cue2")
        );
        assert_eq!(
            arrangement.cue_before(20.0).map(|c| c.uuid.as_str()),
            Some("cue0")
        );
    }

    /// The skip is a millisecond wide rather than exact, because a position
    /// that arrived through a locate, a frame of playback, and a snap is never
    /// bit-identical to the cue it came from.
    #[test]
    fn a_cue_a_hair_from_the_playhead_is_skipped_too() {
        let arrangement = cued(&[10.0, 20.0]);

        assert_eq!(
            arrangement.cue_after(19.9995).map(|c| c.uuid.as_str()),
            None,
            "a cue half a millisecond ahead is the one we are standing on"
        );
        assert_eq!(
            arrangement.cue_after(19.9).map(|c| c.uuid.as_str()),
            Some("cue1"),
            "a tenth of a second ahead is a cue to walk to"
        );
        assert_eq!(
            arrangement.cue_before(20.0005).map(|c| c.uuid.as_str()),
            Some("cue0"),
            "and standing a hair past one does not count as being before it"
        );
    }

    /// Two cues at one instant are legal, because an import can produce them
    /// and refusing them would lose data. Navigation has to stay finite there.
    #[test]
    fn cues_at_the_same_instant_are_kept_and_walked_past() {
        let arrangement = cued(&[10.0, 10.0, 20.0]);

        assert_eq!(arrangement.cues.len(), 2 + 1, "neither is dropped");
        assert_eq!(
            arrangement.cue_after(5.0).map(|c| c.at),
            Some(10.0),
            "the first of the pair is the one ahead"
        );
        assert_eq!(
            arrangement.cue_after(10.0).map(|c| c.at),
            Some(20.0),
            "a press from the pair walks past both rather than sticking"
        );
        assert_eq!(
            arrangement.cue_before(15.0).map(|c| c.at),
            Some(10.0),
            "and back reaches them as one instant"
        );
    }
}
