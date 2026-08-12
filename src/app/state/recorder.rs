//! Automation recording: keeping the gesture the performer already played.
//!
//! Arm, run the transport, and every parameter written by a hand (the mouse, a
//! MIDI knob, OSC, the API) is captured as breakpoints at the position it was
//! written. The recorder listens on the same event that powers the live
//! override, so it hears every write path without a second one being built for
//! it. See /spec/automation-recording.md.

use std::collections::HashMap;

use crate::modulation::{Breakpoint, ModulationSource};

use super::super::VardaApp;

/// Smallest change worth a breakpoint, in normalized units. Finer than a fader
/// is wide on screen, so nothing a hand can express is thinned out here.
const DEADBAND: f32 = 0.001;

/// A gap longer than this is a hold rather than a slow move.
///
/// Writes only arrive when a value changes, so a parameter left alone for four
/// seconds and then moved would otherwise be recorded as a four-second ramp.
const HOLD_SECONDS: f64 = 0.2;

/// How far before the move that ended it a hold is closed off. One frame at
/// 60fps: the hold ended just now, and saying so exactly would put two points
/// at one position.
const ANCHOR_LEAD: f64 = 1.0 / 60.0;

/// How far a point may sit from the line between its neighbours and still be
/// dropped, in normalized units.
const SIMPLIFY_TOLERANCE: f32 = 0.002;

/// One parameter being written, from the position it was first touched.
struct Take {
    /// The envelope the take will be committed into.
    envelope: String,
    points: Vec<Breakpoint>,
    /// The last value seen, whether or not it was captured, so a hold can be
    /// closed off at the value it was held at.
    last: (f64, f32),
}

/// Arm state and whatever is being written right now.
///
/// Session state: an arm is a mode the performer is in, not part of the show,
/// and a scene written by an older build has none to miss.
#[derive(Default)]
pub struct Recorder {
    armed: bool,
    takes: HashMap<String, Take>,
    /// Whether this pass has already pushed its undo entry.
    snapshot_taken: bool,
}

impl Recorder {
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Parameters with a take open, for the badge on their lanes.
    pub fn recording_params(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.takes.keys().cloned().collect();
        keys.sort();
        keys
    }
}

impl VardaApp {
    /// Arm or disarm, closing whatever was being written.
    ///
    /// Arming while stopped also rolls the transport: arming and then reaching
    /// for play is two gestures for one intent. While chasing timecode the
    /// transport is not ours to start, so this arms and waits for the master.
    pub(crate) fn set_record_armed(&mut self, armed: bool) {
        if !armed {
            self.close_takes();
        }
        self.session.recorder.armed = armed;
        if armed && !self.transport.running() {
            // A refusal here is the chase case, and arming is still what was
            // asked for.
            if let Err(e) = self.transport.play() {
                log::debug!("Record armed without rolling the transport: {e}");
            }
        }
    }

    pub fn record_armed(&self) -> bool {
        self.session.recorder.armed()
    }

    /// Whether a pass is under way and has already pushed its undo entry.
    ///
    /// Everything from the first touch to the end of the pass folds into that
    /// one entry, the way a fader drag folds into one: a take is a gesture, and
    /// undoing it a frame at a time is not what "that take was no good" means.
    pub fn is_recording(&self) -> bool {
        self.session.recorder.snapshot_taken
    }

    pub fn recording_params(&self) -> Vec<String> {
        self.session.recorder.recording_params()
    }

    /// Capture one live write, opening a take for the parameter if this is the
    /// first one.
    ///
    /// Called from [`VardaApp::note_live_param_write`] ahead of the override it
    /// also triggers, so recording works on a scene with no arrangement in it:
    /// a curve-only show never engages arrangement authority, and waiting for
    /// that would mean the first pass could never be recorded.
    pub(crate) fn record_param_write(&mut self, param_key: &str, normalized: f32) {
        if !self.session.recorder.armed || !self.transport.running() {
            return;
        }
        let at = self.transport.position();

        if let Some(take) = self.session.recorder.takes.get_mut(param_key) {
            take.capture(at, normalized);
        } else {
            if !self.session.recorder.snapshot_taken {
                // One entry for the pass: undo means "that take was no good".
                // Before the envelope is created, or undo would return to a
                // scene that already has the lane this pass is about to write.
                let snapshot = self.history_snapshot_default();
                self.push_history(snapshot);
                self.session.recorder.snapshot_taken = true;
            }
            let envelope = self.envelope_to_record_into(param_key);
            self.session.recorder.takes.insert(
                param_key.to_string(),
                Take {
                    envelope,
                    points: vec![Breakpoint::new(at, normalized)],
                    last: (at, normalized),
                },
            );
        }

        // The hand owns the parameter for as long as it is being written, or
        // the stretch of old curve still ahead of the playhead would pull the
        // value out from under it mid-take.
        self.mixer
            .modulation_mut()
            .override_param(param_key, normalized);
    }

