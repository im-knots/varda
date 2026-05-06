use crate::modulation::ModulationSource;
use crate::params::ParamValue;
use crate::{Deck, Effect, Mixer, RenderContext, ShaderRegistry};
use super::{ParamUpdate, ModulationAction, CrossfaderAction, UIActions, RENDER_WIDTH, RENDER_HEIGHT};

/// Apply crossfader actions
pub fn apply_crossfader_actions(mixer: &mut Mixer, actions: &UIActions) {
    if let Some(action) = &actions.crossfader_action {
        match action {
            CrossfaderAction::SetPosition(pos) => {
                mixer.snap_crossfader(*pos);
            }
            CrossfaderAction::SnapA => {
                mixer.snap_crossfader(0.0);
            }
            CrossfaderAction::SnapB => {
                mixer.snap_crossfader(1.0);
            }
            CrossfaderAction::AutoTransition { target, duration_secs, easing } => {
                mixer.start_crossfade(*target, *duration_secs, *easing);
            }
            CrossfaderAction::BeatTransition { target, beats } => {
                mixer.start_beat_crossfade(*target, *beats);
            }
        }
    }
}

/// Apply channel property updates (opacity, blend mode)
pub fn apply_channel_updates(mixer: &mut Mixer, actions: &UIActions) {
    for &(ch_idx, opacity, blend_mode) in &actions.channel_updates {
        if let Some(ch) = mixer.channel_mut(ch_idx) {
            ch.opacity = opacity.clamp(0.0, 1.0);
            ch.blend_mode = blend_mode;
        }
    }
}

/// Apply deck property updates (opacity, blend mode, solo, mute)
pub fn apply_deck_updates(mixer: &mut Mixer, actions: &UIActions) {
    for &(ch_idx, deck_idx, opacity, blend_mode, solo, mute) in &actions.deck_updates {
        if let Some(ch) = mixer.channel_mut(ch_idx) {
            if deck_idx < ch.decks.len() {
                ch.decks[deck_idx].opacity = opacity;
                ch.decks[deck_idx].blend_mode = blend_mode;
                ch.decks[deck_idx].solo = solo;
                ch.decks[deck_idx].mute = mute;
            }
        }
    }
}

/// Apply scaling mode updates
pub fn apply_scaling_mode_updates(mixer: &mut Mixer, actions: &UIActions) {
    for &(ch_idx, deck_idx, scaling_mode) in &actions.scaling_mode_updates {
        if let Some(ch) = mixer.channel_mut(ch_idx) {
            if deck_idx < ch.decks.len() {
                ch.decks[deck_idx].deck.set_scaling_mode(scaling_mode);
            }
        }
    }
}

/// Apply video playback actions (play/pause, seek, speed, loop mode)
pub fn apply_video_actions(mixer: &mut Mixer, actions: &UIActions) {
    use super::VideoAction;
    for (ch_idx, deck_idx, action) in &actions.video_actions {
        if let Some(ch) = mixer.channel_mut(*ch_idx) {
            if *deck_idx < ch.decks.len() {
                let deck = &mut ch.decks[*deck_idx].deck;
                match action {
                    VideoAction::TogglePlay => {
                        if let Some(ps) = deck.playback_state_mut() {
                            ps.playing = !ps.playing;
                        }
                    }
                    VideoAction::Seek(pos) => {
                        if let Err(e) = deck.video_seek(*pos) {
                            log::warn!("Video seek failed: {}", e);
                        }
                    }
                    VideoAction::SetSpeed(speed) => {
                        if let Some(ps) = deck.playback_state_mut() {
                            ps.speed = *speed;
                        }
                    }
                    VideoAction::SetLoopMode(mode) => {
                        if let Some(ps) = deck.playback_state_mut() {
                            ps.loop_mode = *mode;
                        }
                    }
                    VideoAction::SetInPoint(t) => {
                        if let Some(ps) = deck.playback_state_mut() {
                            ps.in_point = *t;
                        }
                    }
                    VideoAction::SetOutPoint(t) => {
                        if let Some(ps) = deck.playback_state_mut() {
                            ps.out_point = *t;
                        }
                    }
                    VideoAction::ClearInOutPoints => {
                        if let Some(ps) = deck.playback_state_mut() {
                            ps.in_point = 0.0;
                            ps.out_point = 0.0;
                        }
                    }
                }
            }
        }
    }
}

