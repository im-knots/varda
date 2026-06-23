//! Mixer & crossfader write routes.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct CrossfaderBody {
    /// Crossfader position from 0.0 (left) to 1.0 (right).
    pub position: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct AutoCrossfadeBody {
    /// Target crossfader position (0.0–1.0).
    pub target: f32,
    /// Duration of the automated crossfade in seconds.
    pub duration_secs: f32,
    /// Easing curve applied to the crossfade animation.
    pub easing: crate::engine::CrossfadeEasing,
}

#[derive(Deserialize, ToSchema)]
pub struct BeatCrossfadeBody {
    /// Target crossfader position (0.0–1.0).
    pub target: f32,
    /// Number of beats over which the crossfade occurs.
    pub beats: f32,
}

#[utoipa::path(put, path = "/api/mixer/crossfader", request_body = CrossfaderBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn set_crossfader(
    State(state): State<SharedState>,
    Json(body): Json<CrossfaderBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetCrossfader(body.position))
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/mixer/auto-crossfade", request_body = AutoCrossfadeBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn auto_crossfade(
    State(state): State<SharedState>,
    Json(body): Json<AutoCrossfadeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AutoCrossfade {
            target: body.target,
            duration_secs: body.duration_secs,
            easing: body.easing,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/mixer/beat-crossfade", request_body = BeatCrossfadeBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn beat_crossfade(
    State(state): State<SharedState>,
    Json(body): Json<BeatCrossfadeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::BeatCrossfade {
            target: body.target,
            beats: body.beats,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct TonemapModeBody {
    /// Tonemapping mode: "bypass" or "aces".
    pub mode: crate::renderer::tonemap::TonemapMode,
}

#[utoipa::path(put, path = "/api/mixer/tonemap", request_body = TonemapModeBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn set_tonemap_mode(
    State(state): State<SharedState>,
    Json(body): Json<TonemapModeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetTonemapMode(body.mode))
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct LoadLutBody {
    /// LUT filename (relative to `.varda/luts/`)
    pub filename: String,
}

#[utoipa::path(put, path = "/api/mixer/lut", request_body = LoadLutBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn load_lut(
    State(state): State<SharedState>,
    Json(body): Json<LoadLutBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::LoadLut {
            filename: body.filename,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/mixer/lut", responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn unload_lut(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::UnloadLut).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
