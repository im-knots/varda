//! Cross-thread command dispatch for `VardaApp`.
//!
//! Houses `execute_command`, the exhaustive match over every `EngineCommand`
//! variant that cross-thread consumers (HTTP API, WebSocket, CLI) drive through
//! the command channel.

use super::VardaApp;
use crate::engine::{CommandResult, EngineCommand, ErrorCode};

impl VardaApp {
    /// Execute a single command and return the result.
    pub(crate) fn execute_command(&mut self, cmd: EngineCommand) -> CommandResult {
        use crate::engine::traits::*;
        use crate::modulation::ModulationSource;
        match cmd {
            // ── Mixer ────────────────────────────────────────
            EngineCommand::SetCrossfader(pos) => {
                self.set_crossfader(pos);
                CommandResult::Ok
            }
            EngineCommand::SetTonemapMode(mode) => {
                self.set_tonemap_mode(mode);
                CommandResult::Ok
            }
            EngineCommand::LoadLut { filename } => match self.load_lut(&filename) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::UnloadLut => {
                self.unload_lut();
                CommandResult::Ok
            }
            EngineCommand::AutoCrossfade {
                target,
                duration_secs,
                easing,
            } => {
                self.start_auto_crossfade(target, duration_secs, easing);
                CommandResult::Ok
            }
            EngineCommand::BeatCrossfade { target, beats } => {
                self.start_beat_crossfade(target, beats);
                CommandResult::Ok
            }
            EngineCommand::AddDeck {
                channel_idx,
                shader_name,
            } => match self.add_deck(channel_idx, &shader_name) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::AddImageDeck { channel_idx, path } => {
                match self.add_image_deck(channel_idx, &path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddVideoDeck { channel_idx, path } => {
                match self.add_video_deck(channel_idx, &path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddSolidColorDeck { channel_idx, color } => {
                match self.add_solid_color_deck(channel_idx, color) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddCameraDeck {
                channel_idx,
                camera_id,
            } => match self.add_camera_deck(channel_idx, camera_id) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::RemoveDeck {
                channel_idx,
                deck_idx,
            } => match self.remove_deck(channel_idx, deck_idx) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: e.to_string(),
                },
            },
            EngineCommand::MoveDeck {
                src_ch,
                src_deck,
                dst_ch,
            } => match self.move_deck(src_ch, src_deck, dst_ch) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::ReorderDeck {
                ch,
                from_idx,
                to_idx,
            } => {
                self.reorder_deck(ch, from_idx, to_idx);
                CommandResult::Ok
            }
            EngineCommand::SetDeckOpacity {
                channel_idx,
                deck_idx,
                opacity,
            } => {
                self.set_deck_opacity(channel_idx, deck_idx, opacity);
                CommandResult::Ok
            }
            EngineCommand::SetDeckBlendMode {
                channel_idx,
                deck_idx,
                mode,
            } => {
                self.set_deck_blend_mode(channel_idx, deck_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::SetDeckSolo {
                channel_idx,
                deck_idx,
                solo,
            } => {
                self.set_deck_solo(channel_idx, deck_idx, solo);
                CommandResult::Ok
            }
            EngineCommand::SetDeckMute {
                channel_idx,
                deck_idx,
                mute,
            } => {
                self.set_deck_mute(channel_idx, deck_idx, mute);
                CommandResult::Ok
            }
            EngineCommand::SetDeckRenderFps {
                channel_idx,
                deck_idx,
                render_fps,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if let Some(slot) = ch.decks.get_mut(deck_idx) {
                        slot.render_fps = render_fps;
                        CommandResult::Ok
                    } else {
                        CommandResult::Err {
                            code: ErrorCode::NotFound,
                            message: format!("Deck {} not found", deck_idx),
                        }
                    }
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("Channel {} not found", channel_idx),
                    }
                }
            }
            EngineCommand::SetDeckScalingMode {
                channel_idx,
                deck_idx,
                mode,
            } => {
                self.set_deck_scaling_mode(channel_idx, deck_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::SetDeckTransparent {
                channel_idx,
                deck_idx,
                transparent,
            } => {
                self.set_deck_transparent(channel_idx, deck_idx, transparent);
                CommandResult::Ok
            }
            EngineCommand::SetChannelOpacity {
                channel_idx,
                opacity,
            } => {
                self.set_channel_opacity(channel_idx, opacity);
                CommandResult::Ok
            }
            EngineCommand::SetChannelBlendMode { channel_idx, mode } => {
                self.set_channel_blend_mode(channel_idx, mode);
                CommandResult::Ok
            }
            EngineCommand::AddChannel => match self.add_channel() {
                Ok(_idx) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::RemoveChannel { channel_idx } => {
                match self.remove_channel(channel_idx) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::AddEffect {
                target,
                shader_name,
            } => match self.add_effect(target, &shader_name) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::RemoveEffect { target, effect_idx } => {
                self.remove_effect(target, effect_idx);
                CommandResult::Ok
            }
            EngineCommand::ToggleEffect { target, effect_idx } => {
                self.toggle_effect(target, effect_idx);
                CommandResult::Ok
            }
            EngineCommand::MoveEffect {
                target,
                from_idx,
                to_idx,
            } => {
                self.move_effect(target, from_idx, to_idx);
                CommandResult::Ok
            }
            EngineCommand::SetTransition { shader_name } => {
                match self.set_transition(shader_name.as_deref()) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::SetParam { path, value } => {
                self.set_param(&path, value);
                CommandResult::Ok
            }

            // ── Audio ────────────────────────────────────────
            EngineCommand::OpenAudioSource { source_id } => {
                match self.open_audio_source(source_id) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::CloseAudioSource { source_id } => {
                self.close_audio_source(source_id);
                CommandResult::Ok
            }
            EngineCommand::ScanAudioDevices => {
                self.scan_audio_devices();
                CommandResult::Ok
            }

            // ── Modulation ───────────────────────────────────
            EngineCommand::AddLfo {
                waveform,
                frequency,
            } => {
                self.add_lfo(waveform, frequency);
                CommandResult::Ok
            }
            EngineCommand::AddAudioBand { preset, source_id } => {
                // Capture is reconciled per-frame from modulator demand
                // (see /spec/audio-capture-lifecycle.md); adding the band is enough.
                self.add_audio_band(preset, source_id);
                CommandResult::Ok
            }
            EngineCommand::AddAdsr {
                attack,
                decay,
                sustain,
                release,
            } => {
                self.add_adsr(attack, decay, sustain, release);
                CommandResult::Ok
            }
            EngineCommand::AddStepSequencer { num_steps, rate } => {
                self.add_step_sequencer(num_steps, rate);
                CommandResult::Ok
            }
            EngineCommand::RemoveModulationSource { uuid } => {
                self.remove_modulation_source(&uuid);
                CommandResult::Ok
            }
            EngineCommand::AssignModulation {
                target,
                source_id,
                amount,
            } => {
                self.assign_modulation(&target, &source_id, amount);
                CommandResult::Ok
            }
            EngineCommand::ClearModulation { target } => {
                self.clear_modulation(&target);
                CommandResult::Ok
            }
            EngineCommand::ClearModulationSource { target, source_id } => {
                self.clear_modulation_source(&target, &source_id);
                CommandResult::Ok
            }

            // ── Output ───────────────────────────────────────
            EngineCommand::CreateOutput => {
                self.request_create_output();
                CommandResult::Ok
            }
            EngineCommand::CloseOutput { idx } => {
                self.close_output(idx);
                CommandResult::Ok
            }
            EngineCommand::SetOutputDisplay { idx, monitor_name } => {
                self.set_output_display(idx, &monitor_name);
                CommandResult::Ok
            }
            EngineCommand::SetOutputTarget { idx, target } => {
                self.cmd_set_output_target(idx, target)
            }

            // ── Surfaces ────────────────────────────────────
            EngineCommand::AddSurface { name, source } => {
                self.add_surface(&name, source);
                CommandResult::Ok
            }
            EngineCommand::AddPolygonSurface {
                name,
                vertices,
                source,
            } => {
                self.add_polygon_surface(&name, &vertices, source);
                CommandResult::Ok
            }
            EngineCommand::AddCircleSurface {
                name,
                center,
                radius,
                sides,
                aspect_ratio,
                source,
            } => {
                self.add_circle_surface(&name, center, radius, sides, aspect_ratio, source);
                CommandResult::Ok
            }
            EngineCommand::RemoveSurface { uuid } => self.cmd_remove_surface(&uuid),
            EngineCommand::ReorderSurface { uuid, op } => self.cmd_reorder_surface(&uuid, op),
            EngineCommand::SetSurfaceSource { uuid, source } => {
                self.set_surface_source(&uuid, source);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::SetSurfaceOutputType { uuid, output_type } => {
                self.set_surface_output_type(&uuid, output_type);
                CommandResult::Ok
            }
            EngineCommand::SetSurfaceContentMapping { uuid, mapping } => {
                self.set_surface_content_mapping(&uuid, mapping);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::RenameSurface { uuid, name } => {
                self.rename_surface(&uuid, &name);
                CommandResult::Ok
            }
            EngineCommand::UpdateSurfaceVertices { uuid, vertices } => {
                self.cmd_update_surface_vertices(&uuid, vertices)
            }
            EngineCommand::DuplicateSurface { uuid } => self.cmd_duplicate_surface(&uuid),
            EngineCommand::FlipSurfaceHorizontal { uuid } => {
                self.cmd_flip_surface_horizontal(&uuid)
            }
            EngineCommand::FlipSurfaceVertical { uuid } => self.cmd_flip_surface_vertical(&uuid),
            EngineCommand::InsertSurfaceVertex {
                uuid,
                after_vert_idx,
                position,
            } => self.cmd_insert_surface_vertex(&uuid, after_vert_idx, position),
            EngineCommand::SetCircleRadius { uuid, radius } => {
                self.cmd_set_circle_radius(&uuid, radius)
            }
            EngineCommand::SetCircleSides { uuid, sides } => {
                self.cmd_set_circle_sides(&uuid, sides)
            }
            EngineCommand::ConvertSurfaceToPolygon { uuid } => {
                self.cmd_convert_surface_to_polygon(&uuid)
            }
            EngineCommand::CombineSurfaces { uuids } => self.cmd_combine_surfaces(&uuids),
            EngineCommand::MoveSurface { uuid, dx, dy } => self.cmd_move_surface(&uuid, dx, dy),
            EngineCommand::RotateSurface { uuid, angle, pivot } => {
                self.cmd_rotate_surface(&uuid, angle, pivot)
            }
            EngineCommand::ScaleSurface {
                uuid,
                sx,
                sy,
                pivot,
            } => self.cmd_scale_surface(&uuid, sx, sy, pivot),
            EngineCommand::UpdateSurfaceContourVertices {
                uuid,
                contour,
                vertices,
            } => self.cmd_update_surface_contour_vertices(&uuid, contour, vertices),
            EngineCommand::ConvertSurfaceEdge {
                uuid,
                edge_idx,
                to_cubic,
            } => self.cmd_convert_surface_edge(&uuid, edge_idx, to_cubic),
            EngineCommand::MovePathAnchor {
                uuid,
                anchor_idx,
                pos,
            } => self.cmd_move_path_anchor(&uuid, anchor_idx, pos),
            EngineCommand::MovePathHandle {
                uuid,
                segment_idx,
                handle,
                pos,
            } => self.cmd_move_path_handle(&uuid, segment_idx, handle, pos),
            EngineCommand::AddSurfaceHole { uuid, hole } => self.cmd_add_surface_hole(&uuid, hole),
            EngineCommand::RemoveSurfaceHole { uuid, hole_index } => {
                self.cmd_remove_surface_hole(&uuid, hole_index)
            }
            EngineCommand::PunchSurfaceHole { source_uuid } => {
                self.cmd_punch_surface_hole(&source_uuid)
            }
            EngineCommand::AssignSurfaceToOutput {
                output_uuid,
                surface_uuid,
            } => {
                self.assign_surface_to_output(&output_uuid, &surface_uuid);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::UnassignSurfaceFromOutput {
                output_uuid,
                assignment_idx,
            } => {
                self.unassign_surface_from_output(&output_uuid, assignment_idx);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::AssignSurfaceToOutputByIdx {
                output_idx,
                surface_uuid,
            } => self.cmd_assign_surface_to_output_by_idx(output_idx, &surface_uuid),
            EngineCommand::UnassignSurfaceFromOutputByIdx {
                output_idx,
                assignment_idx,
            } => self.cmd_unassign_surface_from_output_by_idx(output_idx, assignment_idx),

            // ── Surface Auto-Detection ────────────────────────
            EngineCommand::DetectFromImage { image_data, params } => {
                match self.detect_from_image(&image_data, &params) {
                    Ok(result) => CommandResult::OkWithData {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    },
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::DetectFromSvg { svg_data } => match self.detect_from_svg(&svg_data) {
                Ok(result) => CommandResult::OkWithData {
                    data: serde_json::to_value(&result).unwrap_or_default(),
                },
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::DetectFromDxf { dxf_data } => match self.detect_from_dxf(&dxf_data) {
                Ok(result) => CommandResult::OkWithData {
                    data: serde_json::to_value(&result).unwrap_or_default(),
                },
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::ConfirmDetectedContours { contours } => {
                let uuids = self.confirm_detected_contours(&contours);
                CommandResult::OkWithData {
                    data: serde_json::json!({ "surface_uuids": uuids }),
                }
            }
            EngineCommand::DetectFromCamera { camera_id, params } => {
                match self.detect_from_camera(camera_id, &params) {
                    Ok(result) => {
                        let uuids = self.confirm_detected_contours(&result.contours);
                        CommandResult::OkWithData {
                            data: serde_json::json!({ "surface_uuids": uuids, "contours_found": result.contours.len() }),
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }

            // ── Video Playback ────────────────────────────────
            EngineCommand::VideoTogglePlay {
                channel_idx,
                deck_idx,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() && ch.decks[deck_idx].deck.video_toggle_play() {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSeek {
                channel_idx,
                deck_idx,
                position_secs,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_seek(position_secs)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found".into(),
                }
            }
            EngineCommand::VideoSetSpeed {
                channel_idx,
                deck_idx,
                speed,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() && ch.decks[deck_idx].deck.video_set_speed(speed) {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSetLoopMode {
                channel_idx,
                deck_idx,
                mode,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_set_loop_mode(mode)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSetInPoint {
                channel_idx,
                deck_idx,
                secs,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() && ch.decks[deck_idx].deck.video_set_in_point(secs)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoSetOutPoint {
                channel_idx,
                deck_idx,
                secs,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_set_out_point(secs)
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }
            EngineCommand::VideoClearInOutPoints {
                channel_idx,
                deck_idx,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len()
                        && ch.decks[deck_idx].deck.video_clear_in_out_points()
                    {
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found or not a video".into(),
                }
            }

            // ── Deck Auto-Transitions ─────────────────────────
            EngineCommand::SetAutoTransitionEnabled {
                channel_idx,
                deck_idx,
                enabled,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.enabled = enabled;
                if !enabled {
                    at.phase = crate::channel::DeckTransitionPhase::Inactive;
                }
            }),
            EngineCommand::SetAutoTransitionTrigger {
                channel_idx,
                deck_idx,
                clip_end,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.trigger = if clip_end {
                    crate::channel::TransitionTrigger::ClipEnd
                } else {
                    crate::channel::TransitionTrigger::Timer
                };
            }),
            EngineCommand::SetAutoTransitionPlayDuration {
                channel_idx,
                deck_idx,
                value,
                unit,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.play_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
            }),
            EngineCommand::SetAutoTransitionDuration {
                channel_idx,
                deck_idx,
                value,
                unit,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.transition_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
            }),
            EngineCommand::SetAutoTransitionShader {
                channel_idx,
                deck_idx,
                shader_name,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        let slot = &mut ch.decks[deck_idx];
                        if slot.auto_transition.is_none() {
                            slot.auto_transition = Some(crate::channel::DeckAutoTransition::new());
                        }
                        if let Some(at) = slot.auto_transition.as_mut() {
                            at.transition_shader_name = shader_name.clone();
                        }
                        if let Some(shader_name) = &shader_name {
                            if let Some(shader) = self
                                .registry
                                .transitions()
                                .iter()
                                .find(|s| s.name() == *shader_name)
                            {
                                let _ =
                                    slot.set_transition_shader(&self.context, (*shader).clone());
                            }
                        } else {
                            slot.transition_effect = None;
                        }
                        return CommandResult::Ok;
                    }
                }
                CommandResult::Err {
                    code: ErrorCode::NotFound,
                    message: "Deck not found".into(),
                }
            }
            EngineCommand::ToggleAutoTransitionPlayDurationUnit {
                channel_idx,
                deck_idx,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                let next_unit = at.play_duration.unit().next();
                at.play_duration = crate::channel::DurationSpec::from_value_unit(
                    at.play_duration.value(),
                    next_unit,
                );
            }),
            EngineCommand::ToggleAutoTransitionDurationUnit {
                channel_idx,
                deck_idx,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                let next_unit = at.transition_duration.unit().next();
                at.transition_duration = crate::channel::DurationSpec::from_value_unit(
                    at.transition_duration.value(),
                    next_unit,
                );
            }),
            EngineCommand::SetAutoTransitionPlayDurationValue {
                channel_idx,
                deck_idx,
                value,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.play_duration.set_value(value);
            }),
            EngineCommand::SetAutoTransitionDurationValue {
                channel_idx,
                deck_idx,
                value,
            } => self.exec_auto_transition(channel_idx, deck_idx, |at| {
                at.transition_duration.set_value(value);
            }),

            // ── External I/O Deck Sources ─────────────────────
            EngineCommand::AddNdiDeck {
                channel_idx,
                source_name,
            } => self.cmd_add_ndi_deck(channel_idx, source_name),
            EngineCommand::AddSyphonDeck {
                channel_idx,
                server_name,
            } => self.cmd_add_syphon_deck(channel_idx, server_name),
            EngineCommand::AddSrtDeck {
                channel_idx,
                url,
                mode,
            } => self.cmd_add_srt_deck(channel_idx, url, mode),
            EngineCommand::AddHlsDeck { channel_idx, url } => {
                self.cmd_add_hls_deck(channel_idx, url)
            }
            EngineCommand::AddDashDeck { channel_idx, url } => {
                self.cmd_add_dash_deck(channel_idx, url)
            }
            EngineCommand::AddRtmpDeck {
                channel_idx,
                url,
                mode,
            } => self.cmd_add_rtmp_deck(channel_idx, url, mode),
            EngineCommand::ReloadHtmlDeck {
                channel_idx,
                deck_idx,
            } => self.cmd_reload_html_deck(channel_idx, deck_idx),
            EngineCommand::AddHtmlDeck { channel_idx, url } => {
                self.cmd_add_html_deck(channel_idx, url)
            }
            EngineCommand::OpenHtmlInteractive {
                channel_idx,
                deck_idx,
            } => {
                #[cfg(feature = "html")]
                {
                    self.cmd_open_html_interactive(channel_idx, deck_idx)
                }
                #[cfg(not(feature = "html"))]
                {
                    let _ = (channel_idx, deck_idx);
                    crate::engine::CommandResult::Err {
                        code: crate::engine::ErrorCode::InvalidInput,
                        message: "HTML feature not built".into(),
                    }
                }
            }
            EngineCommand::CloseHtmlInteractive => {
                #[cfg(feature = "html")]
                {
                    self.cmd_close_html_interactive()
                }
                #[cfg(not(feature = "html"))]
                {
                    crate::engine::CommandResult::Ok
                }
            }

            // ── Transition Sequences ──────────────────────────
            EngineCommand::CreateSequence => self.cmd_create_sequence(),
            EngineCommand::DeleteSequence { idx } => self.cmd_delete_sequence(idx),
            EngineCommand::PlaySequence { idx } => self.cmd_play_sequence(idx),
            EngineCommand::StopSequence { idx } => self.cmd_stop_sequence(idx),
            EngineCommand::ToggleSequence { idx } => self.cmd_toggle_sequence(idx),
            EngineCommand::AddFadeStep {
                seq_idx,
                from_ch,
                to_ch,
            } => self.cmd_add_fade_step(seq_idx, from_ch, to_ch),
            EngineCommand::AddWaitStep { seq_idx } => self.cmd_add_wait_step(seq_idx),
            EngineCommand::AddGoToStep {
                seq_idx,
                step_index,
            } => self.cmd_add_goto_step(seq_idx, step_index),
            EngineCommand::RemoveStep { seq_idx, step_idx } => {
                self.cmd_remove_step(seq_idx, step_idx)
            }
            EngineCommand::SetStepDuration {
                seq_idx,
                step_idx,
                value,
                unit,
            } => self.cmd_set_step_duration(seq_idx, step_idx, value, unit),
            EngineCommand::SetStepEasing {
                seq_idx,
                step_idx,
                easing,
            } => self.cmd_set_step_easing(seq_idx, step_idx, easing),
            EngineCommand::SetStepTransitionShader {
                seq_idx,
                step_idx,
                shader_name,
            } => self.cmd_set_step_transition_shader(seq_idx, step_idx, shader_name),
            EngineCommand::MoveStep { seq_idx, from, to } => self.cmd_move_step(seq_idx, from, to),
            EngineCommand::SetStepDurationUnit {
                seq_idx,
                step_idx,
                unit,
            } => self.cmd_set_step_duration_unit(seq_idx, step_idx, unit),
            EngineCommand::ToggleStepDurationUnit { seq_idx, step_idx } => {
                self.cmd_toggle_step_duration_unit(seq_idx, step_idx)
            }
            EngineCommand::SetStepDurationValue {
                seq_idx,
                step_idx,
                value,
            } => self.cmd_set_step_duration_value(seq_idx, step_idx, value),
            EngineCommand::SetStepFromCh {
                seq_idx,
                step_idx,
                ch,
            } => self.cmd_set_step_from_ch(seq_idx, step_idx, ch),
            EngineCommand::SetStepToCh {
                seq_idx,
                step_idx,
                ch,
            } => self.cmd_set_step_to_ch(seq_idx, step_idx, ch),
            EngineCommand::SetGoToTarget {
                seq_idx,
                step_idx,
                target,
            } => self.cmd_set_goto_target(seq_idx, step_idx, target),
            EngineCommand::SetStepTargetAmount {
                seq_idx,
                step_idx,
                amount,
            } => self.cmd_set_step_target_amount(seq_idx, step_idx, amount),

            // ── Stream Library ─────────────────────────────────
            EngineCommand::AddStreamLibraryEntry { url, mode } => {
                self.cmd_add_stream_library_entry(url, mode)
            }
            EngineCommand::RemoveStreamLibraryEntry { url } => {
                self.cmd_remove_stream_library_entry(url)
            }
            EngineCommand::AddHlsLibraryEntry { url } => self.cmd_add_hls_library_entry(url),
            EngineCommand::RemoveHlsLibraryEntry { url } => self.cmd_remove_hls_library_entry(url),
            EngineCommand::AddDashLibraryEntry { url } => self.cmd_add_dash_library_entry(url),
            EngineCommand::RemoveDashLibraryEntry { url } => {
                self.cmd_remove_dash_library_entry(url)
            }
            EngineCommand::AddRtmpLibraryEntry { url, mode } => {
                self.cmd_add_rtmp_library_entry(url, mode)
            }
            EngineCommand::RemoveRtmpLibraryEntry { url } => {
                self.cmd_remove_rtmp_library_entry(url)
            }

            // ── Output Management ─────────────────────────────────
            EngineCommand::CreateHeadlessOutput { target } => {
                self.cmd_create_headless_output(target)
            }
            EngineCommand::StartOutput { idx } => self.cmd_start_output(idx),
            EngineCommand::StopOutput { idx } => self.cmd_stop_output(idx),
            EngineCommand::SetCalibrationMode { idx, mode } => {
                self.cmd_set_calibration_mode(idx, mode)
            }
            EngineCommand::SetWarpCorner {
                surface_uuid,
                corner_idx,
                position,
            } => self.cmd_set_warp_corner(&surface_uuid, corner_idx, position),
            EngineCommand::ResetWarp { surface_uuid } => self.cmd_reset_warp(&surface_uuid),
            EngineCommand::SetWarpSubdivisions {
                surface_uuid,
                cols,
                rows,
            } => self.cmd_set_warp_subdivisions(&surface_uuid, cols, rows),
            EngineCommand::SetWarpMeshPoint {
                surface_uuid,
                row,
                col,
                position,
            } => self.cmd_set_warp_mesh_point(&surface_uuid, row, col, position),
            EngineCommand::SetWarpBound {
                surface_uuid,
                bound,
            } => self.cmd_set_warp_bound(&surface_uuid, bound),
            EngineCommand::ConvertWarpToBezier { surface_uuid } => {
                self.cmd_convert_warp_to_bezier(&surface_uuid)
            }
            EngineCommand::MoveWarpAnchor {
                surface_uuid,
                row,
                col,
                position,
            } => self.cmd_move_warp_anchor(&surface_uuid, row, col, position),
            EngineCommand::MoveWarpHandle {
                surface_uuid,
                horizontal,
                row,
                col,
                which,
                position,
            } => self.cmd_move_warp_handle(&surface_uuid, horizontal, row, col, which, position),
            EngineCommand::SetBezierCageSubdivisions {
                surface_uuid,
                cols,
                rows,
            } => self.cmd_set_bezier_cage_subdivisions(&surface_uuid, cols, rows),
            EngineCommand::SetEdgeBlend { output_idx, config } => {
                self.cmd_set_edge_blend(output_idx, config)
            }
            EngineCommand::SetEdgeBlendMode { output_idx, mode } => {
                self.cmd_set_edge_blend_mode(output_idx, mode)
            }
            EngineCommand::SetOutputRotation { idx, rotation } => {
                self.cmd_set_output_rotation(idx, rotation)
            }

            // ── Modulation Updates ────────────────────────────────
            EngineCommand::UpdateLfoFrequency { uuid, frequency } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        frequency: ref mut f,
                        ..
                    } = s
                    {
                        *f = frequency;
                    }
                })
            }
            EngineCommand::UpdateLfoWaveform { uuid, waveform } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        waveform: ref mut w,
                        ..
                    } = s
                    {
                        *w = waveform;
                    }
                })
            }
            EngineCommand::UpdateLfoPhase { uuid, phase } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        phase: ref mut p, ..
                    } = s
                    {
                        *p = phase;
                    }
                })
            }
            EngineCommand::UpdateLfoAmplitude { uuid, amplitude } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        amplitude: ref mut a,
                        ..
                    } = s
                    {
                        *a = amplitude;
                    }
                })
            }
            EngineCommand::UpdateLfoBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO {
                        bipolar: ref mut b, ..
                    } = s
                    {
                        *b = bipolar;
                    }
                })
            }
            EngineCommand::UpdateAudioSmoothing { uuid, smoothing } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        smoothing: ref mut sm,
                        ..
                    } = s
                    {
                        *sm = smoothing;
                    }
                })
            }
            EngineCommand::UpdateAudioFreqRange {
                uuid,
                freq_low,
                freq_high,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::AudioBand {
                    freq_low: ref mut fl,
                    freq_high: ref mut fh,
                    ..
                } = s
                {
                    *fl = freq_low;
                    *fh = freq_high;
                }
            }),
            EngineCommand::UpdateAudioGain { uuid, gain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        gain: ref mut g, ..
                    } = s
                    {
                        *g = gain;
                    }
                })
            }
            EngineCommand::UpdateAudioPreset { uuid, preset } => {
                let (lo, hi) = preset.freq_range();
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_low: ref mut fl,
                        freq_high: ref mut fh,
                        ..
                    } = s
                    {
                        *fl = lo;
                        *fh = hi;
                    }
                })
            }
            EngineCommand::UpdateAudioMode { uuid, mode } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        mode: ref mut m, ..
                    } = s
                    {
                        *m = mode;
                    }
                })
            }
            EngineCommand::UpdateAdsrAttack { uuid, attack } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        attack: ref mut a, ..
                    } = s
                    {
                        *a = attack;
                    }
                })
            }
            EngineCommand::UpdateAdsrDecay { uuid, decay } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        decay: ref mut d, ..
                    } = s
                    {
                        *d = decay;
                    }
                })
            }
            EngineCommand::UpdateAdsrSustain { uuid, sustain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        sustain: ref mut su,
                        ..
                    } = s
                    {
                        *su = sustain;
                    }
                })
            }
            EngineCommand::UpdateAdsrRelease { uuid, release } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR {
                        release: ref mut r, ..
                    } = s
                    {
                        *r = release;
                    }
                })
            }
            EngineCommand::TriggerAdsr { uuid } => {
                self.mixer.modulation_mut().trigger_adsr(&uuid);
                CommandResult::Ok
            }
            EngineCommand::ReleaseAdsr { uuid } => {
                self.mixer.modulation_mut().release_adsr(&uuid);
                CommandResult::Ok
            }
            EngineCommand::UpdateStepSeqSteps { uuid, steps } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer {
                        steps: ref mut st, ..
                    } = s
                    {
                        *st = steps;
                    }
                })
            }
            EngineCommand::UpdateStepSeqRate { uuid, rate } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer {
                        rate: ref mut r, ..
                    } = s
                    {
                        *r = rate;
                    }
                })
            }
            EngineCommand::UpdateStepSeqInterpolation {
                uuid,
                interpolation,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::StepSequencer {
                    interpolation: ref mut i,
                    ..
                } = s
                {
                    *i = interpolation;
                }
            }),
            EngineCommand::UpdateStepSeqBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer {
                        bipolar: ref mut b, ..
                    } = s
                    {
                        *b = bipolar;
                    }
                })
            }
            EngineCommand::SetStepSeqCount { uuid, count } => {
                let count = count.clamp(2, 64);
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { steps, .. } = s {
                        steps.resize(count, 0.0);
                    }
                })
            }
            EngineCommand::UpdateStepSeqValue {
                uuid,
                step_idx,
                value,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::StepSequencer { steps, .. } = s {
                    if step_idx < steps.len() {
                        steps[step_idx] = value;
                    }
                }
            }),
            EngineCommand::UpdateAudioFreqLow { uuid, freq_low } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_low: ref mut fl,
                        ..
                    } = s
                    {
                        *fl = freq_low;
                    }
                })
            }
            EngineCommand::UpdateAudioFreqHigh { uuid, freq_high } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_high: ref mut fh,
                        ..
                    } = s
                    {
                        *fh = freq_high;
                    }
                })
            }
            EngineCommand::UpdateAudioSource { uuid, source_id } => {
                // Switching device just updates the modulator; the per-frame
                // reconcile opens the new device and closes the old one when it is
                // no longer referenced (see /spec/audio-capture-lifecycle.md).
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        source_id: ref mut sid,
                        ..
                    } = s
                    {
                        *sid = source_id;
                    }
                })
            }
            EngineCommand::UpdateAudioNoiseGate { uuid, noise_gate } => self
                .exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        noise_gate: ref mut ng,
                        ..
                    } = s
                    {
                        *ng = noise_gate;
                    }
                }),
            EngineCommand::AssignModOnMod {
                target_source_id,
                param_name,
                modulator_id,
                amount,
            } => {
                self.mixer.modulation_mut().assign_mod_on_mod(
                    &target_source_id,
                    &param_name,
                    &modulator_id,
                    amount,
                );
                CommandResult::Ok
            }
            EngineCommand::RemoveModOnMod {
                target_source_id,
                param_name,
            } => {
                self.mixer
                    .modulation_mut()
                    .clear_mod_on_mod(&target_source_id, &param_name);
                CommandResult::Ok
            }

            // ── Macros ───────────────────────────────────────────
            EngineCommand::AddMacro { kind } => {
                let uuid = self.add_macro(kind);
                CommandResult::OkWithId { uuid }
            }
            EngineCommand::RemoveMacro { uuid } => {
                self.remove_macro(&uuid);
                CommandResult::Ok
            }
            EngineCommand::RenameMacro { uuid, name } => {
                self.rename_macro(&uuid, &name);
                CommandResult::Ok
            }
            EngineCommand::SetMacroKind { uuid, kind } => {
                self.set_macro_kind(&uuid, kind);
                CommandResult::Ok
            }
            EngineCommand::SetMacroValue { uuid, value } => {
                self.set_macro_value(&uuid, value);
                CommandResult::Ok
            }
            EngineCommand::AddMacroTarget { uuid, path } => {
                self.add_macro_target(&uuid, &path);
                CommandResult::Ok
            }
            EngineCommand::RemoveMacroTarget { uuid, target_idx } => {
                self.remove_macro_target(&uuid, target_idx);
                CommandResult::Ok
            }
            EngineCommand::UpdateMacroTarget {
                uuid,
                target_idx,
                min,
                max,
                curve,
                invert,
            } => {
                self.update_macro_target(&uuid, target_idx, min, max, curve, invert);
                CommandResult::Ok
            }
            EngineCommand::SetMacroButtonBehavior { uuid, behavior } => {
                self.set_macro_button_behavior(&uuid, behavior);
                CommandResult::Ok
            }
            EngineCommand::SetMacroTriggers { uuid, actions } => {
                self.set_macro_triggers(&uuid, actions);
                CommandResult::Ok
            }

            // ── Analyzers ────────────────────────────────────────
            EngineCommand::RequestAnalyzer {
                deck_id,
                analyzer_type,
                options,
            } => match self.request_analyzer(&deck_id, &analyzer_type, &options) {
                Ok(_) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: e.to_string(),
                },
            },
            EngineCommand::ReleaseAnalyzer {
                deck_id,
                analyzer_type,
            } => {
                self.release_analyzer(&deck_id, &analyzer_type);
                CommandResult::Ok
            }
            EngineCommand::AddAnalyzerModSource {
                deck_id,
                analyzer_type,
                output_name,
            } => {
                let source = crate::modulation::ModulationSource::Analyzer {
                    deck_id,
                    analyzer_type,
                    output_name,
                    smoothing: 0.3,
                };
                let uuid = self.mixer.modulation_mut().add_source(source);
                CommandResult::OkWithId { uuid }
            }
            EngineCommand::UpdateAnalyzerSmoothing { uuid, smoothing } => {
                if let Some(src) = self.mixer.modulation_mut().source_mut(&uuid) {
                    if let crate::modulation::ModulationSource::Analyzer { smoothing: s, .. } = src
                    {
                        *s = smoothing.clamp(0.0, 0.99);
                        CommandResult::Ok
                    } else {
                        CommandResult::Err {
                            code: ErrorCode::InvalidInput,
                            message: "Source is not an analyzer".into(),
                        }
                    }
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("Modulation source '{uuid}' not found"),
                    }
                }
            }

            // ── Device Scanning ───────────────────────────────────
            EngineCommand::RescanNdi => {
                self.external_io.ndi_manager.discover();
                CommandResult::Ok
            }
            EngineCommand::RescanSyphon => {
                // Run discovery inline on the render thread and return the fresh
                // source list in the same response. This makes an external
                // probe a single non-racy call: the old fire-and-forget rescan +
                // separate snapshot GET could read a pre-discover (empty) list and
                // spuriously "defer Syphon init".
                #[cfg(target_os = "macos")]
                {
                    self.external_io.syphon_manager.discover();
                    let names = self.external_io.syphon_manager.discovered_sources();
                    CommandResult::OkWithData {
                        data: serde_json::json!(names),
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    CommandResult::OkWithData {
                        data: serde_json::json!([] as [String; 0]),
                    }
                }
            }
            EngineCommand::RescanCameras => {
                self.camera_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::RescanMidi => {
                if let Some(ref mut midi) = self.input.midi_devices {
                    midi.load_user_profiles(&self.session.workspace.controller_profiles_dir());
                    if let Err(e) = midi.scan_devices() {
                        return CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: e.to_string(),
                        };
                    }
                    self.input.controller_led_mgr.sync_devices(midi);
                    self.input.auto_map_engine.sync_devices(midi);
                }
                CommandResult::Ok
            }
            EngineCommand::RescanAudio => {
                self.audio_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::ToggleAudioSource { source_id, enabled } => {
                if enabled {
                    if let Err(e) = self.audio_manager.open_source(source_id) {
                        log::warn!("Failed to open audio source {}: {}", source_id, e);
                        return CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: format!("Failed to open audio source: {}", e),
                        };
                    }
                } else {
                    self.audio_manager.close_source(source_id);
                }
                CommandResult::Ok
            }
            EngineCommand::SetMidiDeviceEnabled { device_id, enabled } => {
                if let Some(ref mut midi) = self.input.midi_devices {
                    midi.set_device_enabled(device_id, enabled);
                }
                CommandResult::Ok
            }

            // ── MIDI Mappings ─────────────────────────────────────
            EngineCommand::ClearMidiMappings => {
                self.input.midi_mappings.clear_all();
                CommandResult::Ok
            }
            EngineCommand::RemoveMidiMapping { key } => {
                self.input.midi_mappings.remove(&key);
                CommandResult::Ok
            }

            // ── Clock ─────────────────────────────────────────────
            EngineCommand::SetClockPreference { preference } => {
                self.input.clock_manager.set_preference(preference);
                CommandResult::Ok
            }
            EngineCommand::SetManualBpm { bpm } => {
                self.input
                    .clock_manager
                    .set_preference(crate::clock::ClockPreference::ForceManual { bpm });
                CommandResult::Ok
            }

            // ── Parameters (index-based) ────────────────────────────
            EngineCommand::SetGeneratorParam {
                channel_idx,
                deck_idx,
                name,
                value,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        ch.decks[deck_idx].deck.generator_params.set(&name, value);
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetEffectParam {
                channel_idx,
                deck_idx,
                effect_idx,
                name,
                value,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        let deck = &mut ch.decks[deck_idx].deck;
                        if effect_idx < deck.effects.len() {
                            deck.effects[effect_idx].params.set(&name, value);
                        }
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetChannelEffectParam {
                channel_idx,
                effect_idx,
                name,
                value,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if effect_idx < ch.effects.len() {
                        ch.effects[effect_idx].params.set(&name, value);
                    }
                }
                CommandResult::Ok
            }
            EngineCommand::SetMasterEffectParam {
                effect_idx,
                name,
                value,
            } => {
                if effect_idx < self.mixer.master_effects().len() {
                    self.mixer.master_effects_mut()[effect_idx]
                        .params
                        .set(&name, value);
                }
                CommandResult::Ok
            }
            EngineCommand::ResetGeneratorParamsToDefaults {
                channel_idx,
                deck_idx,
            } => {
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    if deck_idx < ch.decks.len() {
                        ch.decks[deck_idx].deck.generator_params.reset_to_defaults();
                    }
                }
                CommandResult::Ok
            }

            // ── Resolution ────────────────────────────────────────
            EngineCommand::SetRenderResolution { width, height } => {
                self.set_render_resolution(width, height);
                CommandResult::Ok
            }

            EngineCommand::SetTargetFps { fps } => {
                self.set_target_fps(fps);
                CommandResult::Ok
            }

            EngineCommand::StartPerfProfile { frames } => {
                self.mixer.start_perf_profile(frames);
                CommandResult::Ok
            }

            // ── Persistence ───────────────────────────────────────
            EngineCommand::SaveWorkspace => {
                let layout = crate::usecases::ui::UILayoutState::default();
                self.save_workspace(&layout);
                CommandResult::Ok
            }
            EngineCommand::LoadWorkspace => {
                let _ = self.load_workspace();
                CommandResult::Ok
            }

            // ── History ───────────────────────────────────────────
            // Restore is shared with the windowed runner via `history_undo` /
            // `history_redo` on the unified timeline. The headless/API path has
            // no UI layout, so it uses `history_snapshot_default()` for the
            // "current" state pushed onto the opposite stack.
            EngineCommand::Undo => {
                let current = self.history_snapshot_default();
                if self.history_undo(current).is_some() {
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: "Nothing to undo".into(),
                    }
                }
            }
            EngineCommand::Redo => {
                let current = self.history_snapshot_default();
                if self.history_redo(current).is_some() {
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: "Nothing to redo".into(),
                    }
                }
            }

            // ── System ────────────────────────────────────────────
            EngineCommand::Shutdown => {
                self.shutdown_requested = true;
                CommandResult::Ok
            }
        }
    }
}

