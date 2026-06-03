//! Audio write routes.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct AudioSourceBody {
    /// Numeric identifier of the audio source device.
    pub source_id: u32,
}

#[utoipa::path(post, path = "/api/audio/scan", responses((status = 200, body = CommandResult)), tag = "Audio")]
pub async fn scan_devices(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::ScanAudioDevices).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/audio/open", request_body = AudioSourceBody, responses((status = 200, body = CommandResult)), tag = "Audio")]
pub async fn open_source(
    State(state): State<SharedState>,
    Json(body): Json<AudioSourceBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::OpenAudioSource {
            source_id: body.source_id,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/audio/close", request_body = AudioSourceBody, responses((status = 200, body = CommandResult)), tag = "Audio")]
pub async fn close_source(
    State(state): State<SharedState>,
    Json(body): Json<AudioSourceBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::CloseAudioSource {
            source_id: body.source_id,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