    /// The envelope a take writes into, created if the parameter had no curve.
    ///
    /// A parameter can carry more than one assignment (an LFO beside a curve),
    /// so this looks for an envelope rather than for any assignment at all.
    fn envelope_to_record_into(&mut self, param_key: &str) -> String {
        let existing = self
            .mixer
            .modulation()
            .assignments_for(param_key)
            .iter()
            .map(|m| m.source_id.clone())
            .find(|uuid| {
                matches!(
                    self.mixer
                        .modulation()
                        .find_source_by_uuid(uuid)
                        .map(|entry| &entry.source),
                    Some(ModulationSource::Envelope { .. })
                )
            });
        existing.unwrap_or_else(|| {
            <Self as crate::engine::ModulationCommands>::add_automation_lane(
                self,
                param_key,
                crate::timebase::Timebase::Transport,
            )
        })
    }

    /// Close every open take, committing what was played.
    ///
    /// Run when the arm goes off, when the transport stops, and on any jump in
    /// position: a locate or a loop wrap ends the stretch that was being
    /// written, and the next write after it opens a take of its own.
    pub(crate) fn close_takes(&mut self) {
        let takes = std::mem::take(&mut self.session.recorder.takes);
        self.session.recorder.snapshot_taken = false;
        for (param_key, take) in takes {
            let recorded = simplify(&take.points, SIMPLIFY_TOLERANCE);
            let (Some(first), Some(last)) = (recorded.first(), recorded.last()) else {
                continue;
            };
            let (from, to) = (first.position, last.position);
            let existing = match self
                .mixer
                .modulation()
                .find_source_by_uuid(&take.envelope)
                .map(|entry| &entry.source)
            {
                Some(ModulationSource::Envelope { breakpoints, .. }) => breakpoints.clone(),
                // The envelope was deleted mid-pass. Nothing to commit into.
                _ => continue,
            };
            let merged = replace_span(&existing, &recorded, from, to);
            self.mixer
                .modulation_mut()
                .set_envelope_breakpoints(&take.envelope, merged);
            // Hand the parameter back at once rather than over the usual ramp:
            // the recorded curve passes through the value the hand left, so
            // there is nothing to ramp between.
            self.mixer.modulation_mut().rearm_param(&param_key, 0.0);
        }
    }

    /// Per-frame housekeeping, after the transport has been ticked.
    pub(crate) fn tick_recorder(&mut self) {
        if self.session.recorder.takes.is_empty() {
            return;
        }
        if !self.transport.running() || self.transport.discontinuity() {
            self.close_takes();
        }
    }
}

impl Take {
    /// Add a point for a value that just changed, keeping a hold a hold.
    fn capture(&mut self, at: f64, value: f32) {
        let (last_at, last_value) = self.last;
        self.last = (at, value);
        let Some(previous) = self.points.last().copied() else {
            self.points.push(Breakpoint::new(at, value));
            return;
        };
        if (value - previous.value).abs() < DEADBAND {
            return;
        }
        // A jump backwards is a locate the frame has not closed the take on
        // yet; dropping it keeps the take in order.
        if at <= previous.position {
            return;
        }
        if at - last_at.max(previous.position) > HOLD_SECONDS {
            let anchor = (at - ANCHOR_LEAD).max(previous.position + f64::EPSILON);
            if anchor > previous.position {
                self.points.push(Breakpoint::new(anchor, last_value));
            }
        }
        self.points.push(Breakpoint::new(at, value));
    }
}

