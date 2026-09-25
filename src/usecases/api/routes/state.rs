//! Read-only runtime state routes: GET /api/state/*

use super::read_or_error;
use crate::usecases::api::projection;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use crate::usecases::api::SharedState;

/// Defines a route that serializes one subtree of `EngineState`.
///
/// The subtree snapshot types are `Serialize` only, so the operation documents
/// its payload in prose rather than referencing a schema — same treatment as
/// `GET /api/state`.
macro_rules! state_route {
    ($name:ident, $path:literal, $summary:literal, $($field:ident).+) => {
        #[doc = $summary]
        #[utoipa::path(get, path = $path,
            responses((status = 200, description = $summary), (status = 503, description = "Engine not yet initialized")),
            tag = "State")]
        pub async fn $name(State(state): State<SharedState>) -> impl IntoResponse {
            match read_or_error(&state) {
                // Serialized straight from the shared snapshot, no copy.
                Ok(s) => Json(&s.state.$($field).+).into_response(),
                Err((status, msg)) => (status, msg).into_response(),
            }
        }
    };
}

state_route!(
    mixer,
    "/api/state/mixer",
    "Mixer state: channels, crossfader position, master effects, active transition, and sequences.",
    mixer
);
state_route!(
    audio,
    "/api/state/audio",
    "Audio analysis state: level, band energies, FFT bins, detected BPM, and input devices.",
    audio
);
state_route!(
    modulation,
    "/api/state/modulation",
    "Modulation state: sources, their current output values, and parameter assignments.",
    modulation
);
state_route!(
    outputs,
    "/api/state/outputs",
    "Output state: output windows, surfaces, and connected monitors.",
    outputs
);
state_route!(
    surfaces,
    "/api/state/surfaces",
    "Every surface with its geometry, warp, and source assignment.",
    outputs.surfaces
);
state_route!(
    registry,
    "/api/state/registry",
    "Shader registry: generator and filter shader names with their indices.",
    registry
);
state_route!(
    macros,
    "/api/state/macros",
    "Every macro control with its kind, current value, and parameter targets.",
    macros
);
state_route!(
    midi,
    "/api/state/midi",
    "MIDI state: devices, mappings, and whether learn mode is active.",
    midi
);
state_route!(
    cameras,
    "/api/state/cameras",
    "Camera devices discovered by the last scan.",
    cameras
);
state_route!(
    depth,
    "/api/state/depth",
    "Depth sensors discovered by the last scan.",
    depth_sensors
);
state_route!(
    screen_capture,
    "/api/state/screen_capture",
    "Screen capture state: enumerated targets, permission state, backend, and active session count.",
    screen_capture
);
state_route!(
    clock,
    "/api/state/clock",
    "Clock state: resolved BPM, beat phase, active source, and detected clock sources.",
    clock
);
state_route!(
    transport,
    "/api/state/transport",
    "Transport state: absolute position, timecode, run status, loop region, and follower count.",
    transport
);
state_route!(
    deck_loads,
    "/api/state/deck-loads",
    "Decks being built in the background, then loads that failed in the last minute with the reason. Shader, image, and video decks answer their create request with a UUID straight away and appear in the mixer once built.",
    deck_loads
);
state_route!(
    dome,
    "/api/state/dome",
    "Dome projection the domemaster is rendered for: projector preset and dome geometry, content rotation included.",
    dome
);
state_route!(
    timecode,
    "/api/state/timecode",
    "Timecode diagnostics: every LTC and MTC input being listened to with its own position and run state, which one is driving the transport, and the current preference and LTC patch.",
    timecode
);
state_route!(
    arrangement,
    "/api/state/arrangement",
    "Arrangement state: authored lanes and regions, whether the arrangement holds authority, and which parameters a performer is holding by hand.",
    arrangement
);
state_route!(
    streams,
    "/api/state/streams",
    "Active stream receivers with their URL, mode, and connection status.",
    stream_receivers
);

/// NDI runtime availability and the source names found by the last scan.
#[utoipa::path(get, path = "/api/state/ndi",
    responses((status = 200, body = projection::NdiResponse), (status = 503, description = "Engine not yet initialized")),
    tag = "State")]
pub async fn ndi(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::NdiResponse {
            available: s.ndi_available,
            sources: s.ndi_sources.clone(),
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
            sources: s.syphon_sources.clone(),
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
