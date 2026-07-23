//! UI action processing — applies UIActions to VardaApp state.
//!
//! These methods were originally in main.rs but belong in the engine layer
//! since they mutate engine-owned state (mixer, surfaces, outputs, etc.).

use super::VardaApp;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand};
use crate::usecases::ui;

impl VardaApp {
    /// Apply UI-driven engine state changes: MIDI learn, notifications.
    /// Selection and layout state is handled by the UI consumer (UIRunner).
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
    /// Routes through engine trait methods where possible, VardaApp methods otherwise.
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
            if matches!(
                outcome,
                CommandOutcome::DeckCreated { .. } | CommandOutcome::DecksReindexed { .. }
            ) {
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
            CommandOutcome::DeckCreated {
                channel_idx,
                deck_idx,
                ..
            } => {
                let name = self
                    .mixer
                    .channels()
                    .get(*channel_idx)
                    .and_then(|ch| ch.decks.get(*deck_idx))
                    .map(|slot| slot.deck.source_name().to_string())
                    .unwrap_or_default();
                self.session
                    .notifications
                    .info(format!("➕ {} → Ch {}", name, channel_idx + 1));
            }
            CommandOutcome::Plain(CommandResult::Err { message, .. }) => {
                log::error!("Failed to add deck: {}", message);
                self.session
                    .notifications
                    .error(format!("Failed to add deck: {}", message));
            }
            _ => {}
        }
    }

    /// Returns the index of the removed channel (if any) so the UI consumer
    /// can fix up selection state.
    fn apply_remove_channel(&mut self, ui_actions: &ui::UIActions) -> Option<usize> {
        let ch_idx = ui_actions.session.remove_channel?;
        let result = self.execute_command(EngineCommand::RemoveChannel {
            channel_idx: ch_idx,
        });
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
    /// `DeckCreated` / `DecksReindexed` outcomes from this frame's command
    /// drain, in order — the caller registers/frees preview textures for each.
    pub texture_outcomes: Vec<CommandOutcome>,
}

/// True for the deck-creating commands the GUI drain toasts + registers a
/// preview texture for. Mirrors the deck-add arm list in `execute_command_gui`.
fn command_is_deck_add(cmd: &EngineCommand) -> bool {
    matches!(
        cmd,
        EngineCommand::AddDeck { .. }
            | EngineCommand::AddImageDeck { .. }
            | EngineCommand::AddVideoDeck { .. }
            | EngineCommand::AddSolidColorDeck { .. }
            | EngineCommand::AddCameraDeck { .. }
            | EngineCommand::AddNdiDeck { .. }
            | EngineCommand::AddSyphonDeck { .. }
            | EngineCommand::AddSrtDeck { .. }
            | EngineCommand::AddHlsDeck { .. }
            | EngineCommand::AddDashDeck { .. }
            | EngineCommand::AddRtmpDeck { .. }
            | EngineCommand::AddHtmlDeck { .. }
    )
}
