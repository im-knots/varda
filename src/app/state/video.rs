//! Video playback and auto-transition state mutations.

use super::super::VardaApp;
use crate::engine::EngineCommand;
use crate::usecases::ui::UIActions;

impl VardaApp {
    /// Apply video playback actions (play/pause, seek, speed, loop mode)
    pub(crate) fn apply_video_actions(&mut self, actions: &UIActions) {
        use crate::usecases::ui::VideoAction;
        for (ch_idx, deck_idx, action) in &actions.video_actions {
            let cmd = match action {
                VideoAction::TogglePlay => EngineCommand::VideoTogglePlay {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                },
                VideoAction::Seek(pos) => EngineCommand::VideoSeek {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    position_secs: *pos,
                },
                VideoAction::SetSpeed(speed) => EngineCommand::VideoSetSpeed {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    speed: *speed,
                },
                VideoAction::SetLoopMode(mode) => EngineCommand::VideoSetLoopMode {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    mode: *mode,
                },
                VideoAction::SetInPoint(t) => EngineCommand::VideoSetInPoint {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    secs: *t,
                },
                VideoAction::SetOutPoint(t) => EngineCommand::VideoSetOutPoint {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    secs: *t,
                },
                VideoAction::ClearInOutPoints => EngineCommand::VideoClearInOutPoints {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                },
            };
            self.execute_command(cmd);
        }
    }

    /// Apply auto-transition configuration actions
    pub(crate) fn apply_auto_transition_actions(&mut self, actions: &UIActions) {
        use crate::usecases::ui::AutoTransitionAction;
        for (ch_idx, deck_idx, action) in &actions.auto_transition_actions {
            let cmd = match action {
                AutoTransitionAction::SetEnabled(en) => EngineCommand::SetAutoTransitionEnabled {
                    channel_idx: *ch_idx,
                    deck_idx: *deck_idx,
                    enabled: *en,
                },
                AutoTransitionAction::SetTrigger(clip_end) => {
                    EngineCommand::SetAutoTransitionTrigger {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                        clip_end: *clip_end,
                    }
                }
                AutoTransitionAction::SetPlayDuration(val) => {
                    EngineCommand::SetAutoTransitionPlayDurationValue {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                        value: *val,
                    }
                }
                AutoTransitionAction::TogglePlayDurationUnit => {
                    EngineCommand::ToggleAutoTransitionPlayDurationUnit {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                    }
                }
                AutoTransitionAction::SetTransitionDuration(val) => {
                    EngineCommand::SetAutoTransitionDurationValue {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                        value: *val,
                    }
                }
                AutoTransitionAction::ToggleTransitionDurationUnit => {
                    EngineCommand::ToggleAutoTransitionDurationUnit {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                    }
                }
                AutoTransitionAction::SetTransitionShader(name_opt) => {
                    EngineCommand::SetAutoTransitionShader {
                        channel_idx: *ch_idx,
                        deck_idx: *deck_idx,
                        shader_name: name_opt.clone(),
                    }
                }
            };
            self.execute_command(cmd);
        }
    }
}
