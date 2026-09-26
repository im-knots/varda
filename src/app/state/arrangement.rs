//! Arrangement mutations on `VardaApp`.
//!
//! Lanes and regions are ordinary scene data, so every mutation here goes
//! through the same command path as the rest of the engine and lands in the
//! undo stack. The one thing that does *not* is the live override, which is
//! session state by design.
//!
//! See /spec/arrangement.md.

use crate::arrangement::Authority;
use crate::engine::{CommandResult, ErrorCode};

use super::super::VardaApp;

impl VardaApp {
    /// Whether the arrangement is driving this frame.
    pub fn arrangement_authority(&self) -> Authority {
        Authority::resolve(
            self.mixer.arrangement(),
            self.show.transport.sample().as_ref(),
        )
    }

    /// Take a parameter back from the arrangement.
    ///
    /// Called by every live write path rather than by a UI button: the gesture
    /// *is* the override, so there is nothing to confirm.
    pub fn note_live_param_write(&mut self, param_key: &str, normalized: f32) {
        // Ahead of the override, and ahead of the authority gate below it: a
        // scene with only curves in it never engages the arrangement, and
        // waiting for that would mean its first pass could never be recorded.
        self.record_param_write(param_key, normalized);
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

    // ── Cues ────────────────────────────────────────────────────────

    /// Locate to the neighbouring cue.
    ///
    /// Backwards with no earlier cue goes to zero, which is the way home now
    /// that the return-to-zero arrow walks cues instead. Forwards past the last
    /// cue stays put rather than running off the end.
    pub(crate) fn cmd_locate_cue(&mut self, forward: bool) -> CommandResult {
        let position = self.show.transport.position();
        let anchor = self.show.cue_anchor;
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
        match self.show.transport.locate(target) {
            Ok(()) => {
                self.show.cue_anchor = Some(target);
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
            return crate::mixer::ArrangementError::NoSuchCue(uuid.to_string()).into();
        };
        match self.show.transport.locate(at) {
            Ok(()) => {
                self.show.cue_anchor = Some(at);
                CommandResult::Ok
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: e.to_string(),
            },
        }
    }
}
