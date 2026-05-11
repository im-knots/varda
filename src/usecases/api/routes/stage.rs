//! Stage state routes: GET /api/stage/*

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::usecases::api::SharedState;
use crate::usecases::api::projection::{self, StateReadError};

fn read_or_error(state: &SharedState) -> Result<crate::engine::EngineState, (StatusCode, &'static str)> {
    projection::read_state(&state.engine_state).map_err(|e| match e {
        StateReadError::NotInitialized => (StatusCode::SERVICE_UNAVAILABLE, "Engine not yet initialized"),
        StateReadError::LockPoisoned => (StatusCode::INTERNAL_SERVER_ERROR, "State lock poisoned"),
    })
}

/// GET /api/stage — full stage state
pub async fn stage(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::project_stage(&s)).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/surfaces
pub async fn surfaces(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.outputs.surfaces).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/surfaces/:uuid
pub async fn surface_by_uuid(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_surface(&s, &uuid) {
            Some(surface) => Json(surface).into_response(),
            None => (StatusCode::NOT_FOUND, "Surface not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/outputs
pub async fn outputs(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.outputs.windows).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/outputs/:uuid
pub async fn output_by_uuid(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_output(&s, &uuid) {
            Some(output) => Json(output).into_response(),
            None => (StatusCode::NOT_FOUND, "Output not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}
