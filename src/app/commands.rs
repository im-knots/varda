//! Command dispatch for `VardaApp`: `execute_command` matches every
//! `EngineCommand` variant.

use super::VardaApp;
use super::resolve::UnknownEntity;
use crate::engine::{CommandOutcome, CommandResult, EngineCommand, ErrorCode};

/// The canonical modulation key for a client's target, or the wire error if
/// nothing modulatable matches. Also accepts the pre-v8 `deck_<uuid>:<name>` form.
fn modulation_target(target: &str) -> Result<String, CommandResult> {
    crate::param_router::canonical_modulation_key(target).map_err(|e| CommandResult::Err {
        code: ErrorCode::InvalidInput,
        message: e.to_string(),
    })
}

/// Classify an engine error for the wire: an unresolvable UUID is `NotFound`,
/// anything else `InvalidInput`.
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

/// Classify a parameter-routing failure: an unresolved path, entity or param
/// is `NotFound`; an unacceptable value is `InvalidInput`.
fn param_route_error_code(err: &crate::param_router::ParamRouteError) -> ErrorCode {
    use crate::param_router::ParamRouteError as E;
    match err {
        E::UnknownPath { .. } | E::UnknownEntity { .. } | E::UnknownParam { .. } => {
            ErrorCode::NotFound
        }
        E::IndexOutOfRange { .. } | E::WrongState { .. } => ErrorCode::InvalidInput,
    }
}

/// Wire result for a transport operation the current source disallows, with the reason.
fn transport_rejected(err: crate::transport::TransportError) -> CommandResult {
    CommandResult::Err {
        code: ErrorCode::InvalidInput,
        message: err.to_string(),
    }
}

