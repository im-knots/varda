//! Deferred drag-and-drop handlers.
//!
//! egui's drag-and-drop payload is only readable while the drag is live, so each
//! handler tracks the hovered drop target every frame and applies the action on
//! the frame the payload disappears (mouse released).

use super::super::{EffectDrag, LibraryDrag, SequenceStepDrag, UIActions, UIData};
use crate::engine::{EffectTarget, EngineCommand};

/// Deferred library drag-and-drop handler.
/// Each frame while a `LibraryDrag` payload is active, find which drop target the pointer is over.
/// When the payload disappears (mouse released), apply the drop action.
/// Resolve a filter registry index (as stashed by the library drag source) to
/// its shader name, so the panel can push a canonical `AddEffect` command.
pub(super) fn resolve_filter_name(data: &UIData, filter_idx: usize) -> Option<String> {
    data.filters
        .iter()
        .find(|(_, ri)| *ri == filter_idx)
        .map(|(name, _)| name.clone())
}

pub(super) fn handle_library_dnd(ctx: &egui::Context, data: &UIData, actions: &mut UIActions) {
    let had_payload_id = egui::Id::new("__lib_dnd_had_payload");
    let hover_ch_id = egui::Id::new("__lib_dnd_hover_ch");
    let hover_fx_target_id = egui::Id::new("__lib_dnd_hover_fx_target");
    let on_new_ch_id = egui::Id::new("__lib_dnd_on_new_ch_zone");
    let awaiting_new_ch_id = egui::Id::new("__lib_dnd_awaiting_new_ch");
    let has_payload = egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx);

    if has_payload {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            let mut found_ch: Option<usize> = None;
            for ch_idx in 0..data.channels.len() {
                let key = egui::Id::new("ch_drop_rect").with(ch_idx);
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                    if rect.contains(pos) {
                        found_ch = Some(ch_idx);
                        break;
                    }
                }
            }

            // Check if hovering over either new-channel drop zone (left=0, right=1)
            let mut on_new_ch = false;
            if found_ch.is_none() {
                for side in 0..2 {
                    let key = egui::Id::new("new_ch_drop_rect").with(side);
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                        if rect.contains(pos) {
                            on_new_ch = true;
                            break;
                        }
                    }
                }
            }

            // The chain is recorded by UUID: the drop is applied after release,
            // so an index recorded here could name another entity by then.
            let mut found_fx: Option<(String, String)> = None;
            if data.selected_master {
                let master_key = egui::Id::new("master_fx_drop_rect");
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(master_key)) {
                    if rect.contains(pos) {
                        found_fx = Some(("master".to_string(), String::new()));
                    }
                }
            } else if let Some(ch) = data.selected_channel.and_then(|i| data.channels.get(i)) {
                let key = egui::Id::new("ch_fx_drop_rect").with(ch.ch_idx);
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                    if rect.contains(pos) {
                        found_fx = Some(("channel".to_string(), ch.uuid.clone()));
                    }
                }
            } else if let Some((sel_ch, sel_dk)) = data.selected_deck {
                let key = egui::Id::new("deck_fx_drop_rect").with((sel_ch, sel_dk));
                if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(key)) {
                    if rect.contains(pos) {
                        if let Some(deck) = data
                            .channels
                            .get(sel_ch)
                            .and_then(|ch| ch.decks.get(sel_dk))
                        {
                            found_fx = Some(("deck".to_string(), deck.uuid.clone()));
                        }
                    }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.insert_temp(hover_ch_id, found_ch);
                mem.data.insert_temp(hover_fx_target_id, found_fx);
                mem.data.insert_temp::<bool>(on_new_ch_id, on_new_ch);
                mem.data.insert_temp::<bool>(had_payload_id, true);
            });
        }
    } else {
        let drag_just_ended: bool =
            ctx.memory(|mem| mem.data.get_temp(had_payload_id).unwrap_or(false));
        if drag_just_ended {
            let hover_ch: Option<usize> =
                ctx.memory(|mem| mem.data.get_temp(hover_ch_id).unwrap_or(None));
            let hover_fx: Option<(String, String)> =
                ctx.memory(|mem| mem.data.get_temp(hover_fx_target_id).unwrap_or(None));
            let on_new_ch_zone: bool =
                ctx.memory(|mem| mem.data.get_temp(on_new_ch_id).unwrap_or(false));

            // Channel preset: if dropped on a channel, fill into it; otherwise create new
            let ch_preset_key = egui::Id::new("__lib_dnd_ch_preset_idx");
            let ch_preset_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(ch_preset_key));

            // Resolve the drop target to a channel UUID. A drop on empty space
            // has to create the channel first, and no UUID for it exists until
            // the engine has applied `AddChannel` — so that case parks the
            // payload and resolves on the next frame instead of guessing an
            // index. See `/spec/api-addressing.md`.
            let awaiting_len: Option<usize> =
                ctx.memory(|mem| mem.data.get_temp(awaiting_new_ch_id));
            let mut parked = false;
            let target_channel: Option<String> = if let Some(expected_len) = awaiting_len {
                ctx.memory_mut(|mem| mem.data.remove::<usize>(awaiting_new_ch_id));
                if data.channels.len() >= expected_len {
                    data.channels.last().map(|ch| ch.uuid.clone())
                } else {
                    log::warn!("Dropping library payload: the new channel was never created");
                    None
                }
            } else if let Some(ch_idx) = hover_ch {
                data.channels.get(ch_idx).map(|ch| ch.uuid.clone())
            } else if on_new_ch_zone && ch_preset_idx.is_none() {
                actions.commands.push(EngineCommand::AddChannel);
                ctx.memory_mut(|mem| {
                    mem.data
                        .insert_temp(awaiting_new_ch_id, data.channels.len() + 1);
                    mem.data.insert_temp::<bool>(on_new_ch_id, false);
                });
                parked = true;
                None
            } else {
                None
            };

            if let Some(channel_uuid) = target_channel {
                let gen_key = egui::Id::new("__lib_dnd_gen_idx");
                let gen_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(gen_key));
                if let Some(gen_idx) = gen_idx {
                    log::info!("Library drop (deferred): generator {gen_idx} -> ch {channel_uuid}");
                    actions.session.shader_to_add = Some((channel_uuid.clone(), gen_idx));
                }

                let cam_key = egui::Id::new("__lib_dnd_cam_id");
                let cam_id: Option<crate::camera::CameraId> =
                    ctx.memory(|mem| mem.data.get_temp(cam_key));
                if let Some(cam_id) = cam_id {
                    log::info!("Library drop (deferred): camera {cam_id} -> ch {channel_uuid}");
                    actions.commands.push(EngineCommand::AddCameraDeck {
                        channel_uuid: channel_uuid.clone(),
                        camera_id: cam_id,
                    });
                }

                let depth_key = egui::Id::new("__lib_dnd_depth_sensor_id");
                let depth_id: Option<crate::depth::DepthSensorId> =
                    ctx.memory(|mem| mem.data.get_temp(depth_key));
                if let Some(depth_id) = depth_id {
                    log::info!(
                        "Library drop (deferred): depth sensor {depth_id} -> ch {channel_uuid}"
                    );
                    actions.commands.push(EngineCommand::AddDepthSensorDeck {
                        channel_uuid: channel_uuid.clone(),
                        depth_sensor_id: depth_id,
                    });
                }

                let capture_key = egui::Id::new(super::library::CAPTURE_DND_KEY);
                let capture_target: Option<crate::scene::CaptureTargetConfig> =
                    ctx.memory(|mem| mem.data.get_temp(capture_key));
                if let Some(target) = capture_target {
                    log::info!(
                        "Library drop (deferred): capture '{}' -> ch {channel_uuid}",
                        target.label()
                    );
                    actions.commands.push(EngineCommand::AddScreenCaptureDeck {
                        channel_uuid: channel_uuid.clone(),
                        target,
                        rate: None,
                        crop: None,
                        show_cursor: None,
                        // Per-target default resolved engine-side: displays
                        // exclude Varda, windows do not.
                        exclude_varda: None,
                    });
                }

                let tap_key = egui::Id::new(super::library::TAP_DND_KEY);
                let tap_source: Option<crate::scene::TapSourceConfig> =
                    ctx.memory(|mem| mem.data.get_temp(tap_key));
                if let Some(source) = tap_source {
                    log::info!("Library drop (deferred): tap -> ch {channel_uuid}");
                    actions.commands.push(EngineCommand::AddTapDeck {
                        channel_uuid: channel_uuid.clone(),
                        source,
                    });
                }

                let ndi_key = egui::Id::new("__lib_dnd_ndi_name");
                let ndi_name: Option<String> = ctx.memory(|mem| mem.data.get_temp(ndi_key));
                if let Some(ndi_name) = ndi_name {
                    log::info!("Library drop (deferred): NDI '{ndi_name}' -> ch {channel_uuid}");
                    actions.commands.push(EngineCommand::AddNdiDeck {
                        channel_uuid: channel_uuid.clone(),
                        source_name: ndi_name,
                    });
                }

                let syph_key = egui::Id::new("__lib_dnd_syph_name");
                let syph_name: Option<String> = ctx.memory(|mem| mem.data.get_temp(syph_key));
                if let Some(syph_name) = syph_name {
                    log::info!(
                        "Library drop (deferred): Syphon '{syph_name}' -> ch {channel_uuid}"
                    );
                    actions.commands.push(EngineCommand::AddSyphonDeck {
                        channel_uuid: channel_uuid.clone(),
                        server_name: syph_name,
                    });
                }

                let srt_key = egui::Id::new("__lib_dnd_srt_config");
                let srt_config: Option<(String, crate::stream::SrtMode)> =
                    ctx.memory(|mem| mem.data.get_temp(srt_key));
                if let Some((url, mode)) = srt_config {
                    log::info!(
                        "Library drop (deferred): SRT '{url}' ({mode:?}) -> ch {channel_uuid}"
                    );
                    actions.commands.push(EngineCommand::AddSrtDeck {
                        channel_uuid: channel_uuid.clone(),
                        url,
                        mode,
                    });
                }

                let hls_key = egui::Id::new("__lib_dnd_hls_url");
                if let Some(url) = ctx.memory(|mem| mem.data.get_temp::<String>(hls_key)) {
                    log::info!("Library drop (deferred): HLS '{url}' -> ch {channel_uuid}");
                    actions.commands.push(EngineCommand::AddHlsDeck {
                        channel_uuid: channel_uuid.clone(),
                        url,
                    });
                }

                let dash_key = egui::Id::new("__lib_dnd_dash_url");
                if let Some(url) = ctx.memory(|mem| mem.data.get_temp::<String>(dash_key)) {
                    log::info!("Library drop (deferred): DASH '{url}' -> ch {channel_uuid}");
                    actions.commands.push(EngineCommand::AddDashDeck {
                        channel_uuid: channel_uuid.clone(),
                        url,
                    });
                }

                let rtmp_key = egui::Id::new("__lib_dnd_rtmp_config");
                if let Some((url, mode)) = ctx.memory(|mem| {
                    mem.data
                        .get_temp::<(String, crate::stream::RtmpMode)>(rtmp_key)
                }) {
                    log::info!(
                        "Library drop (deferred): RTMP '{url}' ({mode}) -> ch {channel_uuid}"
                    );
                    actions.commands.push(EngineCommand::AddRtmpDeck {
                        channel_uuid: channel_uuid.clone(),
                        url,
                        mode,
                    });
                }

                let html_key = egui::Id::new("__lib_dnd_html_url");
                if let Some(url) = ctx.memory(|mem| mem.data.get_temp::<String>(html_key)) {
                    log::info!("Library drop (deferred): HTML '{url}' -> ch {channel_uuid}");
                    actions.commands.push(EngineCommand::AddHtmlDeck {
                        channel_uuid: channel_uuid.clone(),
                        url,
                    });
                }

                let deck_preset_key = egui::Id::new("__lib_dnd_deck_preset_idx");
                let deck_preset_idx: Option<usize> =
                    ctx.memory(|mem| mem.data.get_temp(deck_preset_key));
                if let Some(preset_name) =
                    deck_preset_idx.and_then(|idx| data.deck_presets.get(idx))
                {
                    log::info!(
                        "Library drop (deferred): deck preset '{preset_name}' -> ch {channel_uuid}"
                    );
                    actions.commands.push(EngineCommand::LoadDeckPreset {
                        channel_uuid: channel_uuid.clone(),
                        preset_name: preset_name.clone(),
                    });
                }
            }

            if let Some(preset_name) = ch_preset_idx.and_then(|idx| data.channel_presets.get(idx)) {
                let target_channel_uuid =
                    hover_ch.and_then(|ch_idx| data.channels.get(ch_idx).map(|c| c.uuid.clone()));
                log::info!(
                    "Library drop (deferred): channel preset '{}' -> {}",
                    preset_name,
                    target_channel_uuid.as_deref().unwrap_or("new channel")
                );
                actions.commands.push(EngineCommand::LoadChannelPreset {
                    target_channel_uuid,
                    preset_name: preset_name.clone(),
                });
            }

            if let Some((target_type, target_uuid)) = hover_fx {
                let fx_key = egui::Id::new("__lib_dnd_fx_idx");
                let filter_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(fx_key));
                if let Some(filter_idx) = filter_idx {
                    let target = match target_type.as_str() {
                        "deck" => Some(EffectTarget::Deck(target_uuid)),
                        "channel" => Some(EffectTarget::Channel(target_uuid)),
                        "master" => Some(EffectTarget::Master),
                        _ => None,
                    };
                    if let (Some(target), Some(shader_name)) =
                        (target, resolve_filter_name(data, filter_idx))
                    {
                        log::info!("Library drop (deferred): effect {filter_idx} -> {target:?}");
                        actions.commands.push(EngineCommand::AddEffect {
                            target,
                            shader_name,
                        });
                    }
                }
            }

            // A parked payload keeps its keys so the next frame can re-enter and
            // resolve the channel that `AddChannel` created.
            if parked {
                return;
            }

            ctx.memory_mut(|mem| {
                mem.data.remove::<bool>(had_payload_id);
                mem.data.remove::<Option<usize>>(hover_ch_id);
                mem.data
                    .remove::<Option<(String, String)>>(hover_fx_target_id);
                mem.data.remove::<bool>(on_new_ch_id);
                mem.data.remove::<usize>(egui::Id::new("__lib_dnd_gen_idx"));
                mem.data.remove::<usize>(egui::Id::new("__lib_dnd_fx_idx"));
                mem.data
                    .remove::<crate::camera::CameraId>(egui::Id::new("__lib_dnd_cam_id"));
                mem.data
                    .remove::<crate::scene::CaptureTargetConfig>(egui::Id::new(
                        super::library::CAPTURE_DND_KEY,
                    ));
                mem.data
                    .remove::<crate::scene::TapSourceConfig>(egui::Id::new(
                        super::library::TAP_DND_KEY,
                    ));
                mem.data
                    .remove::<String>(egui::Id::new("__lib_dnd_ndi_name"));
                mem.data
                    .remove::<String>(egui::Id::new("__lib_dnd_syph_name"));
                mem.data
                    .remove::<(String, crate::stream::SrtMode)>(egui::Id::new(
                        "__lib_dnd_srt_config",
                    ));
                mem.data
                    .remove::<String>(egui::Id::new("__lib_dnd_hls_url"));
                mem.data
                    .remove::<String>(egui::Id::new("__lib_dnd_dash_url"));
                mem.data
                    .remove::<(String, crate::stream::RtmpMode)>(egui::Id::new(
                        "__lib_dnd_rtmp_config",
                    ));
                mem.data
                    .remove::<String>(egui::Id::new("__lib_dnd_html_url"));
                mem.data
                    .remove::<usize>(egui::Id::new("__lib_dnd_deck_preset_idx"));
                mem.data
                    .remove::<usize>(egui::Id::new("__lib_dnd_ch_preset_idx"));
            });
        }
    }
}

