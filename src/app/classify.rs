//! What a command is, beyond what it does: whether it starts an undo step, and
//! which parameter it writes as a live gesture.

use super::VardaApp;
use crate::engine::EngineCommand;
use crate::engine::value::param::{DeckTarget, ParamAddress};

/// Whether a bus-driven command should record an undo/redo snapshot before it
/// executes. This is what makes API / WebSocket / CLI edits undoable on the
/// same timeline the windowed UI uses (see [undo-redo.md](/spec/undo-redo.md)).
///
/// The predicate is an explicit **denylist** of live-control, transient, and
/// non-authored commands; everything else defaults to undoable. New commands
/// are therefore undoable unless added here — when introducing a live control
/// (transport, device toggle, output-window lifecycle) or a transient action,
/// add it below so it does not pollute the undo timeline. The windowed
/// consumer uses the same predicate through `batch_has_undoable`.
pub(crate) fn command_is_undoable(cmd: &EngineCommand) -> bool {
    use EngineCommand as C;
    !matches!(
        cmd,
        // Live crossfader control (spec: ⚠️ live, excluded).
        C::SetCrossfader(..)
            | C::AutoCrossfade { .. }
            | C::BeatCrossfade { .. }
            // Live macro-knob turn (fans out to targets; config edits stay undoable).
            | C::SetMacroValue { .. }
            // Audio device lifecycle / scanning.
            | C::OpenAudioSource { .. }
            | C::CloseAudioSource { .. }
            | C::ScanAudioDevices
            | C::RescanAudio
            | C::ToggleAudioSource { .. }
            // Video transport (temporal, not structural).
            | C::VideoTogglePlay { .. }
            | C::VideoSeek { .. }
            | C::VideoSetSpeed { .. }
            | C::VideoSetLoopMode { .. }
            | C::VideoSetInPoint { .. }
            | C::VideoSetOutPoint { .. }
            | C::VideoClearInOutPoints { .. }
            | C::VideoSetTransportSync { .. }
            // ADSR live triggers.
            | C::TriggerAdsr { .. }
            | C::ReleaseAdsr { .. }
            // Sequence playback transport (authoring steps stay undoable).
            | C::PlaySequence { .. }
            | C::StopSequence { .. }
            | C::ToggleSequence { .. }
            // Copying reads the scene; paste and duplicate are undoable.
            | C::Copy { .. }
            // Arming is a mode. What a pass records is undoable, in one entry
            // pushed when the first take opens.
            | C::SetRecordArmed { .. }
            // HTML transient window / reload.
            | C::OpenHtmlInteractive { .. }
            | C::CloseHtmlInteractive
            | C::ReloadHtmlDeck { .. }
            // Stream library config (not scene state).
            | C::AddStreamLibraryEntry { .. }
            | C::RemoveStreamLibraryEntry { .. }
            | C::AddHlsLibraryEntry { .. }
            | C::RemoveHlsLibraryEntry { .. }
            | C::AddDashLibraryEntry { .. }
            | C::RemoveDashLibraryEntry { .. }
            | C::AddRtmpLibraryEntry { .. }
            | C::RemoveRtmpLibraryEntry { .. }
            | C::AddHtmlLibraryEntry { .. }
            | C::RemoveHtmlLibraryEntry { .. }
            // Output-window lifecycle / device config (spec: ❌, excluded).
            // Surface→output *assignments* remain undoable (default true).
            | C::CreateOutput
            | C::CreateHeadlessOutput { .. }
            | C::CloseOutput { .. }
            | C::SetOutputDisplay { .. }
            | C::SetOutputTarget { .. }
            | C::StartOutput { .. }
            | C::StopOutput { .. }
            | C::SetCalibrationMode { .. }
            | C::SetOutputRotation { .. }
            | C::SetOutputPresentation { .. }
            | C::SetOutputTonemap { .. }
            | C::SetEdgeBlend { .. }
            | C::SetEdgeBlendMode { .. }
            // Surface auto-detection produces preview contours only; the scene
            // is not mutated until ConfirmDetectedContours (which is undoable).
            | C::DetectFromImage { .. }
            | C::DetectFromSvg { .. }
            | C::DetectFromDxf { .. }
            | C::DetectFromCamera { .. }
            // Analyzer instance lifecycle (runtime, not SceneConfig state).
            | C::RequestAnalyzer { .. }
            | C::ReleaseAnalyzer { .. }
            | C::AddAnalyzerModSource { .. }
            | C::UpdateAnalyzerSmoothing { .. }
            // Device scanning / MIDI mappings (device config, not scene).
            | C::RescanNdi
            | C::RescanSyphon
            | C::RescanCameras
            | C::RescanCaptureTargets
            | C::RequestScreenCapturePermission
            | C::RescanMidi
            | C::SetMidiDeviceEnabled { .. }
            | C::ClearMidiMappings
            | C::RemoveMidiMapping { .. }
            // Clock preference / manual BPM (live sync config).
            | C::SetClockPreference { .. }
            | C::SetManualBpm { .. }
            // Which cable the show is following is live rig config, like the
            // clock's: undoing an edit must not silently re-patch the room.
            | C::SetTimecodePreference { .. }
            | C::SetLtcInput { .. }
            // Show position and re-arm are session state. Lane and region edits
            // are ordinary scene data and stay undoable; undoing one of them
            // must not also rewind the show or revive a released fader.
            | C::TransportPlay
            | C::TransportStop
            | C::TransportLocate { .. }
            | C::TransportPrevCue
            | C::TransportNextCue
            | C::TriggerCue { .. }
            | C::SetTransportSource { .. }
            | C::RearmParam { .. }
            | C::RearmAll { .. }
            // Folding a lane away rearranges the view, not the show.
            | C::SetLaneCollapsed { .. }
            // Global engine settings / profiling.
            | C::SetRenderResolution { .. }
            | C::SetDomemasterResolution { .. }
            // Stage editor view state the engine only stores for the GUI.
            | C::SetEditorPrefs { .. }
            // Learn modes bind controls, and notifications are feedback; neither
            // is an edit to the show.
            | C::MidiLearnToggle
            | C::MidiLearnSelect { .. }
            | C::KeyboardLearnToggle
            | C::KeyboardLearnSelect { .. }
            | C::KeyboardLearnBind { .. }
            | C::DismissNotification { .. }
            | C::NotifyInfo { .. }
            // What a consumer is looking at, not what the show is.
            | C::SetPreviewChannels { .. }
            | C::AcquireDetectionCamera { .. }
            | C::ReleaseDetectionCamera
            | C::SetTargetFps { .. }
            | C::StartPerfProfile { .. }
            // Param toggle is a live keyboard/shortcut affordance (SetParam edits
            // stay undoable; the two-value toggle does not pollute the timeline).
            | C::ToggleParam { .. }
            // Saving a preset writes to disk; loading one (structural) is undoable.
            | C::SaveDeckPreset { .. }
            | C::SaveChannelPreset { .. }
            // Persistence, history control, and shutdown are never undoable.
            | C::SaveWorkspace
            | C::LoadWorkspace
            | C::Undo
            | C::Redo
            | C::Shutdown
    )
}

