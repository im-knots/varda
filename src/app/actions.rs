//! UI action processing — applies UIActions to VardaApp state.
//!
//! These methods were originally in main.rs but belong in the engine layer
//! since they mutate engine-owned state (mixer, surfaces, outputs, etc.).

use super::VardaApp;
use crate::deck::Deck;
use crate::usecases::ui;


impl VardaApp {
    /// Apply UI-driven engine state changes: MIDI learn, notifications.
    /// Selection and layout state is handled by the UI consumer (UIRunner).
    pub fn apply_ui_actions(&mut self, ui_actions: &ui::UIActions) {
        if ui_actions.midi_learn_toggle {
            self.midi_mappings.toggle_learn();
        }
        if let Some(ref path) = ui_actions.midi_learn_select {
            self.midi_mappings.select_learn_target(path.clone());
        }

        let mut dismissals = ui_actions.notifications_to_dismiss.clone();
        dismissals.sort_unstable_by(|a, b| b.cmp(a));
        for idx in dismissals {
            self.notifications.dismiss(idx);
        }
    }

    /// Apply engine mutations: mixer, decks, effects, transitions, channels, cameras.
    /// Routes through engine trait methods where possible, VardaApp methods otherwise.
    /// `egui_renderer` and `deck_preview_textures` are passed in because they are
    /// egui-specific state owned by the window layer.
    ///
    /// Returns the index of a removed channel (if any) so the UI consumer can
    /// fix up selection state.
    pub fn apply_engine_actions(
        &mut self,
        ui_actions: &mut ui::UIActions,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
    ) -> Option<usize> {
        use crate::engine::traits::*;
        use crate::usecases::ui::CrossfaderAction;

        // Crossfader — dispatch through trait methods
        if let Some(action) = &ui_actions.crossfader_action {
            match action {
                CrossfaderAction::SetPosition(pos) => self.set_crossfader(*pos),
                CrossfaderAction::SnapA => self.set_crossfader(0.0),
                CrossfaderAction::SnapB => self.set_crossfader(1.0),
                CrossfaderAction::AutoTransition { target, duration_secs, easing } => {
                    self.start_auto_crossfade(*target, *duration_secs, *easing);
                }
                CrossfaderAction::BeatTransition { target, beats } => {
                    self.start_beat_crossfade(*target, *beats);
                }
            }
        }

        // Channel updates — dispatch through trait methods
        for &(ch_idx, opacity, blend_mode) in &ui_actions.channel_updates {
            self.set_channel_opacity(ch_idx, opacity);
            self.set_channel_blend_mode(ch_idx, blend_mode);
        }

        // Deck updates — dispatch through trait methods
        for &(ch_idx, deck_idx, opacity, blend_mode, solo, mute) in &ui_actions.deck_updates {
            self.set_deck_opacity(ch_idx, deck_idx, opacity);
            self.set_deck_blend_mode(ch_idx, deck_idx, blend_mode);
            self.set_deck_solo(ch_idx, deck_idx, solo);
            self.set_deck_mute(ch_idx, deck_idx, mute);
        }

        // Scaling mode — dispatch through trait methods
        for &(ch_idx, deck_idx, mode) in &ui_actions.scaling_mode_updates {
            self.set_deck_scaling_mode(ch_idx, deck_idx, mode);
        }

        // Complex mutations — VardaApp methods
        self.apply_video_actions(ui_actions);
        self.apply_auto_transition_actions(ui_actions);
        self.apply_param_updates(ui_actions);
        self.apply_modulation_actions(ui_actions);
        self.apply_sequence_actions(ui_actions);

        // Deck/effect add/remove (needs egui texture management)
        self.apply_deck_and_effect_actions(ui_actions, egui_renderer, deck_preview_textures);

        // Transition shader — dispatch through trait method
        if let Some(transition_opt) = &ui_actions.set_transition {
            let name = transition_opt.as_deref();
            if let Err(e) = self.set_transition(name) {
                log::error!("Failed to set transition: {}", e);
            }
        }

        // Channel add/remove + camera
        self.apply_add_channel(ui_actions);
        self.apply_camera_add(ui_actions, egui_renderer, deck_preview_textures);
        let removed_channel = self.apply_remove_channel(ui_actions);

        removed_channel
    }


    fn apply_add_channel(&mut self, ui_actions: &ui::UIActions) {
        if !ui_actions.add_channel { return; }
        match self.mixer.add_channel(&self.context, crate::app::RENDER_WIDTH, crate::app::RENDER_HEIGHT) {
            Ok(idx) => {
                self.notifications.info(format!("Added channel {} (index {})",
                    self.mixer.channels()[idx].name, idx));
            }
            Err(e) => {
                log::error!("Failed to add channel: {}", e);
                self.notifications.error(format!("Error adding channel: {}", e));
            }
        }
    }

