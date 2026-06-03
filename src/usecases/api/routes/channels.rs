//! Channel CRUD routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct ChannelOpacityBody {
    /// Opacity value from 0.0 (transparent) to 1.0 (opaque).
    pub opacity: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct ChannelBlendModeBody {
    /// Blend mode for compositing this channel.
    pub mode: crate::engine::BlendMode,
}

#[utoipa::path(post, path = "/api/channels", responses((status = 200, body = CommandResult)), tag = "Channels")]
pub async fn add_channel(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::AddChannel).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/channels/{idx}", params(("idx" = usize, Path, description = "Channel index")), responses((status = 200, body = CommandResult)), tag = "Channels")]
pub async fn remove_channel(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveChannel { channel_idx: idx })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{idx}/opacity", params(("idx" = usize, Path, description = "Channel index")), request_body = ChannelOpacityBody, responses((status = 200, body = CommandResult)), tag = "Channels")]
pub async fn set_opacity(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(body): Json<ChannelOpacityBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetChannelOpacity {
            channel_idx: idx,
            opacity: body.opacity,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{idx}/blend-mode", params(("idx" = usize, Path, description = "Channel index")), request_body = ChannelBlendModeBody, responses((status = 200, body = CommandResult)), tag = "Channels")]
pub async fn set_blend_mode(
    State(state): State<SharedState>,
    Path(idx): Path<usize>,
    Json(body): Json<ChannelBlendModeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetChannelBlendMode {
            channel_idx: idx,
            mode: body.mode,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
