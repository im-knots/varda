//! Deferred drag-and-drop handlers.
//!
//! egui's drag-and-drop payload is only readable while the drag is live, so each
//! handler tracks the hovered drop target every frame and applies the action on
//! the frame the payload disappears (mouse released).

use super::super::{EffectDrag, LibraryDrag, SequenceStepDrag, UIActions, UIData};
use crate::engine::{EffectTarget, EngineCommand};

/// Where a library effect will land if released now.
///
/// Hit order is deck, then master, then channel, so a deck card inside a channel
/// column claims the drop. See /spec/effect-drop-targets.md.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FxHover {
    Deck {
        uuid: String,
        ch_idx: usize,
        deck_idx: usize,
    },
    Channel {
        uuid: String,
        ch_idx: usize,
    },
    Master,
}

/// Every effect drop surface drawn this frame, in draw order.
type FxSurfaces = Vec<(FxHover, egui::Rect)>;

fn fx_surfaces_id() -> egui::Id {
    egui::Id::new("__fx_drop_surfaces")
}

/// Drop the surfaces published last frame, before the panels republish.
///
/// The registry has to be rebuilt from scratch every frame rather than
/// accumulated: a surface that is no longer drawn (a deselected deck's
/// bottom-bar chain, an arrangement lane after leaving arrangement mode, a card
/// that has since moved) would otherwise keep claiming drops at the place it
/// used to occupy. See /spec/effect-drop-targets.md.
pub(super) fn begin_fx_surface_frame(ctx: &egui::Context) {
    ctx.memory_mut(|mem| mem.data.remove::<FxSurfaces>(fx_surfaces_id()));
}

fn publish_fx_surface(ctx: &egui::Context, owner: FxHover, rect: egui::Rect) {
    ctx.memory_mut(|mem| {
        let id = fx_surfaces_id();
        let mut surfaces: FxSurfaces = mem.data.get_temp(id).unwrap_or_default();
        surfaces.push((owner, rect));
        mem.data.insert_temp(id, surfaces);
    });
}

/// Mixer deck cards, the deck's bottom-bar chain, arrangement lanes, and deck
/// automation rows.
pub(super) fn publish_deck_surface_fx(
    ctx: &egui::Context,
    deck_uuid: &str,
    ch_idx: usize,
    deck_idx: usize,
    rect: egui::Rect,
) {
    publish_fx_surface(
        ctx,
        FxHover::Deck {
            uuid: deck_uuid.to_string(),
            ch_idx,
            deck_idx,
        },
        rect,
    );
}

/// Channel columns (deck cards inside them win by hit priority), the channel's
/// bottom-bar chain, arrangement group rows, and channel automation.
pub(super) fn publish_channel_surface_fx(
    ctx: &egui::Context,
    channel_uuid: &str,
    ch_idx: usize,
    rect: egui::Rect,
) {
    publish_fx_surface(
        ctx,
        FxHover::Channel {
            uuid: channel_uuid.to_string(),
            ch_idx,
        },
        rect,
    );
}

/// Main Output preview, the master bottom-bar chain, and the arrangement Master
/// row.
pub(super) fn publish_master_surface_fx(ctx: &egui::Context, rect: egui::Rect) {
    publish_fx_surface(ctx, FxHover::Master, rect);
}

/// Pure hit test used by the deferred handler and by unit tests.
///
/// Deck surfaces are tried before master and master before channel, so a deck
/// card claims a drop over the channel column it sits in. Within one kind the
/// first surface drawn wins.
fn resolve_fx_from_surfaces(
    pos: egui::Pos2,
    surfaces: &[(FxHover, egui::Rect)],
) -> Option<FxHover> {
    let hit = |want: fn(&FxHover) -> bool| {
        surfaces
            .iter()
            .find(|(owner, rect)| want(owner) && rect.contains(pos))
            .map(|(owner, _)| owner.clone())
    };
    hit(|o| matches!(o, FxHover::Deck { .. }))
        .or_else(|| hit(|o| matches!(o, FxHover::Master)))
        .or_else(|| hit(|o| matches!(o, FxHover::Channel { .. })))
}

fn resolve_fx_hover(ctx: &egui::Context, pos: egui::Pos2) -> Option<FxHover> {
    let surfaces: FxSurfaces =
        ctx.memory(|mem| mem.data.get_temp(fx_surfaces_id()).unwrap_or_default());
    resolve_fx_from_surfaces(pos, &surfaces)
}

/// Selection applied alongside `AddEffect` so the bottom bar shows the chain.
enum FxSelect {
    Deck(usize, usize),
    Channel(usize),
    Master,
}

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

