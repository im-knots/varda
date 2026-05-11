//! Deck add/remove/move, effect CRUD, and transition sequence mutations.

use crate::deck::{Deck, Effect};
use crate::usecases::ui::UIActions;
use super::super::VardaApp;

impl VardaApp {
    /// Apply deck add/remove and effect add/remove/toggle actions.
    /// `egui_renderer` and `deck_preview_textures` are passed in because they
    /// are egui-specific state owned by the window layer.
    pub(crate) fn apply_deck_and_effect_actions(
        &mut self,
        actions: &mut UIActions,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
    ) {
        let context = &self.context;
        let mixer = &mut self.mixer;

        // Remove deck if requested
        if let Some((ch_idx, deck_idx)) = actions.deck_to_remove {
            // Release external resources before removal
            if let Some(ch) = mixer.channels().get(ch_idx) {
                if let Some(slot) = ch.decks.get(deck_idx) {
                    if let Some(cam_id) = slot.deck.camera_id() {
                        self.camera_manager.release_camera(cam_id);
                    }
                    if let Some(idx) = slot.deck.srt_receiver_idx() {
                        self.srt_manager.stop_receive(idx);
                    }
                }
            }
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if deck_idx < ch.decks.len() {
                    ch.remove_deck(deck_idx);
                    log::info!("Removed deck {} from channel {}", deck_idx, ch_idx);

                    let keys_to_remove: Vec<_> = deck_preview_textures.keys()
                        .filter(|(c, _)| *c == ch_idx)
                        .copied()
                        .collect();
                    for key in keys_to_remove {
                        if let Some(tex_id) = deck_preview_textures.remove(&key) {
                            egui_renderer.free_texture(&tex_id);
                        }
                    }
                    for (new_idx, deck_slot) in ch.decks.iter().enumerate() {
                        let texture_id = egui_renderer.register_native_texture(
                            &context.device,
                            &deck_slot.deck.texture_view,
                            wgpu::FilterMode::Linear,
                        );
                        deck_preview_textures.insert((ch_idx, new_idx), texture_id);
                    }
                }
            }
        }

        // Move deck between channels if requested
        if let Some((src_ch, src_deck, dst_ch)) = actions.deck_to_move {
            if src_ch != dst_ch && src_ch < mixer.channel_count() && dst_ch < mixer.channel_count() {
                if src_deck < mixer.channels_mut()[src_ch].decks.len() {
                    let Some(slot) = mixer.channels_mut()[src_ch].remove_deck_slot(src_deck) else {
                        log::warn!("deck_to_move: deck {} not found in channel {}", src_deck, src_ch);
                        return;
                    };
                    let new_idx = mixer.channels_mut()[dst_ch].add_deck_slot(slot);

                    log::info!("Moved deck {} from channel {} to channel {} (new index {})", src_deck, src_ch, dst_ch, new_idx);

                    let src_keys: Vec<_> = deck_preview_textures.keys()
                        .filter(|(c, _)| *c == src_ch)
                        .copied()
                        .collect();
                    for key in src_keys {
                        if let Some(tex_id) = deck_preview_textures.remove(&key) {
                            egui_renderer.free_texture(&tex_id);
                        }
                    }
                    for (i, deck_slot) in mixer.channels_mut()[src_ch].decks.iter().enumerate() {
                        let tex_id = egui_renderer.register_native_texture(
                            &context.device,
                            &deck_slot.deck.texture_view,
                            wgpu::FilterMode::Linear,
                        );
                        deck_preview_textures.insert((src_ch, i), tex_id);
                    }

                    let dst_keys: Vec<_> = deck_preview_textures.keys()
                        .filter(|(c, _)| *c == dst_ch)
                        .copied()
                        .collect();
                    for key in dst_keys {
                        if let Some(tex_id) = deck_preview_textures.remove(&key) {
                            egui_renderer.free_texture(&tex_id);
                        }
                    }
                    for (i, deck_slot) in mixer.channels_mut()[dst_ch].decks.iter().enumerate() {
                        let tex_id = egui_renderer.register_native_texture(
                            &context.device,
                            &deck_slot.deck.texture_view,
                            wgpu::FilterMode::Linear,
                        );
                        deck_preview_textures.insert((dst_ch, i), tex_id);
                    }
                }
            }
        }

        // Shader deck loading is handled asynchronously via spawn_deck_loads in runner.rs.
        // The shader_to_add action is intercepted before apply_engine_actions.

