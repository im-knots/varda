//! Deprecated per-source-type routes: aliases onto the generic source routes
//! in [`super::sources`], marked deprecated in the `OpenAPI` document. Delete
//! this file as a unit when the aliases are dropped.

#![allow(deprecated)] // the aliases are deprecated; registering them is not

use super::read_or_error;
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::value::provider::ControlValue;

use crate::engine::value::source::SourceConfig;
use crate::engine::{CommandResult, EngineCommand, ErrorCode};
use crate::usecases::api::{SharedState, command_response};

const REPLACED: &str = "Use the generic source routes; see GET /api/library/sources";

async fn send(state: &SharedState, cmd: EngineCommand) -> axum::response::Response {
    match state.send_command(cmd).await {
        Ok(result) => command_response(result),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

async fn add(
    state: &SharedState,
    channel_uuid: String,
    source: SourceConfig,
) -> axum::response::Response {
    send(
        state,
        EngineCommand::AddDeck {
            channel_uuid,
            source,
        },
    )
    .await
}

async fn set(
    state: &SharedState,
    deck_uuid: String,
    name: &str,
    value: ControlValue,
) -> axum::response::Response {
    send(
        state,
        EngineCommand::SetSourceParam {
            deck_uuid,
            name: name.to_string(),
            value,
        },
    )
    .await
}

fn invalid(message: &str) -> axum::response::Response {
    command_response(CommandResult::Err {
        code: ErrorCode::InvalidInput,
        message: message.to_string(),
    })
}

/// A deck's published source info field, for aliases that took seconds or a
/// multiplier where the generic control takes a normalized value.
fn deck_info(state: &SharedState, deck_uuid: &str, key: &str) -> Option<serde_json::Value> {
    let published = read_or_error(state).ok()?;
    published
        .mixer
        .channels
        .iter()
        .flat_map(|ch| ch.decks.iter())
        .find(|d| d.uuid == deck_uuid)?
        .source
        .status
        .info
        .get(key)
        .cloned()
}

fn duration(state: &SharedState, deck_uuid: &str) -> f64 {
    deck_info(state, deck_uuid, "duration")
        .and_then(|v| v.as_f64())
        .unwrap_or_default()
}

// ── Deck creation ──────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AddShaderDeckBody {
    /// Name of the shader to load into the new deck.
    pub shader_name: String,
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Shader\", \"name\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/shader", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddShaderDeckBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_shader_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<AddShaderDeckBody>,
) -> impl IntoResponse {
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Shader").with("name", b.shader_name),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct PathBody {
    /// File path to the asset.
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Image\", \"path\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/image", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = PathBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_image_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<PathBody>,
) -> impl IntoResponse {
    let path = super::decks::sanitize_path(&b.path);
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Image").with("path", path),
    )
    .await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Video\", \"path\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/video", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = PathBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_video_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<PathBody>,
) -> impl IntoResponse {
    let path = super::decks::sanitize_path(&b.path);
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Video").with("path", path),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct ColorBody {
    /// RGBA color as four floats in 0.0–1.0.
    pub color: [f32; 4],
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"SolidColor\", \"color\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/solid", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = ColorBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_solid_color_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<ColorBody>,
) -> impl IntoResponse {
    add(
        &state,
        channel_uuid,
        SourceConfig::new("SolidColor").with("color", b.color),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct CameraBody {
    /// Numeric identifier of the camera device.
    pub camera_id: u32,
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Camera\", \"name\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/camera", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = CameraBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_camera_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<CameraBody>,
) -> impl IntoResponse {
    let name = read_or_error(&state).ok().and_then(|s| {
        s.cameras
            .devices
            .iter()
            .find(|(_, id)| *id == b.camera_id)
            .map(|(name, _)| name.clone())
    });
    match name {
        Some(name) => {
            add(
                &state,
                channel_uuid,
                SourceConfig::new("Camera").with("name", name),
            )
            .await
        }
        None => invalid("No camera with that id — rescan and try again"),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DepthBody {
    /// Position of the depth sensor in the last scan's list.
    pub depth_sensor_id: u32,
}

/// The library entry at `index` of a source type. These routes take device
/// ids as positions in the last scan.
fn entry_at(state: &SharedState, source_type: &str, index: usize) -> Option<SourceConfig> {
    let published = read_or_error(state).ok()?;
    published
        .sources
        .iter()
        .find(|t| t.type_id == source_type)?
        .library
        .entries
        .get(index)
        .map(|e| e.config.clone())
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"DepthSensor\", \"name\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/depth", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = DepthBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_depth_sensor_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<DepthBody>,
) -> impl IntoResponse {
    match entry_at(&state, "DepthSensor", b.depth_sensor_id as usize) {
        Some(source) => add(&state, channel_uuid, source).await,
        None => invalid("No depth sensor with that id — rescan and try again"),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ScreenCaptureBody {
    /// `{"kind": "display", "name": ...}` or `{"kind": "window", "app": ..., "title": ...}`.
    #[schema(value_type = Object)]
    pub target: serde_json::Value,
    #[serde(default)]
    pub rate: Option<f32>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub crop: Option<serde_json::Value>,
    #[serde(default)]
    pub show_cursor: Option<bool>,
    #[serde(default)]
    pub exclude_varda: Option<bool>,
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"ScreenCapture\", \"target\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/screen", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = ScreenCaptureBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_screen_capture_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<ScreenCaptureBody>,
) -> impl IntoResponse {
    let mut source = SourceConfig::new("ScreenCapture").with("target", b.target);
    if let Some(rate) = b.rate {
        source.set("rate", rate);
    }
    if let Some(crop) = b.crop {
        source.set("crop", crop);
    }
    if let Some(cursor) = b.show_cursor {
        source.set("show_cursor", cursor);
    }
    if let Some(exclude) = b.exclude_varda {
        source.set("exclude_varda", exclude);
    }
    add(&state, channel_uuid, source).await
}

#[derive(Deserialize, ToSchema)]
pub struct TapBody {
    /// `{"kind": "master_program"}` or `{"kind": "channel", "uuid": ...}`.
    #[schema(value_type = Object)]
    pub source: serde_json::Value,
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Tap\", \"source\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/tap", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = TapBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_tap_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<TapBody>,
) -> impl IntoResponse {
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Tap").with("source", b.source),
    )
    .await
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source with {\"type\": \"Tap\", \"source\": ...}")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/tap/source", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = TapBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn set_tap_source(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<TapBody>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::ReplaceDeckSource {
            deck_uuid,
            source: SourceConfig::new("Tap").with("source", b.source),
        },
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct NamedBody {
    /// Source, server, or sender name.
    #[serde(alias = "source_name", alias = "server_name", alias = "sender_name")]
    pub name: String,
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Ndi\", \"name\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/ndi", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = NamedBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_ndi_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<NamedBody>,
) -> impl IntoResponse {
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Ndi").with("name", b.name),
    )
    .await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Syphon\", \"name\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/syphon", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = NamedBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_syphon_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<NamedBody>,
) -> impl IntoResponse {
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Syphon").with("name", b.name),
    )
    .await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Spout\", \"name\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/spout", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = NamedBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_spout_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<NamedBody>,
) -> impl IntoResponse {
    add(
        &state,
        channel_uuid,
        SourceConfig::new("Spout").with("name", b.name),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct UrlBody {
    /// Stream or page URL.
    pub url: String,
    /// Connection mode, for SRT (`Caller`/`Listener`) and RTMP (`Pull`/`Listen`).
    #[serde(default)]
    pub mode: Option<String>,
}

fn url_source(source_type: &str, b: UrlBody) -> SourceConfig {
    let source = SourceConfig::new(source_type).with("url", b.url);
    match b.mode {
        Some(mode) => source.with("mode", mode.to_ascii_lowercase()),
        None => source,
    }
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Srt\", \"url\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/srt", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_srt_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    add(&state, channel_uuid, url_source("Srt", b)).await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Hls\", \"url\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/hls", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_hls_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    add(&state, channel_uuid, url_source("Hls", b)).await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Dash\", \"url\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/dash", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_dash_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    add(&state, channel_uuid, url_source("Dash", b)).await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Rtmp\", \"url\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/rtmp", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_rtmp_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    add(&state, channel_uuid, url_source("Rtmp", b)).await
}

#[deprecated(
    note = "POST /api/channels/{channel_uuid}/decks with {\"type\": \"Html\", \"url\": ...}"
)]
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/html", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_html_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    add(&state, channel_uuid, url_source("Html", b)).await
}

// ── Deck source controls ───────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ScalingModeBody {
    /// How the deck content is scaled to fit the output.
    pub mode: crate::source::ScalingMode,
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/scaling_mode")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/scaling-mode", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = ScalingModeBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn set_scaling_mode(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<ScalingModeBody>,
) -> impl IntoResponse {
    set(
        &state,
        deck_uuid,
        "scaling_mode",
        ControlValue::Float(b.mode.to_value()),
    )
    .await
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/play")]
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/video/toggle-play", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_toggle_play(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    let playing = deck_info(&state, &deck_uuid, "playing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    set(&state, deck_uuid, "play", ControlValue::Bool(!playing)).await
}

#[derive(Deserialize, ToSchema)]
pub struct SeekBody {
    /// Seek position in seconds from the start of the video.
    pub position_secs: f64,
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/position")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/seek", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = SeekBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_seek(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<SeekBody>,
) -> impl IntoResponse {
    let value = crate::video::provider::secs_to_norm(b.position_secs, duration(&state, &deck_uuid));
    set(&state, deck_uuid, "position", ControlValue::Float(value)).await
}

#[derive(Deserialize, ToSchema)]
pub struct SpeedBody {
    /// Playback speed multiplier (1.0 = normal speed).
    pub speed: f64,
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/speed")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/speed", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = SpeedBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_set_speed(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<SpeedBody>,
) -> impl IntoResponse {
    let value = crate::video::provider::speed_to_norm(b.speed);
    set(&state, deck_uuid, "speed", ControlValue::Float(value)).await
}

#[derive(Deserialize, ToSchema)]
pub struct LoopModeBody {
    /// Loop behavior for the video.
    pub mode: crate::video::LoopMode,
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/loop_mode")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/loop-mode", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = LoopModeBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_set_loop_mode(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<LoopModeBody>,
) -> impl IntoResponse {
    set(
        &state,
        deck_uuid,
        "loop_mode",
        ControlValue::Float(b.mode.to_value()),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct PointBody {
    /// Time position in seconds.
    pub secs: f64,
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/in_point")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/in-point", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = PointBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_set_in_point(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<PointBody>,
) -> impl IntoResponse {
    let value = crate::video::provider::secs_to_norm(b.secs, duration(&state, &deck_uuid));
    set(&state, deck_uuid, "in_point", ControlValue::Float(value)).await
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/out_point")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/out-point", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = PointBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_set_out_point(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<PointBody>,
) -> impl IntoResponse {
    let value = crate::video::provider::secs_to_norm(b.secs, duration(&state, &deck_uuid));
    set(&state, deck_uuid, "out_point", ControlValue::Float(value)).await
}

#[deprecated(note = "POST /api/decks/{deck_uuid}/source/actions/clear")]
#[utoipa::path(delete, path = "/api/decks/{deck_uuid}/video/in-out-points", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_clear_in_out(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::TriggerSourceAction {
            deck_uuid,
            action: "clear".into(),
        },
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct TransportSyncBody {
    pub mode: crate::video::TransportSyncMode,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub delay_frames: i32,
}

#[deprecated(note = "PUT /api/decks/{deck_uuid}/source/params/chase, chase_offset and chase_delay")]
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/transport-sync", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = TransportSyncBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn video_set_transport_sync(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<TransportSyncBody>,
) -> impl IntoResponse {
    use crate::video::TransportSyncMode as M;
    let index = match b.mode {
        M::Auto => 0,
        M::Always => 1,
        M::Never => 2,
    };
    let writes = [
        (
            "chase",
            ControlValue::Float(crate::source::choice_value(index, 3)),
        ),
        ("chase_offset", ControlValue::Float(b.offset as f32)),
        ("chase_delay", ControlValue::Float(b.delay_frames as f32)),
    ];
    // Stop at the first refusal (a deck that is not a clip) and return it.
    let mut answer = invalid(REPLACED);
    for (name, value) in writes {
        answer = set(&state, deck_uuid.clone(), name, value).await;
        if !answer.status().is_success() {
            break;
        }
    }
    answer
}

#[deprecated(note = "POST /api/decks/{deck_uuid}/source/actions/reload")]
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/html/reload", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn reload_html_deck(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::TriggerSourceAction {
            deck_uuid,
            action: "reload".into(),
        },
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct InteractiveBody {
    /// True to open the interactive window, false to close it.
    pub open: bool,
}

#[deprecated(note = "POST /api/decks/{deck_uuid}/source/actions/interactive")]
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/html/interactive", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = InteractiveBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn set_html_interactive(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<InteractiveBody>,
) -> impl IntoResponse {
    let cmd = if b.open {
        EngineCommand::OpenHtmlInteractive { deck_uuid }
    } else {
        EngineCommand::CloseHtmlInteractive
    };
    send(&state, cmd).await
}

// ── Library ────────────────────────────────────────────────────────

async fn library_action(
    state: &SharedState,
    source_type: &str,
    action: &str,
) -> axum::response::Response {
    send(
        state,
        EngineCommand::SourceLibraryAction {
            source_type: source_type.to_string(),
            action: action.to_string(),
        },
    )
    .await
}

#[deprecated(note = "POST /api/sources/Ndi/actions/rescan")]
#[utoipa::path(post, path = "/api/devices/ndi/scan", responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn scan_ndi(State(state): State<SharedState>) -> impl IntoResponse {
    library_action(&state, "Ndi", "rescan").await
}

#[deprecated(note = "POST /api/sources/Syphon/actions/rescan")]
#[utoipa::path(post, path = "/api/devices/syphon/scan", responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn scan_syphon(State(state): State<SharedState>) -> impl IntoResponse {
    library_action(&state, "Syphon", "rescan").await
}

#[deprecated(note = "POST /api/sources/Camera/actions/rescan")]
#[utoipa::path(post, path = "/api/devices/cameras/scan", responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn scan_cameras(State(state): State<SharedState>) -> impl IntoResponse {
    library_action(&state, "Camera", "rescan").await
}

#[deprecated(note = "POST /api/sources/DepthSensor/actions/rescan")]
#[utoipa::path(post, path = "/api/devices/depth/scan", responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn scan_depth_sensors(State(state): State<SharedState>) -> impl IntoResponse {
    library_action(&state, "DepthSensor", "rescan").await
}

#[deprecated(note = "POST /api/sources/ScreenCapture/actions/rescan")]
#[utoipa::path(post, path = "/api/devices/screen/scan", responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn scan_capture_targets(State(state): State<SharedState>) -> impl IntoResponse {
    library_action(&state, "ScreenCapture", "rescan").await
}

