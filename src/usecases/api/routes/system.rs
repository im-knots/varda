//! System routes: health check, shutdown.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::usecases::api::SharedState;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Service status string.
    pub status: &'static str,
}

#[utoipa::path(get, path = "/api/health", responses((status = 200, body = HealthResponse)), tag = "System")]
pub async fn health() -> impl IntoResponse {
    Json(HealthResponse { status: "ok" })
}

#[utoipa::path(get, path = "/api/state", responses((status = 200, description = "Full engine state"), (status = 503, description = "Engine not yet initialized")), tag = "System")]
pub async fn get_state(State(state): State<SharedState>) -> impl IntoResponse {
    match state.engine_state.read() {
        Ok(guard) => match guard.as_ref() {
            Some(engine_state) => Json(serde_json::to_value(engine_state).unwrap()).into_response(),
            None => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Engine not yet initialized",
            )
                .into_response(),
        },
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "State lock poisoned").into_response(),
    }
}

// ── Write commands ──────────────────────────────────────────────────

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::command_response;
use serde::Deserialize;

#[utoipa::path(post, path = "/api/shutdown", responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn shutdown(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::Shutdown).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/undo", responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn undo(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::Undo).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/redo", responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn redo(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::Redo).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ResolutionBody {
    /// Render width in pixels.
    pub width: u32,
    /// Render height in pixels.
    pub height: u32,
}
#[utoipa::path(put, path = "/api/resolution", request_body = ResolutionBody, responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn set_resolution(
    State(state): State<SharedState>,
    Json(b): Json<ResolutionBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetRenderResolution {
            width: b.width,
            height: b.height,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct TargetFpsBody {
    /// Target FPS (0 = uncapped).
    pub fps: u32,
}
#[utoipa::path(put, path = "/api/target-fps", request_body = TargetFpsBody, responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn set_target_fps(
    State(state): State<SharedState>,
    Json(b): Json<TargetFpsBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetTargetFps { fps: b.fps })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ClockPreferenceBody {
    /// Clock source preference.
    pub preference: crate::clock::ClockPreference,
}
#[utoipa::path(put, path = "/api/clock/preference", request_body = ClockPreferenceBody, responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn set_clock_preference(
    State(state): State<SharedState>,
    Json(b): Json<ClockPreferenceBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetClockPreference {
            preference: b.preference,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ManualBpmBody {
    /// Beats per minute.
    pub bpm: f32,
}
#[utoipa::path(put, path = "/api/clock/manual-bpm", request_body = ManualBpmBody, responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn set_manual_bpm(
    State(state): State<SharedState>,
    Json(b): Json<ManualBpmBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetManualBpm { bpm: b.bpm })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/workspace/save", responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn save_workspace(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::SaveWorkspace).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/workspace/load", responses((status = 200, body = CommandResult)), tag = "System")]
pub async fn load_workspace(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::LoadWorkspace).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Device Scanning & MIDI ─────────────────────────────────────────

