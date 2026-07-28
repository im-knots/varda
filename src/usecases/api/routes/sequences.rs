//! Transition sequence write routes.
//!
//! Sequences are addressed by UUID. Steps are positional within their sequence,
//! so `step_idx` stays an ordinal — see `/spec/api-addressing.md`.

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
#[utoipa::path(delete, path = "/api/sequences/{sequence_uuid}", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn delete(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::DeleteSequence { sequence_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/play", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn play(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::PlaySequence { sequence_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/stop", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn stop(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::StopSequence { sequence_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/toggle", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn toggle(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ToggleSequence { sequence_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddFadeStepBody {
    /// UUID of the channel to fade from.
    pub from_channel_uuid: String,
    /// UUID of the channel to fade to.
    pub to_channel_uuid: String,
}
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/steps/fade", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), request_body = AddFadeStepBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence or channel not found")), tag = "Sequences")]
pub async fn add_fade_step(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
    Json(b): Json<AddFadeStepBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddFadeStep {
            sequence_uuid,
            from_channel_uuid: b.from_channel_uuid,
            to_channel_uuid: b.to_channel_uuid,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/steps/wait", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn add_wait_step(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddWaitStep { sequence_uuid })
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
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/steps/goto", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), request_body = AddGoToStepBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn add_goto_step(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
    Json(b): Json<AddGoToStepBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddGoToStep {
            sequence_uuid,
            step_index: b.step_index,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(delete, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), responses((status = 200, body = CommandResult), (status = 404, description = "Sequence or step not found")), tag = "Sequences")]
pub async fn remove_step(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RemoveStep {
            sequence_uuid,
            step_idx,
        })
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
#[utoipa::path(put, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}/duration", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), request_body = StepDurationBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence or step not found")), tag = "Sequences")]
pub async fn set_step_duration(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
    Json(b): Json<StepDurationBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepDuration {
            sequence_uuid,
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
#[utoipa::path(put, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}/easing", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), request_body = StepEasingBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence or step not found")), tag = "Sequences")]
pub async fn set_step_easing(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
    Json(b): Json<StepEasingBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepEasing {
            sequence_uuid,
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
#[utoipa::path(put, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}/shader", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), request_body = StepShaderBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence or step not found")), tag = "Sequences")]
pub async fn set_step_shader(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
    Json(b): Json<StepShaderBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepTransitionShader {
            sequence_uuid,
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
    /// UUID of the channel the step references.
    pub channel_uuid: String,
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

#[utoipa::path(put, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}/from-ch", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), request_body = StepChBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence, step, or channel not found")), tag = "Sequences")]
pub async fn set_step_from_ch(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
    Json(b): Json<StepChBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepFromCh {
            sequence_uuid,
            step_idx,
            channel_uuid: b.channel_uuid,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}/to-ch", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), request_body = StepChBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence, step, or channel not found")), tag = "Sequences")]
pub async fn set_step_to_ch(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
    Json(b): Json<StepChBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepToCh {
            sequence_uuid,
            step_idx,
            channel_uuid: b.channel_uuid,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/sequences/{sequence_uuid}/steps/{step_idx}/goto-target", params(("sequence_uuid" = String, Path, description = "Sequence UUID"), ("step_idx" = usize, Path, description = "Step position within the sequence")), request_body = GoToTargetBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence or step not found")), tag = "Sequences")]
pub async fn set_goto_target(
    State(s): State<SharedState>,
    Path((sequence_uuid, step_idx)): Path<(String, usize)>,
    Json(b): Json<GoToTargetBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetGoToTarget {
            sequence_uuid,
            step_idx,
            target: b.target,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/sequences/{sequence_uuid}/steps/move", params(("sequence_uuid" = String, Path, description = "Sequence UUID")), request_body = MoveStepBody, responses((status = 200, body = CommandResult), (status = 404, description = "Sequence not found")), tag = "Sequences")]
pub async fn move_step(
    State(s): State<SharedState>,
    Path(sequence_uuid): Path<String>,
    Json(b): Json<MoveStepBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MoveStep {
            sequence_uuid,
            from: b.from,
            to: b.to,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
