//! Timecode receiver configuration.
//!
//! What the incoming position *does* is on `/api/transport/*`; these routes only
//! say which signal to listen to. Reading the resolved state is
//! `GET /api/state/timecode`. See `/spec/timecode.md`.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

#[derive(Deserialize, ToSchema)]
pub struct PreferenceBody {
    /// `Auto` (LTC if patched and arriving, else MTC), `ForceLtc`,
    /// `{"ForceMtc": {"device_id": 2}}`, or `Off`.
    pub preference: crate::timecode::TimecodePreference,
}

#[derive(Deserialize, ToSchema)]
pub struct LtcInputBody {
    /// Audio input carrying LTC, and the channel of it. `null` stops listening,
    /// which also releases the device.
    pub input: Option<crate::timecode::LtcInput>,
}

/// Choose which incoming timecode signal the transport follows.
#[utoipa::path(put, path = "/api/timecode/preference", request_body = PreferenceBody, responses((status = 200, body = CommandResult)), tag = "Timecode")]
pub async fn set_preference(
    State(s): State<SharedState>,
    Json(b): Json<PreferenceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetTimecodePreference {
            preference: b.preference,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

/// Name the audio input carrying LTC, or stop listening for it.
///
/// `rate` is optional and normally left unset: the decoder infers the rate from
/// the signal's cadence. Naming it matters only for 29.97 non-drop, which is
/// indistinguishable from 30 in the signal and 3.6 seconds an hour away from it
/// in position.
#[utoipa::path(put, path = "/api/timecode/ltc-input", request_body = LtcInputBody, responses((status = 200, body = CommandResult)), tag = "Timecode")]
pub async fn set_ltc_input(
    State(s): State<SharedState>,
    Json(b): Json<LtcInputBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetLtcInput { input: b.input })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
