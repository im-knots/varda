//! Stage state routes: GET /api/stage/* and POST /api/stage/detect/*

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::internal::surface::detect::{DetectedContour, DetectionParams};
use crate::usecases::api::{SharedState, command_response};
use crate::usecases::api::projection::{self, StateReadError};

fn read_or_error(state: &SharedState) -> Result<crate::engine::EngineState, (StatusCode, &'static str)> {
    projection::read_state(&state.engine_state).map_err(|e| match e {
        StateReadError::NotInitialized => (StatusCode::SERVICE_UNAVAILABLE, "Engine not yet initialized"),
        StateReadError::LockPoisoned => (StatusCode::INTERNAL_SERVER_ERROR, "State lock poisoned"),
    })
}

/// GET /api/stage — full stage state
pub async fn stage(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(projection::project_stage(&s)).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/surfaces
pub async fn surfaces(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.outputs.surfaces).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/surfaces/:uuid
pub async fn surface_by_uuid(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_surface(&s, &uuid) {
            Some(surface) => Json(surface).into_response(),
            None => (StatusCode::NOT_FOUND, "Surface not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/outputs
pub async fn outputs(State(state): State<SharedState>) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => Json(&s.outputs.windows).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

/// GET /api/stage/outputs/:uuid
pub async fn output_by_uuid(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match read_or_error(&state) {
        Ok(s) => match projection::find_output(&s, &uuid) {
            Some(output) => Json(output).into_response(),
            None => (StatusCode::NOT_FOUND, "Output not found").into_response(),
        },
        Err((status, msg)) => (status, msg).into_response(),
    }
}


// ── Detection Routes ────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct DetectImageBody {
    /// Base64-encoded image data (PNG/JPG).
    pub image_data: String,
    /// Detection parameters (optional, uses defaults if omitted).
    #[serde(default)]
    pub params: DetectionParams,
}

#[derive(Deserialize, ToSchema)]
pub struct DetectSvgBody {
    /// Raw SVG content as a string.
    pub svg_data: String,
}

#[derive(Deserialize, ToSchema)]
pub struct DetectDxfBody {
    /// Raw DXF content as a string.
    pub dxf_data: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ConfirmContoursBody {
    /// Contours to create as surfaces.
    pub contours: Vec<DetectedContour>,
}

/// POST /api/stage/detect/image — detect contours from a raster image.
#[utoipa::path(post, path = "/api/stage/detect/image", request_body = DetectImageBody, responses((status = 200, body = CommandResult)), tag = "Stage")]
pub async fn detect_image(
    State(s): State<SharedState>,
    Json(b): Json<DetectImageBody>,
) -> impl IntoResponse {
    use base64::Engine;
    let image_data = match base64::engine::general_purpose::STANDARD.decode(&b.image_data) {
        Ok(data) => data,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid base64: {e}")).into_response(),
    };
    match s
        .send_command(EngineCommand::DetectFromImage {
            image_data,
            params: b.params,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// POST /api/stage/detect/svg — detect contours from SVG data.
#[utoipa::path(post, path = "/api/stage/detect/svg", request_body = DetectSvgBody, responses((status = 200, body = CommandResult)), tag = "Stage")]
pub async fn detect_svg(
    State(s): State<SharedState>,
    Json(b): Json<DetectSvgBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::DetectFromSvg {
            svg_data: b.svg_data.into_bytes(),
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// POST /api/stage/detect/dxf — detect contours from DXF data.
#[utoipa::path(post, path = "/api/stage/detect/dxf", request_body = DetectDxfBody, responses((status = 200, body = CommandResult)), tag = "Stage")]
pub async fn detect_dxf(
    State(s): State<SharedState>,
    Json(b): Json<DetectDxfBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::DetectFromDxf {
            dxf_data: b.dxf_data.into_bytes(),
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// POST /api/stage/detect/confirm — create surfaces from detected contours.
#[utoipa::path(post, path = "/api/stage/detect/confirm", request_body = ConfirmContoursBody, responses((status = 200, body = CommandResult)), tag = "Stage")]
pub async fn detect_confirm(
    State(s): State<SharedState>,
    Json(b): Json<ConfirmContoursBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ConfirmDetectedContours {
            contours: b.contours,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DetectCameraBody {
    /// Camera ID to capture a frame from.
    pub camera_id: u32,
    /// Detection parameters (optional, uses defaults if omitted).
    #[serde(default)]
    pub params: DetectionParams,
}

/// POST /api/stage/detect/camera — detect contours from a camera snapshot.
#[utoipa::path(post, path = "/api/stage/detect/camera", request_body = DetectCameraBody, responses((status = 200, body = CommandResult)), tag = "Stage")]
pub async fn detect_camera(
    State(s): State<SharedState>,
    Json(b): Json<DetectCameraBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::DetectFromCamera {
            camera_id: b.camera_id,
            params: b.params,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}