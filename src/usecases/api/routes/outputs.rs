//! Output management write routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct SetDisplayBody {
    /// Name of the display monitor to target.
    pub monitor_name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignSurfaceBody {
    /// UUID of the surface to assign to this output.
    pub surface_uuid: String,
}

#[utoipa::path(post, path = "/api/outputs", responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn create(State(s): State<SharedState>) -> impl IntoResponse {
    match s.send_command(EngineCommand::CreateOutput).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/outputs/{output_uuid}", params(("output_uuid" = String, Path, description = "Output UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")), tag = "Outputs")]
pub async fn close(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::CloseOutput { output_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/outputs/{output_uuid}/display", params(("output_uuid" = String, Path, description = "Output UUID")), request_body = SetDisplayBody, responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")), tag = "Outputs")]
pub async fn set_display(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
    Json(b): Json<SetDisplayBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetOutputDisplay {
            output_uuid,
            monitor_name: b.monitor_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/outputs/{output_uuid}/surfaces", params(("output_uuid" = String, Path, description = "Output UUID")), request_body = AssignSurfaceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Output or surface not found")), tag = "Outputs")]
pub async fn assign_surface(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
    Json(b): Json<AssignSurfaceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AssignSurfaceToOutput {
            output_uuid,
            surface_uuid: b.surface_uuid,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/outputs/{output_uuid}/surfaces/{surface_uuid}", params(("output_uuid" = String, Path, description = "Output UUID"), ("surface_uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Output or surface assignment not found")), tag = "Outputs")]
pub async fn unassign_surface(
    State(s): State<SharedState>,
    Path((output_uuid, surface_uuid)): Path<(String, String)>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UnassignSurfaceFromOutput {
            output_uuid,
            surface_uuid,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

// ── Headless / Start / Stop / Calibration / Warp ───────────────────

#[derive(Deserialize, ToSchema)]
pub struct CreateHeadlessBody {
    /// Output target configuration for the headless output.
    pub target: crate::renderer::context::OutputTarget,
}

#[utoipa::path(post, path = "/api/outputs/headless", request_body = CreateHeadlessBody, responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn create_headless(
    State(s): State<SharedState>,
    Json(b): Json<CreateHeadlessBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::CreateHeadlessOutput { target: b.target })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/outputs/{output_uuid}/start", params(("output_uuid" = String, Path, description = "Output UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")), tag = "Outputs")]
pub async fn start(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::StartOutput { output_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/outputs/{output_uuid}/stop", params(("output_uuid" = String, Path, description = "Output UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")), tag = "Outputs")]
pub async fn stop(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::StopOutput { output_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[derive(Deserialize, ToSchema)]
pub struct SetCalibrationModeBody {
    /// Calibration display mode (Off / Projector / Surfaces).
    pub mode: crate::renderer::context::CalibrationMode,
}

#[utoipa::path(put, path = "/api/outputs/{output_uuid}/calibration", params(("output_uuid" = String, Path, description = "Output UUID")), request_body = SetCalibrationModeBody, responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")), tag = "Outputs")]
pub async fn set_calibration_mode(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
    Json(b): Json<SetCalibrationModeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetCalibrationMode {
            output_uuid,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Missing Parity Routes ─────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SetOutputTargetBody {
    /// Output target configuration.
    pub target: crate::renderer::context::OutputTarget,
}

#[utoipa::path(put, path = "/api/outputs/{output_uuid}/target", params(("output_uuid" = String, Path, description = "Output UUID")), request_body = SetOutputTargetBody, responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")), tag = "Outputs")]
pub async fn set_target(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
    Json(b): Json<SetOutputTargetBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetOutputTarget {
            output_uuid,
            target: b.target,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Edge Blending ────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SetEdgeBlendBody {
    /// Edge blending configuration (four edges with enabled, width, gamma).
    pub config: crate::renderer::edge_blend::EdgeBlendConfig,
}

#[utoipa::path(
    put,
    path = "/api/outputs/{output_uuid}/edge-blend",
    params(("output_uuid" = String, Path, description = "Output UUID")),
    request_body = SetEdgeBlendBody,
    responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")),
    tag = "Outputs"
)]
pub async fn set_edge_blend(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
    Json(b): Json<SetEdgeBlendBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetEdgeBlend {
            output_uuid,
            config: b.config,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SetEdgeBlendModeBody {
    /// Edge blend mode: "Auto" or "Manual".
    pub mode: crate::renderer::edge_blend::EdgeBlendMode,
}

#[utoipa::path(
    put,
    path = "/api/outputs/{output_uuid}/edge-blend-mode",
    params(("output_uuid" = String, Path, description = "Output UUID")),
    request_body = SetEdgeBlendModeBody,
    responses((status = 200, body = CommandResult), (status = 404, description = "Output not found")),
    tag = "Outputs"
)]
pub async fn set_edge_blend_mode(
    State(s): State<SharedState>,
    Path(output_uuid): Path<String>,
    Json(b): Json<SetEdgeBlendModeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetEdgeBlendMode {
            output_uuid,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
