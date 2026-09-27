//! Deck source routes: every source type, through one set of routes.
//!
//! A deck's source is described by a `SourceConfig` (`{"type": "<id>", ...}`)
//! and controlled by the names its type declares. `GET /api/library/sources` lists the
//! types, their controls, and what their libraries offer. No route here names
//! a source type. See /spec/deck-source-providers.md Decision 6.

use super::read_or_error;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::value::provider::{ControlValue, ProviderTypeSnapshot};

use crate::engine::value::source::SourceConfig;
use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

async fn send(state: &SharedState, cmd: EngineCommand) -> axum::response::Response {
    match state.send_command(cmd).await {
        Ok(result) => command_response(result),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Every registered deck source type: its controls, whether this build can
/// run it, and what its library offers (discovered devices, saved URLs).
#[utoipa::path(get, path = "/api/library/sources",
    responses((status = 200, body = Vec<ProviderTypeSnapshot>), (status = 503, description = "Engine not yet initialized")),
    tag = "Sources")]
pub async fn list(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(s.sources.clone()).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// Add a deck of any source type. The body is the source's config, for
/// example `{"type": "Image", "path": "/art/logo.svg"}`; a library entry's
/// `config` is exactly this body. Answers with the new deck's UUID. A source
/// that builds in the background (a shader, a clip) appears on a later frame;
/// `GET /api/state/deck-loads` reports it until then.
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks",
    params(("channel_uuid" = String, Path, description = "Channel UUID")),
    request_body = SourceConfig,
    responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")),
    tag = "Decks")]
pub async fn add_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(source): Json<SourceConfig>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::AddDeck {
            channel_uuid,
            source,
        },
    )
    .await
}

/// Swap a deck's source for another, keeping the deck's identity, effects,
/// opacity and modulation: repoint a tap, switch to another camera.
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/source",
    params(("deck_uuid" = String, Path, description = "Deck UUID")),
    request_body = SourceConfig,
    responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")),
    tag = "Decks")]
pub async fn replace(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(source): Json<SourceConfig>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::ReplaceDeckSource { deck_uuid, source },
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct SourceValueBody {
    /// Numeric controls take a normalized 0.0–1.0 value (a choice is bucketed,
    /// a toggle is on above 0.5); colors take `[r, g, b, a]`; text takes a
    /// string; a number control takes its own units.
    pub value: ControlValue,
}

/// Write one of a deck's source controls, by the name its type declares in
/// `GET /api/library/sources`.
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/source/params/{name}",
    params(("deck_uuid" = String, Path, description = "Deck UUID"), ("name" = String, Path, description = "Control name")),
    request_body = SourceValueBody,
    responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")),
    tag = "Decks")]
pub async fn set_param(
    State(state): State<SharedState>,
    Path((deck_uuid, name)): Path<(String, String)>,
    Json(body): Json<SourceValueBody>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::SetSourceParam {
            deck_uuid,
            name,
            value: body.value,
        },
    )
    .await
}

/// Fire one of a deck's source actions (reload a page, clear in/out points).
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/source/actions/{name}",
    params(("deck_uuid" = String, Path, description = "Deck UUID"), ("name" = String, Path, description = "Action name")),
    responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")),
    tag = "Decks")]
pub async fn trigger_action(
    State(state): State<SharedState>,
    Path((deck_uuid, name)): Path<(String, String)>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::TriggerSourceAction {
            deck_uuid,
            action: name,
        },
    )
    .await
}

/// Save an entry to a source type's library (a stream URL). The body holds the
/// entry's fields; its `type` is taken from the path.
#[utoipa::path(post, path = "/api/sources/{source_type}/library",
    params(("source_type" = String, Path, description = "Source type id")),
    request_body = Object,
    responses((status = 200, body = CommandResult)),
    tag = "Sources")]
pub async fn add_library_entry(
    State(state): State<SharedState>,
    Path(source_type): Path<String>,
    Json(fields): Json<serde_json::Map<String, serde_json::Value>>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::AddSourceLibraryEntry {
            entry: entry(source_type, fields),
        },
    )
    .await
}

/// Remove an entry from a source type's library.
#[utoipa::path(delete, path = "/api/sources/{source_type}/library",
    params(("source_type" = String, Path, description = "Source type id")),
    request_body = Object,
    responses((status = 200, body = CommandResult)),
    tag = "Sources")]
pub async fn remove_library_entry(
    State(state): State<SharedState>,
    Path(source_type): Path<String>,
    Json(fields): Json<serde_json::Map<String, serde_json::Value>>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::RemoveSourceLibraryEntry {
            entry: entry(source_type, fields),
        },
    )
    .await
}

/// Run a library action a source type offers: `rescan`, or one a library
/// notice names. Answers with the type's fresh entries, so a probe that
/// rescans is one call rather than a rescan and a read that may race it.
#[utoipa::path(post, path = "/api/sources/{source_type}/actions/{action}",
    params(("source_type" = String, Path, description = "Source type id"), ("action" = String, Path, description = "Library action")),
    responses((status = 200, body = CommandResult), (status = 404, description = "Unknown source type")),
    tag = "Sources")]
pub async fn library_action(
    State(state): State<SharedState>,
    Path((source_type, action)): Path<(String, String)>,
) -> impl IntoResponse {
    send(
        &state,
        EngineCommand::SourceLibraryAction {
            source_type,
            action,
        },
    )
    .await
}

/// A library entry of `source_type` with `fields`.
fn entry(source_type: String, fields: serde_json::Map<String, serde_json::Value>) -> SourceConfig {
    let mut config = SourceConfig::new(source_type);
    for (key, value) in fields {
        if key != "type" {
            config.set(&key, value);
        }
    }
    config
}