    fn apply_camera_add(
        &mut self,
        ui_actions: &mut ui::UIActions,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
    ) {
        let Some((ch_idx, camera_id)) = ui_actions.camera_to_add.take() else { return };

        let cam_name = self.camera_manager.devices().iter()
            .find(|d| d.id == camera_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("Camera {}", camera_id));

        match self.camera_manager.open_camera(camera_id, &self.context.device) {
            Ok((src_w, src_h)) => {
                match Deck::new_from_camera(&self.context, camera_id, &cam_name, src_w, src_h,
                    crate::app::RENDER_WIDTH, crate::app::RENDER_HEIGHT)
                {
                    Ok(deck) => {
                        if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                            let idx = ch.add_deck(deck);
                            log::info!("Added camera deck {} to channel {}: {}", idx, ch_idx, cam_name);
                            let texture_id = egui_renderer.register_native_texture(
                                &self.context.device,
                                &ch.decks[idx].deck.texture_view,
                                wgpu::FilterMode::Linear,
                            );
                            deck_preview_textures.insert((ch_idx, idx), texture_id);
                            self.notifications.info(format!("📹 Camera '{}' added to Ch {}", cam_name, ch_idx + 1));
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to create camera deck: {}", e);
                        self.notifications.error(format!("Failed to create camera deck: {}", e));
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to open camera '{}': {}", cam_name, e);
                self.notifications.error(format!("Failed to open camera '{}': {}", cam_name, e));
            }
        }
    }

    /// Returns the index of the removed channel (if any) so the UI consumer
    /// can fix up selection state.
    fn apply_remove_channel(&mut self, ui_actions: &ui::UIActions) -> Option<usize> {
        let ch_idx = ui_actions.remove_channel?;
        let name = self.mixer.channels().get(ch_idx).map(|c| c.name.clone()).unwrap_or_default();
        if self.mixer.remove_channel(ch_idx) {
            self.notifications.info(format!("Removed channel {}", name));
            Some(ch_idx)
        } else {
            self.notifications.error("Cannot remove channel (minimum 2 required)".to_string());
            None
        }
    }

    /// Apply clock preference changes from UI.
    pub fn apply_clock_actions(&mut self, ui_actions: &ui::UIActions) {
        if let Some(ref pref) = ui_actions.clock_preference {
            self.clock_manager.set_preference(pref.clone());
        }
        if let Some(bpm) = ui_actions.manual_bpm {
            self.clock_manager.set_manual_bpm(bpm);
        }
    }

    /// Apply MIDI/audio/camera device actions from UI.
    pub fn apply_device_actions(&mut self, ui_actions: &ui::UIActions) {
        if ui_actions.camera_rescan {
            self.camera_manager.scan_devices();
        }
        if ui_actions.audio_rescan {
            self.audio_manager.scan_devices();
        }
        for (source_id, enabled) in &ui_actions.audio_source_toggles {
            if *enabled {
                if let Err(e) = self.audio_manager.open_source(*source_id) {
                    log::warn!("Failed to open audio source {}: {}", source_id, e);
                    self.notifications.warn(format!("Failed to open audio source: {}", e));
                }
            } else {
                self.audio_manager.close_source(*source_id);
            }
        }
        if ui_actions.midi_rescan {
            if let Some(mgr) = &mut self.midi_devices {
                mgr.load_user_profiles(&self.workspace.controllers_dir());
                if let Err(e) = mgr.scan_devices() {
                    log::warn!("MIDI rescan failed: {}", e);
                }
                self.controller_led_mgr.sync_devices(mgr);
            }
        }
        for (dev_id, enabled) in &ui_actions.midi_device_toggles {
            if let Some(mgr) = &mut self.midi_devices {
                mgr.set_device_enabled(*dev_id, *enabled);
            }
        }
        if ui_actions.midi_clear_mappings {
            self.midi_mappings.clear_all();
        }
        for key in &ui_actions.midi_remove_mapping {
            self.midi_mappings.remove(key);
        }
    }

    /// Update controller LEDs based on current state.
    pub fn update_controller_leds(&mut self) {
        if let Some(mgr) = &self.midi_devices {
            self.controller_led_mgr.update_leds(
                mgr,
                &self.midi_mappings,
                &self.mixer,
                self.midi_mappings.learn_mode,
                self.midi_mappings.learn_target.as_deref(),
            );
        }
    }
}