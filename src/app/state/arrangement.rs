//! Arrangement mutations on `VardaApp`.
//!
//! Lanes and regions are ordinary scene data, so every mutation here goes
//! through the same command path as the rest of the engine and lands in the
//! undo stack. The one thing that does *not* is the live override, which is
//! session state by design.
//!
//! See /spec/arrangement.md.

use crate::arrangement::{Authority, LaneConfig, RegionConfig, DEFAULT_REARM_SECONDS};
use crate::engine::{CommandResult, ErrorCode};

use super::super::VardaApp;

impl VardaApp {
    /// Whether the arrangement is driving this frame.
    pub fn arrangement_authority(&self) -> Authority {
        Authority::resolve(self.mixer.arrangement(), self.transport.sample().as_ref())
    }

    fn deck_exists(&self, deck_uuid: &str) -> bool {
        self.mixer.find_deck_by_uuid(deck_uuid).is_some()
    }

    fn no_such_deck(deck_uuid: &str) -> CommandResult {
        CommandResult::Err {
            code: ErrorCode::NotFound,
            message: format!("Deck '{deck_uuid}' not found"),
        }
    }

    fn no_such_lane(deck_uuid: &str) -> CommandResult {
        CommandResult::Err {
            code: ErrorCode::NotFound,
            message: format!("No arrangement lane for deck '{deck_uuid}'"),
        }
    }

    /// Add a lane for a deck, or return the existing one.
    ///
    /// A lane *is* the deck, so this creates no entity and is idempotent: two
    /// callers racing to arrange the same deck must not produce two rows.
    pub(crate) fn cmd_add_lane(&mut self, deck_uuid: &str) -> CommandResult {
        if !self.deck_exists(deck_uuid) {
            return Self::no_such_deck(deck_uuid);
        }
        let arrangement = self.mixer.arrangement_mut();
        if arrangement.lane(deck_uuid).is_none() {
            arrangement.lanes.push(LaneConfig::new(deck_uuid));
        }
        CommandResult::Ok
    }

    /// Remove a lane and everything it drove, handing the deck back to
    /// Performance mode.
    pub(crate) fn cmd_remove_lane(&mut self, deck_uuid: &str) -> CommandResult {
        let Some(arrangement) = self.mixer.arrangement().cloned() else {
            return Self::no_such_lane(deck_uuid);
        };
        let Some(lane) = arrangement.lane(deck_uuid) else {
            return Self::no_such_lane(deck_uuid);
        };

        // Envelopes belong to the modulation graph, so removing the row has to
        // take them with it or the deck stays driven by an orphan curve.
        for uuid in lane.envelope_uuids() {
            self.mixer.modulation_mut().remove_source(uuid);
        }
        let arrangement = self.mixer.arrangement_mut();
        arrangement.lanes.retain(|l| l.deck_uuid != deck_uuid);
        CommandResult::Ok
    }

    /// Add a visibility span to a lane, creating the lane if needed.
    pub(crate) fn cmd_add_region(
        &mut self,
        deck_uuid: &str,
        region: RegionConfig,
    ) -> CommandResult {
        if !region.is_valid() {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "a region must end after it starts".to_string(),
            };
        }
        if let CommandResult::Err { code, message } = self.cmd_add_lane(deck_uuid) {
            return CommandResult::Err { code, message };
        }

