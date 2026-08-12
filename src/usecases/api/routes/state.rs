//! Read-only runtime state routes: GET /api/state/*

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::usecases::api::projection::{self, StateReadError};
use crate::usecases::api::SharedState;

/// Helper: read state or return appropriate HTTP error.
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

/// Defines a route that serializes one subtree of `EngineState`.
///
/// The subtree snapshot types are `Serialize` only, so the operation documents
/// its payload in prose rather than referencing a schema — same treatment as
/// `GET /api/state`.
macro_rules! state_route {
    ($name:ident, $path:literal, $summary:literal, $field:expr) => {
        #[doc = $summary]
        #[utoipa::path(get, path = $path,
            responses((status = 200, description = $summary), (status = 503, description = "Engine not yet initialized")),
            tag = "State")]
        pub async fn $name(State(state): State<SharedState>) -> impl IntoResponse {
            match read_or_error(&state) {
                // `($field)` needs the parentheses: `#[utoipa::path]` re-emits the
                // body and drops the invisible grouping around an `expr` capture,
                // so `$field(&s)` would parse as a call on the closure's body.
                Ok(s) => Json(serde_json::to_value(($field)(&s)).unwrap()).into_response(),
                Err((status, msg)) => (status, msg).into_response(),
            }
        }
    };
}

state_route!(
    mixer,
    "/api/state/mixer",
    "Mixer state: channels, crossfader position, master effects, active transition, and sequences.",
    |s: &crate::engine::EngineState| s.mixer.clone()
);
state_route!(
    audio,
    "/api/state/audio",
    "Audio analysis state: level, band energies, FFT bins, detected BPM, and input devices.",
    |s: &crate::engine::EngineState| s.audio.clone()
);
state_route!(
    modulation,
    "/api/state/modulation",
    "Modulation state: sources, their current output values, and parameter assignments.",
    |s: &crate::engine::EngineState| s.modulation.clone()
);
state_route!(
    outputs,
    "/api/state/outputs",
    "Output state: output windows, surfaces, and connected monitors.",
    |s: &crate::engine::EngineState| s.outputs.clone()
);
state_route!(
    surfaces,
    "/api/state/surfaces",
    "Every surface with its geometry, warp, and source assignment.",
    |s: &crate::engine::EngineState| s.outputs.surfaces.clone()
);
state_route!(
    registry,
    "/api/state/registry",
    "Shader registry: generator and filter shader names with their indices.",
    |s: &crate::engine::EngineState| s.registry.clone()
);
state_route!(
    macros,
    "/api/state/macros",
    "Every macro control with its kind, current value, and parameter targets.",
    |s: &crate::engine::EngineState| s.macros.clone()
);
state_route!(
    midi,
    "/api/state/midi",
    "MIDI state: devices, mappings, and whether learn mode is active.",
    |s: &crate::engine::EngineState| s.midi.clone()
);
state_route!(
    cameras,
    "/api/state/cameras",
    "Camera devices discovered by the last scan.",
    |s: &crate::engine::EngineState| s.cameras.clone()
);
state_route!(
    depth,
    "/api/state/depth",
    "Depth sensors discovered by the last scan.",
    |s: &crate::engine::EngineState| s.depth_sensors.clone()
);
state_route!(
    screen_capture,
    "/api/state/screen_capture",
    "Screen capture state: enumerated targets, permission state, backend, and active session count.",
    |s: &crate::engine::EngineState| s.screen_capture.clone()
);
state_route!(
    clock,
    "/api/state/clock",
    "Clock state: resolved BPM, beat phase, active source, and detected clock sources.",
    |s: &crate::engine::EngineState| s.clock.clone()
);
state_route!(
    transport,
    "/api/state/transport",
    "Transport state: absolute position, timecode, run status, loop region, and follower count.",
    |s: &crate::engine::EngineState| s.transport.clone()
);
state_route!(
    arrangement,
    "/api/state/arrangement",
    "Arrangement state: authored lanes and regions, whether the arrangement holds authority, and which parameters a performer is holding by hand.",
    |s: &crate::engine::EngineState| s.arrangement.clone()
);
state_route!(
    streams,
    "/api/state/streams",
    "Active stream receivers with their URL, mode, and connection status.",
    |s: &crate::engine::EngineState| s.stream_receivers.clone()
);

/// NDI runtime availability and the source names found by the last scan.
#[utoipa::path(get, path = "/api/state/ndi",
    responses((status = 200, body = projection::NdiResponse), (status = 503, description = "Engine not yet initialized")),
    tag = "State")]
pub async fn ndi(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::NdiResponse {
            available: s.ndi_available,
            sources: s.ndi_sources,
        })
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Syphon framework availability and the server names found by the last scan.
#[utoipa::path(get, path = "/api/state/syphon",
    responses((status = 200, body = projection::SyphonResponse), (status = 503, description = "Engine not yet initialized")),
    tag = "State")]
pub async fn syphon(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::SyphonResponse {
            available: s.syphon_available,
            sources: s.syphon_sources,
        })
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Render loop counters: measured FPS, total frames rendered, and the configured target FPS.
#[utoipa::path(get, path = "/api/state/performance",
    responses((status = 200, body = projection::PerformanceResponse), (status = 503, description = "Engine not yet initialized")),
    tag = "State")]
pub async fn performance(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::PerformanceResponse {
            fps: s.fps,
            frame_count: s.frame_count,
            target_fps: s.target_fps,
        })
        .into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}
