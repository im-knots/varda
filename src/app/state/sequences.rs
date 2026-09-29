//! Running a transition sequence under the arrangement's authority.

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};

impl VardaApp {
    pub(crate) fn cmd_play_sequence(&mut self, sequence_uuid: &str) -> CommandResult {
        // A Fade step and the arrangement's crossfader automation would overwrite
        // each other every frame. The arrangement may still fire a sequence as a cue.
        if self.arrangement_authority().is_engaged() {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "the arrangement holds authority; stop the transport to \
                          run a transition sequence freely"
                    .to_string(),
            };
        }
        match self.mixer.start_sequence(sequence_uuid) {
            Ok(()) => CommandResult::Ok,
            Err(e) => e.into(),
        }
    }
}