/// Deferred effect reorder drag-and-drop handler.
/// Same pattern as library drops — tracks which drop zone the pointer is over,
/// then applies the move when the payload disappears.
pub(super) fn handle_effect_dnd(ctx: &egui::Context, data: &UIData, actions: &mut UIActions) {
    let had_eff_id = egui::Id::new("__eff_dnd_had_payload");
    let hover_dz_id = egui::Id::new("__eff_dnd_hover_dz");
    let has_eff_payload = egui::DragAndDrop::has_payload_of_type::<EffectDrag>(ctx);

    if has_eff_payload {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            let mut found_dz: Option<(String, usize)> = None;

            let check_chain = |chain_key: &str,
                               ctx: &egui::Context,
                               pos: egui::Pos2|
             -> Option<(String, usize)> {
                let count_key = egui::Id::new("eff_dz_count").with(chain_key.to_string());
                let count: usize = ctx.memory(|mem| mem.data.get_temp(count_key).unwrap_or(0));
                for p in 0..count {
                    let rk = egui::Id::new("eff_dz_rect").with((chain_key.to_string(), p));
                    if let Some(rect) = ctx.memory(|mem| mem.data.get_temp::<egui::Rect>(rk)) {
                        if rect.contains(pos) {
                            return Some((chain_key.to_string(), p));
                        }
                    }
                }
                None
            };

            if found_dz.is_none() {
                if let Some(deck) = data
                    .selected_deck
                    .and_then(|(ch, dk)| data.channels.get(ch)?.decks.get(dk))
                {
                    found_dz = check_chain(&format!("deck_{}", deck.uuid), ctx, pos);
                }
            }
            if found_dz.is_none() {
                found_dz = check_chain("master", ctx, pos);
            }
            if found_dz.is_none() {
                for ch in &data.channels {
                    found_dz = check_chain(&format!("ch_{}", ch.uuid), ctx, pos);
                    if found_dz.is_some() {
                        break;
                    }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.insert_temp(hover_dz_id, found_dz);
                mem.data.insert_temp::<bool>(had_eff_id, true);
            });
        }
    } else {
        let had: bool = ctx.memory(|mem| mem.data.get_temp(had_eff_id).unwrap_or(false));
        if had {
            let hover_dz: Option<(String, usize)> =
                ctx.memory(|mem| mem.data.get_temp(hover_dz_id).unwrap_or(None));
            let src_key = egui::Id::new("__eff_dnd_src");
            let src: Option<EffectDrag> = ctx.memory(|mem| mem.data.get_temp(src_key));

            if let (Some((chain_key, target_pos)), Some(src_drag)) = (hover_dz, src) {
                match src_drag {
                    EffectDrag::Deck(src_deck, src_eff) => {
                        let expected_key = format!("deck_{src_deck}");
                        if chain_key == expected_key {
                            let to = if src_eff < target_pos {
                                target_pos - 1
                            } else {
                                target_pos
                            };
                            if to != src_eff {
                                log::info!(
                                    "Effect reorder (deferred): deck {src_deck} effect {src_eff} -> {to}"
                                );
                                actions.commands.push(EngineCommand::MoveEffect {
                                    target: EffectTarget::Deck(src_deck),
                                    from_idx: src_eff,
                                    to_idx: to,
                                });
                            }
                        }
                    }
                    EffectDrag::Channel(src_ch, src_eff) => {
                        let expected_key = format!("ch_{src_ch}");
                        if chain_key == expected_key {
                            let to = if src_eff < target_pos {
                                target_pos - 1
                            } else {
                                target_pos
                            };
                            if to != src_eff {
                                log::info!(
                                    "Effect reorder (deferred): ch {src_ch} effect {src_eff} -> {to}"
                                );
                                actions.commands.push(EngineCommand::MoveEffect {
                                    target: EffectTarget::Channel(src_ch),
                                    from_idx: src_eff,
                                    to_idx: to,
                                });
                            }
                        }
                    }
                    EffectDrag::Master(src_eff) => {
                        if chain_key == "master" {
                            let to = if src_eff < target_pos {
                                target_pos - 1
                            } else {
                                target_pos
                            };
                            if to != src_eff {
                                log::info!(
                                    "Effect reorder (deferred): master effect {src_eff} -> {to}"
                                );
                                actions.commands.push(EngineCommand::MoveEffect {
                                    target: EffectTarget::Master,
                                    from_idx: src_eff,
                                    to_idx: to,
                                });
                            }
                        }
                    }
                }
            }

            ctx.memory_mut(|mem| {
                mem.data.remove::<bool>(had_eff_id);
                mem.data.remove::<Option<(String, usize)>>(hover_dz_id);
                mem.data.remove::<EffectDrag>(src_key);
            });
        }
    }
}