        // Add image decks (synchronous fallback — file dialog path uses background threads)
        for (ch_idx, path) in std::mem::take(&mut actions.images_to_add) {
            match Deck::new_from_image(context, &path, self.render_width, self.render_height) {
                Ok(deck) => {
                    if let Some(ch) = mixer.channel_mut(ch_idx) {
                        let name = deck.source_name().to_string();
                        let idx = ch.add_deck(deck);
                        log::info!("Added image deck {} to channel {}: {}", idx, ch_idx, name);
                        let texture_id = egui_renderer.register_native_texture(
                            &context.device,
                            &ch.decks[idx].deck.texture_view,
                            wgpu::FilterMode::Linear,
                        );
                        deck_preview_textures.insert((ch_idx, idx), texture_id);
                    }
                }
                Err(e) => log::error!("Failed to create image deck from {:?}: {}", path, e),
            }
        }

        // Add video decks (synchronous fallback — file dialog path uses background threads)
        for (ch_idx, path) in std::mem::take(&mut actions.videos_to_add) {
            match Deck::new_from_video(context, &path, self.render_width, self.render_height) {
                Ok(deck) => {
                    if let Some(ch) = mixer.channel_mut(ch_idx) {
                        let name = deck.source_name().to_string();
                        let idx = ch.add_deck(deck);
                        log::info!("Added video deck {} to channel {}: {}", idx, ch_idx, name);
                        let texture_id = egui_renderer.register_native_texture(
                            &context.device,
                            &ch.decks[idx].deck.texture_view,
                            wgpu::FilterMode::Linear,
                        );
                        deck_preview_textures.insert((ch_idx, idx), texture_id);
                    }
                }
                Err(e) => log::error!("Failed to create video deck from {:?}: {}", path, e),
            }
        }

        // Add solid color deck if requested
        if let Some((ch_idx, color)) = actions.solid_color_to_add.take() {
            match Deck::new_solid_color(context, color, self.render_width, self.render_height) {
                Ok(deck) => {
                    if let Some(ch) = mixer.channel_mut(ch_idx) {
                        let name = deck.source_name().to_string();
                        let idx = ch.add_deck(deck);
                        log::info!("Added solid color deck {} to channel {}: {}", idx, ch_idx, name);
                        let texture_id = egui_renderer.register_native_texture(
                            &context.device,
                            &ch.decks[idx].deck.texture_view,
                            wgpu::FilterMode::Linear,
                        );
                        deck_preview_textures.insert((ch_idx, idx), texture_id);
                    }
                }
                Err(e) => log::error!("Failed to create solid color deck: {}", e),
            }
        }

        // Add NDI source deck if requested
        if let Some((ch_idx, ndi_name)) = actions.ndi_to_add.take() {
            match self.ndi_manager.start_receive(&ndi_name, &context.device) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = self.ndi_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                    match Deck::new_from_ndi(context, receiver_idx, &ndi_name, src_w, src_h, self.render_width, self.render_height) {
                        Ok(deck) => {
                            if let Some(ch) = mixer.channel_mut(ch_idx) {
                                let idx = ch.add_deck(deck);
                                log::info!("Added NDI deck {} to channel {}: {}", idx, ch_idx, ndi_name);
                                let texture_id = egui_renderer.register_native_texture(
                                    &context.device,
                                    &ch.decks[idx].deck.texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                deck_preview_textures.insert((ch_idx, idx), texture_id);
                                self.notifications.info(format!("📡 NDI '{}' added to Ch {}", ndi_name, ch_idx + 1));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to create NDI deck: {}", e);
                            self.notifications.error(format!("Failed to create NDI deck: {}", e));
                        }
                    }
                }
                None => {
                    log::error!("Failed to start NDI receive for '{}'", ndi_name);
                    self.notifications.error(format!("Failed to receive NDI source '{}'", ndi_name));
                }
            }
        }

