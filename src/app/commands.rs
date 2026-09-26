//! Cross-thread command dispatch for `VardaApp`.
//!
//! Houses `execute_command`, the exhaustive match over every `EngineCommand`
//! variant that cross-thread consumers (HTTP API, WebSocket, CLI) drive through
//! the command channel.

use super::VardaApp;
use super::resolve::UnknownEntity;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand, ErrorCode};

/// The canonical modulation key for a target a client sent, or the wire error
/// for one that names nothing modulation can drive. Accepts the pre-v8
/// `deck_<uuid>:<name>` spelling too, so existing API scripts keep working.
fn modulation_target(target: &str) -> Result<String, CommandResult> {
    crate::param_router::canonical_modulation_key(target).map_err(|e| CommandResult::Err {
        code: ErrorCode::InvalidInput,
        message: e.to_string(),
    })
}

/// Classify an engine error for the wire. An unresolvable UUID is `NotFound` —
/// the caller's view of the world is stale, which is distinct from a malformed
/// request. See [`/spec/api-addressing.md`].
fn classify(err: &anyhow::Error) -> ErrorCode {
    if err.downcast_ref::<UnknownEntity>().is_some()
        || err.downcast_ref::<crate::mixer::NoSuchStep>().is_some()
    {
        ErrorCode::NotFound
    } else {
        ErrorCode::InvalidInput
    }
}

/// Map a unit-returning engine call onto the wire result.
fn wire<E: Into<anyhow::Error>>(result: Result<(), E>) -> CommandResult {
    match result.map_err(Into::into) {
        Ok(()) => CommandResult::Ok,
        Err(e) => CommandResult::Err {
            code: classify(&e),
            message: e.to_string(),
        },
    }
}

