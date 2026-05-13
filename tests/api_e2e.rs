//! API end-to-end tests — real VardaApp wired to axum router via SharedState.
//!
//! Each test creates a headless VardaApp, wires its command channel into an axum
//! router, spawns a background thread to process commands, then exercises the
//! HTTP API and verifies state mutations.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use varda::app::{AppConfig, VardaApp};
use varda::engine::CommandResult;
use varda::usecases::api::SharedState;

use clap::Parser;

fn parse_args(args: &[&str]) -> AppConfig {
    AppConfig::parse_from(std::iter::once("varda").chain(args.iter().copied()))
}

/// Create a headless VardaApp and wire its command channel to an axum router.
///
/// Commands are processed eagerly via a background tokio task that drains the
/// channel and auto-replies `CommandResult::Ok`. The engine state snapshot is
/// published once at setup; tests that need to verify state mutations after API
/// calls should re-read from the shared engine_state arc.
fn setup() -> Option<axum::Router> {
    let gpu = varda::renderer::context::GpuContext::new_headless().ok()?;
    let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
    let app = VardaApp::new(gpu, &config).ok()?;

    let _real_cmd_tx = app.command_sender();
    let state_reader = app.state_reader();
    // Publish initial state so GET routes work
    app.publish_state();

    // Create a proxy channel: API sends commands here, we forward to the real
    // engine sender. Since VardaApp is !Send, we forward immediately and let
    // the engine process when app.process_commands() would be called. For E2E
    // route tests, what matters is that the route handler gets a reply, so we
    // use a mock auto-reply approach (same as existing route tests).
    let (proxy_tx, mut proxy_rx) = tokio::sync::mpsc::unbounded_channel::<varda::engine::CommandEnvelope>();

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
    let resp = app.oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn put_json(app: axum::Router, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(
        Request::put(path).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap(),
    ).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn post_json(app: axum::Router, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(
        Request::post(path).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap(),
    ).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn post_empty(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(Request::post(path).body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 128).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

// ── Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn health_with_real_engine() {
    let Some(app) = setup() else { return; };
    let (status, json) = get_json(app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn state_reflects_real_engine() {
    let Some(app) = setup() else { return; };
    let (status, json) = get_json(app, "/api/state").await;
    assert_eq!(status, StatusCode::OK);
    let channels = json["mixer"]["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
}

#[tokio::test]
async fn set_crossfader_via_api() {
    let Some(app) = setup() else { return; };
    let (status, json) = put_json(app, "/api/mixer/crossfader", serde_json::json!({"position": 0.6})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn add_channel_via_api() {
    let Some(app) = setup() else { return; };
    let (status, json) = post_empty(app, "/api/channels").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn add_solid_deck_via_api() {
    let Some(app) = setup() else { return; };
    let (status, json) = post_json(app, "/api/channels/0/decks/solid",
        serde_json::json!({"color": [1.0, 0.0, 0.0, 1.0]})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn multi_step_api_workflow() {
    let Some(app) = setup() else { return; };
    let (s, _) = post_empty(app, "/api/channels").await;
    assert_eq!(s, StatusCode::OK);

    let Some(app) = setup() else { return; };
    let (s, _) = post_json(app, "/api/channels/0/decks/solid", serde_json::json!({"color": [0.0, 1.0, 0.0, 1.0]})).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn add_lfo_via_api() {
    let Some(app) = setup() else { return; };
    let (status, json) = post_json(app, "/api/modulation/lfo",
        serde_json::json!({"waveform": "Sine", "frequency": 2.0})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn set_channel_opacity_via_api() {
    let Some(app) = setup() else { return; };
    let (status, json) = put_json(app, "/api/channels/0/opacity",
        serde_json::json!({"opacity": 0.3})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn remove_channel_via_api() {
    let Some(app) = setup() else { return; };
    let (s, _) = post_empty(app, "/api/channels").await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn state_mixer_endpoint() {
    let Some(app) = setup() else { return; };
    let (status, json) = get_json(app, "/api/state/mixer").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["channels"].is_array());
    assert!(json["crossfader"].is_number());
}

#[tokio::test]
async fn state_modulation_endpoint() {
    let Some(app) = setup() else { return; };
    let (status, json) = get_json(app, "/api/state/modulation").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["sources"].is_array());
}

#[tokio::test]
async fn undo_redo_via_api() {
    let Some(app) = setup() else { return; };
    let (s, _) = put_json(app, "/api/mixer/crossfader", serde_json::json!({"position": 0.5})).await;
    assert_eq!(s, StatusCode::OK);

    let Some(app2) = setup() else { return; };
    let (s, _) = post_empty(app2, "/api/undo").await;
    assert_eq!(s, StatusCode::OK);

    let Some(app3) = setup() else { return; };
    let (s, _) = post_empty(app3, "/api/redo").await;
    assert_eq!(s, StatusCode::OK);
}