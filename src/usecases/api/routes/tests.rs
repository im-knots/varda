//! Integration-style tests for all read-only routes.
//! Uses a real axum router with populated engine state.

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::engine::CommandResult;
    use crate::usecases::api::projection::tests::make_test_state;
    use crate::usecases::api::SharedState;

    /// Create a router with populated state and a mock command processor
    /// that auto-replies `CommandResult::Ok` to every command.
    fn router_with_mock_engine() -> axum::Router {
        let state = make_test_state();
        let (cmd_tx, mut cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::engine::CommandEnvelope>();
        let engine_state = std::sync::Arc::new(std::sync::RwLock::new(Some(state)));

        // Spawn a background task that processes commands
        tokio::spawn(async move {
            while let Some((_cmd, reply_tx)) = cmd_rx.recv().await {
                if let Some(tx) = reply_tx {
                    let _ = tx.send(CommandResult::Ok);
                }
            }
        });

        let shared = SharedState {
            command_tx: cmd_tx,
            engine_state,
        };
        crate::usecases::api::runner::build_router(shared)
    }

    fn router_with_state() -> axum::Router {
        let state = make_test_state();
        let shared = SharedState {
            command_tx: tokio::sync::mpsc::unbounded_channel().0,
            engine_state: std::sync::Arc::new(std::sync::RwLock::new(Some(state))),
        };
        crate::usecases::api::runner::build_router(shared)
    }

    async fn get_json(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json)
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
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
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
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    // ── Runtime state routes ────────────────────────────────────────

    #[tokio::test]
    async fn test_state_mixer() {
        let (status, json) = get_json(router_with_state(), "/api/state/mixer").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["crossfader"].as_f64().is_some());
        assert!(json["channels"].is_array());
    }

    #[tokio::test]
    async fn test_state_audio() {
        let (status, json) = get_json(router_with_state(), "/api/state/audio").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], true);
        assert_eq!(json["sample_rate"], 48000.0);
    }

    #[tokio::test]
    async fn test_state_performance() {
        let (status, json) = get_json(router_with_state(), "/api/state/performance").await;
        assert_eq!(status, StatusCode::OK);
        assert!((json["fps"].as_f64().unwrap() - 60.0).abs() < 1e-3);
        assert_eq!(json["frame_count"], 100);
    }

    #[tokio::test]
    async fn test_state_ndi() {
        let (status, json) = get_json(router_with_state(), "/api/state/ndi").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        assert_eq!(json["sources"][0], "OBS");
    }

    #[tokio::test]
    async fn test_state_clock() {
        let (status, json) = get_json(router_with_state(), "/api/state/clock").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["active"], true);
        assert_eq!(json["bpm"], 120.0);
        assert_eq!(json["source_label"], "Audio");
    }

    #[tokio::test]
    async fn test_state_modulation() {
        let (status, json) = get_json(router_with_state(), "/api/state/modulation").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["sources"].is_array());
    }

    #[tokio::test]
    async fn test_state_outputs() {
        let (status, json) = get_json(router_with_state(), "/api/state/outputs").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["windows"].is_array());
        assert!(json["surfaces"].is_array());
        assert!(json["monitors"].is_array());
    }

    #[tokio::test]
    async fn test_state_surfaces() {
        let (status, json) = get_json(router_with_state(), "/api/state/surfaces").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
        assert_eq!(json[0]["uuid"], "srf-001");
    }

    #[tokio::test]
    async fn test_state_registry() {
        let (status, json) = get_json(router_with_state(), "/api/state/registry").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["shader_count"], 2);
    }

    #[tokio::test]
    async fn test_state_midi() {
        let (status, json) = get_json(router_with_state(), "/api/state/midi").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["learn_active"], false);
    }

    #[tokio::test]
    async fn test_state_cameras() {
        let (status, json) = get_json(router_with_state(), "/api/state/cameras").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["devices"].is_array());
    }

    #[tokio::test]
    async fn test_state_syphon() {
        let (status, json) = get_json(router_with_state(), "/api/state/syphon").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert!(json["sources"].is_array());
    }

    #[tokio::test]
    async fn test_state_streams() {
        let (status, json) = get_json(router_with_state(), "/api/state/streams").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    // ── Scene routes ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_scene_full() {
        let (status, json) = get_json(router_with_state(), "/api/scene").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["channels"][0]["uuid"], "ch-001");
        assert!((json["crossfader"].as_f64().unwrap() - 0.5).abs() < 1e-3);
        assert!(json["master_effects"].is_array());
        assert!(json["modulation"].is_object());
        assert!(json["sequences"].is_array());
        assert!(json["streams"].is_array());
    }

    #[tokio::test]
    async fn test_scene_channels() {
        let (status, json) = get_json(router_with_state(), "/api/scene/channels").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "Channel A");
    }

    #[tokio::test]
    async fn test_scene_channel_by_uuid() {
        let (status, json) = get_json(router_with_state(), "/api/scene/channels/ch-001").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["uuid"], "ch-001");
        assert_eq!(json["decks"][0]["uuid"], "dk-001");
    }

    #[tokio::test]
    async fn test_scene_channel_not_found() {
        let (status, _) = get_json(router_with_state(), "/api/scene/channels/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_scene_channel_decks() {
        let (status, json) =
            get_json(router_with_state(), "/api/scene/channels/ch-001/decks").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
        assert_eq!(json[0]["uuid"], "dk-001");
    }

    #[tokio::test]
    async fn test_scene_channel_decks_channel_not_found() {
        let (status, _) = get_json(router_with_state(), "/api/scene/channels/bad/decks").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_scene_deck_by_uuid() {
        let (status, json) = get_json(
            router_with_state(),
            "/api/scene/channels/ch-001/decks/dk-001",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["name"], "Sine");
    }

    #[tokio::test]
    async fn test_scene_deck_not_found() {
        let (status, _) =
            get_json(router_with_state(), "/api/scene/channels/ch-001/decks/bad").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_scene_deck_channel_not_found() {
        let (status, _) =
            get_json(router_with_state(), "/api/scene/channels/bad/decks/dk-001").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_scene_modulation() {
        let (status, json) = get_json(router_with_state(), "/api/scene/modulation").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["sources"].is_array());
    }

    #[tokio::test]
    async fn test_scene_sequences() {
        let (status, json) = get_json(router_with_state(), "/api/scene/sequences").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    #[tokio::test]
    async fn test_scene_streams() {
        let (status, json) = get_json(router_with_state(), "/api/scene/streams").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    // ── Stage routes ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_stage_full() {
        let (status, json) = get_json(router_with_state(), "/api/stage").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["surfaces"][0]["uuid"], "srf-001");
        assert_eq!(json["outputs"][0]["uuid"], "out-001");
        assert!(json["monitors"].is_array());
    }

    #[tokio::test]
    async fn test_stage_surfaces_list() {
        let (status, json) = get_json(router_with_state(), "/api/stage/surfaces").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
        assert_eq!(json[0]["uuid"], "srf-001");
    }

    #[tokio::test]
    async fn test_stage_surface_by_uuid() {
        let (status, json) = get_json(router_with_state(), "/api/stage/surfaces/srf-001").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["name"], "Main");
        assert!(json["vertices"].is_array());
    }

    #[tokio::test]
    async fn test_stage_surface_not_found() {
        let (status, _) = get_json(router_with_state(), "/api/stage/surfaces/bad").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_stage_outputs_list() {
        let (status, json) = get_json(router_with_state(), "/api/stage/outputs").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
        assert_eq!(json[0]["uuid"], "out-001");
    }

    #[tokio::test]
    async fn test_stage_output_by_uuid() {
        let (status, json) = get_json(router_with_state(), "/api/stage/outputs/out-001").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["name"], "Output 1");
    }

    #[tokio::test]
    async fn test_stage_output_not_found() {
        let (status, _) = get_json(router_with_state(), "/api/stage/outputs/bad").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // ── Library routes ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_library_generators() {
        let (status, json) = get_json(router_with_state(), "/api/library/generators").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "Sine");
        assert!(json[0]["index"].is_number());
    }

    #[tokio::test]
    async fn test_library_effects() {
        let (status, json) = get_json(router_with_state(), "/api/library/effects").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "Blur");
    }

    #[tokio::test]
    async fn test_library_transitions() {
        let (status, json) = get_json(router_with_state(), "/api/library/transitions").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "Dissolve");
    }

    #[tokio::test]
    async fn test_library_cameras() {
        let (status, json) = get_json(router_with_state(), "/api/library/cameras").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "FaceTime");
    }

    #[tokio::test]
    async fn test_library_ndi() {
        let (status, json) = get_json(router_with_state(), "/api/library/ndi").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "OBS");
    }

    #[tokio::test]
    async fn test_library_syphon() {
        let (status, json) = get_json(router_with_state(), "/api/library/syphon").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    #[tokio::test]
    async fn test_library_monitors() {
        let (status, json) = get_json(router_with_state(), "/api/library/monitors").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "HDMI-1");
        assert_eq!(json[0]["width"], 1920);
        assert_eq!(json[0]["height"], 1080);
    }

    // ── Write: Mixer routes ─────────────────────────────────────────

    #[tokio::test]
    async fn test_set_crossfader() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/mixer/crossfader",
            serde_json::json!({"position": 0.75}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_auto_crossfade() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/mixer/auto-crossfade",
            serde_json::json!({"target": 1.0, "duration_secs": 2.0, "easing": "Linear"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_beat_crossfade() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/mixer/beat-crossfade",
            serde_json::json!({"target": 0.0, "beats": 4.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Channel routes ───────────────────────────────────────

    #[tokio::test]
    async fn test_add_channel() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_channel() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/channels/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_set_channel_opacity() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/opacity",
            serde_json::json!({"opacity": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_channel_blend_mode() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/blend-mode",
            serde_json::json!({"mode": "Add"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Deck routes ──────────────────────────────────────────

    #[tokio::test]
    async fn test_add_shader_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/shader",
            serde_json::json!({"shader_name": "Sine"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_deck() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/channels/0/decks/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_set_deck_opacity() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/opacity",
            serde_json::json!({"opacity": 0.8}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_deck_solo() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/solo",
            serde_json::json!({"value": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_deck_mute() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/mute",
            serde_json::json!({"value": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_deck_scaling_mode() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/scaling-mode",
            serde_json::json!({"mode": "Fit"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_image_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/image",
            serde_json::json!({"path": "/tmp/test.png"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_solid_color_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/solid",
            serde_json::json!({"color": [1.0, 0.0, 0.0, 1.0]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_move_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/decks/move",
            serde_json::json!({"src_ch": 0, "src_deck": 0, "dst_ch": 1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Effects routes ───────────────────────────────────

    #[tokio::test]
    async fn test_add_effect() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/effects",
            serde_json::json!({"target": "Master", "shader_name": "Blur"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_toggle_effect() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/effects/toggle",
            serde_json::json!({"target": "Master", "effect_idx": 0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Audio routes ─────────────────────────────────────

    #[tokio::test]
    async fn test_audio_scan() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/audio/scan",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_audio_open_source() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/audio/open",
            serde_json::json!({"source_id": 1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_audio_close_source() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/audio/close",
            serde_json::json!({"source_id": 1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Modulation routes ────────────────────────────────

    #[tokio::test]
    async fn test_add_lfo() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/lfo",
            serde_json::json!({"waveform": "Sine", "frequency": 1.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_audio_band() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/audio-band",
            serde_json::json!({"preset": "Low", "source_id": null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_adsr() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/adsr",
            serde_json::json!({"attack": 0.1, "decay": 0.2, "sustain": 0.7, "release": 0.3}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_step_sequencer() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/step-sequencer",
            serde_json::json!({"num_steps": 8, "rate": 2.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_modulation_assign() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/assign",
            serde_json::json!({"target": "deck_a:brightness", "source_id": "lfo-1", "amount": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_modulation_clear() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/clear",
            serde_json::json!({"target": "deck_a:brightness"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_modulation_source() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/modulation/lfo-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── Read: Macro routes ──────────────────────────────────────

    #[tokio::test]
    async fn test_state_macros() {
        let (status, json) = get_json(router_with_state(), "/api/state/macros").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    #[tokio::test]
    async fn test_scene_macros() {
        let (status, json) = get_json(router_with_state(), "/api/scene/macros").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    // ── Write: Macro routes ─────────────────────────────────────

    #[tokio::test]
    async fn test_add_macro() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/macros",
            serde_json::json!({"kind": "Knob"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_macro() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/macros/mac-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_rename_macro() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/name",
            serde_json::json!({"name": "Sweep"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_macro_kind() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/kind",
            serde_json::json!({"kind": "Button"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_macro_value() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/value",
            serde_json::json!({"value": 0.75}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_macro_target() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/targets",
            serde_json::json!({"path": "crossfader"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_macro_target() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/targets/0",
            serde_json::json!({"min": 0.2, "max": 0.9, "curve": "SCurve", "invert": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_macro_target_stepped_curve() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/targets/1",
            serde_json::json!({"min": 0.0, "max": 1.0, "curve": {"Stepped": 4}, "invert": false}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_macro_target() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/macros/mac-123/targets/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_set_macro_button_behavior() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/button/behavior",
            serde_json::json!({"behavior": "Toggle"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_macro_triggers() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/macros/mac-123/button/triggers",
            serde_json::json!({"triggers": [
                {"Global": "Save"},
                {"Param": {"path": "crossfader", "value": 0.0}}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Surface routes ───────────────────────────────────

    #[tokio::test]
    async fn test_add_rect_surface() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/rect",
            serde_json::json!({"name": "Main", "source": "Master"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_polygon_surface() {
        let (status, json) = post_json(
            router_with_mock_engine(), "/api/surfaces/polygon",
            serde_json::json!({"name": "Tri", "vertices": [[0.0,0.0],[1.0,0.0],[0.5,1.0]], "source": "Master"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_circle_surface() {
        let (status, json) = post_json(
            router_with_mock_engine(), "/api/surfaces/circle",
            serde_json::json!({"name": "Spot", "center": [0.5, 0.5], "radius": 0.3, "sides": 32, "source": "Master"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_surface() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/surfaces/srf-001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_set_surface_source() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/source",
            serde_json::json!({"source": "Master"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_surface_output_type() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/output-type",
            serde_json::json!({"output_type": "Projection"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_surface_content_mapping() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/content-mapping",
            serde_json::json!({"mapping": "Fill"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_rename_surface() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/name",
            serde_json::json!({"name": "New Name"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_surface_vertices() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/vertices",
            serde_json::json!({"vertices": [[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_duplicate_surface() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/duplicate",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_flip_surface_horizontal() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/flip-horizontal",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_flip_surface_vertical() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/flip-vertical",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_rotate_surface() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/rotate",
            serde_json::json!({"angle": 0.5, "pivot": [0.5, 0.5]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_scale_surface() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/scale",
            serde_json::json!({"sx": 1.5, "sy": 0.5, "pivot": [0.0, 0.0]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_convert_edge() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/edge/convert",
            serde_json::json!({"edge_idx": 0, "to_cubic": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_move_path_anchor() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/path/anchor",
            serde_json::json!({"anchor_idx": 1, "pos": [0.3, 0.4]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_move_path_handle() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/srf-001/path/handle",
            serde_json::json!({"segment_idx": 0, "handle": "C1", "pos": [0.6, 0.7]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Output routes ────────────────────────────────────

    #[tokio::test]
    async fn test_create_output() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/outputs",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_close_output() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/outputs/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_set_output_display() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/outputs/0/display",
            serde_json::json!({"monitor_name": "HDMI-1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_assign_surface_to_output() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/outputs/out-001/surfaces",
            serde_json::json!({"surface_uuid": "srf-001"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_unassign_surface_from_output() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/outputs/out-001/surfaces/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_start_output() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/outputs/0/start",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_stop_output() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/outputs/0/stop",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_calibration_mode() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/outputs/0/calibration",
            serde_json::json!({"mode": "Projector"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_warp_corner() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/corner",
            serde_json::json!({"corner_idx": 0, "position": [0.1, 0.1]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_reset_warp() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/reset",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_warp_subdivisions() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/subdivisions",
            serde_json::json!({"cols": 3, "rows": 3}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_warp_mesh_point() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/mesh-point",
            serde_json::json!({"row": 1, "col": 1, "position": [0.6, 0.4]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_warp_bound() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/bind",
            serde_json::json!({"bound": false}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_convert_warp_to_bezier() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/bezier",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_move_warp_anchor() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/anchor",
            serde_json::json!({"row": 0, "col": 0, "position": [0.2, 0.3]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_move_warp_handle() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/handle",
            serde_json::json!({"horizontal": true, "row": 0, "col": 0, "which": 0, "position": [0.3, 0.1]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_bezier_cage_subdivisions() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/surfaces/s1/warp/cage",
            serde_json::json!({"cols": 3, "rows": 3}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Transition & Params ──────────────────────────────

    #[tokio::test]
    async fn test_set_transition() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/mixer/transition",
            serde_json::json!({"shader_name": "Dissolve"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_param() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/params",
            serde_json::json!({"path": "deck_a:brightness", "value": 0.8}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Generic command ──────────────────────────────────

    #[tokio::test]
    async fn test_generic_command() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/command",
            serde_json::json!({"SetCrossfader": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Video Playback routes ────────────────────────────

    #[tokio::test]
    async fn test_video_toggle_play() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/video/toggle-play",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_video_seek() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/video/seek",
            serde_json::json!({"position_secs": 10.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_video_set_speed() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/video/speed",
            serde_json::json!({"speed": 2.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_video_set_loop_mode() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/video/loop-mode",
            serde_json::json!({"mode": "Loop"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_video_set_in_point() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/video/in-point",
            serde_json::json!({"secs": 1.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_video_set_out_point() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/video/out-point",
            serde_json::json!({"secs": 30.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_video_clear_in_out_points() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/channels/0/decks/0/video/in-out-points")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── Write: Auto-Transition routes ───────────────────────────

    #[tokio::test]
    async fn test_set_auto_transition_enabled() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/auto-transition/enabled",
            serde_json::json!({"value": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_auto_transition_trigger() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/auto-transition/trigger",
            serde_json::json!({"value": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_auto_transition_play_duration() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/auto-transition/play-duration",
            serde_json::json!({"value": 5.0, "unit": "Seconds"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_auto_transition_duration() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/auto-transition/duration",
            serde_json::json!({"value": 2.0, "unit": "Seconds"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_auto_transition_shader() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/auto-transition/shader",
            serde_json::json!({"shader_name": "Dissolve"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: External I/O Sources ─────────────────────────────

    #[tokio::test]
    async fn test_add_video_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/video",
            serde_json::json!({"path": "/tmp/test.mp4"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_camera_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/camera",
            serde_json::json!({"camera_id": 0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_ndi_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/ndi",
            serde_json::json!({"source_name": "OBS"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_syphon_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/syphon",
            serde_json::json!({"server_name": "TestServer"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_srt_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/srt",
            serde_json::json!({"url": "srt://localhost:9000", "mode": "Caller"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_html_deck() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/html",
            serde_json::json!({"url": "https://example.com/visuals.html"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Effects extras ───────────────────────────────────

    #[tokio::test]
    async fn test_remove_effect() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/effects")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(
                            &serde_json::json!({"target": "Master", "effect_idx": 0}),
                        )
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_move_effect() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/effects/move",
            serde_json::json!({"target": "Master", "from_idx": 0, "to_idx": 1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Sequence routes ──────────────────────────────────

    #[tokio::test]
    async fn test_create_sequence() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_delete_sequence() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/sequences/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_play_sequence() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences/0/play",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_stop_sequence() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences/0/stop",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_toggle_sequence() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences/0/toggle",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_fade_step() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/fade",
            serde_json::json!({"from_ch": 0, "to_ch": 1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_wait_step() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/wait",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_add_goto_step() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/goto",
            serde_json::json!({"step_index": 0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_step() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/sequences/0/steps/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_set_step_duration() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/0/duration",
            serde_json::json!({"value": 3.0, "unit": "Seconds"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_step_easing() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/0/easing",
            serde_json::json!({"easing": "Linear"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_step_shader() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/0/shader",
            serde_json::json!({"shader_name": "Dissolve"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: System / Clock / Resolution / Persistence ────────

    #[tokio::test]
    async fn test_shutdown() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/shutdown",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_undo() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/undo",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_redo() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/redo",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_resolution() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/resolution",
            serde_json::json!({"width": 1920, "height": 1080}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_clock_preference() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/clock/preference",
            serde_json::json!({"preference": "Auto"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_manual_bpm() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/clock/manual-bpm",
            serde_json::json!({"bpm": 128.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_save_workspace() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/workspace/save",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_load_workspace() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/workspace/load",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Write: Device Scanning & MIDI ───────────────────────────

    #[tokio::test]
    async fn test_scan_ndi() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/devices/ndi/scan",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_scan_syphon() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/devices/syphon/scan",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_scan_cameras() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/devices/cameras/scan",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_scan_midi() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/devices/midi/scan",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_set_midi_device_enabled() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/devices/midi/enabled",
            serde_json::json!({"device_id": 0, "enabled": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_clear_midi_mappings() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/midi/mappings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── Write: Stream Library ───────────────────────────────────

    #[tokio::test]
    async fn test_add_stream_library_entry() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/streams/library",
            serde_json::json!({"url": "srt://host:9000", "mode": "Caller"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_remove_stream_library_entry() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::delete("/api/streams/library")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({"url": "srt://host:9000"})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── Write: Modulation Parameter Updates ─────────────────────

    #[tokio::test]
    async fn test_update_lfo_frequency() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/lfo-1/lfo/frequency",
            serde_json::json!({"value": 2.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_lfo_waveform() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/lfo-1/lfo/waveform",
            serde_json::json!({"waveform": "Triangle"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_lfo_phase() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/lfo-1/lfo/phase",
            serde_json::json!({"value": 0.25}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_lfo_amplitude() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/lfo-1/lfo/amplitude",
            serde_json::json!({"value": 0.8}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_lfo_bipolar() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/lfo-1/lfo/bipolar",
            serde_json::json!({"value": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_audio_smoothing() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ab-1/audio/smoothing",
            serde_json::json!({"value": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_audio_freq_range() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ab-1/audio/freq-range",
            serde_json::json!({"freq_low": 20.0, "freq_high": 200.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_audio_gain() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ab-1/audio/gain",
            serde_json::json!({"value": 1.5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_audio_preset() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ab-1/audio/preset",
            serde_json::json!({"preset": "Low"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_audio_mode() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ab-1/audio/mode",
            serde_json::json!({"mode": "Direct"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_adsr_attack() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/adsr-1/adsr/attack",
            serde_json::json!({"value": 0.05}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_adsr_decay() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/adsr-1/adsr/decay",
            serde_json::json!({"value": 0.3}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_adsr_sustain() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/adsr-1/adsr/sustain",
            serde_json::json!({"value": 0.6}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_adsr_release() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/adsr-1/adsr/release",
            serde_json::json!({"value": 0.4}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_trigger_adsr() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/adsr-1/adsr/trigger",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_release_adsr() {
        let (status, json) = post_json(
            router_with_mock_engine(),
            "/api/modulation/adsr-1/adsr/release-gate",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_step_seq_steps() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ss-1/step-seq/steps",
            serde_json::json!({"steps": [0.0, 0.5, 1.0, 0.5, 0.0, 0.5, 1.0, 0.5]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_step_seq_rate() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ss-1/step-seq/rate",
            serde_json::json!({"value": 4.0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_update_step_seq_interpolation() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/modulation/ss-1/step-seq/interpolation",
            serde_json::json!({"interpolation": "Linear"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ══════════════════════════════════════════════════════════════
    // ── Edge Cases & Error Handling ─────────────────────────────
    // ══════════════════════════════════════════════════════════════

    // ── 503 when engine state not initialized ───────────────────

    fn router_without_state() -> axum::Router {
        let shared = SharedState {
            command_tx: tokio::sync::mpsc::unbounded_channel().0,
            engine_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
        };
        crate::usecases::api::runner::build_router(shared)
    }

    #[tokio::test]
    async fn test_state_returns_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/state").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_state_mixer_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/state/mixer").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_scene_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/scene").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_stage_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/stage").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_library_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/library/generators").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_scene_channel_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/scene/channels/ch-001").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_stage_surface_503_when_not_initialized() {
        let (status, _) = get_json(router_without_state(), "/api/stage/surfaces/srf-001").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── Invalid JSON body → 4xx ─────────────────────────────────

    #[tokio::test]
    async fn test_invalid_json_body_returns_4xx() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::put("/api/mixer/crossfader")
                    .header("content-type", "application/json")
                    .body(Body::from(b"not json".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        assert!((400..500).contains(&status), "Expected 4xx, got {status}");
    }

    #[tokio::test]
    async fn test_missing_required_field_returns_4xx() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::put("/api/mixer/crossfader")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({"wrong_field": 0.5})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        assert!((400..500).contains(&status), "Expected 4xx, got {status}");
    }

    // ── Wrong HTTP method → 405 ─────────────────────────────────

    #[tokio::test]
    async fn test_get_on_write_route_returns_405() {
        let (status, _) = get_json(router_with_mock_engine(), "/api/mixer/crossfader").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_post_on_get_route_returns_405() {
        let (status, _) = post_json(
            router_with_state(),
            "/api/state/mixer",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    // ── Nonexistent route → 404 ─────────────────────────────────

    #[tokio::test]
    async fn test_nonexistent_route_returns_404() {
        let (status, _) = get_json(router_with_state(), "/api/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_nonexistent_nested_route_returns_404() {
        let (status, _) = get_json(router_with_state(), "/api/state/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // ── CommandResult mapping (mock engine returning errors) ────

    fn router_with_not_found_engine() -> axum::Router {
        let state = make_test_state();
        let (cmd_tx, mut cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::engine::CommandEnvelope>();
        let engine_state = std::sync::Arc::new(std::sync::RwLock::new(Some(state)));
        tokio::spawn(async move {
            while let Some((_cmd, reply_tx)) = cmd_rx.recv().await {
                if let Some(tx) = reply_tx {
                    let _ = tx.send(CommandResult::Err {
                        code: crate::engine::ErrorCode::NotFound,
                        message: "Entity not found".into(),
                    });
                }
            }
        });
        let shared = SharedState {
            command_tx: cmd_tx,
            engine_state,
        };
        crate::usecases::api::runner::build_router(shared)
    }

    fn router_with_invalid_input_engine() -> axum::Router {
        let state = make_test_state();
        let (cmd_tx, mut cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::engine::CommandEnvelope>();
        let engine_state = std::sync::Arc::new(std::sync::RwLock::new(Some(state)));
        tokio::spawn(async move {
            while let Some((_cmd, reply_tx)) = cmd_rx.recv().await {
                if let Some(tx) = reply_tx {
                    let _ = tx.send(CommandResult::Err {
                        code: crate::engine::ErrorCode::InvalidInput,
                        message: "Invalid value".into(),
                    });
                }
            }
        });
        let shared = SharedState {
            command_tx: cmd_tx,
            engine_state,
        };
        crate::usecases::api::runner::build_router(shared)
    }

    fn router_with_ok_with_id_engine() -> axum::Router {
        let state = make_test_state();
        let (cmd_tx, mut cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::engine::CommandEnvelope>();
        let engine_state = std::sync::Arc::new(std::sync::RwLock::new(Some(state)));
        tokio::spawn(async move {
            while let Some((_cmd, reply_tx)) = cmd_rx.recv().await {
                if let Some(tx) = reply_tx {
                    let _ = tx.send(CommandResult::OkWithId {
                        uuid: "new-uuid-123".into(),
                    });
                }
            }
        });
        let shared = SharedState {
            command_tx: cmd_tx,
            engine_state,
        };
        crate::usecases::api::runner::build_router(shared)
    }

    fn router_with_ok_with_data_engine() -> axum::Router {
        let state = make_test_state();
        let (cmd_tx, mut cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::engine::CommandEnvelope>();
        let engine_state = std::sync::Arc::new(std::sync::RwLock::new(Some(state)));
        tokio::spawn(async move {
            while let Some((_cmd, reply_tx)) = cmd_rx.recv().await {
                if let Some(tx) = reply_tx {
                    let _ = tx.send(CommandResult::OkWithData {
                        data: serde_json::json!({"key": "value"}),
                    });
                }
            }
        });
        let shared = SharedState {
            command_tx: cmd_tx,
            engine_state,
        };
        crate::usecases::api::runner::build_router(shared)
    }

    #[tokio::test]
    async fn test_command_result_not_found_returns_404() {
        let (status, json) = put_json(
            router_with_not_found_engine(),
            "/api/mixer/crossfader",
            serde_json::json!({"position": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "not_found");
        assert!(json["message"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_command_result_invalid_input_returns_400() {
        let (status, json) = put_json(
            router_with_invalid_input_engine(),
            "/api/mixer/crossfader",
            serde_json::json!({"position": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_input");
    }

    #[tokio::test]
    async fn test_command_result_ok_with_id() {
        let (status, json) = post_json(
            router_with_ok_with_id_engine(),
            "/api/channels",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["uuid"], "new-uuid-123");
    }

    #[tokio::test]
    async fn test_command_result_ok_with_data() {
        let (status, json) = post_json(
            router_with_ok_with_data_engine(),
            "/api/channels",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["data"]["key"], "value");
    }

    // ── Engine channel closed → 500 ─────────────────────────────

    #[tokio::test]
    async fn test_engine_channel_closed_returns_500() {
        let state = make_test_state();
        let (cmd_tx, cmd_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::engine::CommandEnvelope>();
        let engine_state = std::sync::Arc::new(std::sync::RwLock::new(Some(state)));
        // Drop receiver immediately so sends fail
        drop(cmd_rx);
        let shared = SharedState {
            command_tx: cmd_tx,
            engine_state,
        };
        let app = crate::usecases::api::runner::build_router(shared);

        let (status, _) = put_json(
            app,
            "/api/mixer/crossfader",
            serde_json::json!({"position": 0.5}),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ── Full state snapshot smoke test ──────────────────────────

    #[tokio::test]
    async fn test_full_state_snapshot_returns_all_fields() {
        let (status, json) = get_json(router_with_state(), "/api/state").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["mixer"].is_object());
        assert!(json["audio"].is_object());
        assert!(json["modulation"].is_object());
        assert!(json["outputs"].is_object());
        assert!(json["registry"].is_object());
        assert!(json["midi"].is_object());
        assert!(json["cameras"].is_object());
        assert!(json["clock"].is_object());
        assert!(json["fps"].is_number());
        assert!(json["frame_count"].is_number());
    }

    // ── Health check is always available ────────────────────────

    #[tokio::test]
    async fn test_health_always_available_even_without_state() {
        let (status, json) = get_json(router_without_state(), "/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Deck blend mode with all variants ───────────────────────

    #[tokio::test]
    async fn test_set_deck_blend_mode_add() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/blend-mode",
            serde_json::json!({"mode": "Add"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Transition with null shader (clear transition) ──────────

    #[tokio::test]
    async fn test_set_transition_null_clears() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/mixer/transition",
            serde_json::json!({"shader_name": null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Auto-transition shader with null (clear) ────────────────

    #[tokio::test]
    async fn test_auto_transition_shader_null() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/channels/0/decks/0/auto-transition/shader",
            serde_json::json!({"shader_name": null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Step shader with null (clear) ───────────────────────────

    #[tokio::test]
    async fn test_step_shader_null() {
        let (status, json) = put_json(
            router_with_mock_engine(),
            "/api/sequences/0/steps/0/shader",
            serde_json::json!({"shader_name": null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    // ── Missing content-type header ─────────────────────────────

    #[tokio::test]
    async fn test_missing_content_type_returns_415() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::put("/api/mixer/crossfader")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({"position": 0.5})).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // ── Empty body on POST endpoint that requires body ──────────

    #[tokio::test]
    async fn test_empty_body_on_post_with_required_body_returns_4xx() {
        let app = router_with_mock_engine();
        let resp = app
            .oneshot(
                Request::post("/api/modulation/lfo")
                    .header("content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        assert!((400..500).contains(&status), "Expected 4xx, got {status}");
    }
}
