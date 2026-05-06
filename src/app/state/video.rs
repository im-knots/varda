//! Video playback and auto-transition state mutations.

use crate::usecases::ui::UIActions;
use super::super::VardaApp;

impl VardaApp {
    /// Apply video playback actions (play/pause, seek, speed, loop mode)
    pub(crate) fn apply_video_actions(&mut self, actions: &UIActions) {
        use crate::usecases::ui::VideoAction;
        for (ch_idx, deck_idx, action) in &actions.video_actions {
            if let Some(ch) = self.mixer.channel_mut(*ch_idx) {
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
    pub(crate) fn apply_auto_transition_actions(&mut self, actions: &UIActions) {
        use crate::usecases::ui::AutoTransitionAction;
        use crate::channel::{DeckAutoTransition, DurationSpec, TransitionTrigger};
        for (ch_idx, deck_idx, action) in &actions.auto_transition_actions {
            if let Some(ch) = self.mixer.channel_mut(*ch_idx) {
                if *deck_idx < ch.decks.len() {
                    let slot = &mut ch.decks[*deck_idx];
                    if slot.auto_transition.is_none() {
                        slot.auto_transition = Some(DeckAutoTransition::new());
                    }
                    let Some(at) = slot.auto_transition.as_mut() else { continue; };
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
                            if let Some(shader_name) = name_opt {
                                if let Some(shader) = self.registry.transitions().iter()
                                    .find(|s| s.name() == *shader_name)
                                {
                                    if let Err(e) = slot.set_transition_shader(&self.context, (*shader).clone()) {
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
}
