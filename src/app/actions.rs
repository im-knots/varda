//! The GUI's command drain: runs a frame's `EngineCommand`s in order, records
//! the undo step the consumer asked for, and raises the GUI's toasts.

use super::VardaApp;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand};

impl VardaApp {
    /// Run the GUI's commands for this frame, in order.
    ///
    /// When `starts_undo_step` is set, the pre-mutation state is recorded as one
    /// undo step before anything runs. The consumer decides that, because only
    /// it knows whether a drag is continuing.
    pub fn apply_engine_actions(&mut self, commands: Vec<EngineCommand>, starts_undo_step: bool) {
        if starts_undo_step {
            let snapshot = self.history_snapshot();
            self.push_history(snapshot);
        }
        // Ordering within the vec is preserved, so a new-channel library drop
        // enqueues `AddChannel` before its `Add*Deck` and the deck resolves
        // against the freshly created channel.
        for cmd in commands {
            let is_deck_add = command_is_deck_add(&cmd);
            let success_toast = gui_success_toast(&cmd);
            let outcome = self.execute_command_gui(cmd);
            if is_deck_add {
                self.notify_deck_add_outcome(&outcome);
            }
            if let (Some(toast), CommandOutcome::Plain(CommandResult::Ok)) =
                (success_toast, &outcome)
            {
                self.session.notifications.info(toast);
            }
        }
    }

    /// Toast a deck-creating command's outcome. The engine logic lives in the
    /// command; this only reports success or failure.
    fn notify_deck_add_outcome(&mut self, outcome: &CommandOutcome) {
        match outcome {
            CommandOutcome::DecksCreated { uuids } => {
                for uuid in uuids {
                    let Ok((ch_idx, deck_idx)) = self.resolve_deck(uuid) else {
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
    matches!(
        cmd,
        EngineCommand::AddDeck { .. }
            | EngineCommand::AddImageDeck { .. }
            | EngineCommand::AddVideoDeck { .. }
            | EngineCommand::AddSolidColorDeck { .. }
            | EngineCommand::AddCameraDeck { .. }
            | EngineCommand::AddDepthSensorDeck { .. }
            | EngineCommand::AddNdiDeck { .. }
            | EngineCommand::AddSyphonDeck { .. }
            | EngineCommand::AddSrtDeck { .. }
            | EngineCommand::AddHlsDeck { .. }
            | EngineCommand::AddDashDeck { .. }
            | EngineCommand::AddRtmpDeck { .. }
            | EngineCommand::AddHtmlDeck { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch() -> String {
        "ch-uuid".to_string()
    }

    #[test]
    fn every_deck_add_variant_is_recognized() {
        // Mirrors the deck-add arm list in execute_command_gui: if a new
        // Add*Deck variant is introduced but omitted from command_is_deck_add,
        // the GUI silently skips its toast + preview-texture registration.
        let deck_adds = [
            EngineCommand::AddDeck {
                channel_uuid: ch(),
                shader_name: "solid".into(),
            },
            EngineCommand::AddImageDeck {
                channel_uuid: ch(),
                path: "/tmp/x.png".into(),
            },
            EngineCommand::AddVideoDeck {
                channel_uuid: ch(),
                path: "/tmp/x.mp4".into(),
            },
            EngineCommand::AddSolidColorDeck {
                channel_uuid: ch(),
                color: [0.0, 0.0, 0.0, 1.0],
            },
            EngineCommand::AddCameraDeck {
                channel_uuid: ch(),
                camera_id: 0,
            },
            EngineCommand::AddDepthSensorDeck {
                channel_uuid: ch(),
                depth_sensor_id: 0,
            },
            EngineCommand::AddNdiDeck {
                channel_uuid: ch(),
                source_name: "src".into(),
            },
            EngineCommand::AddSyphonDeck {
                channel_uuid: ch(),
                server_name: "srv".into(),
            },
            EngineCommand::AddSrtDeck {
                channel_uuid: ch(),
                url: "srt://h:9000".into(),
                mode: crate::stream::SrtMode::Caller,
            },
            EngineCommand::AddHlsDeck {
                channel_uuid: ch(),
                url: "http://h/live.m3u8".into(),
            },
            EngineCommand::AddDashDeck {
                channel_uuid: ch(),
                url: "http://h/live.mpd".into(),
            },
            EngineCommand::AddRtmpDeck {
                channel_uuid: ch(),
                url: "rtmp://h/live".into(),
                mode: crate::stream::RtmpMode::Pull,
            },
            EngineCommand::AddHtmlDeck {
                channel_uuid: ch(),
                url: "http://h".into(),
            },
        ];
        assert_eq!(deck_adds.len(), 13, "expected 13 deck-add variants");
        for cmd in &deck_adds {
            assert!(command_is_deck_add(cmd), "not recognized: {cmd:?}");
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
