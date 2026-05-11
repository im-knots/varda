//! UI action processing — applies UIActions to VardaApp state.
//!
//! These methods were originally in main.rs but belong in the engine layer
//! since they mutate engine-owned state (mixer, surfaces, outputs, etc.).

use super::VardaApp;
use crate::deck::Deck;
use crate::engine::EngineCommand;
use crate::usecases::ui;


impl VardaApp {
    /// Apply UI-driven engine state changes: MIDI learn, notifications.
    /// Selection and layout state is handled by the UI consumer (UIRunner).
    pub fn apply_ui_actions(&mut self, ui_actions: &ui::UIActions) {
        // MIDI learn
        if ui_actions.midi_learn_toggle {
            self.midi_mappings.toggle_learn();
            // Mutually exclusive: exit keyboard learn when entering MIDI learn
            if self.midi_mappings.learn_mode {
                self.keymap.cancel_learn();
            }
        }
        if let Some(ref path) = ui_actions.midi_learn_select {
            self.midi_mappings.select_learn_target(path.clone());
        }

        // Keyboard learn
        if ui_actions.keyboard_learn_toggle {
            self.keymap.toggle_learn();
            // Mutually exclusive: exit MIDI learn when entering keyboard learn
            if self.keymap.learn_mode {
                self.midi_mappings.cancel_learn();
            }
        }
        if let Some(ref target) = ui_actions.keyboard_learn_select {
            self.keymap.select_learn_target(target.clone());
        }
        if let Some(ref combo) = ui_actions.keyboard_learn_bind {
            self.keymap.process_learn(combo.clone());
        }

        // Keyboard param toggle
        if let Some(ref path) = ui_actions.keyboard_param_toggle {
            crate::keymap::apply_keyboard_toggle_param(&mut self.mixer, path);
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
        use crate::usecases::ui::CrossfaderAction;

        // Crossfader — route through execute_command
        if let Some(action) = &ui_actions.crossfader_action {
            let cmd = match action {
                CrossfaderAction::SetPosition(pos) => EngineCommand::SetCrossfader(*pos),
                CrossfaderAction::SnapA => EngineCommand::SetCrossfader(0.0),
                CrossfaderAction::SnapB => EngineCommand::SetCrossfader(1.0),
                CrossfaderAction::AutoTransition { target, duration_secs, easing } => {
                    EngineCommand::AutoCrossfade { target: *target, duration_secs: *duration_secs, easing: *easing }
                }
                CrossfaderAction::BeatTransition { target, beats } => {
                    EngineCommand::BeatCrossfade { target: *target, beats: *beats }
                }
            };
            self.execute_command(cmd);
        }

        // Channel updates — route through execute_command
        for &(ch_idx, opacity, blend_mode) in &ui_actions.channel_updates {
            self.execute_command(EngineCommand::SetChannelOpacity { channel_idx: ch_idx, opacity });
            self.execute_command(EngineCommand::SetChannelBlendMode { channel_idx: ch_idx, mode: blend_mode });
        }

        // Deck updates — route through execute_command
        for &(ch_idx, deck_idx, opacity, blend_mode, solo, mute) in &ui_actions.deck_updates {
            self.execute_command(EngineCommand::SetDeckOpacity { channel_idx: ch_idx, deck_idx, opacity });
            self.execute_command(EngineCommand::SetDeckBlendMode { channel_idx: ch_idx, deck_idx, mode: blend_mode });
            self.execute_command(EngineCommand::SetDeckSolo { channel_idx: ch_idx, deck_idx, solo });
            self.execute_command(EngineCommand::SetDeckMute { channel_idx: ch_idx, deck_idx, mute });
        }

        // Scaling mode — route through execute_command
        for &(ch_idx, deck_idx, mode) in &ui_actions.scaling_mode_updates {
            self.execute_command(EngineCommand::SetDeckScalingMode { channel_idx: ch_idx, deck_idx, mode });
        }

        // Complex mutations — VardaApp methods
        self.apply_video_actions(ui_actions);
        self.apply_auto_transition_actions(ui_actions);
        self.apply_param_updates(ui_actions);
        self.apply_modulation_actions(ui_actions);
        self.apply_sequence_actions(ui_actions);

        // Channel add first — new channel must exist before deck/effect adds target it
        self.apply_add_channel(ui_actions);

        // Deck/effect add/remove (needs egui texture management)
        self.apply_deck_and_effect_actions(ui_actions, egui_renderer, deck_preview_textures);

        // Transition shader — route through execute_command
        if let Some(transition_opt) = &ui_actions.set_transition {
            self.execute_command(EngineCommand::SetTransition { shader_name: transition_opt.clone() });
        }

        // Camera add + channel remove
        self.apply_camera_add(ui_actions, egui_renderer, deck_preview_textures);
        let removed_channel = self.apply_remove_channel(ui_actions);

        removed_channel
    }


    fn apply_add_channel(&mut self, ui_actions: &ui::UIActions) {
        if !ui_actions.add_channel { return; }
        self.execute_command(EngineCommand::AddChannel);
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
                    self.render_width, self.render_height)
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
        let result = self.execute_command(EngineCommand::RemoveChannel { channel_idx: ch_idx });
        match result {
            crate::engine::CommandResult::Ok => Some(ch_idx),
            _ => None,
        }
    }