#[utoipa::path(post, path = "/api/devices/ndi/scan", responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn scan_ndi(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::RescanNdi).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/devices/syphon/scan", responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn scan_syphon(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::RescanSyphon).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/devices/cameras/scan", responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn scan_cameras(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::RescanCameras).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/devices/midi/scan", responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn scan_midi(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::RescanMidi).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/devices/audio/scan", responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn scan_audio(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::RescanAudio).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AudioSourceEnabledBody {
    /// Numeric identifier of the audio source device.
    pub source_id: u32,
    /// Whether the source should be enabled.
    pub enabled: bool,
}
#[utoipa::path(put, path = "/api/devices/audio/enabled", request_body = AudioSourceEnabledBody, responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn set_audio_source_enabled(
    State(state): State<SharedState>,
    Json(b): Json<AudioSourceEnabledBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ToggleAudioSource {
            source_id: b.source_id,
            enabled: b.enabled,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct MidiDeviceEnabledBody {
    /// Numeric identifier of the MIDI device.
    pub device_id: u32,
    /// Whether the device should be enabled.
    pub enabled: bool,
}
#[utoipa::path(put, path = "/api/devices/midi/enabled", request_body = MidiDeviceEnabledBody, responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn set_midi_device_enabled(
    State(state): State<SharedState>,
    Json(b): Json<MidiDeviceEnabledBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetMidiDeviceEnabled {
            device_id: b.device_id,
            enabled: b.enabled,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(delete, path = "/api/midi/mappings", responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn clear_midi_mappings(State(state): State<SharedState>) -> impl IntoResponse {
    match state.send_command(EngineCommand::ClearMidiMappings).await {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveMidiMappingBody {
    /// MIDI key identifying the mapping to remove.
    pub key: crate::midi::MidiKey,
}
#[utoipa::path(post, path = "/api/midi/mappings/remove", request_body = RemoveMidiMappingBody, responses((status = 200, body = CommandResult)), tag = "Devices")]
pub async fn remove_midi_mapping(
    State(state): State<SharedState>,
    Json(b): Json<RemoveMidiMappingBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveMidiMapping { key: b.key })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Stream Library ─────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct StreamLibraryBody {
    /// Stream URL.
    pub url: String,
    /// Stream connection mode (caller or listener for SRT).
    pub mode: crate::stream::SrtMode,
}
#[utoipa::path(post, path = "/api/streams/library", request_body = StreamLibraryBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn add_stream_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<StreamLibraryBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddStreamLibraryEntry {
            url: b.url,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveStreamBody {
    /// SRT stream URL to remove.
    pub url: String,
}
#[utoipa::path(delete, path = "/api/streams/library", request_body = RemoveStreamBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn remove_stream_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<RemoveStreamBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveStreamLibraryEntry { url: b.url })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── HLS Library ────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct HlsLibraryBody {
    /// HLS stream URL (.m3u8).
    pub url: String,
}
#[utoipa::path(post, path = "/api/streams/hls/library", request_body = HlsLibraryBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn add_hls_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<HlsLibraryBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddHlsLibraryEntry { url: b.url })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveHlsBody {
    /// HLS stream URL to remove.
    pub url: String,
}
#[utoipa::path(delete, path = "/api/streams/hls/library", request_body = RemoveHlsBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn remove_hls_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<RemoveHlsBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveHlsLibraryEntry { url: b.url })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── DASH Library ───────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct DashLibraryBody {
    /// DASH stream URL (.mpd).
    pub url: String,
}
#[utoipa::path(post, path = "/api/streams/dash/library", request_body = DashLibraryBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn add_dash_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<DashLibraryBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddDashLibraryEntry { url: b.url })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveDashBody {
    /// DASH stream URL to remove.
    pub url: String,
}
#[utoipa::path(delete, path = "/api/streams/dash/library", request_body = RemoveDashBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn remove_dash_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<RemoveDashBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveDashLibraryEntry { url: b.url })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RtmpLibraryBody {
    /// RTMP stream URL.
    pub url: String,
    /// Connection mode (Pull or Listen).
    pub mode: crate::stream::RtmpMode,
}
#[utoipa::path(post, path = "/api/streams/rtmp/library", request_body = RtmpLibraryBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn add_rtmp_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<RtmpLibraryBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddRtmpLibraryEntry {
            url: b.url,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveRtmpBody {
    /// RTMP stream URL to remove.
    pub url: String,
}
#[utoipa::path(delete, path = "/api/streams/rtmp/library", request_body = RemoveRtmpBody, responses((status = 200, body = CommandResult)), tag = "Streams")]
pub async fn remove_rtmp_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<RemoveRtmpBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveRtmpLibraryEntry { url: b.url })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_router() -> axum::Router {
        let shared = SharedState {
            command_tx: tokio::sync::mpsc::unbounded_channel().0,
            engine_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
        };
        crate::usecases::api::runner::build_router(shared)
    }

    #[tokio::test]
    async fn test_health_returns_ok() {
        let app = test_router();
        let resp = app
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_state_returns_503_when_not_initialized() {
        let app = test_router();
        let resp = app
            .oneshot(Request::get("/api/state").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