impl VardaApp {
    /// Execute a command for the GUI, returning a typed [`CommandOutcome`]
    /// instead of the wire [`CommandResult`]. Deck-creating commands report
    /// location and UUID so the runner can register a preview texture; the rest
    /// go to [`Self::execute_command`].
    pub(crate) fn execute_command_gui(&mut self, cmd: EngineCommand) -> CommandOutcome {
        // A preset load can create any number of decks, so diff the deck set.
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

    /// Every live deck UUID.
    fn deck_uuid_set(&self) -> std::collections::HashSet<String> {
        self.mixer
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .map(|slot| slot.deck.uuid().to_string())
            .collect()
    }

    /// True if any command in the batch is undoable, per [`VardaApp::is_undoable`].
    pub(crate) fn batch_has_undoable(&self, cmds: &[EngineCommand]) -> bool {
        cmds.iter().any(|cmd| self.is_undoable(cmd))
    }

    /// Execute a single command. A successful live parameter write is passed to
    /// the recorder and overrides the arrangement.
    pub(crate) fn execute_command(&mut self, cmd: EngineCommand) -> CommandResult {
        let live = self.live_writes(&cmd);
        // Any command but a live parameter write may change a library listing.
        if live.is_empty() {
            self.sources.type_cache.invalidate();
            self.output.sink_type_cache.invalidate();
        }
        let result = self.dispatch_command(cmd);
        if !matches!(result, CommandResult::Err { .. }) {
            for (key, value) in live {
                self.note_live_param_write(&key, value);
            }
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
                source,
            } => wire_id(self.add_deck(&channel_uuid, &source)),
            EngineCommand::ReplaceDeckSource { deck_uuid, source } => {
                wire(self.replace_deck_source(&deck_uuid, &source))
            }
            EngineCommand::SetSourceParam {
                deck_uuid,
                name,
                value,
            } => wire(self.mixer.set_source_param(&deck_uuid, &name, &value)),
            EngineCommand::TriggerSourceAction { deck_uuid, action } => {
                wire(self.mixer.trigger_source_action(&deck_uuid, &action))
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
                if let Some(cmd) = self.surface_toggle(&path) {
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
                // Capture opens per frame from modulator demand.
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
                // The caller needs the UUID to show the lane and add breakpoints.
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
                        .assign(&target, &source_id, amount);
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
            EngineCommand::CreateOutput { sink } => self.cmd_create_output(sink),
            EngineCommand::CloseOutput { output_uuid } => {
                match self.output.close_output(&output_uuid) {
                    Ok(passthrough) => {
                        self.release_passthrough(passthrough);
                        CommandResult::Ok
                    }
                    Err(e) => wire(Err::<(), _>(e)),
                }
            }
            EngineCommand::SetOutputTarget { output_uuid, sink } => {
                self.cmd_set_output_target(&output_uuid, &sink)
            }
            EngineCommand::SetSinkParam {
                output_uuid,
                name,
                value,
            } => self.cmd_set_sink_param(&output_uuid, &name, &value),
            EngineCommand::SinkLibraryAction { sink_type, action } => {
                self.cmd_sink_library_action(&sink_type, &action)
            }
            EngineCommand::SetSurfaceAssignmentEnabled {
                output_uuid,
                surface_uuid,
                enabled,
            } => {
                let result = self.output.set_surface_assignment_enabled(
                    &output_uuid,
                    &surface_uuid,
                    enabled,
                );
                self.output.recompute_auto_edge_blend();
                wire(result)
            }
            EngineCommand::SetPathText { path, value } => self.set_path_text(&path, value),
            EngineCommand::SetOutputUnassigned {
                output_uuid,
                unassigned,
            } => self.cmd_set_output_unassigned(&output_uuid, unassigned),

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

            // ── HTML interactive window ───────────────────────
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

            // ── Source Library ─────────────────────────────────
            EngineCommand::AddSourceLibraryEntry { entry } => {
                wire(self.source_library(entry.type_id(), |p| p.add_library_entry(entry.clone())))
            }
            EngineCommand::RemoveSourceLibraryEntry { entry } => {
                wire(self.source_library(entry.type_id(), |p| p.remove_library_entry(&entry)))
            }
            EngineCommand::SourceLibraryAction {
                source_type,
                action,
            } => self.source_library_action(&source_type, &action),

            // ── Output Management ─────────────────────────────────
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
                // A second stop returns to zero, so cue stepping restarts.
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
                // API JSON can't enforce the invariant, so check it here.
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
                // The per-frame reconcile opens and closes the devices.
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
                    self.sources
                        .service_mut::<crate::camera::CameraManager>()
                        .release_camera(previous);
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
                    self.sources
                        .service_mut::<crate::camera::CameraManager>()
                        .release_camera(previous);
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
            // The current state goes onto the opposite stack.
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

    // ── Error to wire mapping ──────────────────────────────────────────
    //
    // An unresolvable UUID becomes `NotFound`; any other error `InvalidInput`.

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
        // The downcast finds UnknownEntity through an anyhow context.
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
        // Unresolvable UUID: NotFound, message preserved.
        match wire(Err::<(), _>(unknown())) {
            CommandResult::Err { code, message } => {
                assert_eq!(code, ErrorCode::NotFound);
                assert_eq!(message, "No deck with UUID 'abc123'");
            }
            other => panic!("expected Err, got {other:?}"),
        }
        // Other errors: InvalidInput.
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

    // ── Typed GUI results ──────────────────────────────────────────────
    //
    // These need a GPU and return early without one.

    use crate::engine::{CommandOutcome, CommandResult};

    fn headless_app() -> Option<super::VardaApp> {
        crate::testing::headless_app()
    }

    #[test]
    fn deck_add_command_returns_resolvable_uuid() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let result = app.execute_command(C::AddDeck {
            channel_uuid,
            source: crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
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
        let outcome = app.execute_command_gui(C::AddDeck {
            channel_uuid,
            source: crate::solid_color::SolidColor::config_for([0.0, 1.0, 0.0, 1.0]),
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
            vec![C::AddDeck {
                channel_uuid,
                source: crate::solid_color::SolidColor::config_for([0.0, 0.0, 1.0, 1.0]),
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

    fn channel_uuids(app: &super::VardaApp) -> Vec<String> {
        app.mixer_ref()
            .channels()
            .iter()
            .map(|ch| ch.uuid().to_string())
            .collect()
    }

    fn deck_uuids(app: &super::VardaApp, channel: usize) -> Vec<String> {
        app.mixer_ref().channels()[channel]
            .decks
            .iter()
            .map(|slot| slot.deck.uuid().to_string())
            .collect()
    }

    fn solid_deck(app: &mut super::VardaApp, channel_uuid: &str) {
        drain(
            app,
            vec![C::AddDeck {
                channel_uuid: channel_uuid.to_string(),
                source: crate::solid_color::SolidColor::config_for([0.0, 0.0, 1.0, 1.0]),
            }],
            true,
        );
    }

    /// Undo restores a removed channel with its UUID.
    #[test]
    fn undoing_a_channel_removal_restores_its_identity() {
        let Some(mut app) = headless_app() else {
            return;
        };
        drain(&mut app, vec![C::AddChannel], true);
        let first = channel_uuids(&app)[0].clone();
        solid_deck(&mut app, &first);
        let channels = channel_uuids(&app);
        let decks = deck_uuids(&app, 0);

        drain(
            &mut app,
            vec![C::RemoveChannel {
                channel_uuid: first,
            }],
            true,
        );
        assert_eq!(channel_uuids(&app).len(), channels.len() - 1);
        drain(&mut app, vec![C::Undo], false);

        assert_eq!(channel_uuids(&app), channels);
        assert_eq!(deck_uuids(&app, 0), decks);
    }

    /// Two decks with the same source swap and swap back, each keeping its
    /// own UUID and settings.
    #[test]
    fn undoing_a_reorder_restores_each_decks_identity_and_settings() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel = channel_uuids(&app)[0].clone();
        solid_deck(&mut app, &channel);
        solid_deck(&mut app, &channel);
        let decks = deck_uuids(&app, 0);
        drain(
            &mut app,
            vec![C::SetDeckOpacity {
                deck_uuid: decks[0].clone(),
                opacity: 0.25,
            }],
            true,
        );

        drain(
            &mut app,
            vec![C::ReorderDeck {
                channel_uuid: channel,
                from_idx: 0,
                to_idx: 1,
            }],
            true,
        );
        drain(&mut app, vec![C::Undo], false);

        assert_eq!(deck_uuids(&app, 0), decks);
        let first = &app.mixer_ref().channels()[0].decks[0];
        assert!((first.opacity - 0.25).abs() < f32::EPSILON);
    }

    /// The same for effects, whose modulation is keyed on their UUID.
    #[test]
    fn undoing_an_effect_removal_restores_effect_identities() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let add = || C::AddEffect {
            target: crate::engine::value::entity::EffectTarget::Master,
            shader_name: "invert".into(),
        };
        drain(&mut app, vec![add()], true);
        drain(&mut app, vec![add()], true);
        let effects = |app: &super::VardaApp| -> Vec<String> {
            app.mixer_ref()
                .master_effects()
                .iter()
                .map(|e| e.uuid().to_string())
                .collect()
        };
        let before = effects(&app);
        assert_eq!(before.len(), 2);

        drain(
            &mut app,
            vec![C::RemoveEffect {
                effect_uuid: before[0].clone(),
            }],
            true,
        );
        drain(&mut app, vec![C::Undo], false);

        assert_eq!(effects(&app), before);
    }

    /// Add a deck and undo it, leaving one step to redo.
    fn app_with_a_redo() -> Option<super::VardaApp> {
        let mut app = headless_app()?;
        let channel = app.mixer_ref().channels()[0].uuid().to_string();
        solid_deck(&mut app, &channel);
        drain(&mut app, vec![C::Undo], false);
        assert!(app.history_can_redo());
        Some(app)
    }

    fn stale_write() -> C {
        C::RemoveDeck {
            deck_uuid: "deadbeef".into(),
        }
    }

    /// A rejected API write keeps the redo stack.
    #[test]
    fn a_rejected_bus_command_keeps_the_redo_history() {
        let Some(mut app) = app_with_a_redo() else {
            return;
        };
        let _ = app.command_sender().send((stale_write(), None));
        app.process_commands();
        assert!(app.history_can_redo());
    }

    /// The same for a GUI frame whose commands all fail.
    #[test]
    fn a_rejected_gui_frame_keeps_the_redo_history() {
        let Some(mut app) = app_with_a_redo() else {
            return;
        };
        // The frame's housekeeping succeeds; only the rejected edit counts.
        drain(
            &mut app,
            vec![
                stale_write(),
                C::SetPreviewChannels {
                    channel_uuids: Vec::new(),
                },
            ],
            true,
        );
        assert!(app.history_can_redo());
        drain(&mut app, vec![C::Redo], false);
        assert_eq!(app.mixer_ref().channels()[0].decks.len(), 1);
    }

    /// Only a frame that starts an undo step records one.
    #[test]
    fn gui_drain_records_history_only_when_a_step_starts() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let add = || C::AddDeck {
            channel_uuid: channel_uuid.clone(),
            source: crate::solid_color::SolidColor::config_for([0.0, 0.0, 1.0, 1.0]),
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

    /// Parameter exploration is undoable.
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

    /// Over the bus, randomize changes the shader and undo restores it.
    /// `plasma` has one ranged float and two colors; randomize skips colors.
    #[test]
    fn randomizing_over_the_bus_is_one_undo_from_the_prior_look() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let channel_uuid = app.mixer_ref().channels()[0].uuid().to_string();
        let CommandResult::OkWithId { uuid } = app.execute_command(C::AddDeck {
            channel_uuid,
            source: crate::source::SourceConfig::new("Shader").with("name", "plasma"),
        }) else {
            return;
        };
        app.settle_deck_loads();

        // The ranged float and an excluded color.
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

    /// The timecode preference sent over the bus reaches the reader.
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

    /// Patching and unpatching LTC over the bus reaches the reader.
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

    /// The timecode patch is not undoable, like the clock.
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

    /// A patch sent over the bus leaves the undo stack unchanged.
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

    // ── Learn modes and notifications ──────────────────────────────────

    /// MIDI learn is a command. Entering it leaves keyboard learn.
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

    /// Notifications are dismissed by id, which stays stable as others expire.
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
