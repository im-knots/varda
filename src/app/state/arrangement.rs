//! Arrangement mutations on `VardaApp`.
//!
//! Lanes and regions are scene data and go through the undo stack. The live
//! override is session state and does not.

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

    /// Take a parameter back from the arrangement. Called by every live write path.
    pub fn note_live_param_write(&mut self, param_key: &str, normalized: f32) {
        // Before the authority gate: a scene with only curves never engages the
        // arrangement, so its first pass must still be recorded.
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

    /// [`Self::note_live_param_write`] for a router path (OSC, MIDI, API).
    /// Route values are already normalized.
    pub fn note_live_route_write(&mut self, path: &str, normalized: f32) {
        if let Some(key) = crate::param_router::modulation_key_for_path(path) {
            self.note_live_param_write(&key, normalized);
        }
    }

    // ── Cues ────────────────────────────────────────────────────────

    /// Locate to the neighboring cue. Backwards with no earlier cue goes to
    /// zero; forwards past the last cue stays put.
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

    /// Locate to one cue (a Performance mode cue bank button). The transport's
    /// run state is unchanged, and the cue arrows continue from this cue.
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