impl VardaApp {
    /// The parameter a command writes as a live gesture, as the modulation key
    /// and the normalized value it lands on: what the automation recorder
    /// captures and what takes the lane back from the arrangement.
    ///
    /// Read before the command runs, because some values depend on the state
    /// it replaces (a toggle's new play state, a seek's clip length). The match
    /// lists every command so a new one cannot be added without deciding
    /// whether it is a live write. See /spec/vardapp-decomposition.md.
    pub(crate) fn live_write(&self, cmd: &EngineCommand) -> Option<(String, f32)> {
        use EngineCommand as C;
        match cmd {
            C::SetDeckOpacity { deck_uuid, opacity } => Some((
                ParamAddress::deck(deck_uuid, DeckTarget::Opacity).to_string(),
                *opacity,
            )),
            C::SetChannelOpacity {
                channel_uuid,
                opacity,
            } => Some((
                ParamAddress::channel_opacity(channel_uuid).to_string(),
                *opacity,
            )),
            // Scaling belongs to any deck with a source texture, not just a
            // video one, but it is modulatable on the same terms.
            C::SetDeckScalingMode { deck_uuid, mode } => Some((
                ParamAddress::deck(deck_uuid, DeckTarget::ScalingMode).to_string(),
                mode.to_value(),
            )),
            // Playback gestures record the normalized value a curve would need
            // to hold to reproduce them, because that is the space the override
            // ramp and the recorder both work in.
            C::VideoTogglePlay { deck_uuid } => {
                let was_playing = self
                    .video_playback_snapshot(deck_uuid)
                    .is_some_and(|s| s.playing);
                Some((
                    ParamAddress::deck(deck_uuid, DeckTarget::VideoPlay).to_string(),
                    f32::from(u8::from(!was_playing)),
                ))
            }
            C::VideoSeek {
                deck_uuid,
                position_secs,
            } => {
                let duration = self
                    .video_playback_snapshot(deck_uuid)
                    .map_or(0.0, |s| s.duration);
                Some((
                    ParamAddress::deck(deck_uuid, DeckTarget::VideoPosition).to_string(),
                    crate::param_router::duration_to_norm(*position_secs, duration),
                ))
            }
            C::VideoSetSpeed { deck_uuid, speed } => Some((
                ParamAddress::deck(deck_uuid, DeckTarget::VideoSpeed).to_string(),
                crate::param_router::speed_to_norm(*speed),
            )),
            C::VideoSetLoopMode { deck_uuid, mode } => Some((
                ParamAddress::deck(deck_uuid, DeckTarget::VideoLoopMode).to_string(),
                mode.to_value(),
            )),
            // A shader parameter is a live write only when it has a range to
            // normalize into.
            C::SetGeneratorParam {
                deck_uuid,
                name,
                value,
            } => {
                let (ch, dk) = self.mixer.find_deck_by_uuid(deck_uuid)?;
                let params = &self.mixer.channels()[ch].decks[dk].deck.generator_params;
                let normalized = params.normalize(name, value)?;
                Some((ParamAddress::deck_param(deck_uuid, name).to_string(), normalized))
            }
            C::SetEffectParam {
                effect_uuid,
                name,
                value,
            } => {
                let location = self.mixer.find_effect_by_uuid(effect_uuid)?;
                let normalized = self.mixer.effect_at(location)?.params.normalize(name, value)?;
                Some((ParamAddress::effect_param(effect_uuid, name).to_string(), normalized))
            }
            C::SetParam { path, value } => Some((
                crate::param_router::modulation_key_for_path(path)?,
                crate::param_router::param_value_to_norm_f32(value),
            )),
        // Mixer
        C::SetCrossfader(..)
        | C::SetTonemapMode(..)
        | C::LoadLut { .. }
        | C::UnloadLut
        | C::LoadLookLut { .. }
        | C::UnloadLookLut
        | C::AutoCrossfade { .. }
        | C::BeatCrossfade { .. }
        | C::AddDeck { .. }
        | C::AddImageDeck { .. }
        | C::AddVideoDeck { .. }
        | C::AddSolidColorDeck { .. }
        | C::AddCameraDeck { .. }
        | C::AddDepthSensorDeck { .. }
        | C::AddScreenCaptureDeck { .. }
        | C::AddTapDeck { .. }
        | C::SetTapSource { .. }
        | C::RemoveDeck { .. }
        | C::MoveDeck { .. }
        | C::ReorderDeck { .. }
        | C::SetDeckBlendMode { .. }
        | C::SetDeckSolo { .. }
        | C::SetDeckMute { .. }
        | C::SetDeckRenderFps { .. }
        | C::SetDeckTransparent { .. }
        | C::SetChannelBlendMode { .. }
        | C::AddChannel
        | C::RemoveChannel { .. }
        | C::AddEffect { .. }
        | C::RemoveEffect { .. }
        | C::ToggleEffect { .. }
        | C::MoveEffect { .. }
        // Clipboard (see /spec/clipboard.md)
        | C::Copy { .. }
        | C::Paste { .. }
        | C::Duplicate { .. }
        | C::SetTransition { .. }
        | C::ToggleParam { .. }
        // Audio
        | C::OpenAudioSource { .. }
        | C::CloseAudioSource { .. }
        | C::ScanAudioDevices
        // Modulation
        | C::AddLfo { .. }
        | C::AddAudioBand { .. }
        | C::AddAdsr { .. }
        | C::AddStepSequencer { .. }
        | C::AddAutomationLane { .. }
        | C::SetEnvelopeBreakpoints { .. }
        | C::RemoveModulationSource { .. }
        | C::AssignModulation { .. }
        | C::ClearModulation { .. }
        | C::ClearModulationSource { .. }
        // Video Playback
        | C::VideoSetInPoint { .. }
        | C::VideoSetOutPoint { .. }
        | C::VideoClearInOutPoints { .. }
        | C::VideoSetTransportSync { .. }
        // Deck Auto-Transitions
        | C::SetAutoTransitionEnabled { .. }
        | C::SetAutoTransitionTrigger { .. }
        | C::SetAutoTransitionPlayDuration { .. }
        | C::SetAutoTransitionDuration { .. }
        | C::SetAutoTransitionShader { .. }
        | C::ToggleAutoTransitionPlayDurationUnit { .. }
        | C::ToggleAutoTransitionDurationUnit { .. }
        | C::SetAutoTransitionPlayDurationValue { .. }
        | C::SetAutoTransitionDurationValue { .. }
        // External I/O Deck Sources
        | C::AddNdiDeck { .. }
        | C::AddSyphonDeck { .. }
        | C::AddSpoutDeck { .. }
        | C::AddSrtDeck { .. }
        | C::AddHlsDeck { .. }
        | C::AddDashDeck { .. }
        | C::AddRtmpDeck { .. }
        | C::AddHtmlDeck { .. }
        | C::ReloadHtmlDeck { .. }
        | C::OpenHtmlInteractive { .. }
        | C::CloseHtmlInteractive
        // Transition Sequences
        | C::CreateSequence
        | C::DeleteSequence { .. }
        | C::PlaySequence { .. }
        | C::StopSequence { .. }
        | C::ToggleSequence { .. }
        | C::AddFadeStep { .. }
        | C::AddWaitStep { .. }
        | C::AddGoToStep { .. }
        | C::RemoveStep { .. }
        | C::SetStepDuration { .. }
        | C::SetStepEasing { .. }
        | C::SetStepTransitionShader { .. }
        | C::MoveStep { .. }
        | C::SetStepDurationUnit { .. }
        | C::SetStepFromCh { .. }
        | C::SetStepToCh { .. }
        | C::SetGoToTarget { .. }
        | C::ToggleStepDurationUnit { .. }
        | C::SetStepDurationValue { .. }
        | C::SetStepTargetAmount { .. }
        // Stream Library
        | C::AddStreamLibraryEntry { .. }
        | C::RemoveStreamLibraryEntry { .. }
        | C::AddHlsLibraryEntry { .. }
        | C::RemoveHlsLibraryEntry { .. }
        | C::AddDashLibraryEntry { .. }
        | C::RemoveDashLibraryEntry { .. }
        | C::AddRtmpLibraryEntry { .. }
        | C::RemoveRtmpLibraryEntry { .. }
        | C::AddHtmlLibraryEntry { .. }
        | C::RemoveHtmlLibraryEntry { .. }
        // Output
        | C::CreateOutput
        | C::CreateHeadlessOutput { .. }
        | C::CloseOutput { .. }
        | C::SetOutputDisplay { .. }
        | C::SetOutputTarget { .. }
        | C::StartOutput { .. }
        | C::StopOutput { .. }
        | C::SetCalibrationMode { .. }
        | C::SetWarpCorner { .. }
        | C::ResetWarp { .. }
        | C::SetWarpSubdivisions { .. }
        | C::SetWarpMeshPoint { .. }
        | C::SetWarpBound { .. }
        | C::ConvertWarpToBezier { .. }
        | C::MoveWarpAnchor { .. }
        | C::MoveWarpHandle { .. }
        | C::SetBezierCageSubdivisions { .. }
        | C::SetEdgeBlend { .. }
        | C::SetEdgeBlendMode { .. }
        | C::SetOutputRotation { .. }
        | C::SetOutputPresentation { .. }
        | C::SetOutputTonemap { .. }
        // Surfaces
        | C::AddSurface { .. }
        | C::AddPolygonSurface { .. }
        | C::AddCircleSurface { .. }
        | C::RemoveSurface { .. }
        | C::ReorderSurface { .. }
        | C::SetSurfaceSource { .. }
        | C::SetSurfaceOutputType { .. }
        | C::SetSurfaceContentMapping { .. }
        | C::RenameSurface { .. }
        | C::UpdateSurfaceVertices { .. }
        | C::DuplicateSurface { .. }
        | C::FlipSurfaceHorizontal { .. }
        | C::FlipSurfaceVertical { .. }
        | C::InsertSurfaceVertex { .. }
        | C::SetCircleRadius { .. }
        | C::SetCircleSides { .. }
        | C::ConvertSurfaceToPolygon { .. }
        | C::CombineSurfaces { .. }
        | C::MoveSurface { .. }
        | C::RotateSurface { .. }
        | C::ScaleSurface { .. }
        | C::UpdateSurfaceContourVertices { .. }
        | C::ConvertSurfaceEdge { .. }
        | C::MovePathAnchor { .. }
        | C::MovePathHandle { .. }
        | C::AddSurfaceHole { .. }
        | C::RemoveSurfaceHole { .. }
        | C::PunchSurfaceHole { .. }
        | C::AssignSurfaceToOutput { .. }
        | C::UnassignSurfaceFromOutput { .. }
        // Surface Auto-Detection
        | C::DetectFromImage { .. }
        | C::DetectFromSvg { .. }
        | C::DetectFromDxf { .. }
        | C::ConfirmDetectedContours { .. }
        | C::ImportSurfacesFromFile { .. }
        | C::GenerateDomeSlices { .. }
        | C::DetectFromCamera { .. }
        // Transport
        | C::TransportPlay
        | C::TransportStop
        | C::TransportLocate { .. }
        | C::SetTransportSource { .. }
        | C::SetTransportLoop { .. }
        | C::SetTimecodeRate { .. }
        | C::SetTimecodePreference { .. }
        | C::SetLtcInput { .. }
        | C::SetRecordArmed { .. }
        | C::TransportPrevCue
        | C::TransportNextCue
        | C::TriggerCue { .. }
        // Arrangement
        | C::AddLane { .. }
        | C::RemoveLane { .. }
        | C::AddRegion { .. }
        | C::UpdateRegion { .. }
        | C::RemoveRegion { .. }
        | C::SetLaneCollapsed { .. }
        | C::SetIdleBehaviour { .. }
        | C::RearmParam { .. }
        | C::RearmAll { .. }
        | C::AddCue { .. }
        | C::UpdateCue { .. }
        | C::RemoveCue { .. }
        // Modulation Updates
        | C::UpdateModulationTimebase { .. }
        | C::UpdateLfoFrequency { .. }
        | C::UpdateLfoWaveform { .. }
        | C::UpdateLfoPhase { .. }
        | C::UpdateLfoAmplitude { .. }
        | C::UpdateLfoBipolar { .. }
        | C::UpdateAudioSmoothing { .. }
        | C::UpdateAudioFreqRange { .. }
        | C::UpdateAudioFreqLow { .. }
        | C::UpdateAudioFreqHigh { .. }
        | C::UpdateAudioGain { .. }
        | C::UpdateAudioPreset { .. }
        | C::UpdateAudioMode { .. }
        | C::UpdateAudioSource { .. }
        | C::UpdateAudioNoiseGate { .. }
        | C::UpdateAdsrAttack { .. }
        | C::UpdateAdsrDecay { .. }
        | C::UpdateAdsrSustain { .. }
        | C::UpdateAdsrRelease { .. }
        | C::TriggerAdsr { .. }
        | C::ReleaseAdsr { .. }
        | C::UpdateStepSeqSteps { .. }
        | C::UpdateStepSeqRate { .. }
        | C::UpdateStepSeqInterpolation { .. }
        | C::UpdateStepSeqBipolar { .. }
        | C::SetStepSeqCount { .. }
        | C::UpdateStepSeqValue { .. }
        | C::AssignModOnMod { .. }
        | C::RemoveModOnMod { .. }
        // Macros
        | C::AddMacro { .. }
        | C::RemoveMacro { .. }
        | C::RenameMacro { .. }
        | C::SetMacroKind { .. }
        | C::SetMacroValue { .. }
        | C::AddMacroTarget { .. }
        | C::RemoveMacroTarget { .. }
        | C::UpdateMacroTarget { .. }
        | C::SetMacroButtonBehavior { .. }
        | C::SetMacroTriggers { .. }
        // Analyzers
        | C::RequestAnalyzer { .. }
        | C::ReleaseAnalyzer { .. }
        | C::AddAnalyzerModSource { .. }
        | C::UpdateAnalyzerSmoothing { .. }
        // Device Scanning
        | C::RescanNdi
        | C::RescanSyphon
        | C::RescanSpout
        | C::RescanCameras
        | C::RescanDepthSensors
        | C::RescanCaptureTargets
        | C::RequestScreenCapturePermission
        | C::RescanMidi
        | C::RescanAudio
        | C::ToggleAudioSource { .. }
        | C::SetMidiDeviceEnabled { .. }
        // MIDI Mappings
        | C::ClearMidiMappings
        | C::RemoveMidiMapping { .. }
        // Clock
        | C::SetClockPreference { .. }
        | C::SetManualBpm { .. }
        // Parameters
        | C::ResetGeneratorParamsToDefaults { .. }
        | C::RandomizeGeneratorParams { .. }
        | C::MutateGeneratorParams { .. }
        // Resolution
        | C::SetRenderResolution { .. }
        | C::SetDomemasterResolution { .. }
        | C::SetDomePreset { .. }
        | C::SetDomeGeometry { .. }
        | C::SetEditorPrefs { .. }
        // Frame pacing
        | C::SetTargetFps { .. }
        // Performance profiling
        | C::StartPerfProfile { .. }
        // Presets
        | C::LoadDeckPreset { .. }
        | C::LoadChannelPreset { .. }
        | C::SaveDeckPreset { .. }
        | C::SaveChannelPreset { .. }
        // Learn modes and notifications
        | C::MidiLearnToggle
        | C::MidiLearnSelect { .. }
        | C::KeyboardLearnToggle
        | C::KeyboardLearnSelect { .. }
        | C::KeyboardLearnBind { .. }
        | C::DismissNotification { .. }
        | C::NotifyInfo { .. }
        // Consumer views
        | C::SetPreviewChannels { .. }
        | C::AcquireDetectionCamera { .. }
        | C::ReleaseDetectionCamera
        // Persistence
        | C::SaveWorkspace
        | C::LoadWorkspace
        // History
        | C::Undo
        | C::Redo
        // System
        | C::Shutdown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::command_is_undoable;
    use crate::engine::EngineCommand as C;
    use crate::engine::types::{BlendMode, ParamValue, ScalingMode};
    use crate::engine::value::param::{DeckTarget, ParamAddress};

    fn headless_app() -> Option<super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        super::VardaApp::new(gpu, &crate::testing::headless_config()).ok()
    }

    /// Every gesture that writes a parameter reaches the recorder through
    /// `execute_command`, with no per-command wiring to forget.
    #[test]
    fn parameter_gestures_record_while_a_pass_is_armed() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel = app.mixer.channels()[0].uuid().to_string();
        let solid = app
            .add_solid_color_deck(&channel, [1.0, 0.0, 0.0, 1.0])
            .expect("solid deck");
        let shader = app
            .sources
            .registry
            .generators()
            .iter()
            .map(|s| (*s).clone())
            .find_map(|shader| {
                let deck = crate::deck::Deck::new(&app.render.context, shader, 64, 64).ok()?;
                let ranged = deck
                    .generator_params
                    .values
                    .iter()
                    .find(|(name, value)| deck.generator_params.normalize(name, value).is_some())
                    .map(|(name, value)| (name.clone(), *value))?;
                Some((deck, ranged))
            });
        let (shader_deck, (param, param_value)) =
            shader.expect("a generator with a ranged parameter");
        let shader_uuid = shader_deck.uuid().to_string();
        app.mixer.channels_mut()[0].add_deck(shader_deck);

        app.set_record_armed(true);
        let gestures = [
            (
                C::SetDeckOpacity {
                    deck_uuid: solid.clone(),
                    opacity: 0.4,
                },
                ParamAddress::deck(&solid, DeckTarget::Opacity).to_string(),
            ),
            (
                C::SetChannelOpacity {
                    channel_uuid: channel.clone(),
                    opacity: 0.6,
                },
                ParamAddress::channel_opacity(&channel).to_string(),
            ),
            (
                C::SetDeckScalingMode {
                    deck_uuid: solid.clone(),
                    mode: ScalingMode::Fit,
                },
                ParamAddress::deck(&solid, DeckTarget::ScalingMode).to_string(),
            ),
            (
                C::SetGeneratorParam {
                    deck_uuid: shader_uuid.clone(),
                    name: param.clone(),
                    value: param_value,
                },
                ParamAddress::deck_param(&shader_uuid, &param).to_string(),
            ),
            (
                C::SetParam {
                    path: format!("deck/{shader_uuid}/opacity"),
                    value: ParamValue::Float(0.5),
                },
                ParamAddress::deck(&shader_uuid, DeckTarget::Opacity).to_string(),
            ),
        ];
        for (cmd, key) in gestures {
            let label = format!("{cmd:?}");
            assert!(
                !matches!(
                    app.execute_command(cmd),
                    crate::engine::CommandResult::Err { .. }
                ),
                "{label} failed"
            );
            assert!(
                app.show.recorder.recording_params().contains(&key),
                "{label} did not reach the recorder as {key}"
            );
        }
    }