        // Add SRT source deck if requested
        if let Some((ch_idx, url, mode)) = actions.srt_to_add.take() {
            match self.srt_manager.start_receive(&url, mode, &context.device) {
                Some(receiver_idx) => {
                    let (src_w, src_h) = self.srt_manager.receiver_dimensions(receiver_idx).unwrap_or((1920, 1080));
                    match Deck::new_from_srt(context, receiver_idx, &url, src_w, src_h, self.render_width, self.render_height) {
                        Ok(deck) => {
                            if let Some(ch) = mixer.channel_mut(ch_idx) {
                                let idx = ch.add_deck(deck);
                                log::info!("Added SRT deck {} to channel {}: {}", idx, ch_idx, url);
                                let texture_id = egui_renderer.register_native_texture(
                                    &context.device,
                                    &ch.decks[idx].deck.texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                deck_preview_textures.insert((ch_idx, idx), texture_id);
                                self.notifications.info(format!("📺 SRT '{}' added to Ch {}", url, ch_idx + 1));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to create SRT deck: {}", e);
                            self.notifications.error(format!("Failed to create SRT deck: {}", e));
                        }
                    }
                }
                None => {
                    log::error!("Failed to start SRT receive for '{}'", url);
                    self.notifications.error(format!("Failed to receive SRT source '{}'", url));
                }
            }
        }

        // Add SRT source to library (no deck created — user drags to channel)
        if let Some((url, mode)) = actions.srt_library_add.take() {
            if !self.srt_library.iter().any(|(u, _)| u == &url) {
                log::info!("Added SRT source to library: {} ({})", url, mode);
                self.srt_library.push((url, mode));
            }
        }

        // Remove SRT source from library
        if let Some(url) = actions.srt_library_remove.take() {
            self.srt_library.retain(|(u, _)| u != &url);
            log::info!("Removed SRT source from library: {}", url);
        }

        // Add Syphon server deck if requested
        #[cfg(target_os = "macos")]
        if let Some((ch_idx, syph_name)) = actions.syphon_to_add.take() {
            match self.syphon_manager.start_receive(&syph_name, &context.device) {
                Some(client_idx) => {
                    let (src_w, src_h) = self.syphon_manager.client_dimensions(client_idx).unwrap_or((1920, 1080));
                    match Deck::new_from_syphon(context, client_idx, &syph_name, src_w, src_h, self.render_width, self.render_height) {
                        Ok(deck) => {
                            if let Some(ch) = mixer.channel_mut(ch_idx) {
                                let idx = ch.add_deck(deck);
                                log::info!("Added Syphon deck {} to channel {}: {}", idx, ch_idx, syph_name);
                                let texture_id = egui_renderer.register_native_texture(
                                    &context.device,
                                    &ch.decks[idx].deck.texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                deck_preview_textures.insert((ch_idx, idx), texture_id);
                                self.notifications.info(format!("🔗 Syphon '{}' added to Ch {}", syph_name, ch_idx + 1));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to create Syphon deck: {}", e);
                            self.notifications.error(format!("Failed to create Syphon deck: {}", e));
                        }
                    }
                }
                None => {
                    log::error!("Failed to start Syphon receive for '{}'", syph_name);
                    self.notifications.error(format!("Failed to receive Syphon server '{}'", syph_name));
                }
            }
        }

        // Load deck preset into a channel
        if let Some((ch_idx, preset_idx)) = actions.deck_preset_to_add.take() {
            if preset_idx < self.preset_library.deck_presets.len() {
                let preset = self.preset_library.deck_presets[preset_idx].clone();
                match Self::restore_deck_into_channel(
                    &preset.config, ch_idx, context, &self.registry,
                    &mut self.camera_manager, &mut self.ndi_manager, &mut self.srt_manager,
                    self.render_width, self.render_height,
                    mixer, egui_renderer, deck_preview_textures,
                ) {
                    Ok(()) => self.notifications.info(format!("💾 Loaded deck preset '{}'", preset.name)),
                    Err(e) => {
                        log::warn!("Failed to load deck preset '{}': {}", preset.name, e);
                        self.notifications.warn(format!("Failed to load preset '{}': {}", preset.name, e));
                    }
                }
            }
        }

        // Load channel preset: fill into existing channel or create a new one
        if let Some((target_ch, preset_idx)) = actions.channel_preset_to_add.take() {
            if preset_idx < self.preset_library.channel_presets.len() {
                let preset = self.preset_library.channel_presets[preset_idx].clone();

                // Only fill into the target channel if it's empty (no decks);
                // otherwise create a new channel to avoid clobbering existing content.
                let use_existing = target_ch.and_then(|idx| {
                    mixer.channel_mut(idx).filter(|ch| ch.decks.is_empty()).map(|_| idx)
                });

                let resolved = if let Some(ch_idx) = use_existing {
                    // Fill into existing empty channel
                    if let Some(channel) = mixer.channel_mut(ch_idx) {
                        channel.opacity = preset.config.opacity;
                        channel.blend_mode = preset.config.blend_mode.into();
                    }
                    Some((ch_idx, false))
                } else {
                    // Create a new channel
                    let ch_name = mixer.take_next_channel_name();
                    match crate::channel::Channel::new(
                        ch_name, context,
                        self.render_width, self.render_height,
                    ) {
                        Ok(mut channel) => {
                            channel.opacity = preset.config.opacity;
                            channel.blend_mode = preset.config.blend_mode.into();
                            let idx = mixer.channels().len();
                            mixer.channels_mut().push(channel);
                            Some((idx, true))
                        }
                        Err(e) => {
                            log::error!("Failed to create channel for preset: {}", e);
                            self.notifications.error(format!("Failed to load channel preset: {}", e));
                            None
                        }
                    }
                };

                if let Some((ch_idx, created_new)) = resolved {

                let mut had_errors = false;

                // Restore channel effects (only for new channels to avoid duplicating effects)
                if created_new {
                    for eff_config in &preset.config.effects {
                        match crate::persistence::restore_effect(eff_config, context, context.texture_format) {
                            Ok(eff) => {
                                if let Some(ch) = mixer.channel_mut(ch_idx) {
                                    ch.add_effect(eff);
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to restore channel effect '{}': {}", eff_config.path, e);
                                had_errors = true;
                            }
                        }
                    }
                }

                // Bulk-load decks into the channel
                for deck_config in &preset.config.decks {
                    if let Err(e) = Self::restore_deck_into_channel(
                        deck_config, ch_idx, context, &self.registry,
                        &mut self.camera_manager, &mut self.ndi_manager, &mut self.srt_manager,
                        self.render_width, self.render_height,
                        mixer, egui_renderer, deck_preview_textures,
                    ) {
                        log::warn!("Failed to restore deck '{}' in channel preset: {}", deck_config.name, e);
                        had_errors = true;
                    }
                }

                let target_desc = if created_new { "new channel".to_string() } else { format!("ch{}", ch_idx) };
                let msg = if had_errors {
                    format!("💾 Loaded channel preset '{}' into {} (with warnings)", preset.name, target_desc)
                } else {
                    format!("💾 Loaded channel preset '{}' into {}", preset.name, target_desc)
                };
                self.notifications.info(msg);
                }
            }
        }

        // Save deck preset
        if let Some((ch_idx, deck_idx, name)) = actions.save_deck_preset.take() {
            let scene = crate::persistence::snapshot_scene(mixer, self.render_width, self.render_height);
            if let Some(ch_config) = scene.channels.get(ch_idx) {
                if let Some(deck_config) = ch_config.decks.get(deck_idx) {
                    let mut preset_config = deck_config.clone();
                    preset_config.name = name.clone();
                    let deck_uuid = mixer.channel(ch_idx)
                        .and_then(|ch| ch.decks.get(deck_idx))
                        .map(|slot| slot.deck.uuid().to_string())
                        .unwrap_or_default();
                    let effect_uuids: Vec<String> = mixer.channel(ch_idx)
                        .and_then(|ch| ch.decks.get(deck_idx))
                        .map(|slot| slot.deck.effects.iter().map(|e| e.uuid.clone()).collect())
                        .unwrap_or_default();
                    let prefix = format!("deck_{}", deck_uuid);
                    preset_config.modulation = extract_modulation_recipes(mixer.modulation(), &prefix, &effect_uuids);
                    match crate::persistence::presets::PresetLibrary::save_deck_preset(
                        &self.workspace, &name, &preset_config,
                    ) {
                        Ok(()) => {
                            // Update the deck's display name to match the saved preset name
                            if let Some(ch) = mixer.channel_mut(ch_idx) {
                                if let Some(slot) = ch.decks.get_mut(deck_idx) {
                                    slot.deck.set_source_name(name.clone());
                                }
                            }
                            self.preset_library.refresh(&self.workspace);
                            self.notifications.info(format!("💾 Saved deck preset '{}'", name));
                        }
                        Err(e) => {
                            log::error!("Failed to save deck preset: {}", e);
                            self.notifications.error(format!("Failed to save preset: {}", e));
                        }
                    }
                }
            }
        }

        // Save channel preset
        if let Some((ch_idx, name)) = actions.save_channel_preset.take() {
            let scene = crate::persistence::snapshot_scene(mixer, self.render_width, self.render_height);
            if let Some(ch_config) = scene.channels.get(ch_idx) {
                let mut preset_ch_config = ch_config.clone();
                for (deck_idx, deck_config) in preset_ch_config.decks.iter_mut().enumerate() {
                    let deck_uuid = mixer.channel(ch_idx)
                        .and_then(|ch| ch.decks.get(deck_idx))
                        .map(|slot| slot.deck.uuid().to_string())
                        .unwrap_or_default();
                    let effect_uuids: Vec<String> = mixer.channel(ch_idx)
                        .and_then(|ch| ch.decks.get(deck_idx))
                        .map(|slot| slot.deck.effects.iter().map(|e| e.uuid.clone()).collect())
                        .unwrap_or_default();
                    let prefix = format!("deck_{}", deck_uuid);
                    deck_config.modulation = extract_modulation_recipes(mixer.modulation(), &prefix, &effect_uuids);
                }
                match crate::persistence::presets::PresetLibrary::save_channel_preset(
                    &self.workspace, &name, &preset_ch_config,
                ) {
                    Ok(()) => {
                        self.preset_library.refresh(&self.workspace);
                        self.notifications.info(format!("💾 Saved channel preset '{}'", name));
                    }
                    Err(e) => {
                        log::error!("Failed to save channel preset: {}", e);
                        self.notifications.error(format!("Failed to save preset: {}", e));
                    }
                }
            }
        }

        // Add effect to deck
        if let Some((ch_idx, deck_idx, filter_idx)) = actions.effect_to_add {
            let filters = self.registry.filters();
            if filter_idx < filters.len() {
                let filter_shader = filters[filter_idx].clone();
                if let Some(ch) = mixer.channel_mut(ch_idx) {
                    if deck_idx < ch.decks.len() {
                        match Effect::new(context, filter_shader.clone()) {
                            Ok(effect) => {
                                ch.decks[deck_idx].deck.add_effect(effect);
                                log::info!("Added effect {} to ch{} deck {}", filter_shader.name(), ch_idx, deck_idx);
                            }
                            Err(e) => log::error!("Failed to create effect: {}", e),
                        }
                    }
                }
            }
        }

        // Remove effect from deck
        if let Some((ch_idx, deck_idx, effect_idx)) = actions.effect_to_remove {
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if deck_idx < ch.decks.len() {
                    if let Some(_removed) = ch.decks[deck_idx].deck.remove_effect(effect_idx) {
                        log::info!("Removed effect {} from ch{} deck {}", effect_idx, ch_idx, deck_idx);
                    }
                }
            }
        }

        // Toggle effect enabled state
        if let Some((ch_idx, deck_idx, effect_idx)) = actions.effect_to_toggle {
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if deck_idx < ch.decks.len() {
                    let deck = &mut ch.decks[deck_idx].deck;
                    if effect_idx < deck.effects.len() {
                        deck.effects[effect_idx].enabled = !deck.effects[effect_idx].enabled;
                        log::info!("Toggled effect {} on ch{} deck {}", effect_idx, ch_idx, deck_idx);
                    }
                }
            }
        }

        // Add effect to channel
        if let Some((ch_idx, filter_idx)) = actions.ch_effect_to_add {
            let filters = self.registry.filters();
            if filter_idx < filters.len() {
                let filter_shader = filters[filter_idx].clone();
                if let Some(ch) = mixer.channel_mut(ch_idx) {
                    match Effect::new_with_format(context, filter_shader.clone(), context.texture_format) {
                        Ok(effect) => {
                            ch.add_effect(effect);
                            log::info!("Added channel effect {} to ch{}", filter_shader.name(), ch_idx);
                        }
                        Err(e) => log::error!("Failed to create channel effect '{}': {}", filter_shader.name(), e),
                    }
                }
            }
        }

        // Remove effect from channel
        if let Some((ch_idx, effect_idx)) = actions.ch_effect_to_remove {
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if ch.remove_effect(effect_idx) {
                    log::info!("Removed channel effect {} from ch{}", effect_idx, ch_idx);
                }
            }
        }

        // Toggle channel effect
        if let Some((ch_idx, effect_idx)) = actions.ch_effect_to_toggle {
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if effect_idx < ch.effects.len() {
                    ch.effects[effect_idx].enabled = !ch.effects[effect_idx].enabled;
                    log::info!("Toggled channel effect {} on ch{}", effect_idx, ch_idx);
                }
            }
        }

        // Add master effect
        if let Some(filter_idx) = actions.master_effect_to_add {
            let filters = self.registry.filters();
            if filter_idx < filters.len() {
                let filter_shader = filters[filter_idx].clone();
                let filter_name = filter_shader.name();
                match Effect::new_with_format(context, filter_shader, context.texture_format) {
                    Ok(effect) => {
                        mixer.add_master_effect(effect);
                        log::info!("Added master effect: {}", filter_name);
                    }
                    Err(e) => log::error!("Failed to create master effect '{}': {}", filter_name, e),
                }
            }
        }

        // Toggle master effect
        if let Some(effect_idx) = actions.master_effect_to_toggle {
            if effect_idx < mixer.master_effects().len() {
                mixer.master_effects_mut()[effect_idx].enabled = !mixer.master_effects_mut()[effect_idx].enabled;
            }
        }

        // Remove master effect
        if let Some(effect_idx) = actions.master_effect_to_remove {
            if mixer.remove_master_effect(effect_idx) {
                log::info!("Removed master effect {}", effect_idx);
            }
        }

        // Move (reorder) effect within a deck's chain
        if let Some((ch_idx, deck_idx, from_idx, to_idx)) = actions.effect_to_move {
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if deck_idx < ch.decks.len() {
                    let effects = &mut ch.decks[deck_idx].deck.effects;
                    if from_idx < effects.len() && to_idx < effects.len() && from_idx != to_idx {
                        let effect = effects.remove(from_idx);
                        effects.insert(to_idx, effect);
                        log::info!("Moved deck effect {} -> {} on ch{} deck{}", from_idx, to_idx, ch_idx, deck_idx);
                    }
                }
            }
        }