/// Drop points that lie close enough to the line between their neighbours that
/// the envelope's own interpolation already draws them.
///
/// Shape-preserving rather than a resample: a fast gesture keeps its detail and
/// a slow ramp collapses to its endpoints. The first and last points are always
/// kept, because they are the edges of the span the take replaces.
fn simplify(points: &[Breakpoint], tolerance: f32) -> Vec<Breakpoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut kept = vec![points[0]];
    keep_between(points, 0, points.len() - 1, tolerance, &mut kept);
    kept.push(points[points.len() - 1]);
    kept
}

/// Douglas-Peucker over (position, value): keep the point furthest from the
/// chord if it is further than the tolerance, and recurse either side of it.
fn keep_between(
    points: &[Breakpoint],
    first: usize,
    last: usize,
    tolerance: f32,
    kept: &mut Vec<Breakpoint>,
) {
    if last <= first + 1 {
        return;
    }
    let (a, b) = (points[first], points[last]);
    let span = b.position - a.position;
    let mut worst = (first, 0.0_f32);
    for (idx, point) in points.iter().enumerate().take(last).skip(first + 1) {
        let t = if span.abs() < f64::EPSILON {
            0.0
        } else {
            (point.position - a.position) / span
        };
        let chord = a.value + (b.value - a.value) * t as f32;
        let error = (point.value - chord).abs();
        if error > worst.1 {
            worst = (idx, error);
        }
    }
    if worst.1 <= tolerance {
        return;
    }
    keep_between(points, first, worst.0, tolerance, kept);
    kept.push(points[worst.0]);
    keep_between(points, worst.0, last, tolerance, kept);
}

