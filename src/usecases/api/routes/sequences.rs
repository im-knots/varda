//! Transition sequence write routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{command_response, SharedState};

#[utoipa::path(post, path = "/api/sequences", responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn create(State(s): State<SharedState>) -> impl IntoResponse {
    match s.send_command(EngineCommand::CreateSequence).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(delete, path = "/api/sequences/{idx}", params(("idx" = usize, Path, description = "Sequence index")), responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn delete(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::DeleteSequence { idx }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{idx}/play", params(("idx" = usize, Path, description = "Sequence index")), responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn play(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::PlaySequence { idx }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{idx}/stop", params(("idx" = usize, Path, description = "Sequence index")), responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn stop(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::StopSequence { idx }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{idx}/toggle", params(("idx" = usize, Path, description = "Sequence index")), responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn toggle(State(s): State<SharedState>, Path(idx): Path<usize>) -> impl IntoResponse {
    match s.send_command(EngineCommand::ToggleSequence { idx }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddFadeStepBody {
    /// Source channel index for the crossfade.
    pub from_ch: usize,
    /// Destination channel index for the crossfade.
    pub to_ch: usize,
}
#[utoipa::path(post, path = "/api/sequences/{idx}/steps/fade", params(("idx" = usize, Path, description = "Sequence index")), request_body = AddFadeStepBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn add_fade_step(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<AddFadeStepBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddFadeStep {
            seq_idx: idx,
            from_ch: b.from_ch,
            to_ch: b.to_ch,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{idx}/steps/wait", params(("idx" = usize, Path, description = "Sequence index")), responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn add_wait_step(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddWaitStep { seq_idx: idx })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddGoToStepBody {
    /// Index of the step to jump to.
    pub step_index: usize,
}
#[utoipa::path(post, path = "/api/sequences/{idx}/steps/goto", params(("idx" = usize, Path, description = "Sequence index")), request_body = AddGoToStepBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn add_goto_step(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<AddGoToStepBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddGoToStep {
            seq_idx: idx,
            step_index: b.step_index,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(delete, path = "/api/sequences/{seq_idx}/steps/{step_idx}", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index within the sequence")), responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn remove_step(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RemoveStep { seq_idx, step_idx })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct StepDurationBody {
    /// Numeric duration value.
    pub value: f64,
    /// Unit of the duration (seconds or beats).
    pub unit: crate::channel::DurationUnit,
}
#[utoipa::path(put, path = "/api/sequences/{seq_idx}/steps/{step_idx}/duration", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index")), request_body = StepDurationBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn set_step_duration(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
    Json(b): Json<StepDurationBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepDuration {
            seq_idx,
            step_idx,
            value: b.value,
            unit: b.unit,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct StepEasingBody {
    /// Name of the easing curve.
    pub easing: String,
}
#[utoipa::path(put, path = "/api/sequences/{seq_idx}/steps/{step_idx}/easing", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index")), request_body = StepEasingBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn set_step_easing(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
    Json(b): Json<StepEasingBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepEasing {
            seq_idx,
            step_idx,
            easing: b.easing,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct StepShaderBody {
    /// Shader name for the transition, or null to clear.
    pub shader_name: Option<String>,
}
#[utoipa::path(put, path = "/api/sequences/{seq_idx}/steps/{step_idx}/shader", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index")), request_body = StepShaderBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn set_step_shader(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
    Json(b): Json<StepShaderBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepTransitionShader {
            seq_idx,
            step_idx,
            shader_name: b.shader_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Missing Parity Routes ─────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct StepChBody {
    /// Channel index.
    pub ch: usize,
}
#[derive(Deserialize, ToSchema)]
pub struct GoToTargetBody {
    /// Index of the step to jump to.
    pub target: usize,
}
#[derive(Deserialize, ToSchema)]
pub struct MoveStepBody {
    /// Current position of the step.
    pub from: usize,
    /// New position for the step.
    pub to: usize,
}

#[utoipa::path(put, path = "/api/sequences/{seq_idx}/steps/{step_idx}/from-ch", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index")), request_body = StepChBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn set_step_from_ch(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
    Json(b): Json<StepChBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepFromCh {
            seq_idx,
            step_idx,
            ch: b.ch,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/sequences/{seq_idx}/steps/{step_idx}/to-ch", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index")), request_body = StepChBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn set_step_to_ch(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
    Json(b): Json<StepChBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepToCh {
            seq_idx,
            step_idx,
            ch: b.ch,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/sequences/{seq_idx}/steps/{step_idx}/goto-target", params(("seq_idx" = usize, Path, description = "Sequence index"), ("step_idx" = usize, Path, description = "Step index")), request_body = GoToTargetBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn set_goto_target(
    State(s): State<SharedState>,
    Path((seq_idx, step_idx)): Path<(usize, usize)>,
    Json(b): Json<GoToTargetBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetGoToTarget {
            seq_idx,
            step_idx,
            target: b.target,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{idx}/steps/move", params(("idx" = usize, Path, description = "Sequence index")), request_body = MoveStepBody, responses((status = 200, body = CommandResult)), tag = "Sequences")]
pub async fn move_step(
    State(s): State<SharedState>,
    Path(idx): Path<usize>,
    Json(b): Json<MoveStepBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MoveStep {
            seq_idx: idx,
            from: b.from,
            to: b.to,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