        // Move (reorder) channel effect
        if let Some((ch_idx, from_idx, to_idx)) = actions.ch_effect_to_move {
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if from_idx < ch.effects.len() && to_idx < ch.effects.len() && from_idx != to_idx {
                    let effect = ch.effects.remove(from_idx);
                    ch.effects.insert(to_idx, effect);
                    log::info!("Moved channel effect {} -> {} on ch{}", from_idx, to_idx, ch_idx);
                }
            }
        }

        // Move (reorder) master effect
        if let Some((from_idx, to_idx)) = actions.master_effect_to_move {
            if from_idx < mixer.master_effects().len() && to_idx < mixer.master_effects().len() && from_idx != to_idx {
                let effect = mixer.master_effects_mut().remove(from_idx);
                mixer.master_effects_mut().insert(to_idx, effect);
                log::info!("Moved master effect {} -> {}", from_idx, to_idx);
            }
        }
    }

    /// Apply transition sequence builder actions
    pub(crate) fn apply_sequence_actions(&mut self, actions: &UIActions) {
        use crate::engine::EngineCommand;
        use crate::usecases::ui::SequenceAction;
        for action in &actions.sequence_actions {
            let cmd = match action {
                SequenceAction::Create => EngineCommand::CreateSequence,
                SequenceAction::Delete(idx) => EngineCommand::DeleteSequence { idx: *idx },
                SequenceAction::ToggleEnabled(idx) => EngineCommand::ToggleSequence { idx: *idx },
                SequenceAction::Play(idx) => EngineCommand::PlaySequence { idx: *idx },
                SequenceAction::Stop(idx) => EngineCommand::StopSequence { idx: *idx },
                SequenceAction::AddFade { seq_idx, from_ch, to_ch } =>
                    EngineCommand::AddFadeStep { seq_idx: *seq_idx, from_ch: *from_ch, to_ch: *to_ch },
                SequenceAction::AddWait(idx) => EngineCommand::AddWaitStep { seq_idx: *idx },
                SequenceAction::AddGoTo { seq_idx, step_index } =>
                    EngineCommand::AddGoToStep { seq_idx: *seq_idx, step_index: *step_index },
                SequenceAction::RemoveStep { seq_idx, step_idx } =>
                    EngineCommand::RemoveStep { seq_idx: *seq_idx, step_idx: *step_idx },
                SequenceAction::MoveStep { seq_idx, from, to } =>
                    EngineCommand::MoveStep { seq_idx: *seq_idx, from: *from, to: *to },
                SequenceAction::SetStepDuration { seq_idx, step_idx, value } =>
                    EngineCommand::SetStepDurationValue { seq_idx: *seq_idx, step_idx: *step_idx, value: *value },
                SequenceAction::ToggleStepDurationUnit { seq_idx, step_idx } =>
                    EngineCommand::ToggleStepDurationUnit { seq_idx: *seq_idx, step_idx: *step_idx },
                SequenceAction::SetStepDurationUnit { seq_idx, step_idx, unit } =>
                    EngineCommand::SetStepDurationUnit { seq_idx: *seq_idx, step_idx: *step_idx, unit: *unit },
                SequenceAction::SetStepEasing { seq_idx, step_idx, easing } =>
                    EngineCommand::SetStepEasing { seq_idx: *seq_idx, step_idx: *step_idx, easing: easing.clone() },
                SequenceAction::SetStepFromCh { seq_idx, step_idx, ch } =>
                    EngineCommand::SetStepFromCh { seq_idx: *seq_idx, step_idx: *step_idx, ch: *ch },
                SequenceAction::SetStepToCh { seq_idx, step_idx, ch } =>
                    EngineCommand::SetStepToCh { seq_idx: *seq_idx, step_idx: *step_idx, ch: *ch },
                SequenceAction::SetGoToTarget { seq_idx, step_idx, target } =>
                    EngineCommand::SetGoToTarget { seq_idx: *seq_idx, step_idx: *step_idx, target: *target },
                SequenceAction::SetStepTransitionShader { seq_idx, step_idx, shader } =>
                    EngineCommand::SetStepTransitionShader { seq_idx: *seq_idx, step_idx: *step_idx, shader_name: shader.clone() },
            };
            self.execute_command(cmd);
        }
    }

    /// Restore a single DeckConfig into an existing channel.
    /// Shared by both deck preset loading and channel preset bulk-loading.
    #[allow(clippy::too_many_arguments)]
    fn restore_deck_into_channel(
        config: &crate::scene::DeckConfig,
        ch_idx: usize,
        context: &crate::renderer::GpuContext,
        registry: &crate::registry::ShaderRegistry,
        camera_manager: &mut crate::camera::CameraManager,
        ndi_manager: &mut crate::ndi::NdiManager,
        srt_manager: &mut crate::srt::SrtManager,
        render_width: u32,
        render_height: u32,
        mixer: &mut crate::mixer::Mixer,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
    ) -> anyhow::Result<()> {
        let mut deck = crate::persistence::restore_deck(
            config, context, registry,
            camera_manager, ndi_manager, srt_manager,
            render_width, render_height,
        )?;
        // Apply the preset's display name (overrides the generator/source name)
        if !config.name.is_empty() {
            deck.set_source_name(config.name.clone());
        }
        let dk_idx = {
            let ch = mixer.channel_mut(ch_idx)
                .ok_or_else(|| anyhow::anyhow!("Channel {} not found", ch_idx))?;
            let mut slot = crate::channel::DeckSlot::new(deck);
            slot.opacity = config.opacity;
            slot.blend_mode = config.blend_mode.into();
            slot.mute = config.mute;
            slot.solo = config.solo;
            slot.z_index = config.z_index;
            let dk_idx = ch.decks.len();
            ch.add_deck_slot(slot);
            let texture_id = egui_renderer.register_native_texture(
                &context.device,
                &ch.decks[dk_idx].deck.texture_view,
                wgpu::FilterMode::Linear,
            );
            deck_preview_textures.insert((ch_idx, dk_idx), texture_id);
            dk_idx
        };
        // Apply modulation recipes with deduplication
        if !config.modulation.is_empty() {
            let deck_uuid = mixer.channel(ch_idx)
                .and_then(|ch| ch.decks.get(dk_idx))
                .map(|slot| slot.deck.uuid().to_string())
                .unwrap_or_default();
            let new_prefix = format!("deck_{}", deck_uuid);
            apply_modulation_recipes(&config.modulation, &new_prefix, mixer.modulation_mut());
        }
        Ok(())
    }
}