pub(super) fn handle_library_dnd(ui: &egui::Ui, data: &UIData, actions: &mut UIActions) {
    let ctx = ui.ctx();
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

            let found_fx = resolve_fx_hover(ctx, pos);

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
            let hover_fx: Option<FxHover> =
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

            if let Some(hover) = hover_fx {
                let fx_key = egui::Id::new("__lib_dnd_fx_idx");
                let filter_idx: Option<usize> = ctx.memory(|mem| mem.data.get_temp(fx_key));
                if let Some(filter_idx) = filter_idx {
                    if let Some(shader_name) = resolve_filter_name(data, filter_idx) {
                        let (target, select) = match hover {
                            FxHover::Deck {
                                uuid,
                                ch_idx,
                                deck_idx,
                            } => (EffectTarget::Deck(uuid), FxSelect::Deck(ch_idx, deck_idx)),
                            FxHover::Channel { uuid, ch_idx } => {
                                (EffectTarget::Channel(uuid), FxSelect::Channel(ch_idx))
                            }
                            FxHover::Master => (EffectTarget::Master, FxSelect::Master),
                        };
                        log::info!("Library drop (deferred): effect {filter_idx} -> {target:?}");
                        match select {
                            FxSelect::Deck(ch, dk) => actions.session.select_deck = Some((ch, dk)),
                            FxSelect::Channel(ch) => actions.session.select_channel = Some(ch),
                            FxSelect::Master => actions.session.select_master = true,
                        }
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
                mem.data.remove::<Option<FxHover>>(hover_fx_target_id);
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
pub(super) fn handle_effect_dnd(ui: &egui::Ui, data: &UIData, actions: &mut UIActions) {
    let ctx = ui.ctx();
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
pub(super) fn handle_sequence_step_dnd(ui: &egui::Ui, _data: &UIData, actions: &mut UIActions) {
    let ctx = ui.ctx();
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

#[cfg(test)]
mod tests {
    use super::{resolve_fx_from_surfaces, FxHover};
    use egui::{pos2, Rect};

    fn rect(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Rect {
        Rect::from_min_max(pos2(min_x, min_y), pos2(max_x, max_y))
    }

    fn deck(uuid: &str, ch_idx: usize, deck_idx: usize) -> FxHover {
        FxHover::Deck {
            uuid: uuid.to_string(),
            ch_idx,
            deck_idx,
        }
    }

    fn channel(uuid: &str, ch_idx: usize) -> FxHover {
        FxHover::Channel {
            uuid: uuid.to_string(),
            ch_idx,
        }
    }

    #[test]
    fn deck_inside_channel_wins() {
        let surfaces = vec![
            (channel("ch-a", 0), rect(0.0, 0.0, 100.0, 100.0)),
            (deck("deck-a", 0, 0), rect(10.0, 10.0, 50.0, 50.0)),
        ];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(20.0, 20.0), &surfaces),
            Some(deck("deck-a", 0, 0))
        );
    }

    #[test]
    fn channel_empty_space_hits_channel() {
        let surfaces = vec![
            (channel("ch-a", 0), rect(0.0, 0.0, 100.0, 100.0)),
            (deck("deck-a", 0, 0), rect(10.0, 10.0, 50.0, 50.0)),
        ];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(80.0, 80.0), &surfaces),
            Some(channel("ch-a", 0))
        );
    }

    /// The card under the pointer takes the drop, not the deck whose chain
    /// happens to be open in the bottom bar.
    #[test]
    fn a_drop_lands_on_the_hovered_deck_not_the_selected_one() {
        let surfaces = vec![
            (deck("deck-a", 0, 0), rect(0.0, 0.0, 100.0, 100.0)),
            (deck("deck-a", 0, 0), rect(0.0, 400.0, 800.0, 500.0)),
            (deck("deck-b", 0, 1), rect(120.0, 0.0, 220.0, 100.0)),
        ];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(150.0, 50.0), &surfaces),
            Some(deck("deck-b", 0, 1))
        );
    }

    /// Only the selected deck draws a bottom-bar chain, so the frame's surfaces
    /// name it and no deck that was selected earlier can still claim the area.
    #[test]
    fn the_bottom_bar_chain_belongs_to_whichever_deck_drew_it() {
        let chain = rect(0.0, 400.0, 800.0, 500.0);
        let first = vec![(deck("deck-a", 0, 0), chain)];
        let second = vec![(deck("deck-b", 0, 1), chain)];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(400.0, 450.0), &first),
            Some(deck("deck-a", 0, 0))
        );
        assert_eq!(
            resolve_fx_from_surfaces(pos2(400.0, 450.0), &second),
            Some(deck("deck-b", 0, 1))
        );
    }

    #[test]
    fn master_preview_hits_without_selection() {
        let surfaces = vec![(FxHover::Master, rect(200.0, 0.0, 300.0, 100.0))];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(250.0, 50.0), &surfaces),
            Some(FxHover::Master)
        );
    }

    #[test]
    fn master_beats_overlapping_channel() {
        let surfaces = vec![
            (channel("ch-a", 0), rect(0.0, 0.0, 100.0, 100.0)),
            (FxHover::Master, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(50.0, 50.0), &surfaces),
            Some(FxHover::Master)
        );
    }

    #[test]
    fn automation_under_deck_is_deck_surface() {
        let surfaces = vec![(deck("deck-a", 0, 1), rect(0.0, 40.0, 400.0, 60.0))];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(100.0, 50.0), &surfaces),
            Some(deck("deck-a", 0, 1))
        );
    }

    #[test]
    fn empty_space_resolves_to_nothing() {
        let surfaces = vec![(deck("deck-a", 0, 0), rect(0.0, 0.0, 100.0, 100.0))];
        assert_eq!(
            resolve_fx_from_surfaces(pos2(500.0, 500.0), &surfaces),
            None
        );
    }
}