/// Apply auto-transition configuration actions
pub fn apply_auto_transition_actions(
    mixer: &mut Mixer,
    actions: &UIActions,
    context: &RenderContext,
    registry: &ShaderRegistry,
) {
    use super::AutoTransitionAction;
    use crate::channel::{DeckAutoTransition, DurationSpec, TransitionTrigger};
    for (ch_idx, deck_idx, action) in &actions.auto_transition_actions {
        if let Some(ch) = mixer.channel_mut(*ch_idx) {
            if *deck_idx < ch.decks.len() {
                let slot = &mut ch.decks[*deck_idx];
                // Ensure auto_transition config exists
                if slot.auto_transition.is_none() {
                    slot.auto_transition = Some(DeckAutoTransition::new());
                }
                let at = slot.auto_transition.as_mut().unwrap();
                match action {
                    AutoTransitionAction::SetEnabled(en) => {
                        at.enabled = *en;
                        if !*en {
                            at.phase = crate::channel::DeckTransitionPhase::Inactive;
                        }
                    }
                    AutoTransitionAction::SetTrigger(clip_end) => {
                        at.trigger = if *clip_end { TransitionTrigger::ClipEnd } else { TransitionTrigger::Timer };
                    }
                    AutoTransitionAction::SetPlayDuration(val) => {
                        at.play_duration = match at.play_duration {
                            DurationSpec::Beats(_) => DurationSpec::Beats(*val),
                            DurationSpec::Seconds(_) => DurationSpec::Seconds(*val),
                        };
                    }
                    AutoTransitionAction::TogglePlayDurationUnit => {
                        at.play_duration = match at.play_duration {
                            DurationSpec::Beats(v) => DurationSpec::Seconds(v),
                            DurationSpec::Seconds(v) => DurationSpec::Beats(v),
                        };
                    }
                    AutoTransitionAction::SetTransitionDuration(val) => {
                        at.transition_duration = match at.transition_duration {
                            DurationSpec::Beats(_) => DurationSpec::Beats(*val),
                            DurationSpec::Seconds(_) => DurationSpec::Seconds(*val),
                        };
                    }
                    AutoTransitionAction::ToggleTransitionDurationUnit => {
                        at.transition_duration = match at.transition_duration {
                            DurationSpec::Beats(v) => DurationSpec::Seconds(v),
                            DurationSpec::Seconds(v) => DurationSpec::Beats(v),
                        };
                    }
                    AutoTransitionAction::SetTransitionShader(name_opt) => {
                        at.transition_shader_name = name_opt.clone();
                        // Compile or clear the transition shader
                        if let Some(shader_name) = name_opt {
                            if let Some(shader) = registry.transitions().iter()
                                .find(|s| s.name() == *shader_name)
                            {
                                if let Err(e) = slot.set_transition_shader(context, (*shader).clone()) {
                                    log::warn!("Failed to set deck transition shader: {}", e);
                                }
                            }
                        } else {
                            slot.transition_effect = None;
                        }
                    }
                }
            }
        }
    }
}

