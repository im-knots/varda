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

#[utoipa::path(delete, path = "/api/outputs/{idx}", params(("idx" = usize, Path, description = "Output index")), responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn close(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::CloseOutput { idx }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/outputs/{idx}/display", params(("idx" = usize, Path, description = "Output index")), request_body = SetDisplayBody, responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn set_display(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<SetDisplayBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetOutputDisplay {
            idx,
            monitor_name: b.monitor_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/outputs/{output_uuid}/surfaces", params(("output_uuid" = String, Path, description = "Output UUID")), request_body = AssignSurfaceBody, responses((status = 200, body = CommandResult)), tag = "Outputs")]
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

#[utoipa::path(delete, path = "/api/outputs/{output_uuid}/surfaces/{assignment_idx}", params(("output_uuid" = String, Path, description = "Output UUID"), ("assignment_idx" = usize, Path, description = "Surface assignment index")), responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn unassign_surface(
    State(s): State<SharedState>,
    Path((output_uuid, assignment_idx)): Path<(String, usize)>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UnassignSurfaceFromOutput {
            output_uuid,
            assignment_idx,
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
#[utoipa::path(post, path = "/api/outputs/{idx}/start", params(("idx" = usize, Path, description = "Output index")), responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn start(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::StartOutput { idx }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/outputs/{idx}/stop", params(("idx" = usize, Path, description = "Output index")), responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn stop(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::StopOutput { idx }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[derive(Deserialize, ToSchema)]
pub struct SetCalibrationModeBody {
    /// Calibration display mode (Off / Projector / Surfaces).
    pub mode: crate::renderer::context::CalibrationMode,
}

#[utoipa::path(put, path = "/api/outputs/{idx}/calibration", params(("idx" = usize, Path, description = "Output index")), request_body = SetCalibrationModeBody, responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn set_calibration_mode(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<SetCalibrationModeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetCalibrationMode { idx, mode: b.mode })
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

#[utoipa::path(put, path = "/api/outputs/{idx}/target", params(("idx" = usize, Path, description = "Output index")), request_body = SetOutputTargetBody, responses((status = 200, body = CommandResult)), tag = "Outputs")]
pub async fn set_target(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<SetOutputTargetBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetOutputTarget {
            idx,
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
    path = "/api/outputs/{idx}/edge-blend",
    params(("idx" = usize, Path, description = "Output index")),
    request_body = SetEdgeBlendBody,
    responses((status = 200, body = CommandResult)),
    tag = "Outputs"
)]
pub async fn set_edge_blend(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<SetEdgeBlendBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetEdgeBlend {
            output_idx: idx,
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
    path = "/api/outputs/{idx}/edge-blend-mode",
    params(("idx" = usize, Path, description = "Output index")),
    request_body = SetEdgeBlendModeBody,
    responses((status = 200, body = CommandResult)),
    tag = "Outputs"
)]
pub async fn set_edge_blend_mode(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<SetEdgeBlendModeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetEdgeBlendMode {
            output_idx: idx,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