#[deprecated(note = "POST /api/sources/ScreenCapture/actions/grant_permission")]
#[utoipa::path(post, path = "/api/devices/screen/permission", responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn request_screen_capture_permission(
    State(state): State<SharedState>,
) -> impl IntoResponse {
    library_action(&state, "ScreenCapture", "grant_permission").await
}

async fn library_entry(
    state: &SharedState,
    source_type: &str,
    b: UrlBody,
    add: bool,
) -> axum::response::Response {
    let entry = url_source(source_type, b);
    send(
        state,
        if add {
            EngineCommand::AddSourceLibraryEntry { entry }
        } else {
            EngineCommand::RemoveSourceLibraryEntry { entry }
        },
    )
    .await
}

#[deprecated(note = "POST /api/sources/Srt/library")]
#[utoipa::path(post, path = "/api/streams/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_srt_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Srt", b, true).await
}

#[deprecated(note = "DELETE /api/sources/Srt/library")]
#[utoipa::path(delete, path = "/api/streams/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn remove_srt_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Srt", b, false).await
}

#[deprecated(note = "POST /api/sources/Hls/library")]
#[utoipa::path(post, path = "/api/streams/hls/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_hls_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Hls", b, true).await
}

#[deprecated(note = "DELETE /api/sources/Hls/library")]
#[utoipa::path(delete, path = "/api/streams/hls/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn remove_hls_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Hls", b, false).await
}

