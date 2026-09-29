//! Automation recording. While armed and the transport runs, every live
//! parameter write (mouse, MIDI, OSC, API) is captured as breakpoints. The
//! recorder hooks the same call as the live override.

use std::collections::HashMap;

use crate::modulation::{Breakpoint, ModulationSource};

use super::super::VardaApp;

/// Smallest change recorded as a breakpoint, in normalized units. Finer than
/// a fader's on-screen resolution.
const DEADBAND: f32 = 0.001;

/// A gap longer than this, in seconds, is a hold rather than a slow move.
/// Writes arrive only on change, so without this a long hold records as a ramp.
const HOLD_SECONDS: f64 = 0.2;

/// How far before the next move a hold's closing point is placed, in seconds.
/// One frame at 60fps, so two points never share a position.
const ANCHOR_LEAD: f64 = 1.0 / 60.0;

/// How far a point may sit from the line between its neighbors and still be
/// dropped, in normalized units.
const SIMPLIFY_TOLERANCE: f32 = 0.002;

/// One parameter being written, from the position it was first touched.
struct Take {
    /// The envelope the take will be committed into.
    envelope: String,
    points: Vec<Breakpoint>,
    /// The last value seen, captured or not, used to close a hold.
    last: (f64, f32),
}

/// Arm state and open takes. Session state; not saved with the scene.
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
    /// Arm or disarm, closing open takes.
    ///
    /// Arming while stopped also starts the transport, except while chasing
    /// timecode, where it arms and waits for the master.
    pub(crate) fn set_record_armed(&mut self, armed: bool) {
        if !armed {
            self.close_takes();
        }
        self.show.recorder.armed = armed;
        if armed && !self.show.transport.running() {
            // Refused while chasing timecode; stay armed.
            if let Err(e) = self.show.transport.play() {
                log::debug!("Record armed without rolling the transport: {e}");
            }
        }
    }

    /// Whether a pass is under way and has pushed its undo entry. A whole pass
    /// is one undo step.
    pub fn is_recording(&self) -> bool {
        self.show.recorder.snapshot_taken
    }

    /// Capture one live write, opening a take for the parameter if needed.
    ///
    /// Called from [`VardaApp::note_live_param_write`] before the authority
    /// gate, since a curve-only scene never engages the arrangement.
    pub(crate) fn record_param_write(&mut self, param_key: &str, normalized: f32) {
        if !self.show.recorder.armed || !self.show.transport.running() {
            return;
        }
        let at = self.show.transport.position();

        if let Some(take) = self.show.recorder.takes.get_mut(param_key) {
            take.capture(at, normalized);
        } else {
            if !self.show.recorder.snapshot_taken {
                // One undo entry per pass, taken before the envelope is created
                // so undo also removes a new lane.
                let snapshot = self.history_snapshot();
                self.session.history.push(snapshot);
                self.show.recorder.snapshot_taken = true;
            }
            let envelope = self.envelope_to_record_into(param_key);
            self.show.recorder.takes.insert(
                param_key.to_string(),
                Take {
                    envelope,
                    points: vec![Breakpoint::new(at, normalized)],
                    last: (at, normalized),
                },
            );
        }

        // Override the parameter during the take, or the old curve ahead of
        // the playhead would fight the live value.
        self.mixer
            .modulation_mut()
            .override_param(param_key, normalized);
    }

    /// The envelope a take writes into, created if the parameter had no curve.
    ///
    /// Looks for an envelope specifically, since a parameter can also carry an LFO.
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
            self.mixer
                .modulation_mut()
                .add_automation_lane(param_key, crate::timebase::Timebase::Transport)
        })
    }

    /// Close and commit every open take. Runs on disarm, transport stop, and
    /// any position jump (locate or loop wrap).
    pub(crate) fn close_takes(&mut self) {
        let takes = std::mem::take(&mut self.show.recorder.takes);
        self.show.recorder.snapshot_taken = false;
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
                // Envelope deleted mid-pass.
                _ => continue,
            };
            let merged = replace_span(&existing, &recorded, from, to);
            self.mixer
                .modulation_mut()
                .set_envelope_breakpoints(&take.envelope, merged);
            // No re-arm ramp: the recorded curve already ends at the live value.
            self.mixer.modulation_mut().rearm_param(&param_key, 0.0);
        }
    }

    /// Per-frame housekeeping, after the transport has been ticked.
    pub(crate) fn tick_recorder(&mut self) {
        if self.show.recorder.takes.is_empty() {
            return;
        }
        if !self.show.transport.running() || self.show.transport.discontinuity() {
            self.close_takes();
        }
    }
}