/// Apply parameter value updates (generator, effect, master effect params)
pub fn apply_param_updates(mixer: &mut Mixer, actions: &UIActions) {
    for update in &actions.param_updates {
        match update {
            ParamUpdate::GeneratorFloat { ch_idx, deck_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        ch.decks[*deck_idx].deck.generator_params.set(name, ParamValue::Float(*value));
                    }
                }
            }
            ParamUpdate::GeneratorBool { ch_idx, deck_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        ch.decks[*deck_idx].deck.generator_params.set(name, ParamValue::Bool(*value));
                    }
                }
            }
            ParamUpdate::GeneratorColor { ch_idx, deck_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        ch.decks[*deck_idx].deck.generator_params.set(name, ParamValue::Color(*value));
                    }
                }
            }
            ParamUpdate::GeneratorResetToDefaults { ch_idx, deck_idx } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        ch.decks[*deck_idx].deck.generator_params.reset_to_defaults();
                    }
                }
            }
            ParamUpdate::EffectFloat { ch_idx, deck_idx, effect_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        let deck = &mut ch.decks[*deck_idx].deck;
                        if *effect_idx < deck.effects.len() {
                            deck.effects[*effect_idx].params.set(name, ParamValue::Float(*value));
                        }
                    }
                }
            }
            ParamUpdate::EffectBool { ch_idx, deck_idx, effect_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        let deck = &mut ch.decks[*deck_idx].deck;
                        if *effect_idx < deck.effects.len() {
                            deck.effects[*effect_idx].params.set(name, ParamValue::Bool(*value));
                        }
                    }
                }
            }
            ParamUpdate::EffectColor { ch_idx, deck_idx, effect_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *deck_idx < ch.decks.len() {
                        let deck = &mut ch.decks[*deck_idx].deck;
                        if *effect_idx < deck.effects.len() {
                            deck.effects[*effect_idx].params.set(name, ParamValue::Color(*value));
                        }
                    }
                }
            }
            ParamUpdate::ChannelEffectFloat { ch_idx, effect_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *effect_idx < ch.effects.len() {
                        ch.effects[*effect_idx].params.set(name, ParamValue::Float(*value));
                    }
                }
            }
            ParamUpdate::ChannelEffectBool { ch_idx, effect_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *effect_idx < ch.effects.len() {
                        ch.effects[*effect_idx].params.set(name, ParamValue::Bool(*value));
                    }
                }
            }
            ParamUpdate::ChannelEffectColor { ch_idx, effect_idx, name, value } => {
                if let Some(ch) = mixer.channel_mut(*ch_idx) {
                    if *effect_idx < ch.effects.len() {
                        ch.effects[*effect_idx].params.set(name, ParamValue::Color(*value));
                    }
                }
            }
            ParamUpdate::MasterEffectFloat { effect_idx, name, value } => {
                if *effect_idx < mixer.master_effects.len() {
                    mixer.master_effects[*effect_idx].params.set(name, ParamValue::Float(*value));
                }
            }
            ParamUpdate::MasterEffectBool { effect_idx, name, value } => {
                if *effect_idx < mixer.master_effects.len() {
                    mixer.master_effects[*effect_idx].params.set(name, ParamValue::Bool(*value));
                }
            }
            ParamUpdate::MasterEffectColor { effect_idx, name, value } => {
                if *effect_idx < mixer.master_effects.len() {
                    mixer.master_effects[*effect_idx].params.set(name, ParamValue::Color(*value));
                }
            }
        }
    }
}