#[deprecated(note = "POST /api/sources/Dash/library")]
#[utoipa::path(post, path = "/api/streams/dash/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_dash_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Dash", b, true).await
}

#[deprecated(note = "DELETE /api/sources/Dash/library")]
#[utoipa::path(delete, path = "/api/streams/dash/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn remove_dash_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Dash", b, false).await
}

#[deprecated(note = "POST /api/sources/Rtmp/library")]
#[utoipa::path(post, path = "/api/streams/rtmp/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn add_rtmp_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Rtmp", b, true).await
}

#[deprecated(note = "DELETE /api/sources/Rtmp/library")]
#[utoipa::path(delete, path = "/api/streams/rtmp/library", request_body = UrlBody, responses((status = 200, body = CommandResult)), tag = "Deprecated")]
pub async fn remove_rtmp_library_entry(
    State(state): State<SharedState>,
    Json(b): Json<UrlBody>,
) -> impl IntoResponse {
    library_entry(&state, "Rtmp", b, false).await
}

/// A source type's library entries as `{name}` rows.
fn entry_names(state: &SharedState, source_type: &str) -> axum::response::Response {
    match read_or_error(state) {
        Ok(s) => {
            let names: Vec<serde_json::Value> = s
                .sources
                .iter()
                .find(|t| t.type_id == source_type)
                .map(|t| {
                    t.library
                        .entries
                        .iter()
                        .enumerate()
                        .map(|(id, e)| serde_json::json!({ "name": e.label, "id": id, "config": e.config }))
                        .collect()
                })
                .unwrap_or_default();
            Json(names).into_response()
        }
        Err((status, msg)) => (status, msg).into_response(),
    }
}

