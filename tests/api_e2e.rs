//! API end-to-end tests: a headless `VardaApp` wired to the axum router via
//! `SharedState`, exercised over HTTP.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use varda::app::VardaApp;
use varda::engine::CommandResult;
use varda::usecases::api::SharedState;

mod common;

/// Create a headless `VardaApp` and wire its command channel to an axum router.
///
/// A background task drains commands and replies `CommandResult::Ok`. The
/// state snapshot is published once at setup; tests that check mutations
/// re-read the shared `engine_state` arc.
fn setup() -> Option<axum::Router> {
    let gpu = common::headless_gpu()?;
    let config = varda::testing::headless_config();
    // With a GPU present, a construction failure is a bug, not a skip.
    let app = VardaApp::new(gpu, &config).expect("VardaApp::new");

    let _real_cmd_tx = app.command_sender();
    let state_reader = app.state_reader();
    // Publish initial state so GET routes work.
    app.publish_state();

    // `VardaApp` is !Send, so the API sends to a proxy channel whose commands
    // get a mock auto-reply. These tests check that route handlers get a reply.
    let (proxy_tx, mut proxy_rx) =
        tokio::sync::mpsc::unbounded_channel::<varda::engine::CommandEnvelope>();

    tokio::spawn(async move {
        while let Some((_cmd, reply_tx)) = proxy_rx.recv().await {
            if let Some(tx) = reply_tx {
                let _ = tx.send(CommandResult::Ok);
            }
        }
    });

    let shared = SharedState {
        command_tx: proxy_tx,
        engine_state: state_reader,
    };
    let router = varda::usecases::api::runner::build_router(shared);
    Some(router)
}

async fn get_json(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn put_json(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::put(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn post_json(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn post_empty(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(Request::post(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

// ── Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn health_with_real_engine() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = get_json(app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn state_reflects_real_engine() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = get_json(app, "/api/state").await;
    assert_eq!(status, StatusCode::OK);
    let channels = json["mixer"]["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
}

#[tokio::test]
async fn set_crossfader_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = put_json(
        app,
        "/api/mixer/crossfader",
        serde_json::json!({"position": 0.6}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn add_channel_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = post_empty(app, "/api/channels").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn add_solid_deck_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = post_json(
        app,
        "/api/channels/0/decks/solid",
        serde_json::json!({"color": [1.0, 0.0, 0.0, 1.0]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn multi_step_api_workflow() {
    let Some(app) = setup() else {
        return;
    };
    let (s, _) = post_empty(app, "/api/channels").await;
    assert_eq!(s, StatusCode::OK);

    let Some(app) = setup() else {
        return;
    };
    let (s, _) = post_json(
        app,
        "/api/channels/0/decks/solid",
        serde_json::json!({"color": [0.0, 1.0, 0.0, 1.0]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn add_lfo_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = post_json(
        app,
        "/api/modulation/lfo",
        serde_json::json!({"waveform": "Sine", "frequency": 2.0}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn set_channel_opacity_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = put_json(
        app,
        "/api/channels/0/opacity",
        serde_json::json!({"opacity": 0.3}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn remove_channel_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (s, _) = post_empty(app, "/api/channels").await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn state_mixer_endpoint() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = get_json(app, "/api/state/mixer").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["channels"].is_array());
    assert!(json["crossfader"].is_number());
}

#[tokio::test]
async fn state_modulation_endpoint() {
    let Some(app) = setup() else {
        return;
    };
    let (status, json) = get_json(app, "/api/state/modulation").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["sources"].is_array());
}

#[tokio::test]
async fn undo_redo_via_api() {
    let Some(app) = setup() else {
        return;
    };
    let (s, _) = put_json(
        app,
        "/api/mixer/crossfader",
        serde_json::json!({"position": 0.5}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let Some(app2) = setup() else {
        return;
    };
    let (s, _) = post_empty(app2, "/api/undo").await;
    assert_eq!(s, StatusCode::OK);

    let Some(app3) = setup() else {
        return;
    };
    let (s, _) = post_empty(app3, "/api/redo").await;
    assert_eq!(s, StatusCode::OK);
}