/// Apply modulation actions
/// Modulation engine lives on the Mixer; param keys use ch0_deck{idx} prefix
pub fn apply_modulation_actions(mixer: &mut Mixer, actions: &UIActions) {
    for action in &actions.modulation_actions {
        match action {
            ModulationAction::AddLFO { waveform, frequency } => {
                let source = ModulationSource::LFO {
                    waveform: *waveform, frequency: *frequency, phase: 0.0, amplitude: 1.0, bipolar: false,
                };
                let idx = mixer.modulation.add_source(source);
                log::info!("Added LFO modulation source {}", idx);
            }
            ModulationAction::AddAudioFFT { preset, source_id } => {
                let (freq_low, freq_high) = preset.freq_range();
                let source = ModulationSource::AudioBand { source_id: *source_id, freq_low, freq_high, gain: 1.0, smoothing: 0.6, mode: crate::modulation::AudioReactMode::Direct, noise_gate: 0.1 };
                let idx = mixer.modulation.add_source(source);
                log::info!("Added Audio FFT modulation source {} ({:?}, {}-{}Hz)", idx, preset, freq_low, freq_high);
            }
            ModulationAction::RemoveSource { idx } => {
                mixer.modulation.remove_source(*idx);
                log::info!("Removed modulation source {}", idx);
            }
            ModulationAction::UpdateLFOFrequency { idx, frequency } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { frequency: ref mut f, .. } = mixer.modulation.sources[*idx] {
                        *f = *frequency;
                    }
                }
            }
            ModulationAction::UpdateLFOWaveform { idx, waveform } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { waveform: ref mut w, .. } = mixer.modulation.sources[*idx] {
                        *w = *waveform;
                    }
                }
            }
            ModulationAction::UpdateLFOPhase { idx, phase } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { phase: ref mut p, .. } = mixer.modulation.sources[*idx] {
                        *p = *phase;
                    }
                }
            }
            ModulationAction::UpdateLFOAmplitude { idx, amplitude } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { amplitude: ref mut a, .. } = mixer.modulation.sources[*idx] {
                        *a = *amplitude;
                    }
                }
            }
            ModulationAction::UpdateLFOBipolar { idx, bipolar } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::LFO { bipolar: ref mut b, .. } = mixer.modulation.sources[*idx] {
                        *b = *bipolar;
                    }
                }
            }
            ModulationAction::UpdateAudioSmoothing { idx, smoothing } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { smoothing: ref mut s, .. } = mixer.modulation.sources[*idx] {
                        *s = *smoothing;
                    }
                }
            }
            ModulationAction::UpdateAudioFreqLow { idx, freq_low } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, .. } = mixer.modulation.sources[*idx] {
                        *fl = *freq_low;
                    }
                }
            }
            ModulationAction::UpdateAudioFreqHigh { idx, freq_high } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { freq_high: ref mut fh, .. } = mixer.modulation.sources[*idx] {
                        *fh = *freq_high;
                    }
                }
            }
            ModulationAction::UpdateAudioGain { idx, gain } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { gain: ref mut g, .. } = mixer.modulation.sources[*idx] {
                        *g = *gain;
                    }
                }
            }
            ModulationAction::UpdateAudioPreset { idx, preset } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { freq_low: ref mut fl, freq_high: ref mut fh, .. } = mixer.modulation.sources[*idx] {
                        let (lo, hi) = preset.freq_range();
                        *fl = lo;
                        *fh = hi;
                    }
                }
            }
            ModulationAction::UpdateAudioSource { idx, source_id } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { source_id: ref mut sid, .. } = mixer.modulation.sources[*idx] {
                        *sid = *source_id;
                    }
                }
            }
            ModulationAction::UpdateAudioMode { idx, mode } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { mode: ref mut m, .. } = mixer.modulation.sources[*idx] {
                        *m = *mode;
                    }
                }
            }
            ModulationAction::UpdateAudioNoiseGate { idx, noise_gate } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::AudioBand { noise_gate: ref mut ng, .. } = mixer.modulation.sources[*idx] {
                        *ng = *noise_gate;
                    }
                }
            }
            ModulationAction::AddADSR { attack, decay, sustain, release } => {
                let source = ModulationSource::adsr(*attack, *decay, *sustain, *release);
                let idx = mixer.modulation.add_source(source);
                log::info!("Added ADSR modulation source {}", idx);
            }
            ModulationAction::AddStepSequencer { num_steps, rate } => {
                let source = ModulationSource::step_sequencer(*num_steps, *rate);
                let idx = mixer.modulation.add_source(source);
                log::info!("Added StepSequencer modulation source {} ({} steps)", idx, num_steps);
            }
            ModulationAction::UpdateADSRAttack { idx, attack } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { attack: ref mut a, .. } = mixer.modulation.sources[*idx] {
                        *a = *attack;
                    }
                }
            }
            ModulationAction::UpdateADSRDecay { idx, decay } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { decay: ref mut d, .. } = mixer.modulation.sources[*idx] {
                        *d = *decay;
                    }
                }
            }
            ModulationAction::UpdateADSRSustain { idx, sustain } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { sustain: ref mut s, .. } = mixer.modulation.sources[*idx] {
                        *s = *sustain;
                    }
                }
            }
            ModulationAction::UpdateADSRRelease { idx, release } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::ADSR { release: ref mut r, .. } = mixer.modulation.sources[*idx] {
                        *r = *release;
                    }
                }
            }
            ModulationAction::TriggerADSR { idx } => {
                mixer.modulation.trigger_adsr(*idx);
            }
            ModulationAction::ReleaseADSR { idx } => {
                mixer.modulation.release_adsr(*idx);
            }
            ModulationAction::UpdateStepValue { idx, step_idx, value } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { steps, .. } = &mut mixer.modulation.sources[*idx] {
                        if *step_idx < steps.len() {
                            steps[*step_idx] = *value;
                        }
                    }
                }
            }
            ModulationAction::UpdateStepRate { idx, rate } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { rate: ref mut r, .. } = mixer.modulation.sources[*idx] {
                        *r = *rate;
                    }
                }
            }
            ModulationAction::UpdateStepInterpolation { idx, interpolation } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { interpolation: ref mut interp, .. } = mixer.modulation.sources[*idx] {
                        *interp = *interpolation;
                    }
                }
            }
            ModulationAction::UpdateStepBipolar { idx, bipolar } => {
                if *idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { bipolar: ref mut b, .. } = mixer.modulation.sources[*idx] {
                        *b = *bipolar;
                    }
                }
            }
            ModulationAction::AssignModOnMod { target_source_idx, param_name, modulator_idx, amount } => {
                mixer.modulation.assign_mod_on_mod(*target_source_idx, param_name, *modulator_idx, *amount);
                log::info!("Assigned mod-on-mod: source {} modulates source {} param {} (amount {})", modulator_idx, target_source_idx, param_name, amount);
            }
            ModulationAction::RemoveModOnMod { target_source_idx, param_name } => {
                mixer.modulation.clear_mod_on_mod(*target_source_idx, param_name);
                log::info!("Removed mod-on-mod from source {} param {}", target_source_idx, param_name);
            }
            ModulationAction::AssignModulation { ch_idx, deck_idx, param_name, source_idx, amount } => {
                mixer.modulation.assign(&format!("ch{}_deck{}:{}", ch_idx, deck_idx, param_name), *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to ch{} deck {} param {} with amount {}", source_idx, ch_idx, deck_idx, param_name, amount);
            }
            ModulationAction::RemoveAssignment { ch_idx, deck_idx, param_name, .. } => {
                mixer.modulation.clear_assignments(&format!("ch{}_deck{}:{}", ch_idx, deck_idx, param_name));
                log::info!("Removed modulation assignment from ch{} deck {} param {}", ch_idx, deck_idx, param_name);
            }
            ModulationAction::AssignEffectModulation { ch_idx, deck_idx, effect_idx, param_name, source_idx, amount } => {
                let key = format!("ch{}_deck{}_fx{}:{}", ch_idx, deck_idx, effect_idx, param_name);
                mixer.modulation.assign(&key, *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to ch{} deck {} effect {} param {}", source_idx, ch_idx, deck_idx, effect_idx, param_name);
            }
            ModulationAction::RemoveEffectAssignment { ch_idx, deck_idx, effect_idx, param_name } => {
                let key = format!("ch{}_deck{}_fx{}:{}", ch_idx, deck_idx, effect_idx, param_name);
                mixer.modulation.clear_assignments(&key);
                log::info!("Removed effect modulation from ch{} deck {} effect {} param {}", ch_idx, deck_idx, effect_idx, param_name);
            }
            ModulationAction::AssignChannelEffectModulation { ch_idx, effect_idx, param_name, source_idx, amount } => {
                let key = format!("ch{}_fx{}:{}", ch_idx, effect_idx, param_name);
                mixer.modulation.assign(&key, *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to ch{} channel effect {} param {}", source_idx, ch_idx, effect_idx, param_name);
            }
            ModulationAction::RemoveChannelEffectAssignment { ch_idx, effect_idx, param_name } => {
                let key = format!("ch{}_fx{}:{}", ch_idx, effect_idx, param_name);
                mixer.modulation.clear_assignments(&key);
                log::info!("Removed channel effect modulation from ch{} effect {} param {}", ch_idx, effect_idx, param_name);
            }
            ModulationAction::AssignMasterEffectModulation { effect_idx, param_name, source_idx, amount } => {
                let key = format!("master_fx{}:{}", effect_idx, param_name);
                mixer.modulation.assign(&key, *source_idx, *amount, None);
                log::info!("Assigned modulation source {} to master effect {} param {}", source_idx, effect_idx, param_name);
            }
            ModulationAction::RemoveMasterEffectAssignment { effect_idx, param_name } => {
                let key = format!("master_fx{}:{}", effect_idx, param_name);
                mixer.modulation.clear_assignments(&key);
                log::info!("Removed master effect modulation from effect {} param {}", effect_idx, param_name);
            }
        }
    }
}

