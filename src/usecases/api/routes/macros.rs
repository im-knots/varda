//! Macro control write routes — full parity with the GUI macro strip.
//!
//! Config mutations (add/remove/rename/kind/target edits/button config) are
//! undoable via the scene snapshot. `PUT /api/macros/{uuid}/value` is a live
//! performance turn that fans out to targets and is intentionally not undoable
//! (mirrors crossfader/opacity/MIDI live control). Macros are also reachable
//! through the shared parameter router as `macro/<uuid>/value` via
//! `PUT /api/params`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::macros::{ButtonBehavior, MacroCurve, MacroKind, TriggerAction};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct AddMacroBody {
    /// Control kind: `Knob`, `Fader`, or `Button`.
    pub kind: MacroKind,
}

#[derive(Deserialize, ToSchema)]
pub struct RenameMacroBody {
    /// New display name.
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct MacroKindBody {
    /// New control kind.
    pub kind: MacroKind,
}

#[derive(Deserialize, ToSchema)]
pub struct MacroValueBody {
    /// Normalized value 0.0–1.0 (knob/fader position, or button 0=off/1=on).
    pub value: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct AddMacroTargetBody {
    /// Parameter-router path to drive, e.g. `deck/<uuid>/effect/<uuid>/param/scale`.
    /// May not be a `macro/*` path (loop prevention).
    pub path: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateMacroTargetBody {
    /// Lower bound of the target's normalized sub-range (0.0–1.0).
    pub min: f32,
    /// Upper bound of the target's normalized sub-range (0.0–1.0). `min > max` inverts.
    pub max: f32,
    /// Response curve applied before mapping into `[min, max]`.
    pub curve: MacroCurve,
    /// Flip the response (equivalent to swapping min/max).
    pub invert: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct ButtonBehaviorBody {
    /// Button response: `Momentary`, `Toggle`, or `Trigger`.
    pub behavior: ButtonBehavior,
}

#[derive(Deserialize, ToSchema)]
pub struct TriggersBody {
    /// Discrete actions fired on a Trigger button's rising edge.
    pub triggers: Vec<TriggerAction>,
}

#[derive(Deserialize, ToSchema)]
pub struct MacroModulationBody {
    /// UUID of the modulation source (LFO/ADSR/etc.) that drives this macro's value.
    pub source_id: String,
    /// Modulation depth (0.0–1.0), added as an offset to the macro's manual set point.
    pub amount: f32,
}

#[utoipa::path(post, path = "/api/macros", request_body = AddMacroBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn add_macro(
    State(state): State<SharedState>,
    Json(body): Json<AddMacroBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddMacro { kind: body.kind })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/macros/{uuid}", params(("uuid" = String, Path, description = "Macro UUID")), responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn remove_macro(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveMacro { uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/macros/{uuid}/name", params(("uuid" = String, Path, description = "Macro UUID")), request_body = RenameMacroBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn rename_macro(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<RenameMacroBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RenameMacro {
            uuid,
            name: body.name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/macros/{uuid}/kind", params(("uuid" = String, Path, description = "Macro UUID")), request_body = MacroKindBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn set_kind(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<MacroKindBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetMacroKind {
            uuid,
            kind: body.kind,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/macros/{uuid}/value", params(("uuid" = String, Path, description = "Macro UUID")), request_body = MacroValueBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn set_value(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<MacroValueBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetMacroValue {
            uuid,
            value: body.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/macros/{uuid}/targets", params(("uuid" = String, Path, description = "Macro UUID")), request_body = AddMacroTargetBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn add_target(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<AddMacroTargetBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddMacroTarget {
            uuid,
            path: body.path,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/macros/{uuid}/targets/{target_idx}", params(("uuid" = String, Path, description = "Macro UUID"), ("target_idx" = usize, Path, description = "Zero-based target index")), responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn remove_target(
    State(state): State<SharedState>,
    Path((uuid, target_idx)): Path<(String, usize)>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveMacroTarget { uuid, target_idx })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/macros/{uuid}/targets/{target_idx}", params(("uuid" = String, Path, description = "Macro UUID"), ("target_idx" = usize, Path, description = "Zero-based target index")), request_body = UpdateMacroTargetBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn update_target(
    State(state): State<SharedState>,
    Path((uuid, target_idx)): Path<(String, usize)>,
    Json(body): Json<UpdateMacroTargetBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::UpdateMacroTarget {
            uuid,
            target_idx,
            min: body.min,
            max: body.max,
            curve: body.curve,
            invert: body.invert,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Drive a Knob/Fader macro's value from a modulation source. The source adds a
/// signed offset to the macro's manual set point each frame, re-fanning to all
/// targets. Mirrors the modulation control in the macro detail panel.
#[utoipa::path(put, path = "/api/macros/{uuid}/modulation", params(("uuid" = String, Path, description = "Macro UUID")), request_body = MacroModulationBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn assign_modulation(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<MacroModulationBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AssignModulation {
            target: crate::macros::Macro::value_mod_key(&uuid),
            source_id: body.source_id,
            amount: body.amount,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Remove all modulation driving this macro's value.
#[utoipa::path(delete, path = "/api/macros/{uuid}/modulation", params(("uuid" = String, Path, description = "Macro UUID")), responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn clear_modulation(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ClearModulation {
            target: crate::macros::Macro::value_mod_key(&uuid),
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Remove only one modulation source from this macro's value, leaving any other
/// sources intact. Mirrors the per-assignment delete in the macro detail panel.
#[utoipa::path(delete, path = "/api/macros/{uuid}/modulation/{source_id}", params(("uuid" = String, Path, description = "Macro UUID"), ("source_id" = String, Path, description = "Modulation source UUID")), responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn clear_modulation_source(
    State(state): State<SharedState>,
    Path((uuid, source_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ClearModulationSource {
            target: crate::macros::Macro::value_mod_key(&uuid),
            source_id,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/macros/{uuid}/button/behavior", params(("uuid" = String, Path, description = "Macro UUID")), request_body = ButtonBehaviorBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn set_button_behavior(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<ButtonBehaviorBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetMacroButtonBehavior {
            uuid,
            behavior: body.behavior,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/macros/{uuid}/button/triggers", params(("uuid" = String, Path, description = "Macro UUID")), request_body = TriggersBody, responses((status = 200, body = CommandResult)), tag = "Macros")]
pub async fn set_triggers(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(body): Json<TriggersBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetMacroTriggers {
            uuid,
            actions: body.triggers,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
