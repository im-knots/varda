//! Effects write routes.
//!
//! Effect UUIDs are globally unique, so routes that name an existing effect are
//! flat (`/api/effects/{effect_uuid}`). Creation and reorder keep the owning
//! chain in the path: creation has no effect UUID yet, and reorder ordinals are
//! scoped to a single chain.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::types::EffectTarget;
use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct AddEffectBody {
    /// Name of the shader to use as an effect.
    pub shader_name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderEffectBody {
    /// Current index of the effect within the chain.
    pub from_idx: usize,
    /// Destination index within the chain.
    pub to_idx: usize,
}

async fn add_effect(
    state: SharedState,
    target: EffectTarget,
    shader_name: String,
) -> axum::response::Response {
    match state
        .send_command(EngineCommand::AddEffect {
            target,
            shader_name,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

async fn move_effect(
    state: SharedState,
    target: EffectTarget,
    body: ReorderEffectBody,
) -> axum::response::Response {
    match state
        .send_command(EngineCommand::MoveEffect {
            target,
            from_idx: body.from_idx,
            to_idx: body.to_idx,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/effects", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddEffectBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Effects")]
pub async fn add_channel_effect(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddEffectBody>,
) -> impl IntoResponse {
    add_effect(state, EffectTarget::Channel(channel_uuid), body.shader_name).await
}

#[utoipa::path(post, path = "/api/decks/{deck_uuid}/effects", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = AddEffectBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Effects")]
pub async fn add_deck_effect(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<AddEffectBody>,
) -> impl IntoResponse {
    add_effect(state, EffectTarget::Deck(deck_uuid), body.shader_name).await
}

#[utoipa::path(post, path = "/api/master/effects", request_body = AddEffectBody, responses((status = 200, body = CommandResult)), tag = "Effects")]
pub async fn add_master_effect(
    State(state): State<SharedState>,
    Json(body): Json<AddEffectBody>,
) -> impl IntoResponse {
    add_effect(state, EffectTarget::Master, body.shader_name).await
}

#[utoipa::path(delete, path = "/api/effects/{effect_uuid}", params(("effect_uuid" = String, Path, description = "Effect UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Effect not found")), tag = "Effects")]
pub async fn remove_effect(
    State(state): State<SharedState>,
    Path(effect_uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveEffect { effect_uuid })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/effects/{effect_uuid}/toggle", params(("effect_uuid" = String, Path, description = "Effect UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Effect not found")), tag = "Effects")]
pub async fn toggle_effect(
    State(state): State<SharedState>,
    Path(effect_uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ToggleEffect { effect_uuid })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{channel_uuid}/effects/reorder", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = ReorderEffectBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Effects")]
pub async fn reorder_channel_effect(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<ReorderEffectBody>,
) -> impl IntoResponse {
    move_effect(state, EffectTarget::Channel(channel_uuid), body).await
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/effects/reorder", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = ReorderEffectBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Effects")]
pub async fn reorder_deck_effect(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<ReorderEffectBody>,
) -> impl IntoResponse {
    move_effect(state, EffectTarget::Deck(deck_uuid), body).await
}

#[utoipa::path(put, path = "/api/master/effects/reorder", request_body = ReorderEffectBody, responses((status = 200, body = CommandResult)), tag = "Effects")]
pub async fn reorder_master_effect(
    State(state): State<SharedState>,
    Json(body): Json<ReorderEffectBody>,
) -> impl IntoResponse {
    move_effect(state, EffectTarget::Master, body).await
}
