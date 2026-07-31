//! UI action processing — applies `UIActions` to `VardaApp` state.
//!
//! These methods were originally in main.rs but belong in the engine layer
//! since they mutate engine-owned state (mixer, surfaces, outputs, etc.).

use super::VardaApp;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand};
use crate::usecases::ui;

impl VardaApp {
    /// Apply UI-driven engine state changes: MIDI learn, notifications.
    /// Selection and layout state is handled by the UI consumer (`UIRunner`).
    pub fn apply_ui_actions(&mut self, ui_actions: &ui::UIActions) {
        // MIDI learn
        if ui_actions.session.midi_learn_toggle {
            self.input.midi_mappings.toggle_learn();
            // Mutually exclusive: exit keyboard learn when entering MIDI learn
            if self.input.midi_mappings.learn_mode {
                self.input.keymap.cancel_learn();
            }
        }
        if let Some(ref path) = ui_actions.session.midi_learn_select {
            self.input.midi_mappings.select_learn_target(path.clone());
        }

        // Keyboard learn
        if ui_actions.session.keyboard_learn_toggle {
            self.input.keymap.toggle_learn();
            // Mutually exclusive: exit MIDI learn when entering keyboard learn
            if self.input.keymap.learn_mode {
                self.input.midi_mappings.cancel_learn();
            }
        }
        if let Some(ref target) = ui_actions.session.keyboard_learn_select {
            self.input.keymap.select_learn_target(target.clone());
        }
        if let Some(ref combo) = ui_actions.session.keyboard_learn_bind {
            self.input.keymap.process_learn(combo.clone());
        }

        let mut dismissals = ui_actions.session.notifications_to_dismiss.clone();
        dismissals.sort_unstable_by(|a, b| b.cmp(a));
        for idx in dismissals {
            self.session.notifications.dismiss(idx);
        }

        for msg in &ui_actions.session.info_notifications {
            self.session.notifications.info(msg);
        }
    }

    /// Apply engine mutations: mixer, decks, effects, transitions, channels, cameras.
    /// Routes through engine trait methods where possible, `VardaApp` methods otherwise.
    ///
    /// Returns an [`EngineActionsOutcome`] carrying the GUI post-steps the runner
    /// must apply after the drain: the removed channel index (selection fixup),
    /// whether the render resolution changed (egui texture re-point), and the
    /// `CommandOutcome`s a preview-texture-registering consumer needs to act on.
    /// This method itself never touches egui — see `/spec/app-presentation-boundary.md`.
    pub fn apply_engine_actions(&mut self, ui_actions: &mut ui::UIActions) -> EngineActionsOutcome {
        // ── Unified command stream (WS2) ──────────────────────────────────
        // Panels push `EngineCommand`s directly; drain them through the same
        // dispatch as the bus. Ordering within the vec is preserved, so a
        // new-channel library drop enqueues `AddChannel` before its `Add*Deck`
        // and the deck resolves against the freshly created channel. Deck-
        // creating / reindexing outcomes are handed back to the caller, which
        // registers preview textures — all via the typed `CommandOutcome`.
        let mut resolution_changed = false;
        let mut texture_outcomes = Vec::new();
        let commands = std::mem::take(&mut ui_actions.commands);
        for cmd in commands {
            let is_deck_add = command_is_deck_add(&cmd);
            // A resolution change recreates every GPU texture; flag it so the
            // runner re-points its egui texture registrations after the drain.
            let is_resolution_change = matches!(
                &cmd,
                EngineCommand::SetRenderResolution { width, height }
                    if *width > 0
                        && *height > 0
                        && (*width != self.render_width || *height != self.render_height)
            );
            let outcome = self.execute_command_gui(cmd);
            if matches!(outcome, CommandOutcome::DecksCreated { .. }) {
                texture_outcomes.push(outcome.clone());
            }
            if is_deck_add {
                self.notify_deck_add_outcome(&outcome);
            }
            if is_resolution_change {
                resolution_changed = true;
            }
        }

        EngineActionsOutcome {
            removed_channel: self.apply_remove_channel(ui_actions),
            resolution_changed,
            texture_outcomes,
        }
    }

    /// Emit the GUI toast for a deck-creating command's outcome — the post-step
    /// that mirrors the old `dispatch_source_deck_add`. The engine logic lives
    /// in the command; this only surfaces success/failure to the notification
    /// center (preview texture registration is the caller's job — see
    /// `EngineActionsOutcome::texture_outcomes`).
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
            _ => {}
        }
    }

    /// Returns the index of the removed channel (if any) so the UI consumer
    /// can fix up selection state.
    fn apply_remove_channel(&mut self, ui_actions: &ui::UIActions) -> Option<usize> {
        let ch_idx = ui_actions.session.remove_channel?;
        let channel_uuid = self.mixer.channels().get(ch_idx)?.uuid().to_string();
        let result = self.execute_command(EngineCommand::RemoveChannel { channel_uuid });
        match result {
            crate::engine::CommandResult::Ok => Some(ch_idx),
            _ => None,
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

/// GUI post-steps the runner applies after [`VardaApp::apply_engine_actions`]:
/// selection fixup for a removed channel, egui texture re-point after a
/// render-resolution change, and the deck-texture-relevant command outcomes
/// to register/free egui preview textures for (both need window-layer state
/// the engine can't touch — see `/spec/app-presentation-boundary.md`).
pub struct EngineActionsOutcome {
    /// Index of a channel removed this frame (for UI selection fixup).
    pub removed_channel: Option<usize>,
    /// Whether the render resolution changed (recreated GPU textures).
    pub resolution_changed: bool,
    /// `DecksCreated` outcomes from this frame's command drain, in order — the
    /// caller registers a preview texture for each new deck UUID.
    pub texture_outcomes: Vec<CommandOutcome>,
}

/// True for the deck-creating commands the GUI drain toasts + registers a
/// preview texture for. Mirrors the deck-add arm list in `execute_command_gui`.
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