    /// Apply resolution change from UI. Returns true if resolution was changed
    /// (caller must re-register egui textures).
    pub fn apply_resolution_change(&mut self, ui_actions: &ui::UIActions) -> bool {
        if let Some((w, h)) = ui_actions.resolution_change {
            if w > 0 && h > 0 && (w != self.render_width || h != self.render_height) {
                self.execute_command(EngineCommand::SetRenderResolution { width: w, height: h });
                return true;
            }
        }
        false
    }

    /// Apply clock preference changes from UI.
    pub fn apply_clock_actions(&mut self, ui_actions: &ui::UIActions) {
        if let Some(ref pref) = ui_actions.clock_preference {
            self.execute_command(EngineCommand::SetClockPreference { preference: pref.clone() });
        }
        if let Some(bpm) = ui_actions.manual_bpm {
            self.execute_command(EngineCommand::SetManualBpm { bpm });
        }
    }

    /// Apply MIDI/audio/camera/NDI/Syphon device actions from UI.
    pub fn apply_device_actions(&mut self, ui_actions: &ui::UIActions) {
        if ui_actions.camera_rescan {
            self.execute_command(EngineCommand::RescanCameras);
        }
        if ui_actions.audio_rescan {
            self.execute_command(EngineCommand::RescanAudio);
        }
        for (source_id, enabled) in &ui_actions.audio_source_toggles {
            self.execute_command(EngineCommand::ToggleAudioSource { source_id: *source_id, enabled: *enabled });
        }
        if ui_actions.midi_rescan {
            self.execute_command(EngineCommand::RescanMidi);
        }
        for (dev_id, enabled) in &ui_actions.midi_device_toggles {
            self.execute_command(EngineCommand::SetMidiDeviceEnabled { device_id: *dev_id, enabled: *enabled });
        }
        if ui_actions.midi_clear_mappings {
            self.execute_command(EngineCommand::ClearMidiMappings);
        }
        for key in &ui_actions.midi_remove_mapping {
            self.execute_command(EngineCommand::RemoveMidiMapping { key: key.clone() });
        }
        if ui_actions.ndi_rescan {
            self.execute_command(EngineCommand::RescanNdi);
        }
        #[cfg(target_os = "macos")]
        if ui_actions.syphon_rescan {
            self.execute_command(EngineCommand::RescanSyphon);
        }
    }

    // Recording/SRT start/stop is now handled per-output in apply_output_actions()

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
            self.auto_map_engine.update_leds(mgr, &self.mixer);
        }
    }
}