//! Library routes: GET /api/library/*

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::usecases::api::projection::{
    self, CameraEntry, MonitorEntry, NdiSourceEntry, ShaderEntry, StateReadError,
    SyphonSourceEntry, TransitionEntry,
};
use crate::usecases::api::SharedState;

fn read_or_error(
    state: &SharedState,
) -> Result<crate::engine::EngineState, (StatusCode, &'static str)> {
    projection::read_state(&state.engine_state).map_err(|e| match e {
        StateReadError::NotInitialized => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Engine not yet initialized",
        ),
        StateReadError::LockPoisoned => (StatusCode::INTERNAL_SERVER_ERROR, "State lock poisoned"),
    })
}

/// GET /api/library/generators
pub async fn generators(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.registry
                .generators
                .iter()
                .map(|(name, idx)| ShaderEntry {
                    name: name.clone(),
                    index: *idx,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/effects
pub async fn effects(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.registry
                .filters
                .iter()
                .map(|(name, idx)| ShaderEntry {
                    name: name.clone(),
                    index: *idx,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/transitions
pub async fn transitions(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.mixer
                .transition_names
                .iter()
                .map(|name| TransitionEntry { name: name.clone() })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/cameras
pub async fn cameras(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.cameras
                .devices
                .iter()
                .map(|(name, id)| CameraEntry {
                    name: name.clone(),
                    id: *id,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/ndi
pub async fn ndi(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.ndi_sources
                .iter()
                .map(|name| NdiSourceEntry { name: name.clone() })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/syphon
pub async fn syphon(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.syphon_sources
                .iter()
                .map(|name| SyphonSourceEntry { name: name.clone() })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/monitors
pub async fn monitors(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.outputs
                .monitors
                .iter()
                .map(|m| MonitorEntry {
                    name: m.name.clone(),
                    index: m.index,
                    width: m.width,
                    height: m.height,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/library/analyzers
#[utoipa::path(get, path = "/api/library/analyzers",
    responses((status = 200, body = Vec<crate::engine::types::AnalyzerTypeInfo>)),
    tag = "Analyzers")]
pub async fn analyzers(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(s.analyzers).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}