    #[test]
    fn failed_and_structural_commands_do_not_record() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel = app.mixer.channels()[0].uuid().to_string();
        let solid = app
            .add_solid_color_deck(&channel, [1.0, 0.0, 0.0, 1.0])
            .expect("solid deck");
        app.set_record_armed(true);
        app.execute_command(C::SetDeckOpacity {
            deck_uuid: "nosuchdk".to_string(),
            opacity: 0.4,
        });
        app.execute_command(C::SetDeckBlendMode {
            deck_uuid: solid,
            mode: BlendMode::Add,
        });
        assert!(app.show.recorder.recording_params().is_empty());
    }

    #[test]
    fn authoring_commands_are_undoable() {
        assert!(command_is_undoable(&C::AddChannel));
        assert!(command_is_undoable(&C::RemoveChannel {
            channel_uuid: "ch".into(),
        }));
        assert!(command_is_undoable(&C::SetChannelOpacity {
            channel_uuid: "ch".into(),
            opacity: 0.5,
        }));
        assert!(command_is_undoable(&C::SetParam {
            path: "deck/abc/opacity".into(),
            value: crate::engine::ParamValue::Float(0.5),
        }));
        assert!(command_is_undoable(&C::RemoveSurface { uuid: "s".into() }));
        // Surface→output assignment is authoring and must be undoable.
        assert!(command_is_undoable(&C::AssignSurfaceToOutput {
            output_uuid: "o".into(),
            surface_uuid: "s".into(),
        }));
    }

    #[test]
    fn live_and_transient_commands_are_not_undoable() {
        assert!(!command_is_undoable(&C::SetCrossfader(0.5)));
        assert!(!command_is_undoable(&C::VideoTogglePlay {
            deck_uuid: "dk".into(),
        }));
        assert!(!command_is_undoable(&C::VideoSetTransportSync {
            deck_uuid: "dk".into(),
            sync: crate::video::DeckTransportSync::default(),
        }));
        assert!(!command_is_undoable(&C::PlaySequence {
            sequence_uuid: "sq".into(),
        }));
        assert!(!command_is_undoable(&C::StartOutput {
            output_uuid: "o".into(),
        }));
        assert!(!command_is_undoable(&C::CreateOutput));
        assert!(!command_is_undoable(&C::Undo));
        assert!(!command_is_undoable(&C::Redo));
        assert!(!command_is_undoable(&C::SaveWorkspace));
        assert!(!command_is_undoable(&C::Shutdown));
        // Audio device lifecycle, detection previews, and global settings are
        // all live/transient — excluded from the undo timeline.
        assert!(!command_is_undoable(&C::ScanAudioDevices));
        assert!(!command_is_undoable(&C::RescanAudio));
        assert!(!command_is_undoable(&C::DetectFromImage {
            image_data: Vec::new(),
            params: crate::engine::value::detect::DetectionParams::default(),
        }));
        assert!(!command_is_undoable(&C::SetRenderResolution {
            width: 1280,
            height: 720,
        }));
        assert!(!command_is_undoable(&C::SetDomemasterResolution {
            resolution: crate::renderer::dome::DomemasterResolution::R4K,
        }));
        assert!(!command_is_undoable(&C::SetTargetFps { fps: 30 }));
        // Saving a preset writes to disk; it is not undoable (loading is).
        assert!(!command_is_undoable(&C::SaveChannelPreset {
            channel_uuid: "ch".into(),
            name: "p".into(),
        }));
    }
}
