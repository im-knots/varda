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
            // Release camera before removal
            if let Some(ch) = mixer.channels().get(ch_idx) {
                if let Some(slot) = ch.decks.get(deck_idx) {
                    if let Some(cam_id) = slot.deck.camera_id() {
                        self.camera_manager.release_camera(cam_id);
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

        // Add new deck if requested
        if let Some((ch_idx, gen_idx)) = actions.shader_to_add {
            let generators = self.registry.generators();
            if gen_idx < generators.len() {
                let shader = generators[gen_idx].clone();
                match Deck::new(context, shader.clone(), self.render_width, self.render_height) {
                    Ok(deck) => {
                        if let Some(ch) = mixer.channel_mut(ch_idx) {
                            let idx = ch.add_deck(deck);
                            log::info!("Added deck {} to channel {} with shader: {}", idx, ch_idx, shader.name());
                            let texture_id = egui_renderer.register_native_texture(
                                &context.device,
                                &ch.decks[idx].deck.texture_view,
                                wgpu::FilterMode::Linear,
                            );
                            deck_preview_textures.insert((ch_idx, idx), texture_id);
                        }
                    }
                    Err(e) => log::error!("Failed to create deck: {}", e),
                }
            }
        }

        // Add image deck if requested
        if let Some((ch_idx, path)) = actions.image_to_add.take() {
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
                Err(e) => log::error!("Failed to create image deck: {}", e),
            }
        }

        // Add video deck if requested
        if let Some((ch_idx, path)) = actions.video_to_add.take() {
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
                Err(e) => log::error!("Failed to create video deck: {}", e),
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
        use crate::usecases::ui::SequenceAction;
        use crate::channel::DurationSpec;
        use crate::mixer::{TransitionSequence, TransitionStep, StepKind, CrossfadeEasing};
        let mixer = &mut self.mixer;
        for action in &actions.sequence_actions {
            match action {
                SequenceAction::Create => {
                    let n = mixer.transition_sequences().len() + 1;
                    mixer.transition_sequences_mut().push(TransitionSequence::new(format!("Sequence {}", n)));
                    log::info!("Created transition sequence {}", n);
                }
                SequenceAction::Delete(idx) => {
                    if *idx < mixer.transition_sequences().len() {
                        let name = mixer.transition_sequences_mut()[*idx].name.clone();
                        mixer.transition_sequences_mut().remove(*idx);
                        log::info!("Deleted transition sequence '{}'", name);
                    }
                }
                SequenceAction::ToggleEnabled(idx) => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*idx) {
                        seq.enabled = !seq.enabled;
                        if !seq.enabled { seq.state.reset(); }
                    }
                }
                SequenceAction::Play(idx) => { mixer.start_sequence(*idx); }
                SequenceAction::Stop(idx) => { mixer.stop_sequence(*idx); }
                SequenceAction::AddFade { seq_idx, from_ch, to_ch } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        seq.steps.push(TransitionStep { kind: StepKind::Fade {
                            from_ch: *from_ch, to_ch: *to_ch,
                            duration: DurationSpec::Seconds(2.0),
                            easing: CrossfadeEasing::EaseInOut, transition_shader: None,
                        }});
                    }
                }
                SequenceAction::AddWait(idx) => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*idx) {
                        seq.steps.push(TransitionStep { kind: StepKind::Wait {
                            duration: DurationSpec::Seconds(2.0),
                        }});
                    }
                }
                SequenceAction::AddGoTo { seq_idx, step_index } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        seq.steps.push(TransitionStep { kind: StepKind::GoTo { step_index: *step_index } });
                    }
                }
                SequenceAction::RemoveStep { seq_idx, step_idx } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if *step_idx < seq.steps.len() { seq.steps.remove(*step_idx); }
                    }
                }
                SequenceAction::MoveStep { seq_idx, from, to } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if *from < seq.steps.len() && *to < seq.steps.len() && from != to {
                            let step = seq.steps.remove(*from);
                            seq.steps.insert(*to, step);
                        }
                    }
                }
                SequenceAction::SetStepDuration { seq_idx, step_idx, value } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            match &mut step.kind {
                                StepKind::Fade { duration, .. } | StepKind::Wait { duration } => {
                                    *duration = match *duration {
                                        DurationSpec::Beats(_) => DurationSpec::Beats(*value),
                                        DurationSpec::Seconds(_) => DurationSpec::Seconds(*value),
                                    };
                                }
                                _ => {}
                            }
                        }
                    }
                }
                SequenceAction::ToggleStepDurationUnit { seq_idx, step_idx } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            match &mut step.kind {
                                StepKind::Fade { duration, .. } | StepKind::Wait { duration } => {
                                    *duration = match *duration {
                                        DurationSpec::Beats(v) => DurationSpec::Seconds(v),
                                        DurationSpec::Seconds(v) => DurationSpec::Beats(v),
                                    };
                                }
                                _ => {}
                            }
                        }
                    }
                }
                SequenceAction::SetStepEasing { seq_idx, step_idx, easing } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            if let StepKind::Fade { easing: e, .. } = &mut step.kind {
                                *e = match easing.as_str() {
                                    "Linear" => CrossfadeEasing::Linear,
                                    "EaseIn" => CrossfadeEasing::EaseIn,
                                    "EaseOut" => CrossfadeEasing::EaseOut,
                                    _ => CrossfadeEasing::EaseInOut,
                                };
                            }
                        }
                    }
                }
                SequenceAction::SetStepFromCh { seq_idx, step_idx, ch } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            if let StepKind::Fade { from_ch, .. } = &mut step.kind { *from_ch = *ch; }
                        }
                    }
                }
                SequenceAction::SetStepToCh { seq_idx, step_idx, ch } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            if let StepKind::Fade { to_ch, .. } = &mut step.kind { *to_ch = *ch; }
                        }
                    }
                }
                SequenceAction::SetGoToTarget { seq_idx, step_idx, target } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            if let StepKind::GoTo { step_index } = &mut step.kind { *step_index = *target; }
                        }
                    }
                }
                SequenceAction::SetStepTransitionShader { seq_idx, step_idx, shader } => {
                    if let Some(seq) = mixer.transition_sequences_mut().get_mut(*seq_idx) {
                        if let Some(step) = seq.steps.get_mut(*step_idx) {
                            if let StepKind::Fade { transition_shader, .. } = &mut step.kind {
                                *transition_shader = shader.clone();
                            }
                        }
                    }
                }
            }
        }
    }
}
