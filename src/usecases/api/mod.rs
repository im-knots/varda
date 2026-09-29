//! HTTP API: an axum REST and WebSocket server for remote control.
//!
//! - Read: `EngineState` → projection → response DTOs
//! - Write: request → validated `EngineCommand` → mpsc channel → engine
//! - `ApiRunner` holds WS connection tracking and the diff cache

pub mod projection;
pub mod routes;
pub mod runner;
pub mod ws;

use crate::app::publish::StatePublication;
use crate::engine::{CommandEnvelope, CommandResult, EngineCommand, ErrorCode};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Shared state for route handlers. Handlers reach the engine only through
/// the command channel and read-only snapshots.
#[derive(Clone)]
pub struct SharedState {
    /// Commands to the engine, processed once per frame.
    pub command_tx: mpsc::UnboundedSender<CommandEnvelope>,
    /// The engine's published snapshot, read without a lock or a copy.
    pub engine_state: Arc<StatePublication>,
}

impl SharedState {
    /// Send a command and wait for the engine's reply.
    ///
    /// # Errors
    ///
    /// Returns `"Engine channel closed"` if the engine's receiver is gone, or
    /// `"Engine dropped reply channel"` if it never answered.
    pub async fn send_command(&self, cmd: EngineCommand) -> Result<CommandResult, &'static str> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send((cmd, Some(tx)))
            .map_err(|_| "Engine channel closed")?;
        rx.await.map_err(|_| "Engine dropped reply channel")
    }
}

/// Map a `CommandResult` to an axum HTTP response.
pub fn command_response(result: CommandResult) -> axum::response::Response {
    use axum::Json;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    match result {
        CommandResult::Ok => {
            (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
        }
        CommandResult::OkWithId { uuid } => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "uuid": uuid})),
        )
            .into_response(),
        CommandResult::OkWithData { data } => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "data": data})),
        )
            .into_response(),
        CommandResult::Err { code, message } => {
            let (status, code_str) = match code {
                ErrorCode::NotFound => (StatusCode::NOT_FOUND, "not_found"),
                ErrorCode::InvalidInput => (StatusCode::BAD_REQUEST, "invalid_input"),
                ErrorCode::InternalError => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
                ErrorCode::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            };
            (
                status,
                Json(serde_json::json!({"error": code_str, "message": message})),
            )
                .into_response()
        }
    }
}