/// Extract modulation recipes for a specific deck from the global engine.
/// Scans all assignments matching the deck's prefix and effect UUIDs,
/// groups by source, and strips prefixes to make them portable.
fn extract_modulation_recipes(
    engine: &crate::modulation::ModulationEngine,
    prefix: &str,
    effect_uuids: &[String],
) -> Vec<crate::scene::ModulationRecipe> {
    let prefix_colon = format!("{}:", prefix);
    let mut source_map: std::collections::HashMap<String, Vec<crate::scene::ModulationRecipeAssignment>> =
        std::collections::HashMap::new();

    // Build a set of effect key prefixes for this deck's effects
    let fx_prefixes: Vec<String> = effect_uuids.iter().map(|u| format!("fx_{}:", u)).collect();

    for (key, mods) in engine.assignments_iter() {
        // Match generator params: "deck_{uuid}:brightness" → relative "brightness"
        let relative_param = if let Some(rel) = key.strip_prefix(&prefix_colon) {
            Some(rel.to_string())
        } else {
            // Match effect params: "fx_{fx_uuid}:param" → relative "fx_{fx_uuid}:param"
            // We store the full effect key as-is so it can be re-applied with the same UUID
            fx_prefixes.iter().find(|p| key.starts_with(p.as_str())).map(|_| key.clone())
        };

        if let Some(relative_param) = relative_param {
            for m in mods {
                source_map.entry(m.source_id.clone())
                    .or_default()
                    .push(crate::scene::ModulationRecipeAssignment {
                        param: relative_param.clone(),
                        amount: m.amount,
                        component: m.component,
                    });
            }
        }
    }

    source_map.into_iter().filter_map(|(source_uuid, assignments)| {
        engine.find_source_by_uuid(&source_uuid).map(|entry| {
            crate::scene::ModulationRecipe {
                source_uuid: entry.uuid.clone(),
                source: entry.source.clone(),
                assignments,
            }
        })
    }).collect()
}

