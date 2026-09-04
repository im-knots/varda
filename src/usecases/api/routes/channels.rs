//! Channel CRUD routes.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

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

#[utoipa::path(delete, path = "/api/channels/{channel_uuid}", params(("channel_uuid" = String, Path, description = "Channel UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Channels")]
pub async fn remove_channel(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveChannel { channel_uuid })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{channel_uuid}/opacity", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = ChannelOpacityBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Channels")]
pub async fn set_opacity(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<ChannelOpacityBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetChannelOpacity {
            channel_uuid,
            opacity: body.opacity,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{channel_uuid}/blend-mode", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = ChannelBlendModeBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Channels")]
pub async fn set_blend_mode(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<ChannelBlendModeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetChannelBlendMode {
            channel_uuid,
            mode: body.mode,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
