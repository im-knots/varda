//! Deck CRUD and property routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

/// Strip `..` components from a path to prevent directory traversal attacks.
/// If the path can be canonicalized (i.e. it exists), use the canonical form;
/// otherwise strip `..` components manually and return the cleaned path.
fn sanitize_path(p: std::path::PathBuf) -> std::path::PathBuf {
    if let Ok(canonical) = p.canonicalize() {
        return canonical;
    }
    // File doesn't exist yet or can't be resolved — strip traversal components
    p.components()
        .filter(|c| !matches!(c, std::path::Component::ParentDir))
        .collect()
}

#[derive(Deserialize, ToSchema)]
pub struct AddShaderDeckBody {
    /// Name of the shader to load into the new deck.
    pub shader_name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckOpacityBody {
    /// Opacity value from 0.0 (transparent) to 1.0 (opaque).
    pub opacity: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckBlendModeBody {
    /// Blend mode for compositing this deck.
    pub mode: crate::engine::BlendMode,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckBoolBody {
    /// Boolean toggle value.
    pub value: bool,
}

#[utoipa::path(post, path = "/api/channels/{ch_idx}/decks/shader", params(("ch_idx" = usize, Path, description = "Channel index")), request_body = AddShaderDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_shader_deck(
    State(state): State<SharedState>,
    Path(ch_idx): Path<usize>,
    Json(body): Json<AddShaderDeckBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::AddDeck {
        channel_idx: ch_idx,
        shader_name: body.shader_name,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/channels/{ch_idx}/decks/{deck_idx}", params(("ch_idx" = usize, Path, description = "Channel index"), ("deck_idx" = usize, Path, description = "Deck index within the channel")), responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn remove_deck(
    State(state): State<SharedState>,
    Path((ch_idx, deck_idx)): Path<(usize, usize)>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::RemoveDeck {
        channel_idx: ch_idx,
        deck_idx,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{ch_idx}/decks/{deck_idx}/opacity", params(("ch_idx" = usize, Path, description = "Channel index"), ("deck_idx" = usize, Path, description = "Deck index within the channel")), request_body = DeckOpacityBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn set_opacity(
    State(state): State<SharedState>,
    Path((ch_idx, deck_idx)): Path<(usize, usize)>,
    Json(body): Json<DeckOpacityBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetDeckOpacity {
        channel_idx: ch_idx,
        deck_idx,
        opacity: body.opacity,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{ch_idx}/decks/{deck_idx}/blend-mode", params(("ch_idx" = usize, Path, description = "Channel index"), ("deck_idx" = usize, Path, description = "Deck index within the channel")), request_body = DeckBlendModeBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn set_blend_mode(
    State(state): State<SharedState>,
    Path((ch_idx, deck_idx)): Path<(usize, usize)>,
    Json(body): Json<DeckBlendModeBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetDeckBlendMode {
        channel_idx: ch_idx,
        deck_idx,
        mode: body.mode,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{ch_idx}/decks/{deck_idx}/solo", params(("ch_idx" = usize, Path, description = "Channel index"), ("deck_idx" = usize, Path, description = "Deck index within the channel")), request_body = DeckBoolBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn set_solo(
    State(state): State<SharedState>,
    Path((ch_idx, deck_idx)): Path<(usize, usize)>,
    Json(body): Json<DeckBoolBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetDeckSolo {
        channel_idx: ch_idx,
        deck_idx,
        solo: body.value,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{ch_idx}/decks/{deck_idx}/mute", params(("ch_idx" = usize, Path, description = "Channel index"), ("deck_idx" = usize, Path, description = "Deck index within the channel")), request_body = DeckBoolBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn set_mute(
    State(state): State<SharedState>,
    Path((ch_idx, deck_idx)): Path<(usize, usize)>,
    Json(body): Json<DeckBoolBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetDeckMute {
        channel_idx: ch_idx,
        deck_idx,
        mute: body.value,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddImageDeckBody {
    /// File path to the image asset.
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

#[derive(Deserialize, ToSchema)]
pub struct AddVideoDeckBody {
    /// File path to the video asset.
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

#[derive(Deserialize, ToSchema)]
pub struct AddSolidColorDeckBody {
    /// RGBA colour as four floats in 0.0–1.0.
    pub color: [f32; 4],
}

#[derive(Deserialize, ToSchema)]
pub struct AddCameraDeckBody {
    /// Numeric identifier of the camera device.
    pub camera_id: u32,
}

#[derive(Deserialize, ToSchema)]
pub struct MoveDeckBody {
    /// Source channel index.
    pub src_ch: usize,
    /// Source deck index within that channel.
    pub src_deck: usize,
    /// Destination channel index.
    pub dst_ch: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderDeckBody {
    /// Channel index.
    pub ch: usize,
    /// Current deck index within the channel.
    pub from_idx: usize,
    /// Target deck index within the channel.
    pub to_idx: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckScalingModeBody {
    /// How the deck content is scaled to fit the output.
    pub mode: crate::internal::deck::ScalingMode,
}

#[derive(Deserialize, ToSchema)]
pub struct SetTransitionBody {
    /// Shader name for the transition, or null to clear.
    pub shader_name: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetParamBody {
    /// Dot-separated path identifying the parameter.
    pub path: String,
    /// New value for the parameter.
    pub value: crate::internal::params::ParamValue,
}

#[utoipa::path(post, path = "/api/channels/{ch_idx}/decks/image", params(("ch_idx" = usize, Path, description = "Channel index")), request_body = AddImageDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_image_deck(
    State(state): State<SharedState>,
    Path(ch_idx): Path<usize>,
    Json(body): Json<AddImageDeckBody>,
) -> impl IntoResponse {
    let path = sanitize_path(body.path);
    match state.send_command(EngineCommand::AddImageDeck { channel_idx: ch_idx, path }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{ch_idx}/decks/video", params(("ch_idx" = usize, Path, description = "Channel index")), request_body = AddVideoDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_video_deck(
    State(state): State<SharedState>,
    Path(ch_idx): Path<usize>,
    Json(body): Json<AddVideoDeckBody>,
) -> impl IntoResponse {
    let path = sanitize_path(body.path);
    match state.send_command(EngineCommand::AddVideoDeck { channel_idx: ch_idx, path }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{ch_idx}/decks/solid", params(("ch_idx" = usize, Path, description = "Channel index")), request_body = AddSolidColorDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_solid_color_deck(
    State(state): State<SharedState>,
    Path(ch_idx): Path<usize>,
    Json(body): Json<AddSolidColorDeckBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::AddSolidColorDeck { channel_idx: ch_idx, color: body.color }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{ch_idx}/decks/camera", params(("ch_idx" = usize, Path, description = "Channel index")), request_body = AddCameraDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_camera_deck(
    State(state): State<SharedState>,
    Path(ch_idx): Path<usize>,
    Json(body): Json<AddCameraDeckBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::AddCameraDeck { channel_idx: ch_idx, camera_id: body.camera_id }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/decks/move", request_body = MoveDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn move_deck(
    State(state): State<SharedState>,
    Json(body): Json<MoveDeckBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::MoveDeck {
        src_ch: body.src_ch, src_deck: body.src_deck, dst_ch: body.dst_ch,
    }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/decks/reorder", request_body = ReorderDeckBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn reorder_deck(
    State(state): State<SharedState>,
    Json(body): Json<ReorderDeckBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::ReorderDeck {
        ch: body.ch, from_idx: body.from_idx, to_idx: body.to_idx,
    }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{ch_idx}/decks/{deck_idx}/scaling-mode", params(("ch_idx" = usize, Path, description = "Channel index"), ("deck_idx" = usize, Path, description = "Deck index within the channel")), request_body = DeckScalingModeBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn set_scaling_mode(
    State(state): State<SharedState>,
    Path((ch_idx, deck_idx)): Path<(usize, usize)>,
    Json(body): Json<DeckScalingModeBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetDeckScalingMode {
        channel_idx: ch_idx, deck_idx, mode: body.mode,
    }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/mixer/transition", request_body = SetTransitionBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn set_transition(
    State(state): State<SharedState>,
    Json(body): Json<SetTransitionBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetTransition { shader_name: body.shader_name }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/params", request_body = SetParamBody, responses((status = 200, body = CommandResult)), tag = "Params")]
pub async fn set_param(
    State(state): State<SharedState>,
    Json(body): Json<SetParamBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::SetParam { path: body.path, value: body.value }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// POST /api/command — generic command passthrough (not documented in OpenAPI due to large enum body)
pub async fn generic_command(
    State(state): State<SharedState>,
    Json(cmd): Json<EngineCommand>,
) -> impl IntoResponse {
    match state.send_command(cmd).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

// ── Video Playback ─────────────────────────────────────────────────

#[utoipa::path(post, path = "/api/channels/{ch}/decks/{dk}/video/toggle-play", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_toggle_play(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoTogglePlay { channel_idx: ch, deck_idx: dk }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoSeekBody {
    /// Seek position in seconds from the start of the video.
    pub position_secs: f64,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/video/seek", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = VideoSeekBody, responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_seek(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<VideoSeekBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoSeek { channel_idx: ch, deck_idx: dk, position_secs: b.position_secs }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoSpeedBody {
    /// Playback speed multiplier (1.0 = normal speed).
    pub speed: f64,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/video/speed", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = VideoSpeedBody, responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_set_speed(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<VideoSpeedBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoSetSpeed { channel_idx: ch, deck_idx: dk, speed: b.speed }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoLoopModeBody {
    /// Loop behaviour for the video.
    pub mode: crate::video::LoopMode,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/video/loop-mode", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = VideoLoopModeBody, responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_set_loop_mode(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<VideoLoopModeBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoSetLoopMode { channel_idx: ch, deck_idx: dk, mode: b.mode }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoPointBody {
    /// Time position in seconds.
    pub secs: f64,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/video/in-point", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = VideoPointBody, responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_set_in_point(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<VideoPointBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoSetInPoint { channel_idx: ch, deck_idx: dk, secs: b.secs }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/video/out-point", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = VideoPointBody, responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_set_out_point(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<VideoPointBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoSetOutPoint { channel_idx: ch, deck_idx: dk, secs: b.secs }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(delete, path = "/api/channels/{ch}/decks/{dk}/video/in-out-points", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), responses((status = 200, body = CommandResult)), tag = "Video")]
pub async fn video_clear_in_out(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>) -> impl IntoResponse {
    match s.send_command(EngineCommand::VideoClearInOutPoints { channel_idx: ch, deck_idx: dk }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Auto-Transitions ───────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AutoTransBoolBody {
    /// Boolean toggle value.
    pub value: bool,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/auto-transition/enabled", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = AutoTransBoolBody, responses((status = 200, body = CommandResult)), tag = "Auto Transitions")]
pub async fn set_auto_transition_enabled(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<AutoTransBoolBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::SetAutoTransitionEnabled { channel_idx: ch, deck_idx: dk, enabled: b.value }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/auto-transition/trigger", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = AutoTransBoolBody, responses((status = 200, body = CommandResult)), tag = "Auto Transitions")]
pub async fn set_auto_transition_trigger(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<AutoTransBoolBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::SetAutoTransitionTrigger { channel_idx: ch, deck_idx: dk, clip_end: b.value }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DurationBody {
    /// Numeric duration value.
    pub value: f64,
    /// Unit of the duration (seconds or beats).
    pub unit: crate::channel::DurationUnit,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/auto-transition/play-duration", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = DurationBody, responses((status = 200, body = CommandResult)), tag = "Auto Transitions")]
pub async fn set_auto_transition_play_duration(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<DurationBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::SetAutoTransitionPlayDuration { channel_idx: ch, deck_idx: dk, value: b.value, unit: b.unit }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/auto-transition/duration", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = DurationBody, responses((status = 200, body = CommandResult)), tag = "Auto Transitions")]
pub async fn set_auto_transition_duration(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<DurationBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::SetAutoTransitionDuration { channel_idx: ch, deck_idx: dk, value: b.value, unit: b.unit }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ShaderNameBody {
    /// Shader name, or null to clear.
    pub shader_name: Option<String>,
}
#[utoipa::path(put, path = "/api/channels/{ch}/decks/{dk}/auto-transition/shader", params(("ch" = usize, Path, description = "Channel index"), ("dk" = usize, Path, description = "Deck index")), request_body = ShaderNameBody, responses((status = 200, body = CommandResult)), tag = "Auto Transitions")]
pub async fn set_auto_transition_shader(State(s): State<SharedState>, Path((ch, dk)): Path<(usize, usize)>, Json(b): Json<ShaderNameBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::SetAutoTransitionShader { channel_idx: ch, deck_idx: dk, shader_name: b.shader_name }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── External I/O Sources ───────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct NdiSourceBody {
    /// Name of the NDI source to receive.
    pub source_name: String,
}
#[utoipa::path(post, path = "/api/channels/{ch}/decks/ndi", params(("ch" = usize, Path, description = "Channel index")), request_body = NdiSourceBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_ndi_deck(State(s): State<SharedState>, Path(ch): Path<usize>, Json(b): Json<NdiSourceBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::AddNdiDeck { channel_idx: ch, source_name: b.source_name }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SyphonSourceBody {
    /// Name of the Syphon server to receive.
    pub server_name: String,
}
#[utoipa::path(post, path = "/api/channels/{ch}/decks/syphon", params(("ch" = usize, Path, description = "Channel index")), request_body = SyphonSourceBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_syphon_deck(State(s): State<SharedState>, Path(ch): Path<usize>, Json(b): Json<SyphonSourceBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::AddSyphonDeck { channel_idx: ch, server_name: b.server_name }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SrtSourceBody {
    /// SRT stream URL.
    pub url: String,
    /// SRT connection mode (caller or listener).
    pub mode: crate::stream::SrtMode,
}
#[utoipa::path(post, path = "/api/channels/{ch}/decks/srt", params(("ch" = usize, Path, description = "Channel index")), request_body = SrtSourceBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_srt_deck(State(s): State<SharedState>, Path(ch): Path<usize>, Json(b): Json<SrtSourceBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::AddSrtDeck { channel_idx: ch, url: b.url, mode: b.mode }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct HlsSourceBody {
    /// HLS stream URL (.m3u8).
    pub url: String,
}
#[utoipa::path(post, path = "/api/channels/{ch}/decks/hls", params(("ch" = usize, Path, description = "Channel index")), request_body = HlsSourceBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_hls_deck(State(s): State<SharedState>, Path(ch): Path<usize>, Json(b): Json<HlsSourceBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::AddHlsDeck { channel_idx: ch, url: b.url }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DashSourceBody {
    /// DASH stream URL (.mpd).
    pub url: String,
}
#[utoipa::path(post, path = "/api/channels/{ch}/decks/dash", params(("ch" = usize, Path, description = "Channel index")), request_body = DashSourceBody, responses((status = 200, body = CommandResult)), tag = "Decks")]
pub async fn add_dash_deck(State(s): State<SharedState>, Path(ch): Path<usize>, Json(b): Json<DashSourceBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::AddDashDeck { channel_idx: ch, url: b.url }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Missing Parity Routes ─────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ResetParamsBody {
    /// Channel index of the deck whose params to reset.
    pub channel_idx: usize,
    /// Deck index within the channel.
    pub deck_idx: usize,
}

#[utoipa::path(post, path = "/api/params/reset", request_body = ResetParamsBody, responses((status = 200, body = CommandResult)), tag = "Params")]
pub async fn reset_generator_params(State(s): State<SharedState>, Json(b): Json<ResetParamsBody>) -> impl IntoResponse {
    match s.send_command(EngineCommand::ResetGeneratorParamsToDefaults { channel_idx: b.channel_idx, deck_idx: b.deck_idx }).await {
        Ok(r) => command_response(r), Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
