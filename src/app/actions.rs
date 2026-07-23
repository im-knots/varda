//! UI action processing — applies UIActions to VardaApp state.
//!
//! These methods were originally in main.rs but belong in the engine layer
//! since they mutate engine-owned state (mixer, surfaces, outputs, etc.).

use super::VardaApp;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand};
use crate::usecases::ui;
use std::collections::HashMap;

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
    /// `egui_renderer` and `deck_preview_textures` are passed in because they are
    /// egui-specific state owned by the window layer.
    ///
    /// Returns an [`EngineActionsOutcome`] carrying the GUI post-steps the runner
    /// must apply after the drain: the removed channel index (selection fixup)
    /// and whether the render resolution changed (egui texture re-point).
    pub fn apply_engine_actions(
        &mut self,
        ui_actions: &mut ui::UIActions,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
    ) -> EngineActionsOutcome {
        // ── Unified command stream (WS2) ──────────────────────────────────
        // Panels push `EngineCommand`s directly; drain them through the same
        // dispatch as the bus. Ordering within the vec is preserved, so a
        // new-channel library drop enqueues `AddChannel` before its `Add*Deck`
        // and the deck resolves against the freshly created channel. Deck-
        // creating commands register their preview texture + emit a toast;
        // structural changes (remove/move/reorder, preset load) reindex the
        // channel's textures — all via the typed `CommandOutcome`.
        let mut resolution_changed = false;
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
            self.apply_deck_texture_outcome(&outcome, egui_renderer, deck_preview_textures);
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
        }
    }

    /// Emit the GUI toast for a deck-creating command's outcome — the egui-side
    /// post-step that mirrors the old `dispatch_source_deck_add`. The engine
    /// logic lives in the command; this only surfaces success/failure to the
    /// notification center (texture registration is done separately by
    /// `apply_deck_texture_outcome`).
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

    /// Apply the egui texture post-step for a GUI command outcome: register the
    /// created deck's preview, or refresh the index-keyed map for channels whose
    /// deck indices shifted (remove/move/reorder).
    pub(crate) fn apply_deck_texture_outcome(
        &self,
        outcome: &CommandOutcome,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut HashMap<(usize, usize), egui::TextureId>,
    ) {
        match outcome {
            CommandOutcome::DeckCreated {
                channel_idx,
                deck_idx,
                ..
            } => {
                self.register_deck_preview_texture(
                    *channel_idx,
                    *deck_idx,
                    egui_renderer,
                    deck_preview_textures,
                );
            }
            CommandOutcome::DecksReindexed { channels } => {
                for &ch in channels {
                    self.reregister_channel_preview_textures(
                        ch,
                        egui_renderer,
                        deck_preview_textures,
                    );
                }
            }
            _ => {}
        }
    }

    /// Register a single deck's preview texture at `(channel_idx, deck_idx)`.
    pub(crate) fn register_deck_preview_texture(
        &self,
        channel_idx: usize,
        deck_idx: usize,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut HashMap<(usize, usize), egui::TextureId>,
    ) {
        if let Some(slot) = self
            .mixer
            .channels()
            .get(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
        {
            let texture_id = egui_renderer.register_native_texture(
                &self.context.device,
                &slot.deck.texture_view,
                wgpu::FilterMode::Linear,
            );
            deck_preview_textures.insert((channel_idx, deck_idx), texture_id);
        }
    }

    /// Free and re-register every preview texture for `channel_idx`, keeping the
    /// index-keyed map consistent after deck indices shift.
    pub(crate) fn reregister_channel_preview_textures(
        &self,
        channel_idx: usize,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut HashMap<(usize, usize), egui::TextureId>,
    ) {
        let stale: Vec<(usize, usize)> = deck_preview_textures
            .keys()
            .filter(|(c, _)| *c == channel_idx)
            .copied()
            .collect();
        for key in stale {
            if let Some(tex_id) = deck_preview_textures.remove(&key) {
                egui_renderer.free_texture(&tex_id);
            }
        }
        if let Some(ch) = self.mixer.channels().get(channel_idx) {
            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                let texture_id = egui_renderer.register_native_texture(
                    &self.context.device,
                    &slot.deck.texture_view,
                    wgpu::FilterMode::Linear,
                );
                deck_preview_textures.insert((channel_idx, deck_idx), texture_id);
            }
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
/// selection fixup for a removed channel and egui texture re-point after a
/// render-resolution change (both need window-layer state the engine can't touch).
pub struct EngineActionsOutcome {
    /// Index of a channel removed this frame (for UI selection fixup).
    pub removed_channel: Option<usize>,
    /// Whether the render resolution changed (recreated GPU textures).
    pub resolution_changed: bool,
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