/// Map an arrangement edit onto the wire result.
fn arranged(result: Result<(), crate::mixer::ArrangementError>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::Ok,
        Err(e) => e.into(),
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

/// Classify a parameter-routing failure for the wire. A path, entity, or param
/// name that does not resolve is `NotFound` (the caller's view is stale); a
/// value the resolved parameter cannot accept is `InvalidInput`.
fn param_route_error_code(err: &crate::param_router::ParamRouteError) -> ErrorCode {
    use crate::param_router::ParamRouteError as E;
    match err {
        E::UnknownPath { .. } | E::UnknownEntity { .. } | E::UnknownParam { .. } => {
            ErrorCode::NotFound
        }
        E::IndexOutOfRange { .. } | E::WrongState { .. } => ErrorCode::InvalidInput,
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

    /// True if any command in the batch is undoable. Used by the windowed
    /// runner to make one snapshot decision over the GUI's command stream,
    /// sharing the single [`command_is_undoable`] predicate with the bus
    /// consumers.
    // Method form is the engine-facing API used by `usecases/ui/runner.rs`.
    #[allow(clippy::unused_self)]
    pub(crate) fn batch_has_undoable(&self, cmds: &[EngineCommand]) -> bool {
        cmds.iter().any(super::classify::command_is_undoable)
    }

    /// Execute a single command and return the result.
    ///
    /// A command that writes a parameter as a live gesture is noted here,
    /// once, after it succeeds: the recorder captures it and the arrangement
    /// hands that parameter back to the performer.
    pub(crate) fn execute_command(&mut self, cmd: EngineCommand) -> CommandResult {
        let live = self.live_write(&cmd);
        let result = self.dispatch_command(cmd);
        if let Some((key, value)) = live
            && !matches!(result, CommandResult::Err { .. })
        {
            self.note_live_param_write(&key, value);
        }
        result
    }

    fn dispatch_command(&mut self, cmd: EngineCommand) -> CommandResult {
        use crate::modulation::ModulationSource;
        match cmd {
            // ── Mixer ────────────────────────────────────────
            EngineCommand::SetCrossfader(pos) => {
                self.set_crossfader(pos);
                CommandResult::Ok
            }
            EngineCommand::SetTonemapMode(mode) => {
                self.mixer
                    .set_tonemap_mode(&self.render.context.queue, mode);
                CommandResult::Ok
            }
            EngineCommand::LoadLut { filename } => match self.load_lut(&filename) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::LoadLookLut { filename } => match self.load_look_lut(&filename) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::UnloadLookLut => {
                self.mixer.clear_look_lut();
                CommandResult::Ok
            }
            EngineCommand::UnloadLut => {
                self.mixer.unload_lut();
                CommandResult::Ok
            }
            EngineCommand::AutoCrossfade {
                target,
                duration_secs,
                easing,
            } => {
                self.mixer.start_crossfade(target, duration_secs, easing);
                CommandResult::Ok
            }
            EngineCommand::BeatCrossfade { target, beats } => {
                self.mixer.start_beat_crossfade(target, beats);
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
                wire_id(self.add_screen_capture_deck(&channel_uuid, &target, &options))
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
            } => wire(self.mixer.move_deck(&deck_uuid, &dst_channel_uuid)),
            EngineCommand::ReorderDeck {
                channel_uuid,
                from_idx,
                to_idx,
            } => wire(self.mixer.reorder_deck(&channel_uuid, from_idx, to_idx)),
            EngineCommand::SetDeckOpacity { deck_uuid, opacity } => {
                wire(self.mixer.set_deck_opacity(&deck_uuid, opacity))
            }
            EngineCommand::SetDeckBlendMode { deck_uuid, mode } => {
                wire(self.mixer.set_deck_blend_mode(&deck_uuid, mode))
            }
            EngineCommand::SetDeckSolo { deck_uuid, solo } => {
                wire(self.mixer.set_deck_solo(&deck_uuid, solo))
            }
            EngineCommand::SetDeckMute { deck_uuid, mute } => {
                wire(self.mixer.set_deck_mute(&deck_uuid, mute))
            }
            EngineCommand::SetDeckRenderFps {
                deck_uuid,
                render_fps,
            } => match self.mixer.resolve_deck(&deck_uuid) {
                Ok((ch, dk)) => {
                    self.mixer.channels_mut()[ch].decks[dk].render_fps = render_fps;
                    CommandResult::Ok
                }
                Err(e) => not_found(&e),
            },
            EngineCommand::SetDeckScalingMode { deck_uuid, mode } => {
                wire(self.mixer.set_deck_scaling_mode(&deck_uuid, mode))
            }
            EngineCommand::SetDeckTransparent {
                deck_uuid,
                transparent,
            } => wire(self.mixer.set_deck_transparent(&deck_uuid, transparent)),
            EngineCommand::SetChannelOpacity {
                channel_uuid,
                opacity,
            } => wire(self.mixer.set_channel_opacity(&channel_uuid, opacity)),
            EngineCommand::SetChannelBlendMode { channel_uuid, mode } => {
                wire(self.mixer.set_channel_blend_mode(&channel_uuid, mode))
            }
            EngineCommand::AddChannel => wire_id(self.add_channel()),
            EngineCommand::RemoveChannel { channel_uuid } => {
                wire(self.remove_channel(&channel_uuid))
            }
            EngineCommand::AddEffect {
                target,
                shader_name,
            } => wire_id(self.add_effect(&target, &shader_name)),
            EngineCommand::RemoveEffect { effect_uuid } => wire(self.remove_effect(&effect_uuid)),
            EngineCommand::ToggleEffect { effect_uuid } => {
                wire(self.mixer.toggle_effect(&effect_uuid))
            }
            EngineCommand::MoveEffect {
                target,
                from_idx,
                to_idx,
            } => wire(self.mixer.move_effect(&target, from_idx, to_idx)),

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
            EngineCommand::SetParam { path, value } => match self.set_param(&path, value) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: param_route_error_code(&e),
                    message: e.to_string(),
                },
            },
            EngineCommand::ToggleParam { path } => {
                if let Some(cmd) = super::inputs::surface_command(&path, self.interactive_deck()) {
                    return self.execute_command(cmd);
                }
                if let Err(e) = crate::param_router::toggle_param_by_path(&mut self.mixer, &path) {
                    log::debug!("ToggleParam {path}: {e}");
                }
                CommandResult::Ok
            }

            // ── Audio ────────────────────────────────────────
            EngineCommand::OpenAudioSource { source_id } => {
                match self
                    .audio
                    .manager
                    .open_source(source_id)
                    .map_err(|e| anyhow::anyhow!("Failed to open audio source: {e}"))
                {
                    Ok(()) => CommandResult::Ok,
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::CloseAudioSource { source_id } => {
                self.audio.manager.close_source(source_id);
                CommandResult::Ok
            }
            EngineCommand::ScanAudioDevices | EngineCommand::RescanAudio => {
                self.audio.manager.scan_devices();
                CommandResult::Ok
            }

            // ── Modulation ───────────────────────────────────
            EngineCommand::AddLfo {
                waveform,
                frequency,
            } => {
                self.mixer
                    .modulation_mut()
                    .add_source(ModulationSource::lfo(waveform, frequency));
                CommandResult::Ok
            }
            EngineCommand::AddAudioBand { preset, source_id } => {
                // Capture is reconciled per-frame from modulator demand
                // (see /spec/audio-capture-lifecycle.md); adding the band is enough.
                self.mixer
                    .modulation_mut()
                    .add_source(ModulationSource::audio_from_preset(preset, source_id));
                CommandResult::Ok
            }
            EngineCommand::AddAdsr {
                attack,
                decay,
                sustain,
                release,
            } => {
                self.mixer
                    .modulation_mut()
                    .add_source(ModulationSource::adsr(attack, decay, sustain, release));
                CommandResult::Ok
            }
            EngineCommand::AddStepSequencer { num_steps, rate } => {
                self.mixer
                    .modulation_mut()
                    .add_source(ModulationSource::step_sequencer(num_steps, rate));
                CommandResult::Ok
            }
            EngineCommand::AddAutomationLane { target, timebase } => {
                let target = match modulation_target(&target) {
                    Ok(target) => target,
                    Err(e) => return e,
                };
                // Returns the UUID because the caller needs it to reveal the
                // new lane and to push breakpoints into it.
                CommandResult::OkWithId {
                    uuid: self
                        .mixer
                        .modulation_mut()
                        .add_automation_lane(&target, timebase),
                }
            }
            EngineCommand::SetEnvelopeBreakpoints { uuid, breakpoints } => {
                if self
                    .mixer
                    .modulation_mut()
                    .set_envelope_breakpoints(&uuid, breakpoints)
                {
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: format!("no automation envelope with uuid '{uuid}'"),
                    }
                }
            }
            EngineCommand::RemoveModulationSource { uuid } => {
                self.mixer.modulation_mut().remove_source(&uuid);
                CommandResult::Ok
            }
            EngineCommand::AssignModulation {
                target,
                source_id,
                amount,
            } => match modulation_target(&target) {
                Ok(target) => {
                    self.mixer
                        .modulation_mut()
                        .assign(&target, &source_id, amount, None);
                    CommandResult::Ok
                }
                Err(e) => e,
            },
            EngineCommand::ClearModulation { target } => match modulation_target(&target) {
                Ok(target) => {
                    self.mixer.modulation_mut().clear_assignments(&target);
                    CommandResult::Ok
                }
                Err(e) => e,
            },
            EngineCommand::ClearModulationSource { target, source_id } => {
                match modulation_target(&target) {
                    Ok(target) => {
                        self.mixer
                            .modulation_mut()
                            .clear_assignment_source(&target, &source_id);
                        CommandResult::Ok
                    }
                    Err(e) => e,
                }
            }

            // ── Output ───────────────────────────────────────
            EngineCommand::CreateOutput => {
                self.output.request_create_output();
                CommandResult::Ok
            }
            EngineCommand::CloseOutput { output_uuid } => {
                match self.output.close_output(&output_uuid) {
                    Ok(passthrough) => {
                        if let Some(pass) = passthrough {
                            self.audio
                                .manager
                                .unsubscribe_pcm(pass.source_id, pass.token);
                        }
                        CommandResult::Ok
                    }
                    Err(e) => wire(Err::<(), _>(e)),
                }
            }
            EngineCommand::SetOutputDisplay {
                output_uuid,
                monitor_name,
            } => wire(self.output.set_output_display(&output_uuid, &monitor_name)),
            EngineCommand::SetOutputTarget {
                output_uuid,
                target,
            } => match self.output.resolve_output(&output_uuid) {
                Ok(idx) => self.cmd_set_output_target(idx, target),
                Err(e) => not_found(&e),
            },

            // ── Surfaces ────────────────────────────────────
            EngineCommand::AddSurface { name, source } => {
                self.output.surface_manager.add_surface(name, source);
                CommandResult::Ok
            }
            EngineCommand::AddPolygonSurface {
                name,
                vertices,
                source,
            } => {
                self.output
                    .surface_manager
                    .add_polygon_surface(name, vertices, source);
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
                let hint = crate::surface::CircleHint {
                    center,
                    radius,
                    sides,
                    aspect_ratio,
                };
                self.output
                    .surface_manager
                    .add_circle_surface(name, hint, source);
                CommandResult::Ok
            }
            EngineCommand::RemoveSurface { uuid } => self.output.cmd_remove_surface(&uuid),
            EngineCommand::ReorderSurface { uuid, op } => {
                self.output.cmd_reorder_surface(&uuid, op)
            }
            EngineCommand::SetSurfaceSource { uuid, source } => {
                self.output.surface_manager.set_source(&uuid, source);
                self.output.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::SetSurfaceOutputType { uuid, output_type } => {
                self.output
                    .surface_manager
                    .set_output_type(&uuid, output_type);
                CommandResult::Ok
            }
            EngineCommand::SetSurfaceContentMapping { uuid, mapping } => {
                self.output
                    .surface_manager
                    .set_content_mapping(&uuid, mapping);
                self.output.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::RenameSurface { uuid, name } => {
                self.output.surface_manager.rename(&uuid, &name);
                CommandResult::Ok
            }
            EngineCommand::UpdateSurfaceVertices { uuid, vertices } => self
                .output
                .reshape_surface(&uuid, |s| s.vertices = vertices),
            EngineCommand::DuplicateSurface { uuid } => self.output.cmd_duplicate_surface(&uuid),
            EngineCommand::FlipSurfaceHorizontal { uuid } => self
                .output
                .edit_surface(&uuid, crate::surface::Surface::flip_horizontal),
            EngineCommand::FlipSurfaceVertical { uuid } => self
                .output
                .edit_surface(&uuid, crate::surface::Surface::flip_vertical),
            EngineCommand::InsertSurfaceVertex {
                uuid,
                after_vert_idx,
                position,
            } => self
                .output
                .edit_surface(&uuid, |s| s.insert_vertex(after_vert_idx, position)),
            EngineCommand::SetCircleRadius { uuid, radius } => self
                .output
                .edit_surface(&uuid, |s| s.set_circle_radius(radius)),
            EngineCommand::SetCircleSides { uuid, sides } => self
                .output
                .edit_surface(&uuid, |s| s.set_circle_sides(sides)),
            EngineCommand::ConvertSurfaceToPolygon { uuid } => self
                .output
                .edit_surface(&uuid, crate::surface::Surface::convert_to_polygon),
            EngineCommand::CombineSurfaces { uuids } => self.output.cmd_combine_surfaces(&uuids),
            EngineCommand::MoveSurface { uuid, dx, dy } => {
                self.output.reshape_surface(&uuid, |s| s.move_by(dx, dy))
            }
            EngineCommand::RotateSurface { uuid, angle, pivot } => self
                .output
                .reshape_surface(&uuid, |s| s.rotate(angle, pivot)),
            EngineCommand::ScaleSurface {
                uuid,
                sx,
                sy,
                pivot,
            } => self
                .output
                .reshape_surface(&uuid, |s| s.scale(sx, sy, pivot)),
            EngineCommand::UpdateSurfaceContourVertices {
                uuid,
                contour,
                vertices,
            } => self
                .output
                .edit_surface(&uuid, |s| s.set_contour_vertices(contour, vertices)),
            EngineCommand::ConvertSurfaceEdge {
                uuid,
                edge_idx,
                to_cubic,
            } => self
                .output
                .reshape_surface(&uuid, |s| s.convert_edge(edge_idx, to_cubic)),
            EngineCommand::MovePathAnchor {
                uuid,
                anchor_idx,
                pos,
            } => self
                .output
                .reshape_surface(&uuid, |s| s.move_path_anchor(anchor_idx, pos)),
            EngineCommand::MovePathHandle {
                uuid,
                segment_idx,
                handle,
                pos,
            } => self
                .output
                .reshape_surface(&uuid, |s| s.move_path_handle(segment_idx, handle, pos)),
            EngineCommand::AddSurfaceHole { uuid, hole } => {
                self.output.reshape_surface(&uuid, |s| s.add_hole(hole))
            }
            EngineCommand::RemoveSurfaceHole { uuid, hole_index } => {
                self.output.cmd_remove_surface_hole(&uuid, hole_index)
            }
            EngineCommand::PunchSurfaceHole { source_uuid } => {
                self.output.cmd_punch_surface_hole(&source_uuid)
            }
            EngineCommand::AssignSurfaceToOutput {
                output_uuid,
                surface_uuid,
            } => {
                self.output
                    .assign_surface_to_output(&output_uuid, &surface_uuid);
                self.output.recompute_auto_edge_blend();
                CommandResult::Ok
            }
            EngineCommand::UnassignSurfaceFromOutput {
                output_uuid,
                surface_uuid,
            } => {
                self.output
                    .unassign_surface_from_output(&output_uuid, &surface_uuid);
                self.output.recompute_auto_edge_blend();
                CommandResult::Ok
            }

            // ── Surface Auto-Detection ────────────────────────
            EngineCommand::DetectFromImage { image_data, params } => {
                match crate::surface::import::detect_from_image(&image_data, &params) {
                    Ok(result) => CommandResult::OkWithData {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    },
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::DetectFromSvg { svg_data } => {
                match crate::surface::import::detect_from_svg(&svg_data) {
                    Ok(result) => CommandResult::OkWithData {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    },
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::DetectFromDxf { dxf_data } => {
                match crate::surface::import::detect_from_dxf(&dxf_data) {
                    Ok(result) => CommandResult::OkWithData {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    },
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: e.to_string(),
                    },
                }
            }
            EngineCommand::ConfirmDetectedContours { contours } => {
                let uuids = self.output.surface_manager.add_detected(&contours);
                CommandResult::OkWithData {
                    data: serde_json::json!({ "surface_uuids": uuids }),
                }
            }
            EngineCommand::ImportSurfacesFromFile { path } => {
                let params = crate::surface::detect::DetectionParams::default();
                match crate::surface::import::detect_from_file(&path, &params) {
                    Ok(result) => {
                        let uuids = self.output.surface_manager.add_detected(&result.contours);
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
                        let uuids = self.output.surface_manager.add_detected(&result.contours);
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
            EngineCommand::VideoTogglePlay { deck_uuid } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_toggle_play())
            }
            EngineCommand::VideoSeek {
                deck_uuid,
                position_secs,
            } => self.exec_on_deck(&deck_uuid, |d| d.video_seek(position_secs)),
            EngineCommand::VideoSetSpeed { deck_uuid, speed } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_set_speed(speed))
            }
            EngineCommand::VideoSetLoopMode { deck_uuid, mode } => {
                self.exec_on_deck(&deck_uuid, |d| d.video_set_loop_mode(mode))
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
                let (ch_idx, deck_idx) = match self.mixer.resolve_deck(&deck_uuid) {
                    Ok(loc) => loc,
                    Err(e) => return not_found(&e),
                };
                let shader = shader_name.as_ref().and_then(|name| {
                    self.sources
                        .registry
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
                        let _ = slot.set_transition_shader(&self.render.context, shader);
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
            EngineCommand::AddSpoutDeck {
                channel_uuid,
                sender_name,
            } => self.cmd_add_spout_deck(&channel_uuid, &sender_name),
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
            EngineCommand::CreateSequence => CommandResult::OkWithId {
                uuid: self.mixer.create_sequence(),
            },
            EngineCommand::DeleteSequence { sequence_uuid } => {
                wire(self.mixer.delete_sequence(&sequence_uuid))
            }
            EngineCommand::PlaySequence { sequence_uuid } => self.cmd_play_sequence(&sequence_uuid),
            EngineCommand::StopSequence { sequence_uuid } => {
                wire(self.mixer.stop_sequence(&sequence_uuid))
            }
            EngineCommand::ToggleSequence { sequence_uuid } => {
                wire(self.mixer.toggle_sequence(&sequence_uuid))
            }
            EngineCommand::AddFadeStep {
                sequence_uuid,
                from_channel_uuid,
                to_channel_uuid,
            } => wire(self.mixer.add_fade_step(
                &sequence_uuid,
                &from_channel_uuid,
                &to_channel_uuid,
            )),
            EngineCommand::AddWaitStep { sequence_uuid } => {
                wire(self.mixer.add_wait_step(&sequence_uuid))
            }
            EngineCommand::AddGoToStep {
                sequence_uuid,
                step_index,
            } => wire(self.mixer.add_goto_step(&sequence_uuid, step_index)),
            EngineCommand::RemoveStep {
                sequence_uuid,
                step_idx,
            } => wire(self.mixer.remove_step(&sequence_uuid, step_idx)),
            EngineCommand::SetStepDuration {
                sequence_uuid,
                step_idx,
                value,
                unit,
            } => wire(
                self.mixer
                    .set_step_duration(&sequence_uuid, step_idx, value, unit),
            ),
            EngineCommand::SetStepEasing {
                sequence_uuid,
                step_idx,
                easing,
            } => wire(
                self.mixer
                    .set_step_easing(&sequence_uuid, step_idx, &easing),
            ),
            EngineCommand::SetStepTransitionShader {
                sequence_uuid,
                step_idx,
                shader_name,
            } => wire(
                self.mixer
                    .set_step_transition_shader(&sequence_uuid, step_idx, shader_name),
            ),
            EngineCommand::MoveStep {
                sequence_uuid,
                from,
                to,
            } => wire(self.mixer.move_step(&sequence_uuid, from, to)),
            EngineCommand::SetStepDurationUnit {
                sequence_uuid,
                step_idx,
                unit,
            } => wire(
                self.mixer
                    .set_step_duration_unit(&sequence_uuid, step_idx, unit),
            ),
            EngineCommand::ToggleStepDurationUnit {
                sequence_uuid,
                step_idx,
            } => wire(
                self.mixer
                    .toggle_step_duration_unit(&sequence_uuid, step_idx),
            ),
            EngineCommand::SetStepDurationValue {
                sequence_uuid,
                step_idx,
                value,
            } => wire(
                self.mixer
                    .set_step_duration_value(&sequence_uuid, step_idx, value),
            ),
            EngineCommand::SetStepFromCh {
                sequence_uuid,
                step_idx,
                channel_uuid,
            } => wire(
                self.mixer
                    .set_step_from_channel(&sequence_uuid, step_idx, channel_uuid),
            ),
            EngineCommand::SetStepToCh {
                sequence_uuid,
                step_idx,
                channel_uuid,
            } => wire(
                self.mixer
                    .set_step_to_channel(&sequence_uuid, step_idx, channel_uuid),
            ),
            EngineCommand::SetGoToTarget {
                sequence_uuid,
                step_idx,
                target,
            } => wire(self.mixer.set_goto_target(&sequence_uuid, step_idx, target)),
            EngineCommand::SetStepTargetAmount {
                sequence_uuid,
                step_idx,
                amount,
            } => wire(
                self.mixer
                    .set_step_target_amount(&sequence_uuid, step_idx, amount),
            ),

            // ── Stream Library ─────────────────────────────────
            EngineCommand::AddStreamLibraryEntry { url, mode } => {
                self.sources.io.cmd_add_stream_library_entry(url, mode)
            }
            EngineCommand::RemoveStreamLibraryEntry { url } => {
                self.sources.io.cmd_remove_stream_library_entry(&url)
            }
            EngineCommand::AddHlsLibraryEntry { url } => {
                self.sources.io.cmd_add_hls_library_entry(url)
            }
            EngineCommand::RemoveHlsLibraryEntry { url } => {
                self.sources.io.cmd_remove_hls_library_entry(&url)
            }
            EngineCommand::AddDashLibraryEntry { url } => {
                self.sources.io.cmd_add_dash_library_entry(url)
            }
            EngineCommand::RemoveDashLibraryEntry { url } => {
                self.sources.io.cmd_remove_dash_library_entry(&url)
            }
            EngineCommand::AddRtmpLibraryEntry { url, mode } => {
                self.sources.io.cmd_add_rtmp_library_entry(url, mode)
            }
            EngineCommand::RemoveRtmpLibraryEntry { url } => {
                self.sources.io.cmd_remove_rtmp_library_entry(&url)
            }
            EngineCommand::AddHtmlLibraryEntry { url } => {
                self.sources.io.cmd_add_html_library_entry(url)
            }
            EngineCommand::RemoveHtmlLibraryEntry { url } => {
                self.sources.io.cmd_remove_html_library_entry(&url)
            }

            // ── Output Management ─────────────────────────────────
            EngineCommand::CreateHeadlessOutput { target } => {
                self.cmd_create_headless_output(target)
            }
            EngineCommand::StartOutput { output_uuid } => self.cmd_start_output(&output_uuid),
            EngineCommand::StopOutput { output_uuid } => self.cmd_stop_output(&output_uuid),
            EngineCommand::SetCalibrationMode { output_uuid, mode } => {
                self.output.cmd_set_calibration_mode(&output_uuid, mode)
            }
            EngineCommand::SetWarpCorner {
                surface_uuid,
                corner_idx,
                position,
            } => self
                .output
                .edit_surface(&surface_uuid, |s| s.set_warp_corner(corner_idx, position)),
            EngineCommand::ResetWarp { surface_uuid } => self
                .output
                .edit_surface(&surface_uuid, crate::surface::Surface::reset_warp),
            EngineCommand::SetWarpSubdivisions {
                surface_uuid,
                cols,
                rows,
            } => self
                .output
                .edit_surface(&surface_uuid, |s| s.set_warp_subdivisions(cols, rows)),
            EngineCommand::SetWarpMeshPoint {
                surface_uuid,
                row,
                col,
                position,
            } => self
                .output
                .edit_surface(&surface_uuid, |s| s.set_warp_mesh_point(row, col, position)),
            EngineCommand::SetWarpBound {
                surface_uuid,
                bound,
            } => self
                .output
                .edit_surface(&surface_uuid, |s| s.set_warp_bound(bound)),
            EngineCommand::ConvertWarpToBezier { surface_uuid } => self.output.edit_surface(
                &surface_uuid,
                crate::surface::Surface::convert_warp_to_bezier,
            ),
            EngineCommand::MoveWarpAnchor {
                surface_uuid,
                row,
                col,
                position,
            } => self.output.edit_surface(&surface_uuid, |s| {
                s.set_warp_bezier_anchor(row, col, position);
            }),
            EngineCommand::MoveWarpHandle {
                surface_uuid,
                horizontal,
                row,
                col,
                which,
                position,
            } => self.output.edit_surface(&surface_uuid, |s| {
                s.set_warp_bezier_handle(horizontal, row, col, which, position);
            }),
            EngineCommand::SetBezierCageSubdivisions {
                surface_uuid,
                cols,
                rows,
            } => self.output.edit_surface(&surface_uuid, |s| {
                s.set_bezier_cage_subdivisions(cols, rows);
            }),
            EngineCommand::SetEdgeBlend {
                output_uuid,
                config,
            } => self.output.cmd_set_edge_blend(&output_uuid, config),
            EngineCommand::SetEdgeBlendMode { output_uuid, mode } => {
                self.output.cmd_set_edge_blend_mode(&output_uuid, mode)
            }
            EngineCommand::SetOutputRotation {
                output_uuid,
                rotation,
            } => self.cmd_set_output_rotation(&output_uuid, rotation),
            EngineCommand::SetOutputPresentation {
                output_uuid,
                request,
            } => self.cmd_set_output_presentation(&output_uuid, request),
            EngineCommand::SetOutputTonemap {
                output_uuid,
                tonemap,
            } => self.output.cmd_set_output_tonemap(&output_uuid, tonemap),

            // ── Modulation Updates ────────────────────────────────
            EngineCommand::UpdateLfoFrequency { uuid, frequency } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { frequency: f, .. } = s {
                        *f = frequency;
                    }
                })
            }
            EngineCommand::TransportPlay => match self.show.transport.play() {
                Ok(()) => CommandResult::Ok,
                Err(e) => transport_rejected(e),
            },
            EngineCommand::TransportStop => {
                self.show.transport.stop();
                // A second stop returns to zero, which is a move the cue walk
                // did not make and must not keep stepping from.
                self.show.cue_anchor = None;
                CommandResult::Ok
            }
            EngineCommand::TransportLocate { position } => {
                match self.show.transport.locate(position) {
                    Ok(()) => {
                        self.show.cue_anchor = None;
                        CommandResult::Ok
                    }
                    Err(e) => transport_rejected(e),
                }
            }
            EngineCommand::SetTransportSource { source } => {
                self.show.transport.set_source(source);
                self.show.cue_anchor = None;
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
                self.show.transport.set_loop_region(checked);
                CommandResult::Ok
            }
            EngineCommand::SetTimecodeRate { rate } => {
                self.show.transport.set_timecode_rate(rate);
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
            EngineCommand::AddLane { deck_uuid } => arranged(self.mixer.add_lane(&deck_uuid)),
            EngineCommand::RemoveLane { deck_uuid } => arranged(self.mixer.remove_lane(&deck_uuid)),
            EngineCommand::AddRegion { deck_uuid, region } => {
                match self.mixer.add_region(&deck_uuid, region) {
                    Ok(index) => CommandResult::OkWithData {
                        data: serde_json::json!({ "index": index }),
                    },
                    Err(e) => e.into(),
                }
            }
            EngineCommand::UpdateRegion {
                deck_uuid,
                index,
                region,
            } => arranged(self.mixer.update_region(&deck_uuid, index, region)),
            EngineCommand::RemoveRegion { deck_uuid, index } => {
                arranged(self.mixer.remove_region(&deck_uuid, index))
            }
            EngineCommand::SetLaneCollapsed {
                deck_uuid,
                collapsed,
            } => arranged(self.mixer.set_lane_collapsed(&deck_uuid, collapsed)),
            EngineCommand::SetIdleBehaviour { idle } => {
                arranged(self.mixer.set_idle_behaviour(idle))
            }
            EngineCommand::RearmParam { param_key, seconds } => {
                match modulation_target(&param_key) {
                    Ok(param_key) => {
                        self.mixer.modulation_mut().rearm_param(
                            &param_key,
                            seconds.unwrap_or(crate::arrangement::DEFAULT_REARM_SECONDS),
                        );
                        CommandResult::Ok
                    }
                    Err(e) => e,
                }
            }
            EngineCommand::RearmAll { seconds } => {
                self.mixer
                    .modulation_mut()
                    .rearm_all(seconds.unwrap_or(crate::arrangement::DEFAULT_REARM_SECONDS));
                CommandResult::Ok
            }
            EngineCommand::AddCue { at, name } => match self.mixer.add_cue(at, &name) {
                Ok(uuid) => CommandResult::OkWithId { uuid },
                Err(e) => e.into(),
            },
            EngineCommand::UpdateCue { uuid, at, name } => {
                arranged(self.mixer.update_cue(&uuid, at, name))
            }
            EngineCommand::RemoveCue { uuid } => arranged(self.mixer.remove_cue(&uuid)),
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
                    if let ModulationSource::LFO { waveform: w, .. } = s {
                        *w = waveform;
                    }
                })
            }
            EngineCommand::UpdateLfoPhase { uuid, phase } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { phase: p, .. } = s {
                        *p = phase;
                    }
                })
            }
            EngineCommand::UpdateLfoAmplitude { uuid, amplitude } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { amplitude: a, .. } = s {
                        *a = amplitude;
                    }
                })
            }
            EngineCommand::UpdateLfoBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::LFO { bipolar: b, .. } = s {
                        *b = bipolar;
                    }
                })
            }
            EngineCommand::UpdateAudioSmoothing { uuid, smoothing } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { smoothing: sm, .. } = s {
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
                    freq_low: fl,
                    freq_high: fh,
                    ..
                } = s
                {
                    *fl = freq_low;
                    *fh = freq_high;
                }
            }),
            EngineCommand::UpdateAudioGain { uuid, gain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { gain: g, .. } = s {
                        *g = gain;
                    }
                })
            }
            EngineCommand::UpdateAudioPreset { uuid, preset } => {
                let (lo, hi) = preset.freq_range();
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand {
                        freq_low: fl,
                        freq_high: fh,
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
                    if let ModulationSource::AudioBand { mode: m, .. } = s {
                        *m = mode;
                    }
                })
            }
            EngineCommand::UpdateAdsrAttack { uuid, attack } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { attack: a, .. } = s {
                        *a = attack;
                    }
                })
            }
            EngineCommand::UpdateAdsrDecay { uuid, decay } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { decay: d, .. } = s {
                        *d = decay;
                    }
                })
            }
            EngineCommand::UpdateAdsrSustain { uuid, sustain } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { sustain: su, .. } = s {
                        *su = sustain;
                    }
                })
            }
            EngineCommand::UpdateAdsrRelease { uuid, release } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::ADSR { release: r, .. } = s {
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
                    if let ModulationSource::StepSequencer { steps: st, .. } = s {
                        *st = steps;
                    }
                })
            }
            EngineCommand::UpdateStepSeqRate { uuid, rate } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { rate: r, .. } = s {
                        *r = rate;
                    }
                })
            }
            EngineCommand::UpdateStepSeqInterpolation {
                uuid,
                interpolation,
            } => self.exec_modulation_update(&uuid, |s| {
                if let ModulationSource::StepSequencer {
                    interpolation: i, ..
                } = s
                {
                    *i = interpolation;
                }
            }),
            EngineCommand::UpdateStepSeqBipolar { uuid, bipolar } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::StepSequencer { bipolar: b, .. } = s {
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
                if let ModulationSource::StepSequencer { steps, .. } = s
                    && step_idx < steps.len()
                {
                    steps[step_idx] = value;
                }
            }),
            EngineCommand::UpdateAudioFreqLow { uuid, freq_low } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { freq_low: fl, .. } = s {
                        *fl = freq_low;
                    }
                })
            }
            EngineCommand::UpdateAudioFreqHigh { uuid, freq_high } => {
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { freq_high: fh, .. } = s {
                        *fh = freq_high;
                    }
                })
            }
            EngineCommand::UpdateAudioSource { uuid, source_id } => {
                // Switching device just updates the modulator; the per-frame
                // reconcile opens the new device and closes the old one when it is
                // no longer referenced (see /spec/audio-capture-lifecycle.md).
                self.exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { source_id: sid, .. } = s {
                        *sid = source_id;
                    }
                })
            }
            EngineCommand::UpdateAudioNoiseGate { uuid, noise_gate } => self
                .exec_modulation_update(&uuid, |s| {
                    if let ModulationSource::AudioBand { noise_gate: ng, .. } = s {
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
                let uuid = self.mixer.macros_mut().add_macro(kind);
                CommandResult::OkWithId { uuid }
            }
            EngineCommand::RemoveMacro { uuid } => {
                self.mixer.macros_mut().remove_macro(&uuid);
                CommandResult::Ok
            }
            EngineCommand::RenameMacro { uuid, name } => {
                self.mixer.macros_mut().rename(&uuid, &name);
                CommandResult::Ok
            }
            EngineCommand::SetMacroKind { uuid, kind } => {
                self.mixer.macros_mut().set_kind(&uuid, kind);
                CommandResult::Ok
            }
            EngineCommand::SetMacroValue { uuid, value } => {
                self.set_macro_value(&uuid, value);
                CommandResult::Ok
            }
            EngineCommand::AddMacroTarget { uuid, path } => {
                self.mixer.macros_mut().add_target(&uuid, &path);
                CommandResult::Ok
            }
            EngineCommand::RemoveMacroTarget { uuid, target_idx } => {
                self.mixer.macros_mut().remove_target(&uuid, target_idx);
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
                self.mixer
                    .macros_mut()
                    .update_target(&uuid, target_idx, min, max, curve, invert);
                CommandResult::Ok
            }
            EngineCommand::SetMacroButtonBehavior { uuid, behavior } => {
                self.mixer.macros_mut().set_button_behavior(&uuid, behavior);
                CommandResult::Ok
            }
            EngineCommand::SetMacroTriggers { uuid, actions } => {
                self.mixer.macros_mut().set_triggers(&uuid, actions);
                CommandResult::Ok
            }

            // ── Analyzers ────────────────────────────────────────
            EngineCommand::RequestAnalyzer {
                deck_id,
                analyzer_type,
                options,
            } => match self.mixer.request_analyzer(
                &deck_id,
                &analyzer_type,
                &self.sources.analyzer_registry,
                &options,
            ) {
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
                self.mixer.release_analyzer(&deck_id, &analyzer_type);
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
                self.sources.io.ndi_manager.discover();
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
                    self.sources.io.syphon_manager.discover();
                    let names = self.sources.io.syphon_manager.discovered_sources();
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
            EngineCommand::RescanSpout => {
                // Same contract as RescanSyphon: discover inline and answer with
                // the fresh list, so a probe is one non-racy call. No platform
                // gate is needed because discovery is a no-op off Windows.
                self.sources.io.spout_manager.discover();
                CommandResult::OkWithData {
                    data: serde_json::json!(self.sources.io.spout_manager.discovered_sources()),
                }
            }
            EngineCommand::RescanCameras => {
                self.sources.camera_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::RescanDepthSensors => {
                self.sources.depth_manager.scan_devices();
                CommandResult::Ok
            }
            EngineCommand::RescanCaptureTargets => {
                self.sources.screen_capture_manager.scan_targets();
                CommandResult::Ok
            }
            EngineCommand::RequestScreenCapturePermission => {
                self.sources.screen_capture_manager.request_permission();
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
            EngineCommand::ToggleAudioSource { source_id, enabled } => {
                if enabled {
                    if let Err(e) = self.audio.manager.open_source(source_id) {
                        log::warn!("Failed to open audio source {source_id}: {e}");
                        return CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: format!("Failed to open audio source: {e}"),
                        };
                    }
                } else {
                    self.audio.manager.close_source(source_id);
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
            } => wire(self.mixer.set_generator_param(&deck_uuid, &name, value)),
            EngineCommand::SetEffectParam {
                effect_uuid,
                name,
                value,
            } => wire(self.mixer.set_effect_param(&effect_uuid, &name, value)),
            EngineCommand::ResetGeneratorParamsToDefaults { deck_uuid } => {
                match self.mixer.resolve_deck(&deck_uuid) {
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
            } => match self.mixer.resolve_deck(&deck_uuid) {
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
            } => match self.mixer.resolve_deck(&deck_uuid) {
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
            EngineCommand::SetDomePreset { preset } => {
                self.output.dome.preset = preset;
                CommandResult::Ok
            }
            EngineCommand::SetDomeGeometry { geometry } => {
                self.output.dome.geometry = geometry;
                CommandResult::Ok
            }
            EngineCommand::SetEditorPrefs { prefs } => {
                self.session.editor_prefs = prefs;
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

            // ── Learn modes and notifications ─────────────────────
            EngineCommand::MidiLearnToggle => {
                self.input.midi_mappings.toggle_learn();
                if self.input.midi_mappings.learn_mode {
                    self.input.keymap.cancel_learn();
                }
                CommandResult::Ok
            }
            EngineCommand::MidiLearnSelect { path } => {
                self.input
                    .midi_mappings
                    .select_learn_target(crate::engine::value::param::canonical_path(&path));
                CommandResult::Ok
            }
            EngineCommand::KeyboardLearnToggle => {
                self.input.keymap.toggle_learn();
                if self.input.keymap.learn_mode {
                    self.input.midi_mappings.cancel_learn();
                }
                CommandResult::Ok
            }
            EngineCommand::KeyboardLearnSelect { target } => {
                let target = match target {
                    crate::keymap::KeyTarget::ParamPath(path) => {
                        crate::keymap::KeyTarget::ParamPath(
                            crate::engine::value::param::canonical_path(&path),
                        )
                    }
                    action @ crate::keymap::KeyTarget::Action(_) => action,
                };
                self.input.keymap.select_learn_target(target);
                CommandResult::Ok
            }
            EngineCommand::KeyboardLearnBind { combo } => {
                self.input.keymap.process_learn(combo);
                CommandResult::Ok
            }
            EngineCommand::DismissNotification { id } => {
                self.session.notifications.dismiss(id);
                CommandResult::Ok
            }
            EngineCommand::NotifyInfo { message } => {
                self.session.notifications.info(message);
                CommandResult::Ok
            }
            // ── Consumer views ────────────────────────────────────
            EngineCommand::SetPreviewChannels { channel_uuids } => {
                self.preview_channel_uuids = channel_uuids;
                CommandResult::Ok
            }
            EngineCommand::AcquireDetectionCamera { camera_id } => {
                if self.sources.detection_camera == Some(camera_id) {
                    return CommandResult::Ok;
                }
                if let Some(previous) = self.sources.detection_camera.take() {
                    self.sources.camera_manager.release_camera(previous);
                }
                match self.open_camera(camera_id) {
                    Ok(_) => {
                        self.sources.detection_camera = Some(camera_id);
                        CommandResult::Ok
                    }
                    Err(e) => {
                        let message =
                            format!("Camera detection could not open camera {camera_id}: {e}");
                        self.session.notifications.error(message.clone());
                        CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message,
                        }
                    }
                }
            }
            EngineCommand::ReleaseDetectionCamera => {
                if let Some(previous) = self.sources.detection_camera.take() {
                    self.sources.camera_manager.release_camera(previous);
                }
                CommandResult::Ok
            }

            // ── Persistence ───────────────────────────────────────
            EngineCommand::SaveWorkspace => match self.save_workspace() {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message: e.to_string(),
                },
            },
            EngineCommand::LoadWorkspace => match self.load_workspace().error_message() {
                None => CommandResult::Ok,
                Some(message) => CommandResult::Err {
                    code: ErrorCode::InternalError,
                    message,
                },
            },

            // ── History ───────────────────────────────────────────
            // One timeline for every consumer; "current" goes onto the opposite
            // stack so the step can be walked back.
            EngineCommand::Undo => {
                let current = self.history_snapshot();
                if self.history_undo(current) {
                    CommandResult::Ok
                } else {
                    CommandResult::Err {
                        code: ErrorCode::InvalidInput,
                        message: "Nothing to undo".into(),
                    }
                }
            }
            EngineCommand::Redo => {
                let current = self.history_snapshot();
                if self.history_redo(current) {
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

#[cfg(test)]
mod tests {
    use crate::app::classify::command_is_undoable;
    use crate::engine::EngineCommand as C;

    // ── Error → wire mapping (classify / wire / wire_id / not_found) ───
    //
    // These are pure functions with no GPU dependency: they translate engine
    // `anyhow::Result`s into the serializable `CommandResult`. The key contract
    // (api-addressing.md) is that an unresolvable UUID becomes `NotFound`, while
    // any other error is `InvalidInput`.

    use super::{UnknownEntity, classify, not_found, wire, wire_id};
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
        assert!(matches!(
            wire(Ok::<(), anyhow::Error>(())),
            CommandResult::Ok
        ));
    }

    #[test]
    fn wire_err_classifies_and_carries_message() {
        // Unresolvable UUID → NotFound, message preserved.
        match wire(Err::<(), _>(unknown())) {
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

    /// Drain `commands` the way the windowed runner does.
    fn drain(app: &mut super::VardaApp, commands: Vec<C>, starts_undo_step: bool) {
        app.apply_engine_actions(commands, starts_undo_step);
    }

    #[test]
    fn gui_undo_redo_roundtrips_a_structural_deck_add() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        drain(
            &mut app,
            vec![C::AddSolidColorDeck {
                channel_uuid,
                color: [0.0, 0.0, 1.0, 1.0],
            }],
            true,
        );
        assert_eq!(app.mixer_ref().channels()[0].decks.len(), 1);

        drain(&mut app, vec![C::Undo], false);
        assert_eq!(
            app.mixer_ref().channels()[0].decks.len(),
            0,
            "undo must remove the added deck"
        );

        drain(&mut app, vec![C::Redo], false);
        assert_eq!(
            app.mixer_ref().channels()[0].decks.len(),
            1,
            "redo must restore the deck"
        );
    }

    /// Only a frame that starts an undo step records one; a held drag's later
    /// frames must not.
    #[test]
    fn gui_drain_records_history_only_when_a_step_starts() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let add = || C::AddSolidColorDeck {
            channel_uuid: channel_uuid.clone(),
            color: [0.0, 0.0, 1.0, 1.0],
        };
        drain(&mut app, vec![add()], false);
        assert!(!app.history_can_undo());
        drain(&mut app, vec![add()], true);
        assert!(app.history_can_undo());
    }

    #[test]
    fn undo_on_empty_stack_is_err() {
        let Some(mut app) = headless_app() else {
            return;
        };
        assert!(matches!(
            app.execute_command(C::Undo),
            CommandResult::Err { .. }
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
        app.settle_deck_loads();

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

        assert!(
            app.history_can_undo(),
            "one command, one history entry, so one undo undoes the whole draw"
        );
        app.execute_command(C::Undo);
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

    // ── Learn modes and notifications (spec/ui-engine-boundary.md WS7) ──

    /// MIDI learn is a command, so the API can run it too. Entering it leaves
    /// keyboard learn, since one control cannot be bound by two learn modes.
    #[test]
    fn midi_learn_runs_over_the_bus_and_excludes_keyboard_learn() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.execute_command(C::KeyboardLearnToggle);
        assert!(app.input.keymap.learn_mode);

        app.execute_command(C::MidiLearnToggle);
        app.execute_command(C::MidiLearnSelect {
            path: "crossfader".to_string(),
        });

        assert!(app.input.midi_mappings.learn_mode);
        assert_eq!(
            app.input.midi_mappings.learn_target.as_deref(),
            Some("crossfader")
        );
        assert!(!app.input.keymap.learn_mode, "keyboard learn was cancelled");
    }

    #[test]
    fn keyboard_learn_binds_a_combo_over_the_bus() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let combo = crate::keymap::KeyCombo {
            key: "K".to_string(),
            command: false,
            shift: true,
            alt: false,
        };
        let target = crate::keymap::KeyTarget::ParamPath("crossfader".to_string());
        app.execute_command(C::KeyboardLearnToggle);
        app.execute_command(C::KeyboardLearnSelect {
            target: target.clone(),
        });
        app.execute_command(C::KeyboardLearnBind {
            combo: combo.clone(),
        });

        assert_eq!(app.input.keymap.get(&combo), Some(&target));
    }

    /// Notifications are dismissed by id: an index would name a different toast
    /// once an older one expires.
    #[test]
    fn notifications_are_dismissed_by_id() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.execute_command(C::NotifyInfo {
            message: "first".to_string(),
        });
        app.execute_command(C::NotifyInfo {
            message: "second".to_string(),
        });
        let first = app
            .session
            .notifications
            .visible()
            .iter()
            .find(|n| n.message == "first")
            .expect("first is visible")
            .id;

        app.execute_command(C::DismissNotification { id: first });

        let left: Vec<&str> = app
            .session
            .notifications
            .visible()
            .iter()
            .map(|n| n.message.as_str())
            .collect();
        assert_eq!(left, ["second"]);
    }

    #[test]
    fn learn_and_notification_commands_are_not_undoable() {
        for cmd in [
            C::MidiLearnToggle,
            C::MidiLearnSelect {
                path: "crossfader".to_string(),
            },
            C::KeyboardLearnToggle,
            C::KeyboardLearnSelect {
                target: crate::keymap::KeyTarget::ParamPath("crossfader".to_string()),
            },
            C::KeyboardLearnBind {
                combo: crate::keymap::KeyCombo {
                    key: "K".to_string(),
                    command: false,
                    shift: false,
                    alt: false,
                },
            },
            C::DismissNotification { id: 1 },
            C::NotifyInfo {
                message: "hi".to_string(),
            },
        ] {
            assert!(!command_is_undoable(&cmd), "{cmd:?}");
        }
    }
}
