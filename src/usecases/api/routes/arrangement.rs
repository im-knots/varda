//! Arrangement authoring routes.
//!
//! A lane is addressed by the deck it drives, because a lane *is* a deck's row
//! rather than an object with its own identity. Regions are addressed by
//! position within their lane. See `/spec/arrangement.md`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct RegionBody {
    /// Visibility span in show seconds, with its fades.
    #[serde(flatten)]
    pub region: crate::arrangement::RegionConfig,
}

#[derive(Deserialize, ToSchema)]
pub struct CollapsedBody {
    /// Whether the lane's automation rows are folded away.
    pub collapsed: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct IdleBody {
    /// What renders outside the arranged range.
    pub idle: crate::arrangement::IdleBehaviour,
}

#[derive(Deserialize, ToSchema)]
pub struct RearmBody {
    /// Ramp length back to the automated value. Omit for the default.
    #[serde(default)]
    pub seconds: Option<f64>,
}

#[utoipa::path(post, path = "/api/arrangement/lanes/{deck_uuid}", params(("deck_uuid" = String, Path, description = "Deck the lane drives")), responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn add_lane(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match s.send_command(EngineCommand::AddLane { deck_uuid }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/arrangement/lanes/{deck_uuid}", params(("deck_uuid" = String, Path, description = "Deck the lane drives")), responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn remove_lane(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RemoveLane { deck_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/arrangement/lanes/{deck_uuid}/collapsed", params(("deck_uuid" = String, Path, description = "Deck the lane drives")), request_body = CollapsedBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn set_lane_collapsed(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<CollapsedBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetLaneCollapsed {
            deck_uuid,
            collapsed: b.collapsed,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/arrangement/lanes/{deck_uuid}/regions", params(("deck_uuid" = String, Path, description = "Deck the lane drives")), request_body = RegionBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn add_region(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<RegionBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddRegion {
            deck_uuid,
            region: b.region,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/arrangement/lanes/{deck_uuid}/regions/{index}", params(("deck_uuid" = String, Path, description = "Deck the lane drives"), ("index" = usize, Path, description = "Region position within the lane")), request_body = RegionBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn update_region(
    State(s): State<SharedState>,
    Path((deck_uuid, index)): Path<(String, usize)>,
    Json(b): Json<RegionBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateRegion {
            deck_uuid,
            index,
            region: b.region,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/arrangement/lanes/{deck_uuid}/regions/{index}", params(("deck_uuid" = String, Path, description = "Deck the lane drives"), ("index" = usize, Path, description = "Region position within the lane")), responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn remove_region(
    State(s): State<SharedState>,
    Path((deck_uuid, index)): Path<(String, usize)>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RemoveRegion { deck_uuid, index })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/arrangement/idle", request_body = IdleBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn set_idle(State(s): State<SharedState>, Json(b): Json<IdleBody>) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetIdleBehaviour { idle: b.idle })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/arrangement/rearm/{param_key}", params(("param_key" = String, Path, description = "Modulation key of the held parameter")), request_body = RearmBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn rearm_param(
    State(s): State<SharedState>,
    Path(param_key): Path<String>,
    Json(b): Json<RearmBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RearmParam {
            param_key,
            seconds: b.seconds,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/arrangement/rearm", request_body = RearmBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn rearm_all(
    State(s): State<SharedState>,
    Json(b): Json<RearmBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RearmAll { seconds: b.seconds })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddCueBody {
    /// Absolute show seconds.
    pub at: f64,
    /// Omit to be named by how many cues exist.
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateCueBody {
    /// Omit to leave the position alone.
    #[serde(default)]
    pub at: Option<f64>,
    /// Omit to leave the name alone.
    #[serde(default)]
    pub name: Option<String>,
}

#[utoipa::path(post, path = "/api/arrangement/cues", request_body = AddCueBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn add_cue(State(s): State<SharedState>, Json(b): Json<AddCueBody>) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddCue {
            at: b.at,
            name: b.name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/arrangement/cues/{uuid}", params(("uuid" = String, Path, description = "Cue identifier")), request_body = UpdateCueBody, responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn update_cue(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<UpdateCueBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateCue {
            uuid,
            at: b.at,
            name: b.name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/arrangement/cues/{uuid}", params(("uuid" = String, Path, description = "Cue identifier")), responses((status = 200, body = CommandResult)), tag = "Arrangement")]
pub async fn remove_cue(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s.send_command(EngineCommand::RemoveCue { uuid }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