/// Deferred `DnD` handler for reordering steps within a sequence.
/// Follows the same pattern as effect `DnD`: source is stored in egui memory
/// during drag (since `DragAndDrop::payload()` is None after mouse release).
pub(super) fn handle_sequence_step_dnd(
    ctx: &egui::Context,
    _data: &UIData,
    actions: &mut UIActions,
) {
    let had_id = egui::Id::new("__seq_step_dnd_had");
    let target_id = egui::Id::new("__seq_step_dnd_target");
    let src_id = egui::Id::new("__seq_step_dnd_src");
    let has_payload = egui::DragAndDrop::has_payload_of_type::<SequenceStepDrag>(ctx);

    if has_payload {
        ctx.memory_mut(|mem| {
            mem.data.insert_temp::<bool>(had_id, true);
        });
    } else {
        let had: bool = ctx.memory(|mem| mem.data.get_temp(had_id).unwrap_or(false));
        if had {
            // Payload was just released — read source from memory (not DragAndDrop)
            let src: Option<SequenceStepDrag> = ctx.memory(|mem| mem.data.get_temp(src_id));
            let target: Option<usize> = ctx.memory(|mem| mem.data.get_temp(target_id));

            if let (Some(payload), Some(to)) = (src, target) {
                // `to` is the gap position in the original list.
                // After remove(from), indices shift: adjust for insert.
                let insert_idx = if to > payload.step_idx { to - 1 } else { to };
                if insert_idx != payload.step_idx {
                    actions
                        .commands
                        .push(crate::engine::EngineCommand::MoveStep {
                            sequence_uuid: payload.sequence_uuid,
                            from: payload.step_idx,
                            to: insert_idx,
                        });
                }
            }
            ctx.memory_mut(|mem| {
                mem.data.remove::<bool>(had_id);
                mem.data.remove::<usize>(target_id);
                mem.data.remove::<SequenceStepDrag>(src_id);
            });
        }
    }
}
