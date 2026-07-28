//! Library routes: GET /api/library/*

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::usecases::api::projection::{
    self, CameraEntry, DepthSensorEntry, MonitorEntry, NdiSourceEntry, ShaderEntry, StateReadError,
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

/// Generator shaders available in the registry, with their registry indices.
#[utoipa::path(get, path = "/api/library/generators",
    responses((status = 200, body = Vec<ShaderEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Effect (filter) shaders available in the registry, with their registry indices.
#[utoipa::path(get, path = "/api/library/effects",
    responses((status = 200, body = Vec<ShaderEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Names of the transition shaders the crossfader can use.
#[utoipa::path(get, path = "/api/library/transitions",
    responses((status = 200, body = Vec<TransitionEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Camera devices discovered by the last scan, as name and device id.
#[utoipa::path(get, path = "/api/library/cameras",
    responses((status = 200, body = Vec<CameraEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Depth sensors discovered by the last scan, as name and sensor id.
#[utoipa::path(get, path = "/api/library/depth",
    responses((status = 200, body = Vec<DepthSensorEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Depth Sensors")]
pub async fn depth(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(
            s.depth_sensors
                .devices
                .iter()
                .map(|(name, id)| DepthSensorEntry {
                    name: name.clone(),
                    id: *id,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Names of the NDI sources discovered by the last scan.
#[utoipa::path(get, path = "/api/library/ndi",
    responses((status = 200, body = Vec<NdiSourceEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Names of the Syphon servers discovered by the last scan.
#[utoipa::path(get, path = "/api/library/syphon",
    responses((status = 200, body = Vec<SyphonSourceEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Connected monitors available as output displays, with name, index, and pixel size.
#[utoipa::path(get, path = "/api/library/monitors",
    responses((status = 200, body = Vec<MonitorEntry>), (status = 503, description = "Engine not yet initialized")),
    tag = "Library")]
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

/// Analyzer types a deck can attach, with their names and parameter descriptors.
#[utoipa::path(get, path = "/api/library/analyzers",
    responses((status = 200, body = Vec<crate::engine::types::AnalyzerTypeInfo>)),
    tag = "Analyzers")]
pub async fn analyzers(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(s.analyzers).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}