/// Put `recorded` into `existing` in place of everything between `from` and
/// `to`.
///
/// The touched span and nothing else: the rest of the curve was fine, which is
/// why it was not touched. Same rule pasting a curve uses.
fn replace_span(
    existing: &[Breakpoint],
    recorded: &[Breakpoint],
    from: f64,
    to: f64,
) -> Vec<Breakpoint> {
    let mut out: Vec<Breakpoint> = existing
        .iter()
        .copied()
        .filter(|p| p.position < from || p.position > to)
        .collect();
    out.extend_from_slice(recorded);
    out.sort_by(|a, b| a.position.total_cmp(&b.position));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineCommand;

    /// A headless app with one deck, and that deck's opacity key.
    ///
    /// Returns `None` where there is no GPU, like every other test that needs
    /// a real engine.
    fn app_with_a_deck() -> Option<(VardaApp, String, String)> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let mut app = VardaApp::new(gpu, &crate::testing::headless_config()).ok()?;
        let channel = app.mixer_ref().channels()[0].uuid().to_string();
        let deck = match app.execute_command(EngineCommand::AddSolidColorDeck {
            channel_uuid: channel,
            color: [1.0, 1.0, 1.0, 1.0],
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("expected the new deck's uuid, got {other:?}"),
        };
        let key = crate::arrangement::opacity_param_key(&deck);
        Some((app, deck, key))
    }

    /// Play a pass by hand: each move happens `dt` seconds of show later.
    ///
    /// The transport is ticked rather than waited on, so the positions in the
    /// recorded curve are the ones the test asked for.
    fn play_pass(app: &mut VardaApp, deck: &str, moves: &[(f64, f32)]) {
        app.set_record_armed(true);
        for (dt, opacity) in moves {
            app.transport.tick(*dt);
            app.tick_recorder();
            app.execute_command(EngineCommand::SetDeckOpacity {
                deck_uuid: deck.to_string(),
                opacity: *opacity,
            });
        }
        app.set_record_armed(false);
    }

    fn curve(app: &VardaApp, param_key: &str) -> Vec<Breakpoint> {
        let modulation = app.mixer_ref().modulation();
        let uuid = modulation.assignments_for(param_key).first().map_or_else(
            || panic!("nothing is assigned to '{param_key}'"),
            |m| m.source_id.clone(),
        );
        match modulation.find_source_by_uuid(&uuid).map(|e| &e.source) {
            Some(ModulationSource::Envelope { breakpoints, .. }) => breakpoints.clone(),
            _ => panic!("'{param_key}' should be driven by an envelope"),
        }
    }

    /// The point of the feature: play the show and keep what you played. The
    /// curve is created on the spot, because a performer reaching for a fader
    /// has not first gone to the timeline to make one.
    #[test]
    fn a_pass_leaves_a_curve_on_a_parameter_that_had_none() {
        let Some((mut app, deck, key)) = app_with_a_deck() else {
            return;
        };
        play_pass(&mut app, &deck, &[(4.0, 0.2), (2.0, 0.6), (2.0, 1.0)]);

        let recorded: Vec<(f64, f32)> = curve(&app, &key)
            .iter()
            .map(|p| ((p.position * 1000.0).round() / 1000.0, p.value))
            .collect();
        assert_eq!(
            recorded,
            vec![
                (4.0, 0.2),
                // A hand that was still holds its value until it moves, so each
                // move is anchored a frame before it rather than ramped into
                // across the seconds nobody touched anything.
                (5.983, 0.2),
                (6.0, 0.6),
                (7.983, 0.6),
                (8.0, 1.0),
            ],
            "the gesture, at the positions it was played"
        );
    }

    /// Punching in on one phrase leaves the rest of the curve alone, which is
    /// what makes a second pass a fix rather than a rewrite.
    #[test]
    fn a_second_pass_replaces_only_the_stretch_it_covered() {
        let Some((mut app, deck, key)) = app_with_a_deck() else {
            return;
        };
        let envelope = match app.execute_command(EngineCommand::AddAutomationLane {
            target: key.clone(),
            timebase: crate::timebase::Timebase::Transport,
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("expected the new envelope's uuid, got {other:?}"),
        };
        app.execute_command(EngineCommand::SetEnvelopeBreakpoints {
            uuid: envelope,
            breakpoints: vec![
                Breakpoint::new(0.0, 0.0),
                Breakpoint::new(10.0, 1.0),
                Breakpoint::new(30.0, 0.0),
            ],
        });

        play_pass(&mut app, &deck, &[(8.0, 0.25), (4.0, 0.3)]);

        let positions: Vec<f64> = curve(&app, &key)
            .iter()
            .map(|p| (p.position * 1000.0).round() / 1000.0)
            .collect();
        assert_eq!(
            positions,
            // 11.983 is the hold anchor under the second move.
            vec![0.0, 8.0, 11.983, 12.0, 30.0],
            "the points either side survive, and the one at 10s inside the pass \
             is replaced by what was played over it"
        );
    }

    /// A take that outlived the pass would leave the hand holding the parameter
    /// against the curve it just recorded.
    #[test]
    fn the_parameter_is_handed_back_when_the_pass_ends() {
        let Some((mut app, deck, key)) = app_with_a_deck() else {
            return;
        };
        app.set_record_armed(true);
        app.transport.tick(4.0);
        app.execute_command(EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.2,
        });
        assert!(
            app.mixer_ref().modulation().is_overridden(&key),
            "the hand owns the parameter while it is being written"
        );

        app.set_record_armed(false);
        assert!(
            !app.mixer_ref().modulation().is_overridden(&key),
            "and gives it back to the recorded curve at the end of the pass"
        );
    }

    /// A jump ends the stretch that was being written. Without this a loop wrap
    /// would fold two passes over the same bars into one take running backwards.
    #[test]
    fn a_jump_in_position_closes_the_take() {
        let Some((mut app, deck, key)) = app_with_a_deck() else {
            return;
        };
        app.set_record_armed(true);
        app.transport.tick(10.0);
        app.execute_command(EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.2,
        });
        app.execute_command(EngineCommand::TransportLocate { position: 2.0 });
        app.tick_recorder();
        app.execute_command(EngineCommand::SetDeckOpacity {
            deck_uuid: deck.clone(),
            opacity: 0.9,
        });
        app.set_record_armed(false);

        let positions: Vec<f64> = curve(&app, &key).iter().map(|p| p.position).collect();
        assert_eq!(
            positions,
            vec![2.0, 10.0],
            "each side of the jump was written where it was played"
        );
    }

    fn take() -> Take {
        Take {
            envelope: "env00001".to_string(),
            points: vec![Breakpoint::new(0.0, 0.0)],
            last: (0.0, 0.0),
        }
    }

    fn positions(points: &[Breakpoint]) -> Vec<f64> {
        points.iter().map(|p| p.position).collect()
    }

    /// A fader is noisy at rest: an unmoved control reporting the same value
    /// would otherwise leave a point per frame on the curve.
    #[test]
    fn a_value_that_has_not_really_moved_leaves_no_point() {
        let mut take = take();
        take.capture(0.05, 0.0000_5);
        assert_eq!(take.points.len(), 1, "a twitch is not a gesture");
        take.capture(0.10, 0.5);
        assert_eq!(take.points.len(), 2, "a real move is");
    }

    /// Writes only arrive when a value changes, so a parameter held for seconds
    /// and then moved has to be recorded as a hold and a move rather than as
    /// one long ramp between the two.
    #[test]
    fn a_hold_is_closed_off_before_the_move_that_ended_it() {
        let mut take = take();
        take.capture(0.1, 0.5);
        take.capture(5.0, 1.0);

        let held = take.points[2];
        assert!(
            (held.value - 0.5).abs() < f32::EPSILON,
            "the anchor holds the value the parameter sat at"
        );
        assert!(
            held.position > 4.9 && held.position < 5.0,
            "and sits just before the move, at {}",
            held.position
        );
        assert!((take.points[3].value - 1.0).abs() < f32::EPSILON);
    }

    /// Two writes a frame apart are a gesture, not a hold, and anchoring every
    /// one of them would double the points on every drag.
    #[test]
    fn a_continuous_gesture_gets_no_anchors() {
        let mut take = take();
        for frame in 1..=10 {
            take.capture(f64::from(frame) / 60.0, f64::from(frame) as f32 / 10.0);
        }
        assert_eq!(take.points.len(), 11, "{:?}", positions(&take.points));
    }

    /// A straight ramp is two points however many frames it was played over.
    #[test]
    fn a_ramp_collapses_to_its_ends() {
        let points: Vec<Breakpoint> = (0..=60)
            .map(|i| Breakpoint::new(f64::from(i) / 60.0, f64::from(i) as f32 / 60.0))
            .collect();
        assert_eq!(simplify(&points, SIMPLIFY_TOLERANCE).len(), 2);
    }

    /// The corner of a gesture is the shape of it, so it survives the pass that
    /// throws the straight stretches away.
    #[test]
    fn a_corner_survives_simplification() {
        let mut points: Vec<Breakpoint> = (0..=30)
            .map(|i| Breakpoint::new(f64::from(i) / 60.0, f64::from(i) as f32 / 30.0))
            .collect();
        points.extend(
            (1..=30).map(|i| {
                Breakpoint::new(0.5 + f64::from(i) / 60.0, 1.0 - f64::from(i) as f32 / 30.0)
            }),
        );
        let simplified = simplify(&points, SIMPLIFY_TOLERANCE);
        assert_eq!(simplified.len(), 3, "{:?}", positions(&simplified));
        assert!((simplified[1].position - 0.5).abs() < 1e-9);
    }

    /// Punching in on one phrase leaves the rest of the show as it was.
    #[test]
    fn a_take_replaces_the_span_it_covered_and_nothing_else() {
        let existing = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(5.0, 1.0),
            Breakpoint::new(10.0, 0.0),
            Breakpoint::new(20.0, 1.0),
        ];
        let recorded = vec![Breakpoint::new(4.0, 0.25), Breakpoint::new(12.0, 0.75)];
        let merged = replace_span(&existing, &recorded, 4.0, 12.0);

        assert_eq!(positions(&merged), vec![0.0, 4.0, 12.0, 20.0]);
    }

    /// The points that bound a take are its own, so a take that starts exactly
    /// where an old point sits replaces it rather than landing beside it.
    #[test]
    fn an_old_point_on_the_boundary_gives_way() {
        let existing = vec![Breakpoint::new(4.0, 1.0), Breakpoint::new(8.0, 1.0)];
        let recorded = vec![Breakpoint::new(4.0, 0.0), Breakpoint::new(8.0, 0.0)];
        let merged = replace_span(&existing, &recorded, 4.0, 8.0);

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().all(|p| p.value == 0.0));
    }
}