#[deprecated(note = "GET /api/library/sources")]
#[utoipa::path(get, path = "/api/library/depth", responses((status = 200, body = Object)), tag = "Deprecated")]
pub async fn library_depth(State(state): State<SharedState>) -> impl IntoResponse {
    entry_names(&state, "DepthSensor")
}

#[deprecated(note = "GET /api/library/sources")]
#[utoipa::path(get, path = "/api/library/screen", responses((status = 200, body = Object)), tag = "Deprecated")]
pub async fn library_screen(State(state): State<SharedState>) -> impl IntoResponse {
    entry_names(&state, "ScreenCapture")
}

#[deprecated(note = "GET /api/library/sources")]
#[utoipa::path(get, path = "/api/library/ndi", responses((status = 200, body = Object)), tag = "Deprecated")]
pub async fn library_ndi(State(state): State<SharedState>) -> impl IntoResponse {
    entry_names(&state, "Ndi")
}

#[deprecated(note = "GET /api/library/sources")]
#[utoipa::path(get, path = "/api/library/syphon", responses((status = 200, body = Object)), tag = "Deprecated")]
pub async fn library_syphon(State(state): State<SharedState>) -> impl IntoResponse {
    entry_names(&state, "Syphon")
}

/// The aliases' `OpenAPI` operations, merged into the served document so
/// each shows as deprecated beside its replacement.
#[derive(utoipa::OpenApi)]
#[openapi(paths(
    add_shader_deck,
    add_image_deck,
    add_video_deck,
    add_solid_color_deck,
    add_camera_deck,
    add_depth_sensor_deck,
    add_screen_capture_deck,
    add_tap_deck,
    set_tap_source,
    add_ndi_deck,
    add_syphon_deck,
    add_spout_deck,
    add_srt_deck,
    add_hls_deck,
    add_dash_deck,
    add_rtmp_deck,
    add_html_deck,
    set_scaling_mode,
    video_toggle_play,
    video_seek,
    video_set_speed,
    video_set_loop_mode,
    video_set_in_point,
    video_set_out_point,
    video_clear_in_out,
    video_set_transport_sync,
    reload_html_deck,
    set_html_interactive,
    scan_ndi,
    scan_syphon,
    scan_cameras,
    scan_depth_sensors,
    scan_capture_targets,
    request_screen_capture_permission,
    add_srt_library_entry,
    remove_srt_library_entry,
    add_hls_library_entry,
    remove_hls_library_entry,
    add_dash_library_entry,
    remove_dash_library_entry,
    add_rtmp_library_entry,
    remove_rtmp_library_entry,
    library_depth,
    library_screen,
    library_ndi,
    library_syphon,
))]
pub struct DeprecatedApi;

