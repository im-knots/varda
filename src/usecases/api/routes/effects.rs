//! Effects write routes.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::types::EffectTarget;
use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

#[derive(Deserialize, ToSchema)]
pub struct AddEffectBody {
    /// Where to apply the effect (channel, deck, or master).
    pub target: EffectTarget,
    /// Name of the shader to use as an effect.
    pub shader_name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveEffectBody {
    /// Where to remove the effect from.
    pub target: EffectTarget,
    /// Index of the effect in the target's effect chain.
    pub effect_idx: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct ToggleEffectBody {
    /// Where the effect resides.
    pub target: EffectTarget,
    /// Index of the effect to toggle.
    pub effect_idx: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct MoveEffectBody {
    /// Where the effect resides.
    pub target: EffectTarget,
    /// Current index of the effect to move.
    pub from_idx: usize,
    /// Destination index for the effect.
    pub to_idx: usize,
}

#[utoipa::path(post, path = "/api/effects", request_body = AddEffectBody, responses((status = 200, body = CommandResult)), tag = "Effects")]
pub async fn add_effect(
    State(state): State<SharedState>,
    Json(body): Json<AddEffectBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::AddEffect {
        target: body.target,
        shader_name: body.shader_name,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/effects", request_body = RemoveEffectBody, responses((status = 200, body = CommandResult)), tag = "Effects")]
pub async fn remove_effect(
    State(state): State<SharedState>,
    Json(body): Json<RemoveEffectBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::RemoveEffect {
        target: body.target,
        effect_idx: body.effect_idx,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/effects/toggle", request_body = ToggleEffectBody, responses((status = 200, body = CommandResult)), tag = "Effects")]
pub async fn toggle_effect(
    State(state): State<SharedState>,
    Json(body): Json<ToggleEffectBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::ToggleEffect {
        target: body.target,
        effect_idx: body.effect_idx,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/effects/move", request_body = MoveEffectBody, responses((status = 200, body = CommandResult)), tag = "Effects")]
pub async fn move_effect(
    State(state): State<SharedState>,
    Json(body): Json<MoveEffectBody>,
) -> impl IntoResponse {
    match state.send_command(EngineCommand::MoveEffect {
        target: body.target,
        from_idx: body.from_idx,
        to_idx: body.to_idx,
    }).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