/// Whether a bus-driven command should record an undo/redo snapshot before it
/// executes. This is what makes API / WebSocket / CLI edits undoable on the
/// same timeline the windowed UI uses (see [undo-redo.md](/spec/undo-redo.md)).
///
/// The predicate is an explicit **denylist** of live-control, transient, and
/// non-authored commands; everything else defaults to undoable. New commands
/// are therefore undoable unless added here — when introducing a live control
/// (transport, device toggle, output-window lifecycle) or a transient action,
/// add it below so it does not pollute the undo timeline. This mirrors
/// `UIActions::has_undoable_action` / `has_undoable_stage_action`, which are
/// the equivalent gate for the windowed consumer.
pub(crate) fn command_is_undoable(cmd: &EngineCommand) -> bool {
    use EngineCommand as C;
    !matches!(
        cmd,
        // Live crossfader control (spec: ⚠️ live, excluded).
        C::SetCrossfader(..)
            | C::AutoCrossfade { .. }
            | C::BeatCrossfade { .. }
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
            // ADSR live triggers.
            | C::TriggerAdsr { .. }
            | C::ReleaseAdsr { .. }
            // Sequence playback transport (authoring steps stay undoable).
            | C::PlaySequence { .. }
            | C::StopSequence { .. }
            | C::ToggleSequence { .. }
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
            | C::RescanMidi
            | C::SetMidiDeviceEnabled { .. }
            | C::ClearMidiMappings
            | C::RemoveMidiMapping { .. }
            // Clock preference / manual BPM (live sync config).
            | C::SetClockPreference { .. }
            | C::SetManualBpm { .. }
            // Global engine settings / profiling.
            | C::SetRenderResolution { .. }
            | C::SetTargetFps { .. }
            | C::StartPerfProfile { .. }
            // Persistence, history control, and shutdown are never undoable.
            | C::SaveWorkspace
            | C::LoadWorkspace
            | C::Undo
            | C::Redo
            | C::Shutdown
    )
}

