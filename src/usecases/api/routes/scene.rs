//! Scene state routes: GET /api/scene/*

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::usecases::api::projection::{self, StateReadError};
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

/// Full scene: channels, crossfader, master effects, modulation, macros, sequences, and streams.
#[utoipa::path(get, path = "/api/scene",
    responses((status = 200, description = "Full scene structure"), (status = 503, description = "Engine not yet initialized")),
    tag = "Scene")]
pub async fn scene(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::project_scene(&s)).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Every channel with its UUID, opacity, blend mode, decks, and effects.
#[utoipa::path(get, path = "/api/scene/channels",
    responses((status = 200, description = "Every channel with its decks and effects"), (status = 503, description = "Engine not yet initialized")),
    tag = "Scene")]
pub async fn channels(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.mixer.channels).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// A single channel, addressed by UUID.
#[utoipa::path(get, path = "/api/scene/channels/{channel_uuid}",
    params(("channel_uuid" = String, Path, description = "Channel UUID")),
    responses(
        (status = 200, description = "The channel with its decks and effects"),
        (status = 404, description = "Channel not found"),
        (status = 503, description = "Engine not yet initialized")
    ),
    tag = "Scene")]
pub async fn channel_by_uuid(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_channel(&s, &channel_uuid) {
            Some(ch) => Json(ch).into_response(),
            None => (StatusCode::NOT_FOUND, "Channel not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Every deck in one channel, addressed by channel UUID.
#[utoipa::path(get, path = "/api/scene/channels/{channel_uuid}/decks",
    params(("channel_uuid" = String, Path, description = "Channel UUID")),
    responses(
        (status = 200, description = "The channel's decks"),
        (status = 404, description = "Channel not found"),
        (status = 503, description = "Engine not yet initialized")
    ),
    tag = "Scene")]
pub async fn channel_decks(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_channel(&s, &channel_uuid) {
            Some(ch) => Json(&ch.decks).into_response(),
            None => (StatusCode::NOT_FOUND, "Channel not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// A single deck, addressed by its channel's UUID and its own UUID.
#[utoipa::path(get, path = "/api/scene/channels/{channel_uuid}/decks/{deck_uuid}",
    params(
        ("channel_uuid" = String, Path, description = "UUID of the channel holding the deck"),
        ("deck_uuid" = String, Path, description = "Deck UUID")
    ),
    responses(
        (status = 200, description = "The deck with its generator params and effects"),
        (status = 404, description = "Channel or deck not found"),
        (status = 503, description = "Engine not yet initialized")
    ),
    tag = "Scene")]
pub async fn deck_by_uuid(
    State(state): State<SharedState>,
    Path((channel_uuid, deck_uuid)): Path<(String, String)>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_channel(&s, &channel_uuid) {
            Some(ch) => match projection::find_deck(ch, &deck_uuid) {
                Some(deck) => Json(deck).into_response(),
                None => (StatusCode::NOT_FOUND, "Deck not found").into_response(),
            },
            None => (StatusCode::NOT_FOUND, "Channel not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Modulation sources, their current output values, and parameter assignments.
#[utoipa::path(get, path = "/api/scene/modulation",
    responses((status = 200, description = "Modulation sources, values, and assignments"), (status = 503, description = "Engine not yet initialized")),
    tag = "Scene")]
pub async fn modulation(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.modulation).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Every macro control with its kind, current value, and parameter targets.
#[utoipa::path(get, path = "/api/scene/macros",
    responses((status = 200, description = "Every macro control with its targets"), (status = 503, description = "Engine not yet initialized")),
    tag = "Scene")]
pub async fn macros(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.macros).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Every transition sequence with its steps and playback state.
#[utoipa::path(get, path = "/api/scene/sequences",
    responses((status = 200, description = "Every transition sequence with its steps"), (status = 503, description = "Engine not yet initialized")),
    tag = "Scene")]
pub async fn sequences(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.mixer.sequences).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Active stream receivers with their URL, mode, and connection status.
#[utoipa::path(get, path = "/api/scene/streams",
    responses((status = 200, description = "Active stream receivers"), (status = 503, description = "Engine not yet initialized")),
    tag = "Scene")]
pub async fn streams(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.stream_receivers).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}