/// Apply deck add/remove and effect add/remove/toggle actions.
pub fn apply_deck_and_effect_actions(
    mixer: &mut Mixer,
    context: &RenderContext,
    registry: &ShaderRegistry,
    actions: &mut UIActions,
    egui_renderer: &mut egui_wgpu::Renderer,
    deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
) {
    // Remove deck if requested
    if let Some((ch_idx, deck_idx)) = actions.deck_to_remove {
        if let Some(ch) = mixer.channel_mut(ch_idx) {
            if deck_idx < ch.decks.len() {
                ch.remove_deck(deck_idx);
                log::info!("Removed deck {} from channel {}", deck_idx, ch_idx);

                // Re-register deck preview textures for this channel
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
        if src_ch != dst_ch && src_ch < mixer.channels.len() && dst_ch < mixer.channels.len() {
            // Remove the deck slot from source channel (preserving opacity, blend, solo, mute)
            if src_deck < mixer.channels[src_ch].decks.len() {
                let slot = mixer.channels[src_ch].remove_deck_slot(src_deck).unwrap();
                let new_idx = mixer.channels[dst_ch].add_deck_slot(slot);

                log::info!("Moved deck {} from channel {} to channel {} (new index {})", src_deck, src_ch, dst_ch, new_idx);

                // Re-register preview textures for source channel (indices shifted)
                let src_keys: Vec<_> = deck_preview_textures.keys()
                    .filter(|(c, _)| *c == src_ch)
                    .copied()
                    .collect();
                for key in src_keys {
                    if let Some(tex_id) = deck_preview_textures.remove(&key) {
                        egui_renderer.free_texture(&tex_id);
                    }
                }
                for (i, deck_slot) in mixer.channels[src_ch].decks.iter().enumerate() {
                    let tex_id = egui_renderer.register_native_texture(
                        &context.device,
                        &deck_slot.deck.texture_view,
                        wgpu::FilterMode::Linear,
                    );
                    deck_preview_textures.insert((src_ch, i), tex_id);
                }

                // Re-register preview textures for destination channel
                let dst_keys: Vec<_> = deck_preview_textures.keys()
                    .filter(|(c, _)| *c == dst_ch)
                    .copied()
                    .collect();
                for key in dst_keys {
                    if let Some(tex_id) = deck_preview_textures.remove(&key) {
                        egui_renderer.free_texture(&tex_id);
                    }
                }
                for (i, deck_slot) in mixer.channels[dst_ch].decks.iter().enumerate() {
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
        let generators = registry.generators();
        if gen_idx < generators.len() {
            let shader = generators[gen_idx].clone();
            match Deck::new(context, shader.clone(), RENDER_WIDTH, RENDER_HEIGHT) {
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
                Err(e) => {
                    log::error!("Failed to create deck: {}", e);
                }
            }
        }
    }

    // Add image deck if requested
    if let Some((ch_idx, path)) = actions.image_to_add.take() {
        match Deck::new_from_image(context, &path, RENDER_WIDTH, RENDER_HEIGHT) {
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
            Err(e) => {
                log::error!("Failed to create image deck: {}", e);
            }
        }
    }

    // Add video deck if requested
    if let Some((ch_idx, path)) = actions.video_to_add.take() {
        match Deck::new_from_video(context, &path, RENDER_WIDTH, RENDER_HEIGHT) {
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
            Err(e) => {
                log::error!("Failed to create video deck: {}", e);
            }
        }
    }

    // Add solid color deck if requested
    if let Some((ch_idx, color)) = actions.solid_color_to_add.take() {
        match Deck::new_solid_color(context, color, RENDER_WIDTH, RENDER_HEIGHT) {
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
            Err(e) => {
                log::error!("Failed to create solid color deck: {}", e);
            }
        }
    }

    // Add effect to deck
    if let Some((ch_idx, deck_idx, filter_idx)) = actions.effect_to_add {
        let filters = registry.filters();
        if filter_idx < filters.len() {
            let filter_shader = filters[filter_idx].clone();
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                if deck_idx < ch.decks.len() {
                    match Effect::new(context, filter_shader.clone()) {
                        Ok(effect) => {
                            ch.decks[deck_idx].deck.add_effect(effect);
                            log::info!("Added effect {} to ch{} deck {}", filter_shader.name(), ch_idx, deck_idx);
                        }
                        Err(e) => {
                            log::error!("Failed to create effect: {}", e);
                        }
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

    // Add effect to channel (must match channel composite texture format)
    if let Some((ch_idx, filter_idx)) = actions.ch_effect_to_add {
        let filters = registry.filters();
        if filter_idx < filters.len() {
            let filter_shader = filters[filter_idx].clone();
            if let Some(ch) = mixer.channel_mut(ch_idx) {
                match Effect::new_with_format(context, filter_shader.clone(), context.surface_config.format) {
                    Ok(effect) => {
                        ch.add_effect(effect);
                        log::info!("Added channel effect {} to ch{}", filter_shader.name(), ch_idx);
                    }
                    Err(e) => {
                        log::error!("Failed to create channel effect '{}': {}", filter_shader.name(), e);
                    }
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

    // Add master effect (on mixer)
    if let Some(filter_idx) = actions.master_effect_to_add {
        let filters = registry.filters();
        if filter_idx < filters.len() {
            let filter_shader = filters[filter_idx].clone();
            let filter_name = filter_shader.name();
            match Effect::new_with_format(context, filter_shader, context.surface_config.format) {
                Ok(effect) => {
                    mixer.add_master_effect(effect);
                    log::info!("Added master effect: {}", filter_name);
                }
                Err(e) => {
                    log::error!("Failed to create master effect '{}': {}", filter_name, e);
                }
            }
        }
    }

    // Toggle master effect
    if let Some(effect_idx) = actions.master_effect_to_toggle {
        if effect_idx < mixer.master_effects.len() {
            mixer.master_effects[effect_idx].enabled = !mixer.master_effects[effect_idx].enabled;
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
        if from_idx < mixer.master_effects.len() && to_idx < mixer.master_effects.len() && from_idx != to_idx {
            let effect = mixer.master_effects.remove(from_idx);
            mixer.master_effects.insert(to_idx, effect);
            log::info!("Moved master effect {} -> {}", from_idx, to_idx);
        }
    }
}

/// Apply transition shader selection
pub fn apply_transition_actions(
    mixer: &mut Mixer,
    context: &RenderContext,
    registry: &ShaderRegistry,
    actions: &UIActions,
) {
    if let Some(transition_opt) = &actions.set_transition {
        match transition_opt {
            None => {
                // Clear transition — revert to opacity crossfade
                mixer.clear_transition();
            }
            Some(name) => {
                // Look up the shader in the registry and compile it
                if let Some(shader) = registry.get(name) {
                    if let Err(e) = mixer.set_transition(context, shader.clone()) {
                        log::error!("Failed to set transition '{}': {}", name, e);
                    }
                } else {
                    log::warn!("Transition shader '{}' not found in registry", name);
                }
            }
        }
    }
}

/// Apply transition sequence builder actions
pub fn apply_sequence_actions(mixer: &mut Mixer, actions: &UIActions) {
    use super::SequenceAction;
    use crate::channel::DurationSpec;
    use crate::mixer::{TransitionSequence, TransitionStep, StepKind, CrossfadeEasing};

    for action in &actions.sequence_actions {
        match action {
            SequenceAction::Create => {
                let n = mixer.transition_sequences.len() + 1;
                mixer.transition_sequences.push(TransitionSequence::new(format!("Sequence {}", n)));
                log::info!("Created transition sequence {}", n);
            }
            SequenceAction::Delete(idx) => {
                if *idx < mixer.transition_sequences.len() {
                    let name = mixer.transition_sequences[*idx].name.clone();
                    mixer.transition_sequences.remove(*idx);
                    log::info!("Deleted transition sequence '{}'", name);
                }
            }
            SequenceAction::ToggleEnabled(idx) => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*idx) {
                    seq.enabled = !seq.enabled;
                    if !seq.enabled { seq.state.reset(); }
                }
            }
            SequenceAction::Play(idx) => { mixer.start_sequence(*idx); }
            SequenceAction::Stop(idx) => { mixer.stop_sequence(*idx); }
            SequenceAction::AddFade { seq_idx, from_ch, to_ch } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    seq.steps.push(TransitionStep { kind: StepKind::Fade {
                        from_ch: *from_ch, to_ch: *to_ch,
                        duration: DurationSpec::Seconds(2.0),
                        easing: CrossfadeEasing::EaseInOut, transition_shader: None,
                    }});
                }
            }
            SequenceAction::AddWait(idx) => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*idx) {
                    seq.steps.push(TransitionStep { kind: StepKind::Wait {
                        duration: DurationSpec::Seconds(2.0),
                    }});
                }
            }
            SequenceAction::AddGoTo { seq_idx, step_index } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    seq.steps.push(TransitionStep { kind: StepKind::GoTo { step_index: *step_index } });
                }
            }
            SequenceAction::RemoveStep { seq_idx, step_idx } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    if *step_idx < seq.steps.len() { seq.steps.remove(*step_idx); }
                }
            }
            SequenceAction::MoveStep { seq_idx, from, to } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    if *from < seq.steps.len() && *to < seq.steps.len() && from != to {
                        let step = seq.steps.remove(*from);
                        seq.steps.insert(*to, step);
                    }
                }
            }
            SequenceAction::SetStepDuration { seq_idx, step_idx, value } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
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
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
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
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
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
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    if let Some(step) = seq.steps.get_mut(*step_idx) {
                        if let StepKind::Fade { from_ch, .. } = &mut step.kind { *from_ch = *ch; }
                    }
                }
            }
            SequenceAction::SetStepToCh { seq_idx, step_idx, ch } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    if let Some(step) = seq.steps.get_mut(*step_idx) {
                        if let StepKind::Fade { to_ch, .. } = &mut step.kind { *to_ch = *ch; }
                    }
                }
            }
            SequenceAction::SetGoToTarget { seq_idx, step_idx, target } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
                    if let Some(step) = seq.steps.get_mut(*step_idx) {
                        if let StepKind::GoTo { step_index } = &mut step.kind { *step_index = *target; }
                    }
                }
            }
            SequenceAction::SetStepTransitionShader { seq_idx, step_idx, shader } => {
                if let Some(seq) = mixer.transition_sequences.get_mut(*seq_idx) {
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