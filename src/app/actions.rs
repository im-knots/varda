//! The GUI's command drain: runs a frame's `EngineCommand`s in order, records
//! the undo step the consumer asked for, and raises the GUI's toasts.

use super::VardaApp;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand};

impl VardaApp {
    /// Run the GUI's commands for this frame, in order.
    ///
    /// When `starts_undo_step` is set, the state before anything runs becomes
    /// one undo step, kept only if at least one undoable command succeeds: a
    /// frame whose edits were all rejected changed nothing, and recording it
    /// would clear the redo history. The consumer decides when a step starts,
    /// because only it knows whether a drag is continuing.
    pub fn apply_engine_actions(&mut self, commands: Vec<EngineCommand>, starts_undo_step: bool) {
        let before = starts_undo_step.then(|| self.history_snapshot());
        let mut edited = false;
        // Ordering within the vec is preserved, so a new-channel library drop
        // enqueues `AddChannel` before its `Add*Deck` and the deck resolves
        // against the freshly created channel.
        for cmd in commands {
            let is_deck_add = command_is_deck_add(&cmd);
            let success_toast = gui_success_toast(&cmd);
            let undoable = self.is_undoable(&cmd);
            let outcome = self.execute_command_gui(cmd);
            edited |=
                undoable && !matches!(outcome, CommandOutcome::Plain(CommandResult::Err { .. }));
            if is_deck_add {
                self.notify_deck_add_outcome(&outcome);
            }
            if let (Some(toast), CommandOutcome::Plain(CommandResult::Ok)) =
                (success_toast, &outcome)
            {
                self.session.notifications.info(toast);
            }
        }
        if let Some(snapshot) = before
            && edited
        {
            self.session.history.push(snapshot);
        }
    }

    /// Toast a deck-creating command's outcome. The engine logic lives in the
    /// command; this only reports success or failure.
    fn notify_deck_add_outcome(&mut self, outcome: &CommandOutcome) {
        match outcome {
            CommandOutcome::DecksCreated { uuids } => {
                for uuid in uuids {
                    let Ok((ch_idx, deck_idx)) = self.mixer.resolve_deck(uuid) else {
                        continue;
                    };
                    let name = self.mixer.channels()[ch_idx].decks[deck_idx]
                        .deck
                        .source_name()
                        .to_string();
                    self.session
                        .notifications
                        .info(format!("➕ {} → Ch {}", name, ch_idx + 1));
                }
            }
            CommandOutcome::Plain(CommandResult::Err { message, .. }) => {
                log::error!("Failed to add deck: {message}");
                self.session
                    .notifications
                    .error(format!("Failed to add deck: {message}"));
            }
            CommandOutcome::Plain(_) => {}
        }
    }

    /// Update controller LEDs based on current state.
    pub fn update_controller_leds(&mut self) {
        if let Some(mgr) = &self.input.midi_devices {
            self.input.controller_led_mgr.update_leds(
                mgr,
                &self.input.midi_mappings,
                &self.mixer,
                self.input.midi_mappings.learn_mode,
                self.input.midi_mappings.learn_target.as_deref(),
            );
            self.input.auto_map_engine.update_leds(mgr, &self.mixer);
        }
    }
}

/// The toast the GUI shows when a command it sent succeeds, for commands whose
/// success is otherwise invisible. Failures toast from the command itself.
fn gui_success_toast(cmd: &EngineCommand) -> Option<&'static str> {
    match cmd {
        EngineCommand::Undo => Some("↩ Undo"),
        EngineCommand::Redo => Some("↪ Redo"),
        EngineCommand::SaveWorkspace => Some("💾 Workspace saved"),
        _ => None,
    }
}

/// True for the deck-creating commands the GUI drain toasts. Mirrors the deck-add arm list in `execute_command_gui`.
pub(crate) fn command_is_deck_add(cmd: &EngineCommand) -> bool {
    matches!(cmd, EngineCommand::AddDeck { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch() -> String {
        "ch-uuid".to_string()
    }

    #[test]
    fn a_deck_add_of_any_source_type_is_recognized() {
        for source in ["Shader", "Image", "Camera", "FutureThing"] {
            assert!(command_is_deck_add(&EngineCommand::AddDeck {
                channel_uuid: ch(),
                source: crate::source::SourceConfig::new(source),
            }));
        }
    }

    #[test]
    fn non_deck_commands_are_rejected() {
        let others = [
            EngineCommand::AddChannel,
            EngineCommand::RemoveChannel { channel_uuid: ch() },
            EngineCommand::RemoveDeck {
                deck_uuid: "d".into(),
            },
            EngineCommand::SetCrossfader(0.5),
            EngineCommand::SetDeckOpacity {
                deck_uuid: "d".into(),
                opacity: 0.5,
            },
        ];
        for cmd in &others {
            assert!(!command_is_deck_add(cmd), "wrongly recognized: {cmd:?}");
        }
    }
}