impl Take {
    /// Add a point for a changed value, anchoring the end of a hold.
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
        // A backward jump is a locate not yet handled; drop it to keep order.
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

/// Drop points the envelope's interpolation already draws. Fast gestures keep
/// detail and straight ramps collapse to endpoints. The first and last points
/// are always kept, since they bound the replaced span.
fn simplify(points: &[Breakpoint], tolerance: f32) -> Vec<Breakpoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut kept = vec![points[0]];
    keep_between(points, 0, points.len() - 1, tolerance, &mut kept);
    kept.push(points[points.len() - 1]);
    kept
}

/// Douglas-Peucker over (position, value).
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

/// Replace everything in `existing` between `from` and `to` with `recorded`.
/// Points outside the span are kept, as when pasting a curve.
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

    /// A headless app with one deck, and that deck's opacity key. `None` without a GPU.
    fn app_with_a_deck() -> Option<(VardaApp, String, String)> {
        let mut app = crate::testing::headless_app()?;
        let channel = app.mixer_ref().channels()[0].uuid().to_string();
        let deck = match app.execute_command(EngineCommand::AddDeck {
            channel_uuid: channel,
            source: crate::solid_color::SolidColor::config_for([1.0, 1.0, 1.0, 1.0]),
        }) {
            crate::engine::CommandResult::OkWithId { uuid } => uuid,
            other => panic!("expected the new deck's uuid, got {other:?}"),
        };
        let key = crate::arrangement::opacity_param_key(&deck);
        Some((app, deck, key))
    }

    /// Play a pass: each move is `dt` seconds of show later. The transport is
    /// ticked, so recorded positions are exact.
    fn play_pass(app: &mut VardaApp, deck: &str, moves: &[(f64, f32)]) {
        app.set_record_armed(true);
        for (dt, opacity) in moves {
            app.show.transport.tick(*dt);
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

    /// A pass is recorded, creating the curve if there was none.
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
                // Hold anchor one frame before each move.
                (5.983, 0.2),
                (6.0, 0.6),
                (7.983, 0.6),
                (8.0, 1.0),
            ],
            "the gesture, at the positions it was played"
        );
    }

    /// Punching in on one phrase leaves the rest of the curve alone.
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

    /// Takes close at the end of the pass and release the override.
    #[test]
    fn the_parameter_is_handed_back_when_the_pass_ends() {
        let Some((mut app, deck, key)) = app_with_a_deck() else {
            return;
        };
        app.set_record_armed(true);
        app.show.transport.tick(4.0);
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

    /// A jump closes the take, so a loop wrap starts a new one.
    #[test]
    fn a_jump_in_position_closes_the_take() {
        let Some((mut app, deck, key)) = app_with_a_deck() else {
            return;
        };
        app.set_record_armed(true);
        app.show.transport.tick(10.0);
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

    /// Repeated identical values add no points.
    #[test]
    fn a_value_that_has_not_really_moved_leaves_no_point() {
        let mut take = take();
        take.capture(0.05, 0.0000_5);
        assert_eq!(take.points.len(), 1, "a twitch is not a gesture");
        take.capture(0.10, 0.5);
        assert_eq!(take.points.len(), 2, "a real move is");
    }

    /// A long gap between writes records as a hold then a move, not a ramp.
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

    /// Writes a frame apart get no hold anchor.
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

    /// Simplification keeps corners.
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

    /// Replacing a span leaves points outside it.
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

    /// A take starting exactly on an old point replaces that point.
    #[test]
    fn an_old_point_on_the_boundary_gives_way() {
        let existing = vec![Breakpoint::new(4.0, 1.0), Breakpoint::new(8.0, 1.0)];
        let recorded = vec![Breakpoint::new(4.0, 0.0), Breakpoint::new(8.0, 0.0)];
        let merged = replace_span(&existing, &recorded, 4.0, 8.0);

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().all(|p| p.value == 0.0));
    }
}
