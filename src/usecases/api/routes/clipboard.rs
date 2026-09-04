//! Copy, paste, and duplicate of scene objects.
//!
//! The clipboard is engine state rather than a UI convenience, so a show
//! control system can build a rig the same way a performer does: copy a channel
//! that is already dialled in, paste it, then address the copy by the UUID that
//! comes back. See `/spec/clipboard.md`.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{ClipboardSource, CommandResult, EngineCommand, PasteTarget};
use crate::usecases::api::{SharedState, command_response};

#[derive(Deserialize, ToSchema)]
pub struct CopyBody {
    /// The deck, channel, or effect to capture, by UUID.
    pub source: ClipboardSource,
    /// Carry a deck's arrangement regions along with it.
    #[serde(default)]
    pub include_arrangement: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct PasteBody {
    /// Where the copy lands.
    pub target: PasteTarget,
}

#[derive(Deserialize, ToSchema)]
pub struct DuplicateBody {
    /// The object to copy beside itself. The clipboard is left alone.
    pub source: ClipboardSource,
}

#[utoipa::path(post, path = "/api/clipboard/copy", request_body = CopyBody, responses((status = 200, body = CommandResult)), tag = "Clipboard")]
pub async fn copy(State(s): State<SharedState>, Json(b): Json<CopyBody>) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::Copy {
            source: b.source,
            include_arrangement: b.include_arrangement,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/clipboard/paste", request_body = PasteBody, responses((status = 200, body = CommandResult)), tag = "Clipboard")]
pub async fn paste(State(s): State<SharedState>, Json(b): Json<PasteBody>) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::Paste { target: b.target })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/clipboard/duplicate", request_body = DuplicateBody, responses((status = 200, body = CommandResult)), tag = "Clipboard")]
pub async fn duplicate(
    State(s): State<SharedState>,
    Json(b): Json<DuplicateBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::Duplicate { source: b.source })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