        let Some(lane) = self.mixer.arrangement_mut().lane_mut(deck_uuid) else {
            return Self::no_such_lane(deck_uuid);
        };
        lane.regions.push(region);
        let index = lane.regions.len() - 1;
        self.mixer.sync_lane_opacity_envelope(deck_uuid);
        CommandResult::OkWithData {
            data: serde_json::json!({ "index": index }),
        }
    }

    /// Replace one region in place, for a move, a resize, or a fade drag.
    pub(crate) fn cmd_update_region(
        &mut self,
        deck_uuid: &str,
        index: usize,
        region: RegionConfig,
    ) -> CommandResult {
        if !region.is_valid() {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "a region must end after it starts".to_string(),
            };
        }
        let Some(lane) = self.mixer.arrangement_mut().lane_mut(deck_uuid) else {
            return Self::no_such_lane(deck_uuid);
        };
        let Some(slot) = lane.regions.get_mut(index) else {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Region {index} not found on lane '{deck_uuid}'"),
            };
        };
        *slot = region;
        self.mixer.sync_lane_opacity_envelope(deck_uuid);
        CommandResult::Ok
    }

    pub(crate) fn cmd_remove_region(&mut self, deck_uuid: &str, index: usize) -> CommandResult {
        let Some(lane) = self.mixer.arrangement_mut().lane_mut(deck_uuid) else {
            return Self::no_such_lane(deck_uuid);
        };
        if index >= lane.regions.len() {
            return CommandResult::Err {
                code: ErrorCode::NotFound,
                message: format!("Region {index} not found on lane '{deck_uuid}'"),
            };
        }
        lane.regions.remove(index);
        self.mixer.sync_lane_opacity_envelope(deck_uuid);
        CommandResult::Ok
    }

    /// Fold a lane's automation rows away, or open them again.
    pub(crate) fn cmd_set_lane_collapsed(
        &mut self,
        deck_uuid: &str,
        collapsed: bool,
    ) -> CommandResult {
        let Some(lane) = self.mixer.arrangement_mut().lane_mut(deck_uuid) else {
            return Self::no_such_lane(deck_uuid);
        };
        lane.collapsed = collapsed;
        CommandResult::Ok
    }

    pub(crate) fn cmd_set_idle_behaviour(
        &mut self,
        idle: crate::arrangement::IdleBehaviour,
    ) -> CommandResult {
        if let crate::arrangement::IdleBehaviour::ShowDeck { deck_uuid } = &idle {
            if !self.deck_exists(deck_uuid) {
                return Self::no_such_deck(deck_uuid);
            }
        }
        self.mixer.arrangement_mut().idle = idle;
        CommandResult::Ok
    }

    /// Take a parameter back from the arrangement.
    ///
    /// Called by every live write path rather than by a UI button: the gesture
    /// *is* the override, so there is nothing to confirm.
    pub fn note_live_param_write(&mut self, param_key: &str, normalized: f32) {
        if !self.arrangement_authority().is_engaged() {
            return;
        }
        if !self.mixer.modulation().has_modulation(param_key) {
            return;
        }
        self.mixer
            .modulation_mut()
            .override_param(param_key, normalized);
    }

    /// [`Self::note_live_param_write`] for a value that arrived as a router
    /// path, which is how OSC, MIDI, and the API address parameters.
    ///
    /// Route values are already normalized, which is exactly what the re-arm
    /// ramp needs to start from.
    pub fn note_live_route_write(&mut self, path: &str, normalized: f32) {
        if let Some(key) = crate::param_router::modulation_key_for_path(path) {
            self.note_live_param_write(&key, normalized);
        }
    }

    /// Hand one parameter back to the show, ramping rather than snapping.
    pub(crate) fn cmd_rearm_param(
        &mut self,
        param_key: &str,
        seconds: Option<f64>,
    ) -> CommandResult {
        let duration = seconds.unwrap_or(DEFAULT_REARM_SECONDS);
        self.mixer.modulation_mut().rearm_param(param_key, duration);
        CommandResult::Ok
    }

    pub(crate) fn cmd_rearm_all(&mut self, seconds: Option<f64>) -> CommandResult {
        let duration = seconds.unwrap_or(DEFAULT_REARM_SECONDS);
        self.mixer.modulation_mut().rearm_all(duration);
        CommandResult::Ok
    }

    // ── Cues ────────────────────────────────────────────────────────

    /// Mark an instant worth returning to.
    ///
    /// An empty name is filled in from how many cues exist, so the common case
    /// (drop one and keep working) still produces something the arrows can be
    /// read against.
    pub(crate) fn cmd_add_cue(&mut self, at: f64, name: &str) -> CommandResult {
        if !at.is_finite() || at < 0.0 {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "a cue sits at a position of zero or more".to_string(),
            };
        }
        let arrangement = self.mixer.arrangement_mut();
        let name = if name.is_empty() {
            format!("Cue {}", arrangement.cues.len() + 1)
        } else {
            name.to_string()
        };
        let cue = crate::arrangement::Cue {
            uuid: crate::deck::generate_short_uuid(),
            name,
            at,
        };
        let uuid = cue.uuid.clone();
        arrangement.add_cue(cue);
        CommandResult::OkWithId { uuid }
    }

    /// Move or rename a cue. Absent fields are left alone, so a drag does not
    /// have to restate the name.
    pub(crate) fn cmd_update_cue(
        &mut self,
        uuid: &str,
        at: Option<f64>,
        name: Option<String>,
    ) -> CommandResult {
        if at.is_some_and(|at| !at.is_finite() || at < 0.0) {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "a cue sits at a position of zero or more".to_string(),
            };
        }
        let arrangement = self.mixer.arrangement_mut();
        let Some(cue) = arrangement.cue_mut(uuid) else {
            return Self::no_such_cue(uuid);
        };
        if let Some(at) = at {
            cue.at = at;
        }
        if let Some(name) = name {
            cue.name = name;
        }
        // A move can reorder the list, and navigation reads it in order.
        arrangement.sort_cues();
        CommandResult::Ok
    }

    pub(crate) fn cmd_remove_cue(&mut self, uuid: &str) -> CommandResult {
        let arrangement = self.mixer.arrangement_mut();
        let before = arrangement.cues.len();
        arrangement.cues.retain(|c| c.uuid != uuid);
        if arrangement.cues.len() == before {
            return Self::no_such_cue(uuid);
        }
        CommandResult::Ok
    }

    /// Locate to the neighbouring cue.
    ///
    /// Backwards with no earlier cue goes to zero, which is the way home now
    /// that the return-to-zero arrow walks cues instead. Forwards past the last
    /// cue stays put rather than running off the end.
    pub(crate) fn cmd_locate_cue(&mut self, forward: bool) -> CommandResult {
        let position = self.transport.position();
        let anchor = self.session.cue_anchor;
        let arrangement = self.mixer.arrangement();
        let from = arrangement.map_or(position, |a| a.cue_walk_origin(anchor, position));
        let target = if forward {
            match arrangement.and_then(|a| a.cue_after(from)) {
                Some(cue) => cue.at,
                None => return CommandResult::Ok,
            }
        } else {
            arrangement
                .and_then(|a| a.cue_before(from))
                .map_or(0.0, |cue| cue.at)
        };
        match self.transport.locate(target) {
            Ok(()) => {
                self.session.cue_anchor = Some(target);
                CommandResult::Ok
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: e.to_string(),
            },
        }
    }

    /// Locate to one cue by name, which is what a button in the Performance
    /// mode cue bank does.
    ///
    /// The transport keeps running or staying stopped, because a cue is a place
    /// rather than a way to start a show. It counts as a step of the walk, so
    /// the arrows carry on from the cue that was pressed.
    pub(crate) fn cmd_trigger_cue(&mut self, uuid: &str) -> CommandResult {
        let Some(at) = self
            .mixer
            .arrangement()
            .and_then(|a| a.cues.iter().find(|c| c.uuid == uuid))
            .map(|cue| cue.at)
        else {
            return Self::no_such_cue(uuid);
        };
        match self.transport.locate(at) {
            Ok(()) => {
                self.session.cue_anchor = Some(at);
                CommandResult::Ok
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: e.to_string(),
            },
        }
    }

    /// Forget the walk, because the playhead moved by something that is not an
    /// arrow: a scrub, a locate, a return to zero, or a timecode master.
    pub(crate) fn forget_cue_walk(&mut self) {
        self.session.cue_anchor = None;
    }

    fn no_such_cue(uuid: &str) -> CommandResult {
        CommandResult::Err {
            code: ErrorCode::NotFound,
            message: format!("No cue '{uuid}'"),
        }
    }
}
