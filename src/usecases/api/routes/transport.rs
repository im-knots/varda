//! Transport control routes.
//!
//! The transport is a singleton, so these routes take no identifier.
//! See `/spec/transport.md`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct PositionBody {
    /// Absolute show position in seconds.
    pub position: f64,
}

#[derive(Deserialize, ToSchema)]
pub struct SourceBody {
    /// Where position comes from: `Internal` or `Timecode`.
    pub source: crate::transport::TransportSource,
}

#[derive(Deserialize, ToSchema)]
pub struct LoopBody {
    /// Range internal playback wraps within. `null` clears the loop.
    pub region: Option<crate::transport::LoopRegion>,
}

#[derive(Deserialize, ToSchema)]
pub struct RateBody {
    /// Frame rate positions are displayed and quantised at.
    pub rate: crate::transport::TimecodeRate,
}

#[utoipa::path(post, path = "/api/transport/play", responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn play(State(s): State<SharedState>) -> impl IntoResponse {
    match s.send_command(EngineCommand::TransportPlay).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/transport/stop", responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn stop(State(s): State<SharedState>) -> impl IntoResponse {
    match s.send_command(EngineCommand::TransportStop).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/transport/locate", request_body = PositionBody, responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn locate(
    State(s): State<SharedState>,
    Json(b): Json<PositionBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::TransportLocate {
            position: b.position,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/transport/cue/prev", responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn prev_cue(State(s): State<SharedState>) -> impl IntoResponse {
    match s.send_command(EngineCommand::TransportPrevCue).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/transport/cue/next", responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn next_cue(State(s): State<SharedState>) -> impl IntoResponse {
    match s.send_command(EngineCommand::TransportNextCue).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

/// Locate to one named cue, leaving the transport running or stopped as it was.
/// What the Performance-mode cue pads and a mapped foot switch both do.
#[utoipa::path(post, path = "/api/transport/cue/{uuid}", params(("uuid" = String, Path, description = "Cue UUID")), responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn trigger_cue(
    State(s): State<SharedState>,
    axum::extract::Path(uuid): axum::extract::Path<String>,
) -> impl IntoResponse {
    match s.send_command(EngineCommand::TriggerCue { uuid }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/transport/source", request_body = SourceBody, responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn set_source(
    State(s): State<SharedState>,
    Json(b): Json<SourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetTransportSource { source: b.source })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/transport/loop", request_body = LoopBody, responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn set_loop(State(s): State<SharedState>, Json(b): Json<LoopBody>) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetTransportLoop { region: b.region })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/transport/rate", request_body = RateBody, responses((status = 200, body = CommandResult)), tag = "Transport")]
pub async fn set_rate(State(s): State<SharedState>, Json(b): Json<RateBody>) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetTimecodeRate { rate: b.rate })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
