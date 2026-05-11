//! Read-only runtime state routes: GET /api/state/*

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::usecases::api::SharedState;
use crate::usecases::api::projection::{self, StateReadError};

/// Helper: read state or return appropriate HTTP error.
fn read_or_error(state: &SharedState) -> Result<crate::engine::EngineState, (StatusCode, &'static str)> {
    projection::read_state(&state.engine_state).map_err(|e| match e {
        StateReadError::NotInitialized => (StatusCode::SERVICE_UNAVAILABLE, "Engine not yet initialized"),
        StateReadError::LockPoisoned => (StatusCode::INTERNAL_SERVER_ERROR, "State lock poisoned"),
    })
}

macro_rules! state_route {
    ($name:ident, $field:expr) => {
        pub async fn $name(State(state): State<SharedState>) -> impl IntoResponse {
            match read_or_error(&state) {
                Ok(s) => Json(serde_json::to_value($field(&s)).unwrap()).into_response(),
                Err((status, msg)) => (status, msg).into_response(),
            }
        }
    };
}

state_route!(mixer, |s: &crate::engine::EngineState| s.mixer.clone());
state_route!(audio, |s: &crate::engine::EngineState| s.audio.clone());
state_route!(modulation, |s: &crate::engine::EngineState| s.modulation.clone());
state_route!(outputs, |s: &crate::engine::EngineState| s.outputs.clone());
state_route!(surfaces, |s: &crate::engine::EngineState| s.outputs.surfaces.clone());
state_route!(registry, |s: &crate::engine::EngineState| s.registry.clone());
state_route!(midi, |s: &crate::engine::EngineState| s.midi.clone());
state_route!(cameras, |s: &crate::engine::EngineState| s.cameras.clone());
state_route!(clock, |s: &crate::engine::EngineState| s.clock.clone());
state_route!(streams, |s: &crate::engine::EngineState| s.srt_receivers.clone());

pub async fn ndi(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::NdiResponse {
            available: s.ndi_available,
            sources: s.ndi_sources,
        }).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

pub async fn syphon(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::SyphonResponse {
            available: s.syphon_available,
            sources: s.syphon_sources,
        }).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

pub async fn performance(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::PerformanceResponse {
            fps: s.fps,
            frame_count: s.frame_count,
        }).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}
