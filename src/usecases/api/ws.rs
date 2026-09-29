//! WebSocket handler: full state on connect, then JSON Patch (RFC 6902) deltas
//! against the last state sent. Client messages are `EngineCommand` JSON with
//! an optional `id` for matching the response.

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::SharedState;

/// Delta rate, about 30 fps.
const DELTA_INTERVAL_MS: u64 = 33;

/// Envelope for client → server commands with optional correlation id.
#[derive(Deserialize)]
struct WsCommand {
    id: Option<String>,
    #[serde(flatten)]
    command: EngineCommand,
}

/// Envelope for server → client command responses.
#[derive(Serialize)]
struct WsCommandResponse {
    id: Option<String>,
    result: WsResultPayload,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WsResultPayload {
    Ok {
        status: &'static str,
    },
    OkWithId {
        status: &'static str,
        uuid: String,
    },
    OkWithData {
        status: &'static str,
        data: serde_json::Value,
    },
    Err {
        error: &'static str,
        message: String,
    },
}

impl From<CommandResult> for WsResultPayload {
    fn from(r: CommandResult) -> Self {
        match r {
            CommandResult::Ok => WsResultPayload::Ok { status: "ok" },
            CommandResult::OkWithId { uuid } => WsResultPayload::OkWithId { status: "ok", uuid },
            CommandResult::OkWithData { data } => {
                WsResultPayload::OkWithData { status: "ok", data }
            }
            CommandResult::Err { code, message } => {
                let error = match code {
                    crate::engine::ErrorCode::NotFound => "not_found",
                    crate::engine::ErrorCode::InvalidInput => "invalid_input",
                    crate::engine::ErrorCode::InternalError => "internal",
                    crate::engine::ErrorCode::Unavailable => "unavailable",
                };
                WsResultPayload::Err { error, message }
            }
        }
    }
}

/// `GET /api/ws` — upgrade to WebSocket.
pub async fn ws_upgrade(
    State(state): State<SharedState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: SharedState) {
    let (mut sink, mut stream) = socket.split();

    // Full snapshot on connect; later deltas diff against the last one sent.
    let Some(mut last) = state.engine_state.latest() else {
        let _ = sink.send(Message::Close(None)).await;
        return;
    };
    if sink
        .send(Message::Text(last.json().to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    // Channel for forwarding incoming commands to the delta loop task.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<String>();

    // ── Read task: client → server commands ──────────────────────
    let read_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(text) => {
                    let _ = cmd_tx.send(text.to_string());
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // ── Write task: deltas + command responses ───────────────────
    let write_handle = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(DELTA_INTERVAL_MS));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Nothing to do until the engine publishes a new snapshot.
                    if let Some(current) = state.engine_state.latest()
                        && current.generation != last.generation
                    {
                        let patch = json_patch::diff(last.json(), current.json());
                        if !patch.0.is_empty() {
                            let patch_str = serde_json::to_string(&patch).unwrap_or_default();
                            if sink.send(Message::Text(patch_str.into())).await.is_err() {
                                break;
                            }
                        }
                        last = current;
                    }
                }
                received = cmd_rx.recv() => {
                    // The reader is gone, so the client is too.
                    let Some(text) = received else { break };
                    let response = match serde_json::from_str::<WsCommand>(&text) {
                        Ok(ws_cmd) => {
                            let result = state.send_command(ws_cmd.command).await;
                            let payload = match result {
                                Ok(r) => r.into(),
                                Err(msg) => WsResultPayload::Err {
                                    error: "internal",
                                    message: msg.to_string(),
                                },
                            };
                            WsCommandResponse { id: ws_cmd.id, result: payload }
                        }
                        Err(e) => WsCommandResponse {
                            id: None,
                            result: WsResultPayload::Err {
                                error: "invalid_input",
                                message: format!("Invalid command JSON: {e}"),
                            },
                        },
                    };
                    let resp_str = serde_json::to_string(&response).unwrap_or_default();
                    if sink.send(Message::Text(resp_str.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    until_either_ends(read_handle, write_handle).await;
}

/// When either half of a connection ends, cancel the other and wait for it.
/// Dropping a `JoinHandle` would only detach its task.
async fn until_either_ends(
    mut reader: tokio::task::JoinHandle<()>,
    mut writer: tokio::task::JoinHandle<()>,
) {
    let reader_ended = tokio::select! {
        _ = &mut reader => true,
        _ = &mut writer => false,
    };
    let survivor = if reader_ended { writer } else { reader };
    survivor.abort();
    let _ = survivor.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ErrorCode;

    /// Sets its flag when dropped, which is when an aborted task stops.
    struct Stopped(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for Stopped {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// When one half of a connection ends, the other is cancelled and has
    /// stopped by the time the connection handler returns.
    #[tokio::test]
    async fn the_surviving_half_is_stopped_when_the_other_ends() {
        for reader_ends_first in [true, false] {
            let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let guard = Stopped(std::sync::Arc::clone(&stopped));
            let forever = tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            let done = tokio::spawn(async {});
            if reader_ends_first {
                until_either_ends(done, forever).await;
            } else {
                until_either_ends(forever, done).await;
            }
            assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[test]
    fn test_ws_result_payload_ok() {
        let payload: WsResultPayload = CommandResult::Ok.into();
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[test]
    fn test_ws_result_payload_ok_with_id() {
        let payload: WsResultPayload = CommandResult::OkWithId {
            uuid: "abc-123".into(),
        }
        .into();
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["uuid"], "abc-123");
    }

    #[test]
    fn test_ws_result_payload_err() {
        let payload: WsResultPayload = CommandResult::Err {
            code: ErrorCode::NotFound,
            message: "Channel not found".into(),
        }
        .into();
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["error"], "not_found");
        assert_eq!(json["message"], "Channel not found");
    }

    #[test]
    fn test_ws_command_response_serialization() {
        let resp = WsCommandResponse {
            id: Some("req-42".into()),
            result: CommandResult::Ok.into(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "req-42");
        assert_eq!(json["result"]["status"], "ok");
    }

    #[test]
    fn test_json_patch_diff_detects_changes() {
        let old = serde_json::json!({"crossfader": 0.0, "channels": []});
        let new = serde_json::json!({"crossfader": 0.75, "channels": []});
        let patch = json_patch::diff(&old, &new);
        assert!(!patch.0.is_empty());
        let patched = {
            let mut v = old.clone();
            json_patch::patch(&mut v, &patch).unwrap();
            v
        };
        assert_eq!(patched, new);
    }

    #[test]
    fn test_json_patch_no_change_produces_empty_patch() {
        let state = serde_json::json!({"crossfader": 0.5, "channels": [{"name": "A"}]});
        let patch = json_patch::diff(&state, &state);
        assert!(patch.0.is_empty());
    }

    #[test]
    fn test_ws_command_deserialize() {
        let json = r#"{"id":"req-1","SetCrossfader":0.5}"#;
        let ws_cmd: WsCommand = serde_json::from_str(json).unwrap();
        assert_eq!(ws_cmd.id.as_deref(), Some("req-1"));
        assert!(
            matches!(ws_cmd.command, EngineCommand::SetCrossfader(v) if (v - 0.5).abs() < 1e-6)
        );
    }

    #[test]
    fn test_ws_command_deserialize_without_id() {
        let json = r#"{"SetCrossfader":0.5}"#;
        let ws_cmd: WsCommand = serde_json::from_str(json).unwrap();
        assert!(ws_cmd.id.is_none());
    }
}
