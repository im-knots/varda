//! Cross-thread command dispatch for `VardaApp`.
//!
//! Houses `execute_command`, the exhaustive match over every `EngineCommand`
//! variant that cross-thread consumers (HTTP API, WebSocket, CLI) drive through
//! the command channel.

use super::resolve::UnknownEntity;
use super::VardaApp;
use crate::engine::{CommandOutcome, CommandResult, DomeLayoutFields, EngineCommand, ErrorCode};

/// Classify an engine error for the wire. An unresolvable UUID is `NotFound` —
/// the caller's view of the world is stale, which is distinct from a malformed
/// request. See [`/spec/api-addressing.md`].
fn classify(err: &anyhow::Error) -> ErrorCode {
    if err.downcast_ref::<UnknownEntity>().is_some() {
        ErrorCode::NotFound
    } else {
        ErrorCode::InvalidInput
    }
}

/// Map a unit-returning engine call onto the wire result.
fn wire(result: anyhow::Result<()>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::Ok,
        Err(e) => CommandResult::Err {
            code: classify(&e),
            message: e.to_string(),
        },
    }
}

/// Map an id-returning engine call (creation) onto the wire result.
fn wire_id(result: anyhow::Result<String>) -> CommandResult {
    match result {
        Ok(uuid) => CommandResult::OkWithId { uuid },
        Err(e) => CommandResult::Err {
            code: classify(&e),
            message: e.to_string(),
        },
    }
}

/// Wire result for a resolution failure handled inline.
fn not_found(err: &UnknownEntity) -> CommandResult {
    CommandResult::Err {
        code: ErrorCode::NotFound,
        message: err.to_string(),
    }
}

/// Wire result for a transport operation the current source disallows, so a
/// caller learns why rather than watching nothing happen.
/// See /spec/transport.md § Legibility.
fn transport_rejected(err: crate::transport::TransportError) -> CommandResult {
    CommandResult::Err {
        code: ErrorCode::InvalidInput,
        message: err.to_string(),
    }
}

impl VardaApp {
    /// Execute a command on behalf of the windowed GUI, returning a typed,
    /// in-process [`CommandOutcome`] instead of the serializable wire
    /// [`CommandResult`]. Deck-creating commands surface their location + UUID
    /// so the runner can register a preview texture; everything else is
    /// delegated verbatim to [`Self::execute_command`]. See
    /// [`/spec/ui-engine-boundary.md`] WS1/Decision #9.
    pub(crate) fn execute_command_gui(&mut self, cmd: EngineCommand) -> CommandOutcome {
        // A preset load can create several decks at once (a channel preset fills
        // a whole channel), and the count isn't known up front, so diff the deck
        // set across execution rather than reading a single reported id.
        let is_preset_load = matches!(
            &cmd,
            EngineCommand::LoadDeckPreset { .. } | EngineCommand::LoadChannelPreset { .. }
        );
        let is_deck_add = super::actions::command_is_deck_add(&cmd);
        let decks_before = if is_preset_load {
            self.deck_uuid_set()
        } else {
            std::collections::HashSet::new()
        };

        let result = self.execute_command(cmd);
        if matches!(result, CommandResult::Err { .. }) {
            return CommandOutcome::Plain(result);
        }

        if is_preset_load {
            let uuids = self
                .deck_uuid_set()
                .difference(&decks_before)
                .cloned()
                .collect();
            return CommandOutcome::DecksCreated { uuids };
        }
        if is_deck_add {
            if let CommandResult::OkWithId { uuid } = result {
                return CommandOutcome::DecksCreated { uuids: vec![uuid] };
            }
            return CommandOutcome::Plain(result);
        }
        CommandOutcome::Plain(result)
    }

    /// Every live deck UUID. Used to diff deck creation across a command whose
    /// effect on the deck set isn't known in advance.
    fn deck_uuid_set(&self) -> std::collections::HashSet<String> {
        self.mixer
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .map(|slot| slot.deck.uuid().to_string())
            .collect()
    }