#[cfg(test)]
mod tests {
    use super::command_is_undoable;
    use crate::engine::EngineCommand as C;

    #[test]
    fn authoring_commands_are_undoable() {
        assert!(command_is_undoable(&C::AddChannel));
        assert!(command_is_undoable(&C::RemoveChannel { channel_idx: 0 }));
        assert!(command_is_undoable(&C::SetChannelOpacity {
            channel_idx: 0,
            opacity: 0.5,
        }));
        assert!(command_is_undoable(&C::SetParam {
            path: "deck/abc/opacity".into(),
            value: crate::engine::ParamValue::Float(0.5),
        }));
        assert!(command_is_undoable(&C::RemoveSurface { uuid: "s".into() }));
        // Surface→output assignment is authoring and must be undoable.
        assert!(command_is_undoable(&C::AssignSurfaceToOutputByIdx {
            output_idx: 0,
            surface_uuid: "s".into(),
        }));
    }

    #[test]
    fn live_and_transient_commands_are_not_undoable() {
        assert!(!command_is_undoable(&C::SetCrossfader(0.5)));
        assert!(!command_is_undoable(&C::VideoTogglePlay {
            channel_idx: 0,
            deck_idx: 0,
        }));
        assert!(!command_is_undoable(&C::PlaySequence { idx: 0 }));
        assert!(!command_is_undoable(&C::StartOutput { idx: 0 }));
        assert!(!command_is_undoable(&C::CreateOutput));
        assert!(!command_is_undoable(&C::Undo));
        assert!(!command_is_undoable(&C::Redo));
        assert!(!command_is_undoable(&C::SaveWorkspace));
        assert!(!command_is_undoable(&C::Shutdown));
    }
}