/// Every alias, for merging into the API router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route(
            "/api/channels/{channel_uuid}/decks/shader",
            post(add_shader_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/image",
            post(add_image_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/video",
            post(add_video_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/solid",
            post(add_solid_color_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/camera",
            post(add_camera_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/depth",
            post(add_depth_sensor_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/screen",
            post(add_screen_capture_deck),
        )
        .route("/api/channels/{channel_uuid}/decks/tap", post(add_tap_deck))
        .route("/api/channels/{channel_uuid}/decks/ndi", post(add_ndi_deck))
        .route(
            "/api/channels/{channel_uuid}/decks/syphon",
            post(add_syphon_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/spout",
            post(add_spout_deck),
        )
        .route("/api/channels/{channel_uuid}/decks/srt", post(add_srt_deck))
        .route("/api/channels/{channel_uuid}/decks/hls", post(add_hls_deck))
        .route(
            "/api/channels/{channel_uuid}/decks/dash",
            post(add_dash_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/rtmp",
            post(add_rtmp_deck),
        )
        .route(
            "/api/channels/{channel_uuid}/decks/html",
            post(add_html_deck),
        )
        .route("/api/decks/{deck_uuid}/tap/source", put(set_tap_source))
        .route("/api/decks/{deck_uuid}/scaling-mode", put(set_scaling_mode))
        .route(
            "/api/decks/{deck_uuid}/video/toggle-play",
            post(video_toggle_play),
        )
        .route("/api/decks/{deck_uuid}/video/seek", put(video_seek))
        .route("/api/decks/{deck_uuid}/video/speed", put(video_set_speed))
        .route(
            "/api/decks/{deck_uuid}/video/loop-mode",
            put(video_set_loop_mode),
        )
        .route(
            "/api/decks/{deck_uuid}/video/in-point",
            put(video_set_in_point),
        )
        .route(
            "/api/decks/{deck_uuid}/video/out-point",
            put(video_set_out_point),
        )
        .route(
            "/api/decks/{deck_uuid}/video/in-out-points",
            delete(video_clear_in_out),
        )
        .route(
            "/api/decks/{deck_uuid}/video/transport-sync",
            put(video_set_transport_sync),
        )
        .route("/api/decks/{deck_uuid}/html/reload", post(reload_html_deck))
        .route(
            "/api/decks/{deck_uuid}/html/interactive",
            post(set_html_interactive),
        )
        .route("/api/devices/ndi/scan", post(scan_ndi))
        .route("/api/devices/syphon/scan", post(scan_syphon))
        .route("/api/devices/cameras/scan", post(scan_cameras))
        .route("/api/devices/depth/scan", post(scan_depth_sensors))
        .route("/api/devices/screen/scan", post(scan_capture_targets))
        .route(
            "/api/devices/screen/permission",
            post(request_screen_capture_permission),
        )
        .route(
            "/api/streams/library",
            post(add_srt_library_entry).delete(remove_srt_library_entry),
        )
        .route(
            "/api/streams/hls/library",
            post(add_hls_library_entry).delete(remove_hls_library_entry),
        )
        .route(
            "/api/streams/dash/library",
            post(add_dash_library_entry).delete(remove_dash_library_entry),
        )
        .route(
            "/api/streams/rtmp/library",
            post(add_rtmp_library_entry).delete(remove_rtmp_library_entry),
        )
        .route("/api/library/depth", get(library_depth))
        .route("/api/library/screen", get(library_screen))
        .route("/api/library/ndi", get(library_ndi))
        .route("/api/library/syphon", get(library_syphon))
}