/// Apply modulation recipes to the global engine for a newly loaded deck.
/// UUID-is-identity: if a source with the recipe's UUID exists, wire up to it.
/// Otherwise create a new source with that UUID.
fn apply_modulation_recipes(
    recipes: &[crate::scene::ModulationRecipe],
    prefix: &str,
    engine: &mut crate::modulation::ModulationEngine,
) {
    for recipe in recipes {
        let source_uuid = if engine.has_source(&recipe.source_uuid) {
            recipe.source_uuid.clone()
        } else {
            let uuid = engine.add_source_with_uuid(recipe.source_uuid.clone(), recipe.source.clone());
            log::info!("Created new modulation source {} for preset", uuid);
            uuid
        };
        for assignment in &recipe.assignments {
            // Effect params stored as "fx_{uuid}:param" (already fully qualified)
            // Generator params stored as "brightness" → key "deck_{uuid}:brightness"
            let full_key = if assignment.param.starts_with("fx_") {
                assignment.param.clone()
            } else {
                format!("{}:{}", prefix, assignment.param)
            };
            engine.assign(&full_key, &source_uuid, assignment.amount, assignment.component);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulation::{ModulationEngine, ModulationSource};
    use crate::scene::{ModulationRecipe, ModulationRecipeAssignment};

    #[test]
    fn extract_captures_generator_and_effect_params() {
        let mut engine = ModulationEngine::new();
        let src_uuid = engine.add_source(ModulationSource::sine_lfo(2.0));
        // Generator param: deck_abc12345:brightness
        engine.assign("deck_abc12345:brightness", &src_uuid, 0.5, None);
        // Effect param: fx_effuuid1:amount (new format uses effect UUID)
        engine.assign("fx_effuuid1:amount", &src_uuid, 0.3, None);
        // Unrelated key from another deck — should NOT be captured
        engine.assign("deck_def67890:brightness", &src_uuid, 1.0, None);

        let effect_uuids = vec!["effuuid1".to_string()];
        let recipes = extract_modulation_recipes(&engine, "deck_abc12345", &effect_uuids);
        assert_eq!(recipes.len(), 1, "should group into one recipe (one source)");
        let recipe = &recipes[0];
        let mut params: Vec<&str> = recipe.assignments.iter().map(|a| a.param.as_str()).collect();
        params.sort();
        assert_eq!(params, vec!["brightness", "fx_effuuid1:amount"]);
    }

    #[test]
    fn apply_restores_generator_and_effect_keys() {
        let mut engine = ModulationEngine::new();
        let recipes = vec![ModulationRecipe {
            source_uuid: "test0001".to_string(),
            source: ModulationSource::sine_lfo(2.0),
            assignments: vec![
                ModulationRecipeAssignment { param: "brightness".into(), amount: 0.5, component: None },
                ModulationRecipeAssignment { param: "fx_effuuid1:amount".into(), amount: 0.3, component: None },
            ],
        }];

        apply_modulation_recipes(&recipes, "deck_newuuid1", &mut engine);

        assert_eq!(engine.source_count(), 1);
        assert!(engine.has_modulation("deck_newuuid1:brightness"), "generator key missing");
        assert!(engine.has_modulation("fx_effuuid1:amount"), "effect key missing");
    }

    #[test]
    fn roundtrip_extract_then_apply_preserves_effect_modulation() {
        // Simulate save: create engine with assignments, extract recipes
        let mut save_engine = ModulationEngine::new();
        let src_uuid = save_engine.add_source(ModulationSource::sine_lfo(3.0));
        save_engine.assign("deck_saveuuid:contrast", &src_uuid, 0.7, None);
        save_engine.assign("fx_fxuuid01:mix", &src_uuid, 0.4, None);

        let effect_uuids = vec!["fxuuid01".to_string()];
        let recipes = extract_modulation_recipes(&save_engine, "deck_saveuuid", &effect_uuids);

        // Simulate load: fresh engine, apply recipes into a different slot
        let mut load_engine = ModulationEngine::new();
        apply_modulation_recipes(&recipes, "deck_loaduuid", &mut load_engine);

        assert_eq!(load_engine.source_count(), 1);
        assert!(load_engine.has_modulation("deck_loaduuid:contrast"));
        assert!(load_engine.has_modulation("fx_fxuuid01:mix"));
    }
}

