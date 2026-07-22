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

/// GET /api/scene — full scene structure
pub async fn scene(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::project_scene(&s)).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/channels — all channels
pub async fn channels(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.mixer.channels).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/channels/:uuid — single channel
pub async fn channel_by_uuid(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_channel(&s, &uuid) {
            Some(ch) => Json(ch).into_response(),
            None => (StatusCode::NOT_FOUND, "Channel not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/channels/:uuid/decks — all decks in a channel
pub async fn channel_decks(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_channel(&s, &uuid) {
            Some(ch) => Json(&ch.decks).into_response(),
            None => (StatusCode::NOT_FOUND, "Channel not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/channels/:uuid/decks/:deck_uuid — single deck
pub async fn deck_by_uuid(
    State(state): State<SharedState>,
    Path((ch_uuid, deck_uuid)): Path<(String, String)>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_channel(&s, &ch_uuid) {
            Some(ch) => match projection::find_deck(ch, &deck_uuid) {
                Some(deck) => Json(deck).into_response(),
                None => (StatusCode::NOT_FOUND, "Deck not found").into_response(),
            },
            None => (StatusCode::NOT_FOUND, "Channel not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/modulation
pub async fn modulation(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.modulation).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/macros
pub async fn macros(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.macros).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/sequences
pub async fn sequences(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.mixer.sequences).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/scene/streams
pub async fn streams(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.stream_receivers).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}
