//! HTTP API consumer — axum-based REST/WS server for remote control of Varda.
//!
//! Mirrors the UI consumer pattern:
//! - **Read**: `EngineState` → projection → response DTOs
//! - **Write**: HTTP request → validated `EngineCommand` → mpsc channel → engine
//! - **Consumer state**: WS connection tracking, diff cache (owned by ApiRunner)

pub mod projection;
pub mod routes;
pub mod runner;
pub mod ws;

use crate::engine::{CommandEnvelope, CommandResult, ErrorCode, EngineCommand, EngineState};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

/// Shared state passed to all route handlers via axum's `State` extractor.
///
/// Route handlers never hold the engine — they communicate only through
/// channels and read-only state snapshots.
#[derive(Clone)]
pub struct SharedState {
    /// Send commands to the engine. The engine processes them once per frame.
    pub command_tx: mpsc::UnboundedSender<CommandEnvelope>,
    /// Read the latest engine state snapshot (updated each frame by `publish_state`).
    pub engine_state: Arc<RwLock<Option<EngineState>>>,
}

impl SharedState {
    /// Send a command and wait for the engine to process it (with response).
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
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;

    match result {
        CommandResult::Ok => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        CommandResult::OkWithId { uuid } => (StatusCode::OK, Json(serde_json::json!({"status": "ok", "uuid": uuid}))).into_response(),
        CommandResult::OkWithData { data } => (StatusCode::OK, Json(serde_json::json!({"status": "ok", "data": data}))).into_response(),
        CommandResult::Err { code, message } => {
            let (status, code_str) = match code {
                ErrorCode::NotFound => (StatusCode::NOT_FOUND, "not_found"),
                ErrorCode::InvalidInput => (StatusCode::BAD_REQUEST, "invalid_input"),
                ErrorCode::InternalError => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
                ErrorCode::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            };
            (status, Json(serde_json::json!({"error": code_str, "message": message}))).into_response()
        }
    }
}