    /// Undo/redo on behalf of the windowed GUI. Uses the UI `layout` to source
    /// cosmetic/dome prefs for the "current" snapshot (the API path uses
    /// defaults), and returns a typed [`CommandOutcome::HistoryRestored`] so the
    /// runner can re-register textures and sync dome flags. Replaces the runner's
    /// bespoke inline undo branch (see [`/spec/ui-engine-boundary.md`] Decision #10).
    pub(crate) fn history_gui(
        &mut self,
        layout: &crate::usecases::ui::UILayoutState,
        undo: bool,
    ) -> CommandOutcome {
        let current = self.history_snapshot(layout);
        let restore = if undo {
            self.history_undo(current)
        } else {
            self.history_redo(current)
        };
        match restore {
            Some(r) => CommandOutcome::HistoryRestored {
                structural_changed: r.structural_changed,
                dome_layout: DomeLayoutFields {
                    dome_mode_active: r.snapshot.stage.dome_mode_active,
                    dome_preset: r.snapshot.stage.dome_preset,
                    dome_geometry: r.snapshot.stage.dome_geometry,
                },
            },
            None => CommandOutcome::Plain(CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: if undo {
                    "Nothing to undo".into()
                } else {
                    "Nothing to redo".into()
                },
            }),
        }
    }

    /// True if any command in the batch is undoable. Used by the windowed
    /// runner to make one snapshot decision over the GUI's command stream,
    /// sharing the single compiler-checked [`command_is_undoable`] predicate
    /// with the bus consumers.
    // Method form is the engine-facing API used by `usecases/ui/runner.rs`.
    #[allow(clippy::unused_self)]
    pub(crate) fn batch_has_undoable(&self, cmds: &[EngineCommand]) -> bool {
        cmds.iter().any(command_is_undoable)
    }

    /// Execute a single command and return the result.
    pub(crate) fn execute_command(&mut self, cmd: EngineCommand) -> CommandResult {
        use crate::engine::traits::{
            AnalyzerCommands, AudioCommands, DetectCommands, MacroCommands, MixerCommands,
            ModulationCommands, OutputCommands, SurfaceCommands,
        };
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
                channel_uuid,
                shader_name,
            } => wire_id(self.add_deck(&channel_uuid, &shader_name)),
            EngineCommand::AddImageDeck { channel_uuid, path } => {
                wire_id(self.add_image_deck(&channel_uuid, &path))
            }
            EngineCommand::AddVideoDeck { channel_uuid, path } => {
                wire_id(self.add_video_deck(&channel_uuid, &path))
            }
            EngineCommand::AddSolidColorDeck {
                channel_uuid,
                color,
            } => wire_id(self.add_solid_color_deck(&channel_uuid, color)),
            EngineCommand::AddCameraDeck {
                channel_uuid,
                camera_id,
            } => wire_id(self.add_camera_deck(&channel_uuid, camera_id)),
            EngineCommand::AddDepthSensorDeck {
                channel_uuid,
                depth_sensor_id,
            } => wire_id(self.add_depth_sensor_deck(&channel_uuid, depth_sensor_id)),
            EngineCommand::AddScreenCaptureDeck {
                channel_uuid,
                target,
                rate,
                crop,
                show_cursor,
                exclude_varda,
            } => {
                let options = crate::screen_capture::backend::CaptureConfig {
                    rate: rate.unwrap_or(crate::screen_capture::backend::DEFAULT_CAPTURE_RATE),
                    crop: crop.map(Into::into).unwrap_or_default(),
                    show_cursor: show_cursor.unwrap_or(false),
                    // Displays default to excluding Varda so a full-display
                    // capture is not an accidental infinite mirror; picking a
                    // Varda window is an explicit request, so it does not.
                    exclude_varda: exclude_varda.unwrap_or_else(|| target.is_display()),
                    scale_to: None,
                };
                wire_id(self.add_screen_capture_deck(&channel_uuid, &target, options))
            }
            EngineCommand::AddTapDeck {
                channel_uuid,
                source,
            } => wire_id(self.add_tap_deck(&channel_uuid, &source)),
            EngineCommand::SetTapSource { deck_uuid, source } => {
                wire(self.set_tap_source(&deck_uuid, &source))
            }
            EngineCommand::RemoveDeck { deck_uuid } => wire(self.remove_deck(&deck_uuid)),
            EngineCommand::MoveDeck {
                deck_uuid,
                dst_channel_uuid,
            } => wire(self.move_deck(&deck_uuid, &dst_channel_uuid)),
            EngineCommand::ReorderDeck {
                channel_uuid,
                from_idx,
                to_idx,
            } => wire(self.reorder_deck(&channel_uuid, from_idx, to_idx)),
            EngineCommand::SetDeckOpacity { deck_uuid, opacity } => {
                let result = wire(self.set_deck_opacity(&deck_uuid, opacity));
                // Grabbing a deck's fader takes that lane back from the show,
                // immediately and without confirmation.
                self.note_live_param_write(
                    &crate::arrangement::opacity_param_key(&deck_uuid),
                    opacity,
                );
                result
            }
            EngineCommand::SetDeckBlendMode { deck_uuid, mode } => {
                wire(self.set_deck_blend_mode(&deck_uuid, mode))
            }
            EngineCommand::SetDeckSolo { deck_uuid, solo } => {
                wire(self.set_deck_solo(&deck_uuid, solo))
            }
            EngineCommand::SetDeckMute { deck_uuid, mute } => {
                wire(self.set_deck_mute(&deck_uuid, mute))
            }
            EngineCommand::SetDeckRenderFps {
                deck_uuid,
                render_fps,
            } => match self.resolve_deck(&deck_uuid) {
                Ok((ch, dk)) => {
                    self.mixer.channels_mut()[ch].decks[dk].render_fps = render_fps;
                    CommandResult::Ok
                }
                Err(e) => not_found(&e),
            },
            EngineCommand::SetDeckScalingMode { deck_uuid, mode } => {
                let result = wire(self.set_deck_scaling_mode(&deck_uuid, mode));
                // Scaling belongs to any deck with a source texture, not just a
                // video one, but it is modulatable on the same terms.
                if matches!(result, CommandResult::Ok) {
                    self.note_live_video_write(
                        &deck_uuid,
                        crate::video::modulation::SCALING_MODE,
                        crate::param_router::scaling_mode_to_value(mode),
                    );
                }
                result
            }
            EngineCommand::SetDeckTransparent {
                deck_uuid,
                transparent,
            } => wire(self.set_deck_transparent(&deck_uuid, transparent)),
            EngineCommand::SetChannelOpacity {
                channel_uuid,
                opacity,
            } => {
                let result = wire(self.set_channel_opacity(&channel_uuid, opacity));
                // A hand on the channel fader takes it back from any curve on
                // it, the same way a deck's does.
                self.note_live_param_write(
                    &crate::arrangement::channel_opacity_param_key(&channel_uuid),
                    opacity,
                );
                result
            }
            EngineCommand::SetChannelBlendMode { channel_uuid, mode } => {
                wire(self.set_channel_blend_mode(&channel_uuid, mode))
            }
            EngineCommand::AddChannel => wire_id(self.add_channel()),
            EngineCommand::RemoveChannel { channel_uuid } => {
                wire(self.remove_channel(&channel_uuid))
            }
            EngineCommand::AddEffect {
                target,
                shader_name,
            } => wire_id(self.add_effect(target, &shader_name)),
            EngineCommand::RemoveEffect { effect_uuid } => wire(self.remove_effect(&effect_uuid)),
            EngineCommand::ToggleEffect { effect_uuid } => wire(self.toggle_effect(&effect_uuid)),
            EngineCommand::MoveEffect {
                target,
                from_idx,
                to_idx,
            } => wire(self.move_effect(target, from_idx, to_idx)),

            // ── Clipboard ────────────────────────────────────
            EngineCommand::Copy {
                source,
                include_arrangement,
            } => self.cmd_copy(&source, include_arrangement),
            EngineCommand::Paste { target } => self.cmd_paste(&target),
            EngineCommand::Duplicate { source } => self.cmd_duplicate(&source),
            EngineCommand::SetTransition { shader_name } => {
                match self.set_transition(shader_name.as_deref()) {
                    Ok(()) => CommandResult::Ok,
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
            EngineCommand::ToggleParam { path } => {
                if let Err(e) = crate::param_router::toggle_param_by_path(&mut self.mixer, &path) {
                    log::debug!("ToggleParam {path}: {e}");
                }
                CommandResult::Ok
            }

            // ── Audio ────────────────────────────────────────
            EngineCommand::OpenAudioSource { source_id } => {
                match self.open_audio_source(source_id) {
                    Ok(()) => CommandResult::Ok,
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
            EngineCommand::AddAutomationLane { target, timebase } => {
                // Returns the UUID because the caller needs it to reveal the
                // new lane and to push breakpoints into it.
                CommandResult::OkWithId {
                    uuid: self.add_automation_lane(&target, timebase),
                }
            }
            EngineCommand::SetEnvelopeBreakpoints { uuid, breakpoints } => {
                if self.set_envelope_breakpoints(&uuid, breakpoints) {
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("no automation envelope with uuid '{uuid}'"),
                    }
                }
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
            EngineCommand::CloseOutput { output_uuid } => wire(self.close_output(&output_uuid)),
            EngineCommand::SetOutputDisplay {
                output_uuid,
                monitor_name,
            } => wire(self.set_output_display(&output_uuid, &monitor_name)),
            EngineCommand::SetOutputTarget {
                output_uuid,
                target,
            } => match self.resolve_output(&output_uuid) {
                Ok(idx) => self.cmd_set_output_target(idx, target),
                Err(e) => not_found(&e),
            },

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
                surface_uuid,
            } => {
                self.unassign_surface_from_output(&output_uuid, &surface_uuid);
                self.recompute_auto_edge_blend();
                CommandResult::Ok
            }

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
            EngineCommand::ImportSurfacesFromFile { path } => {
                let params = crate::surface::detect::DetectionParams::default();
                match crate::surface::import::detect_from_file(&path, &params) {
                    Ok(result) => {
                        let uuids = self.confirm_detected_contours(&result.contours);
                        log::info!("Imported {} surfaces from {}", uuids.len(), path.display());
                        CommandResult::OkWithData {
                            data: serde_json::json!({ "surface_uuids": uuids }),
                        }
                    }
                    Err(e) => {
                        log::error!("Surface import failed: {e}");
                        CommandResult::Err {
                            code: ErrorCode::InvalidInput,
                            message: e.to_string(),
                        }
                    }
                }
            }
            EngineCommand::GenerateDomeSlices { setup } => {
                self.generate_dome_slices(&setup);
                CommandResult::Ok
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
            // The four playback commands below take their lane back from the
            // show, the same way a hand on a deck fader does. Each records the
            // normalized value a curve would need to hold to reproduce the
            // gesture, because that is the space the override ramp and the
            // recorder both work in.
            EngineCommand::VideoTogglePlay { deck_uuid } => {
                // Read before the toggle: it sends a command to the decode
                // thread, so the snapshot still reports the old state after.
                let was_playing = self
                    .video_playback_snapshot(&deck_uuid)
                    .is_some_and(|s| s.playing);
                let result = self.exec_on_deck(&deck_uuid, |d| d.video_toggle_play());
                if matches!(result, CommandResult::Ok) {
                    self.note_live_video_write(
                        &deck_uuid,
                        crate::video::modulation::PLAY,
                        f32::from(u8::from(!was_playing)),
                    );
                }
                result
            }
            EngineCommand::VideoSeek {
                deck_uuid,
                position_secs,
            } => {
                let duration = self
                    .video_playback_snapshot(&deck_uuid)
                    .map_or(0.0, |s| s.duration);
                let result = self.exec_on_deck(&deck_uuid, |d| d.video_seek(position_secs));
                if matches!(result, CommandResult::Ok) {
                    self.note_live_video_write(
                        &deck_uuid,
                        crate::video::modulation::POSITION,
                        crate::param_router::duration_to_norm(position_secs, duration),
                    );
                }
                result
            }
            EngineCommand::VideoSetSpeed { deck_uuid, speed } => {
                let result = self.exec_on_deck(&deck_uuid, |d| d.video_set_speed(speed));
                if matches!(result, CommandResult::Ok) {
                    self.note_live_video_write(
                        &deck_uuid,
                        crate::video::modulation::SPEED,
                        crate::param_router::speed_to_norm(speed),
                    );
                }
                result
            }
            EngineCommand::VideoSetLoopMode { deck_uuid, mode } => {
                let result = self.exec_on_deck(&deck_uuid, |d| d.video_set_loop_mode(mode));
                if matches!(result, CommandResult::Ok) {
                    self.note_live_video_write(
                        &deck_uuid,
                        crate::video::modulation::LOOP_MODE,
                        crate::param_router::loop_mode_to_value(mode),
                    );
                }
                result
            }
            EngineCommand::VideoSetInPoint { deck_uuid, secs } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_set_in_point(secs))
            }
            EngineCommand::VideoSetOutPoint { deck_uuid, secs } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_set_out_point(secs))
            }
            EngineCommand::VideoClearInOutPoints { deck_uuid } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_clear_in_out_points())
            }
            EngineCommand::VideoSetTransportSync { deck_uuid, sync } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_set_transport_sync(sync))
            }

            // ── Deck Auto-Transitions ─────────────────────────
            EngineCommand::SetAutoTransitionEnabled { deck_uuid, enabled } => self
                .exec_auto_transition(&deck_uuid, |at| {
                    at.enabled = enabled;
                    if !enabled {
                        at.phase = crate::channel::DeckTransitionPhase::Inactive;
                    }
                }),
            EngineCommand::SetAutoTransitionTrigger {
                deck_uuid,
                clip_end,
            } => self.exec_auto_transition(&deck_uuid, |at| {
                at.trigger = if clip_end {
                    crate::channel::TransitionTrigger::ClipEnd
                } else {
                    crate::channel::TransitionTrigger::Timer
                };
            }),
            EngineCommand::SetAutoTransitionPlayDuration {
                deck_uuid,
                value,
                unit,
            } => self.exec_auto_transition(&deck_uuid, |at| {
                at.play_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
            }),
            EngineCommand::SetAutoTransitionDuration {
                deck_uuid,
                value,
                unit,
            } => self.exec_auto_transition(&deck_uuid, |at| {
                at.transition_duration = crate::channel::DurationSpec::from_value_unit(value, unit);
            }),
            EngineCommand::SetAutoTransitionShader {
                deck_uuid,
                shader_name,
            } => {
                let (ch_idx, deck_idx) = match self.resolve_deck(&deck_uuid) {
                    Ok(loc) => loc,
                    Err(e) => return not_found(&e),
                };
                let shader = shader_name.as_ref().and_then(|name| {
                    self.registry
                        .transitions()
                        .iter()
                        .find(|s| s.name() == *name)
                        .map(|s| (*s).clone())
                });
                let slot = &mut self.mixer.channels_mut()[ch_idx].decks[deck_idx];
                slot.auto_transition
                    .get_or_insert_with(crate::channel::DeckAutoTransition::new)
                    .transition_shader_name
                    .clone_from(&shader_name);
                match shader {
                    Some(shader) => {
                        let _ = slot.set_transition_shader(&self.context, shader);
                    }
                    None => slot.transition_effect = None,
                }
                CommandResult::Ok
            }
            EngineCommand::ToggleAutoTransitionPlayDurationUnit { deck_uuid } => self
                .exec_auto_transition(&deck_uuid, |at| {
                    let next_unit = at.play_duration.unit().next();
                    at.play_duration = crate::channel::DurationSpec::from_value_unit(
                        at.play_duration.value(),
                        next_unit,
                    );
                }),
            EngineCommand::ToggleAutoTransitionDurationUnit { deck_uuid } => self
                .exec_auto_transition(&deck_uuid, |at| {
                    let next_unit = at.transition_duration.unit().next();
                    at.transition_duration = crate::channel::DurationSpec::from_value_unit(
                        at.transition_duration.value(),
                        next_unit,
                    );
                }),
            EngineCommand::SetAutoTransitionPlayDurationValue { deck_uuid, value } => self
                .exec_auto_transition(&deck_uuid, |at| {
                    at.play_duration.set_value(value);
                }),
            EngineCommand::SetAutoTransitionDurationValue { deck_uuid, value } => self
                .exec_auto_transition(&deck_uuid, |at| {
                    at.transition_duration.set_value(value);
                }),

            // ── External I/O Deck Sources ─────────────────────
            EngineCommand::AddNdiDeck {
                channel_uuid,
                source_name,
            } => self.cmd_add_ndi_deck(&channel_uuid, &source_name),
            EngineCommand::AddSyphonDeck {
                channel_uuid,
                server_name,
            } => self.cmd_add_syphon_deck(&channel_uuid, &server_name),
            EngineCommand::AddSrtDeck {
                channel_uuid,
                url,
                mode,
            } => self.cmd_add_srt_deck(&channel_uuid, &url, mode),
            EngineCommand::AddHlsDeck { channel_uuid, url } => {
                self.cmd_add_hls_deck(&channel_uuid, &url)
            }
            EngineCommand::AddDashDeck { channel_uuid, url } => {
                self.cmd_add_dash_deck(&channel_uuid, &url)
            }
            EngineCommand::AddRtmpDeck {
                channel_uuid,
                url,
                mode,
            } => self.cmd_add_rtmp_deck(&channel_uuid, &url, mode),
            EngineCommand::ReloadHtmlDeck { deck_uuid } => self.cmd_reload_html_deck(&deck_uuid),
            EngineCommand::AddHtmlDeck { channel_uuid, url } => {
                self.cmd_add_html_deck(&channel_uuid, &url)
            }
            EngineCommand::OpenHtmlInteractive { deck_uuid } => {
                #[cfg(feature = "html")]
                {
                    self.cmd_open_html_interactive(&deck_uuid)
                }
                #[cfg(not(feature = "html"))]
                {
                    let _ = deck_uuid;
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
            EngineCommand::DeleteSequence { sequence_uuid } => {
                self.cmd_delete_sequence(&sequence_uuid)
            }
            EngineCommand::PlaySequence { sequence_uuid } => self.cmd_play_sequence(&sequence_uuid),
            EngineCommand::StopSequence { sequence_uuid } => self.cmd_stop_sequence(&sequence_uuid),
            EngineCommand::ToggleSequence { sequence_uuid } => {
                self.cmd_toggle_sequence(&sequence_uuid)
            }
            EngineCommand::AddFadeStep {
                sequence_uuid,
                from_channel_uuid,
                to_channel_uuid,
            } => self.cmd_add_fade_step(&sequence_uuid, &from_channel_uuid, &to_channel_uuid),
            EngineCommand::AddWaitStep { sequence_uuid } => self.cmd_add_wait_step(&sequence_uuid),
            EngineCommand::AddGoToStep {
                sequence_uuid,
                step_index,
            } => self.cmd_add_goto_step(&sequence_uuid, step_index),
            EngineCommand::RemoveStep {
                sequence_uuid,
                step_idx,
            } => self.cmd_remove_step(&sequence_uuid, step_idx),
            EngineCommand::SetStepDuration {
                sequence_uuid,
                step_idx,
                value,
                unit,
            } => self.cmd_set_step_duration(&sequence_uuid, step_idx, value, unit),
            EngineCommand::SetStepEasing {
                sequence_uuid,
                step_idx,
                easing,
            } => self.cmd_set_step_easing(&sequence_uuid, step_idx, &easing),
            EngineCommand::SetStepTransitionShader {
                sequence_uuid,
                step_idx,
                shader_name,
            } => self.cmd_set_step_transition_shader(&sequence_uuid, step_idx, shader_name),
            EngineCommand::MoveStep {
                sequence_uuid,
                from,
                to,
            } => self.cmd_move_step(&sequence_uuid, from, to),
            EngineCommand::SetStepDurationUnit {
                sequence_uuid,
                step_idx,
                unit,
            } => self.cmd_set_step_duration_unit(&sequence_uuid, step_idx, unit),
            EngineCommand::ToggleStepDurationUnit {
                sequence_uuid,
                step_idx,
            } => self.cmd_toggle_step_duration_unit(&sequence_uuid, step_idx),
            EngineCommand::SetStepDurationValue {
                sequence_uuid,
                step_idx,
                value,
            } => self.cmd_set_step_duration_value(&sequence_uuid, step_idx, value),
            EngineCommand::SetStepFromCh {
                sequence_uuid,
                step_idx,
                channel_uuid,
            } => self.cmd_set_step_from_ch(&sequence_uuid, step_idx, channel_uuid),
            EngineCommand::SetStepToCh {
                sequence_uuid,
                step_idx,
                channel_uuid,
            } => self.cmd_set_step_to_ch(&sequence_uuid, step_idx, channel_uuid),
            EngineCommand::SetGoToTarget {
                sequence_uuid,
                step_idx,
                target,
            } => self.cmd_set_goto_target(&sequence_uuid, step_idx, target),
            EngineCommand::SetStepTargetAmount {
                sequence_uuid,
                step_idx,
                amount,
            } => self.cmd_set_step_target_amount(&sequence_uuid, step_idx, amount),

            // ── Stream Library ─────────────────────────────────
            EngineCommand::AddStreamLibraryEntry { url, mode } => {
                self.cmd_add_stream_library_entry(url, mode)
            }
            EngineCommand::RemoveStreamLibraryEntry { url } => {
                self.cmd_remove_stream_library_entry(&url)
            }
            EngineCommand::AddHlsLibraryEntry { url } => self.cmd_add_hls_library_entry(url),
            EngineCommand::RemoveHlsLibraryEntry { url } => self.cmd_remove_hls_library_entry(&url),
            EngineCommand::AddDashLibraryEntry { url } => self.cmd_add_dash_library_entry(url),
            EngineCommand::RemoveDashLibraryEntry { url } => {
                self.cmd_remove_dash_library_entry(&url)
            }
            EngineCommand::AddRtmpLibraryEntry { url, mode } => {
                self.cmd_add_rtmp_library_entry(url, mode)
            }
            EngineCommand::RemoveRtmpLibraryEntry { url } => {
                self.cmd_remove_rtmp_library_entry(&url)
            }
            EngineCommand::AddHtmlLibraryEntry { url } => self.cmd_add_html_library_entry(url),
            EngineCommand::RemoveHtmlLibraryEntry { url } => {
                self.cmd_remove_html_library_entry(&url)
            }

            // ── Output Management ─────────────────────────────────
            EngineCommand::CreateHeadlessOutput { target } => {
                self.cmd_create_headless_output(target)
            }
            EngineCommand::StartOutput { output_uuid } => self.cmd_start_output(&output_uuid),
            EngineCommand::StopOutput { output_uuid } => self.cmd_stop_output(&output_uuid),
            EngineCommand::SetCalibrationMode { output_uuid, mode } => {
                self.cmd_set_calibration_mode(&output_uuid, mode)
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
            EngineCommand::SetEdgeBlend {
                output_uuid,
                config,
            } => self.cmd_set_edge_blend(&output_uuid, config),
            EngineCommand::SetEdgeBlendMode { output_uuid, mode } => {
                self.cmd_set_edge_blend_mode(&output_uuid, mode)
            }
            EngineCommand::SetOutputRotation {
                output_uuid,
                rotation,
            } => self.cmd_set_output_rotation(&output_uuid, rotation),
            EngineCommand::SetOutputPresentation {
                output_uuid,
                request,
            } => self.cmd_set_output_presentation(&output_uuid, request),

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
            EngineCommand::TransportPlay => match self.transport.play() {
                Ok(()) => CommandResult::Ok,
                Err(e) => transport_rejected(e),
            },
            EngineCommand::TransportStop => {
                self.transport.stop();
                // A second stop returns to zero, which is a move the cue walk
                // did not make and must not keep stepping from.
                self.forget_cue_walk();
                CommandResult::Ok
            }
            EngineCommand::TransportLocate { position } => match self.transport.locate(position) {
                Ok(()) => {
                    self.forget_cue_walk();
                    CommandResult::Ok
                }
                Err(e) => transport_rejected(e),
            },
            EngineCommand::SetTransportSource { source } => {
                self.transport.set_source(source);
                self.forget_cue_walk();
                CommandResult::Ok
            }
            EngineCommand::SetTransportLoop { region } => {
                // Re-checked here rather than trusted: the command arrives from
                // the API as plain JSON, which cannot enforce the invariant.
                let checked = match region {
                    Some(r) => match crate::transport::LoopRegion::new(r.start, r.end) {
                        Ok(r) => Some(r),
                        Err(e) => return transport_rejected(e),
                    },
                    None => None,
                };
                self.transport.set_loop_region(checked);
                CommandResult::Ok
            }
            EngineCommand::SetTimecodeRate { rate } => {
                self.transport.set_timecode_rate(rate);
                CommandResult::Ok
            }
            EngineCommand::SetTimecodePreference { preference } => {
                self.input.timecode.set_preference(preference);
                CommandResult::Ok
            }
            EngineCommand::SetLtcInput { input } => {
                self.input.timecode.set_ltc_input(input);
                CommandResult::Ok
            }
            EngineCommand::SetRecordArmed { armed } => {
                self.set_record_armed(armed);
                CommandResult::Ok
            }
            EngineCommand::TransportPrevCue => self.cmd_locate_cue(false),
            EngineCommand::TransportNextCue => self.cmd_locate_cue(true),
            EngineCommand::TriggerCue { uuid } => self.cmd_trigger_cue(&uuid),
            EngineCommand::AddLane { deck_uuid } => self.cmd_add_lane(&deck_uuid),
            EngineCommand::RemoveLane { deck_uuid } => self.cmd_remove_lane(&deck_uuid),
            EngineCommand::AddRegion { deck_uuid, region } => {
                self.cmd_add_region(&deck_uuid, region)
            }
            EngineCommand::UpdateRegion {
                deck_uuid,
                index,
                region,
            } => self.cmd_update_region(&deck_uuid, index, region),
            EngineCommand::RemoveRegion { deck_uuid, index } => {
                self.cmd_remove_region(&deck_uuid, index)
            }
            EngineCommand::SetLaneCollapsed {
                deck_uuid,
                collapsed,
            } => self.cmd_set_lane_collapsed(&deck_uuid, collapsed),
            EngineCommand::SetIdleBehaviour { idle } => self.cmd_set_idle_behaviour(idle),
            EngineCommand::RearmParam { param_key, seconds } => {
                self.cmd_rearm_param(&param_key, seconds)
            }
            EngineCommand::RearmAll { seconds } => self.cmd_rearm_all(seconds),
            EngineCommand::AddCue { at, name } => self.cmd_add_cue(at, &name),
            EngineCommand::UpdateCue { uuid, at, name } => self.cmd_update_cue(&uuid, at, name),
            EngineCommand::RemoveCue { uuid } => self.cmd_remove_cue(&uuid),
            EngineCommand::UpdateModulationTimebase { uuid, timebase } => {
                if self.mixer.modulation_mut().set_timebase(&uuid, timebase) {
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("Modulation source {uuid} not found"),
                    }
                }
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
                Ok(()) => CommandResult::Ok,
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
            EngineCommand::RescanDepthSensors => {
                self.depth_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::RescanCaptureTargets => {
                self.screen_capture_manager.scan_targets();
                CommandResult::Ok
            }
            EngineCommand::RequestScreenCapturePermission => {
                self.screen_capture_manager.request_permission();
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
                        log::warn!("Failed to open audio source {source_id}: {e}");
                        return CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: format!("Failed to open audio source: {e}"),
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

            // ── Parameters ─────────────────────────────────────────
            EngineCommand::SetGeneratorParam {
                deck_uuid,
                name,
                value,
            } => match self.resolve_deck(&deck_uuid) {
                Ok((ch_idx, dk_idx)) => {
                    let mut taken = None;
                    if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                        let params = &mut ch.decks[dk_idx].deck.generator_params;
                        taken = params.normalize(&name, &value);
                        params.set(&name, value);
                    }
                    if let Some(held) = taken {
                        self.note_live_param_write(&format!("deck_{deck_uuid}:{name}"), held);
                    }
                    CommandResult::Ok
                }
                Err(e) => e.into(),
            },
            EngineCommand::SetEffectParam {
                effect_uuid,
                name,
                value,
            } => match self.resolve_effect(&effect_uuid) {
                Ok(loc) => {
                    let mut taken = None;
                    if let Some(effect) = self.mixer.effect_at_mut(loc) {
                        taken = effect.params.normalize(&name, &value);
                        effect.params.set(&name, value);
                    }
                    if let Some(held) = taken {
                        self.note_live_param_write(&format!("fx_{effect_uuid}:{name}"), held);
                    }
                    CommandResult::Ok
                }
                Err(e) => e.into(),
            },
            EngineCommand::ResetGeneratorParamsToDefaults { deck_uuid } => {
                match self.resolve_deck(&deck_uuid) {
                    Ok((ch_idx, dk_idx)) => {
                        if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                            ch.decks[dk_idx].deck.generator_params.reset_to_defaults();
                        }
                        CommandResult::Ok
                    }
                    Err(e) => e.into(),
                }
            }
            EngineCommand::RandomizeGeneratorParams {
                deck_uuid,
                group,
                seed,
            } => match self.resolve_deck(&deck_uuid) {
                Ok((ch_idx, dk_idx)) => {
                    if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                        ch.decks[dk_idx]
                            .deck
                            .generator_params
                            .randomize(group.as_deref(), seed);
                    }
                    CommandResult::Ok
                }
                Err(e) => e.into(),
            },
            EngineCommand::MutateGeneratorParams {
                deck_uuid,
                group,
                amount,
                seed,
            } => match self.resolve_deck(&deck_uuid) {
                Ok((ch_idx, dk_idx)) => {
                    if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                        ch.decks[dk_idx].deck.generator_params.mutate(
                            group.as_deref(),
                            amount,
                            seed,
                        );
                    }
                    CommandResult::Ok
                }
                Err(e) => e.into(),
            },

            // ── Resolution ────────────────────────────────────────
            EngineCommand::SetRenderResolution { width, height } => {
                self.set_render_resolution(width, height);
                CommandResult::Ok
            }

            EngineCommand::SetDomemasterResolution { resolution } => {
                self.set_domemaster_resolution(resolution);
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

            // ── Presets ───────────────────────────────────────────
            EngineCommand::LoadDeckPreset {
                channel_uuid,
                preset_name,
            } => self.cmd_load_deck_preset(&channel_uuid, &preset_name),
            EngineCommand::LoadChannelPreset {
                target_channel_uuid,
                preset_name,
            } => self.cmd_load_channel_preset(target_channel_uuid.as_deref(), &preset_name),
            EngineCommand::SaveDeckPreset { deck_uuid, name } => {
                self.cmd_save_deck_preset(&deck_uuid, &name)
            }
            EngineCommand::SaveChannelPreset { channel_uuid, name } => {
                self.cmd_save_channel_preset(&channel_uuid, &name)
            }

            // ── Persistence ───────────────────────────────────────
            EngineCommand::SaveWorkspace => {
                // No layout travels with the command, so reuse the last one the
                // engine saw rather than writing defaults over the user's panels.
                let layout = self.session.last_layout.clone();
                match self.save_workspace(&layout) {
                    Ok(()) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::LoadWorkspace => match self.load_workspace().error_message() {
                None => CommandResult::Ok,
                Some(message) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message,
                },
            },

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

#[cfg(test)]
mod tests {
    use super::command_is_undoable;
    use crate::engine::EngineCommand as C;

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

    // ── Error → wire mapping (classify / wire / wire_id / not_found) ───
    //
    // These are pure functions with no GPU dependency: they translate engine
    // `anyhow::Result`s into the serializable `CommandResult`. The key contract
    // (api-addressing.md) is that an unresolvable UUID becomes `NotFound`, while
    // any other error is `InvalidInput`.

    use super::{classify, not_found, wire, wire_id, UnknownEntity};
    use crate::engine::ErrorCode;

    fn unknown() -> UnknownEntity {
        UnknownEntity {
            kind: "deck",
            uuid: "abc123".to_string(),
        }
    }

    #[test]
    fn classify_unknown_entity_is_not_found() {
        let err: anyhow::Error = unknown().into();
        assert_eq!(classify(&err), ErrorCode::NotFound);
    }

    #[test]
    fn classify_unknown_entity_survives_context_wrapping() {
        // Downcast must still find the UnknownEntity through an anyhow context.
        let err = anyhow::Error::from(unknown()).context("while applying command");
        assert_eq!(classify(&err), ErrorCode::NotFound);
    }

    #[test]
    fn classify_generic_error_is_invalid_input() {
        let err = anyhow::anyhow!("bad value");
        assert_eq!(classify(&err), ErrorCode::InvalidInput);
    }

    #[test]
    fn wire_ok_maps_to_ok() {
        assert!(matches!(wire(Ok(())), CommandResult::Ok));
    }

    #[test]
    fn wire_err_classifies_and_carries_message() {
        // Unresolvable UUID → NotFound, message preserved.
        match wire(Err(unknown().into())) {
            CommandResult::Err { code, message } => {
                assert_eq!(code, ErrorCode::NotFound);
                assert_eq!(message, "No deck with UUID 'abc123'");
            }
            other => panic!("expected Err, got {other:?}"),
        }
        // Generic error → InvalidInput.
        match wire(Err(anyhow::anyhow!("nope"))) {
            CommandResult::Err { code, message } => {
                assert_eq!(code, ErrorCode::InvalidInput);
                assert_eq!(message, "nope");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[test]
    fn wire_id_ok_carries_uuid() {
        match wire_id(Ok("deck-42".to_string())) {
            CommandResult::OkWithId { uuid } => assert_eq!(uuid, "deck-42"),
            other => panic!("expected OkWithId, got {other:?}"),
        }
    }

    #[test]
    fn wire_id_err_classifies() {
        match wire_id(Err(unknown().into())) {
            CommandResult::Err { code, .. } => assert_eq!(code, ErrorCode::NotFound),
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[test]
    fn not_found_always_maps_to_not_found_with_display_message() {
        match not_found(&unknown()) {
            CommandResult::Err { code, message } => {
                assert_eq!(code, ErrorCode::NotFound);
                assert_eq!(message, "No deck with UUID 'abc123'");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    // ── WS1: typed return channel (ui-engine-boundary.md) ──────────────
    //
    // These need a GPU adapter to build a real deck; they early-return when
    // none is available (CI / sandbox), matching the engine_impl.rs tests.

    use crate::engine::{CommandOutcome, CommandResult};

    fn headless_app() -> Option<super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let config = crate::testing::headless_config();
        super::VardaApp::new(gpu, &config).ok()
    }

    #[test]
    fn deck_add_command_returns_resolvable_uuid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let result = app.execute_command(C::AddSolidColorDeck {
            channel_uuid,
            color: [1.0, 0.0, 0.0, 1.0],
        });
        let CommandResult::OkWithId { uuid } = result else {
            panic!("expected OkWithId, got {result:?}");
        };
        assert!(
            app.mixer_ref().find_deck_by_uuid(&uuid).is_some(),
            "created deck must be findable by the returned uuid"
        );
    }

    #[test]
    fn gui_deck_add_reports_resolvable_uuid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let outcome = app.execute_command_gui(C::AddSolidColorDeck {
            channel_uuid,
            color: [0.0, 1.0, 0.0, 1.0],
        });
        let CommandOutcome::DecksCreated { uuids } = outcome else {
            panic!("expected DecksCreated, got {outcome:?}");
        };
        assert_eq!(uuids.len(), 1);
        let (channel_idx, deck_idx) = app
            .mixer_ref()
            .find_deck_by_uuid(&uuids[0])
            .expect("reported uuid must resolve to a deck");
        let slot_uuid = app.mixer_ref().channels()[channel_idx].decks[deck_idx]
            .deck
            .uuid()
            .to_string();
        assert_eq!(
            slot_uuid, uuids[0],
            "reported uuid must match the deck it resolves to"
        );
    }

    #[test]
    fn gui_undo_redo_roundtrips_a_structural_deck_add() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let layout = crate::usecases::ui::UILayoutState::default();
        // Runner records the pre-mutation snapshot, then mutates.
        let before = app.history_snapshot(&layout);
        app.push_history(before);
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        app.execute_command(C::AddSolidColorDeck {
            channel_uuid,
            color: [0.0, 0.0, 1.0, 1.0],
        });
        assert_eq!(app.mixer_ref().channels()[0].decks.len(), 1);

        let outcome = app.history_gui(&layout, true);
        let CommandOutcome::HistoryRestored {
            structural_changed, ..
        } = outcome
        else {
            panic!("expected HistoryRestored, got {outcome:?}");
        };
        assert!(structural_changed, "adding a deck is a structural change");
        assert_eq!(
            app.mixer_ref().channels()[0].decks.len(),
            0,
            "undo must remove the added deck"
        );

        let outcome = app.history_gui(&layout, false);
        assert!(matches!(outcome, CommandOutcome::HistoryRestored { .. }));
        assert_eq!(
            app.mixer_ref().channels()[0].decks.len(),
            1,
            "redo must restore the deck"
        );
    }

    #[test]
    fn gui_undo_on_empty_stack_is_plain_err() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let layout = crate::usecases::ui::UILayoutState::default();
        let outcome = app.history_gui(&layout, true);
        assert!(matches!(
            outcome,
            CommandOutcome::Plain(CommandResult::Err { .. })
        ));
    }

    // ── Parameter exploration ───────────────────────────────────

    /// Exploring is what the find-then-name loop does most of, and the "if it is
    /// not good" half of that loop is undo. See /spec/parameter-exploration.md.
    #[test]
    fn exploring_a_deck_is_an_undoable_edit() {
        for cmd in [
            C::RandomizeGeneratorParams {
                deck_uuid: "d0".into(),
                group: None,
                seed: 1,
            },
            C::MutateGeneratorParams {
                deck_uuid: "d0".into(),
                group: None,
                amount: 0.1,
                seed: 1,
            },
        ] {
            assert!(command_is_undoable(&cmd), "{cmd:?} must record history");
        }
    }

    /// And the whole loop over the bus: randomize moves the shader, undo puts it
    /// back. `plasma` declares one ranged float and two colours, so this also
    /// pins the colour exclusion — a randomize must not repaint the palette.
    #[test]
    fn randomizing_over_the_bus_is_one_undo_from_the_prior_look() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let CommandResult::OkWithId { uuid } = app.execute_command(C::AddDeck {
            channel_uuid,
            shader_name: "plasma".into(),
        }) else {
            return;
        };

        // Speed and the palette, the ranged float and an excluded colour.
        let look = |app: &super::VardaApp| {
            let (ch, dk) = app.mixer_ref().find_deck_by_uuid(&uuid).expect("the deck");
            let params = &app.mixer_ref().channels()[ch].decks[dk]
                .deck
                .generator_params;
            let color = match params.values.get("color1") {
                Some(crate::params::ParamValue::Color(c)) => Some(*c),
                _ => None,
            };
            (params.get_float("speed"), color)
        };
        let before = look(&app);

        app.command_sender()
            .send((
                C::RandomizeGeneratorParams {
                    deck_uuid: uuid.clone(),
                    group: None,
                    // Any seed but the one that happens to redraw 1.0.
                    seed: 0x5eed,
                },
                None,
            ))
            .expect("the receiver is in the app");
        app.process_commands();

        let after = look(&app);
        assert_ne!(after.0, before.0, "a ranged float is what randomize is for");
        assert_eq!(
            after.1, before.1,
            "colours are excluded: a palette is chosen, not stumbled upon"
        );

        let layout = crate::usecases::ui::UILayoutState::default();
        assert!(
            app.history_can_undo(),
            "one command, one history entry, so one undo undoes the whole draw"
        );
        app.history_gui(&layout, true);
        assert_eq!(
            look(&app).0,
            before.0,
            "undo must restore the look that was there before"
        );
    }

    // ── Timecode ────────────────────────────────────────────────

    /// Which signal to follow is a decision a headless rig makes over the bus,
    /// so the command has to land on the reader rather than only be accepted.
    #[test]
    fn choosing_a_timecode_signal_reaches_the_reader() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let result = app.execute_command(C::SetTimecodePreference {
            preference: crate::timecode::TimecodePreference::ForceMtc { device_id: 2 },
        });

        assert!(matches!(result, CommandResult::Ok), "got {result:?}");
        assert_eq!(
            app.input.timecode.preference(),
            crate::timecode::TimecodePreference::ForceMtc { device_id: 2 }
        );
        assert!(
            app.input.timecode.wants_mtc(2),
            "and the reader now parses that port"
        );
        assert!(!app.input.timecode.wants_mtc(3), "and only that port");
    }

    /// Patching LTC is the same journey, and unpatching it has to be a real
    /// value rather than a no-op, because that is what releases the interface.
    #[test]
    fn patching_and_unpatching_ltc_reaches_the_reader() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let patch = crate::timecode::LtcInput {
            source_id: 4,
            channel: 1,
            rate: None,
        };

        let result = app.execute_command(C::SetLtcInput { input: Some(patch) });

        assert!(matches!(result, CommandResult::Ok), "got {result:?}");
        assert_eq!(app.input.timecode.ltc_input(), Some(patch));
        assert!(app.input.timecode.wants_ltc());

        app.execute_command(C::SetLtcInput { input: None });

        assert_eq!(app.input.timecode.ltc_input(), None);
        assert!(!app.input.timecode.wants_ltc());
    }

    /// Which cable the show follows is live rig config, like the clock's.
    /// Undoing a deck edit must not silently re-patch the room mid-show.
    #[test]
    fn choosing_a_timecode_signal_is_not_undoable() {
        assert!(!command_is_undoable(&C::SetTimecodePreference {
            preference: crate::timecode::TimecodePreference::Off,
        }));
        assert!(!command_is_undoable(&C::SetLtcInput { input: None }));
        assert!(!command_is_undoable(&C::SetTransportSource {
            source: crate::transport::TransportSource::Timecode,
        }));
    }

    /// And the engine agrees: a patch sent over the bus leaves the undo timeline
    /// exactly as it found it.
    #[test]
    fn a_timecode_patch_over_the_bus_leaves_the_undo_stack_alone() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let tx = app.command_sender();
        tx.send((
            C::SetLtcInput {
                input: Some(crate::timecode::LtcInput {
                    source_id: 1,
                    channel: 0,
                    rate: None,
                }),
            },
            None,
        ))
        .expect("the receiver is in the app");
        tx.send((
            C::SetTimecodePreference {
                preference: crate::timecode::TimecodePreference::ForceLtc,
            },
            None,
        ))
        .expect("the receiver is in the app");
        app.process_commands();

        assert!(app.input.timecode.ltc_input().is_some(), "both landed");
        assert!(
            !app.history_can_undo(),
            "patching a cable is not an edit to the show"
        );
    }
}
