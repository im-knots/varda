//! Surface management write routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::internal::renderer::context::OutputSource;
use crate::internal::surface::{ContentMapping, SurfaceOutputType};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct AddRectSurfaceBody {
    /// Display name for the surface.
    pub name: String,
    /// Content source for the surface.
    pub source: OutputSource,
}

#[derive(Deserialize, ToSchema)]
pub struct AddPolygonSurfaceBody {
    /// Display name for the surface.
    pub name: String,
    /// Vertex positions as [x, y] pairs in normalised coordinates.
    pub vertices: Vec<[f32; 2]>,
    /// Content source for the surface.
    pub source: OutputSource,
}

#[derive(Deserialize, ToSchema)]
pub struct AddCircleSurfaceBody {
    /// Display name for the surface.
    pub name: String,
    /// Centre position as [x, y] in normalised coordinates.
    pub center: [f32; 2],
    /// Circle radius in normalised units.
    pub radius: f32,
    /// Number of polygon sides used to approximate the circle.
    pub sides: u32,
    /// Width-to-height ratio (defaults to 1.0).
    #[serde(default = "default_aspect")]
    pub aspect_ratio: f32,
    /// Content source for the surface.
    pub source: OutputSource,
}

fn default_aspect() -> f32 {
    1.0
}

#[derive(Deserialize, ToSchema)]
pub struct SetSourceBody {
    /// Content source for the surface.
    pub source: OutputSource,
}

#[derive(Deserialize, ToSchema)]
pub struct SetOutputTypeBody {
    /// Output type for the surface.
    pub output_type: SurfaceOutputType,
}

#[derive(Deserialize, ToSchema)]
pub struct SetContentMappingBody {
    /// Content mapping mode.
    pub mapping: ContentMapping,
}

#[derive(Deserialize, ToSchema)]
pub struct RenameBody {
    /// New display name for the surface.
    pub name: String,
}

#[utoipa::path(post, path = "/api/surfaces/rect", request_body = AddRectSurfaceBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn add_rect(
    State(s): State<SharedState>,
    Json(b): Json<AddRectSurfaceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddSurface {
            name: b.name,
            source: b.source,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/surfaces/polygon", request_body = AddPolygonSurfaceBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn add_polygon(
    State(s): State<SharedState>,
    Json(b): Json<AddPolygonSurfaceBody>,
) -> impl IntoResponse {
    const MAX_VERTICES: usize = 10_000;
    if b.vertices.len() > MAX_VERTICES {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!(
                "Too many vertices: {} (max {MAX_VERTICES})",
                b.vertices.len()
            ),
        )
            .into_response();
    }
    match s
        .send_command(EngineCommand::AddPolygonSurface {
            name: b.name,
            vertices: b.vertices,
            source: b.source,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/surfaces/circle", request_body = AddCircleSurfaceBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn add_circle(
    State(s): State<SharedState>,
    Json(b): Json<AddCircleSurfaceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddCircleSurface {
            name: b.name,
            center: b.center,
            radius: b.radius,
            sides: b.sides,
            aspect_ratio: b.aspect_ratio,
            source: b.source,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/surfaces/{uuid}", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn remove(State(s): State<SharedState>, Path(uuid): Path<String>) -> impl IntoResponse {
    match s.send_command(EngineCommand::RemoveSurface { uuid }).await {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/source", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetSourceBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_source(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetSurfaceSource {
            uuid,
            source: b.source,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/output-type", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetOutputTypeBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_output_type(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetOutputTypeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetSurfaceOutputType {
            uuid,
            output_type: b.output_type,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/content-mapping", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetContentMappingBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_content_mapping(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetContentMappingBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetSurfaceContentMapping {
            uuid,
            mapping: b.mapping,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/name", params(("uuid" = String, Path, description = "Surface UUID")), request_body = RenameBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn rename(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<RenameBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RenameSurface { uuid, name: b.name })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VerticesBody {
    /// Vertex positions as [x, y] pairs in normalised coordinates.
    pub vertices: Vec<[f32; 2]>,
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/vertices", params(("uuid" = String, Path, description = "Surface UUID")), request_body = VerticesBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_vertices(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<VerticesBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateSurfaceVertices {
            uuid,
            vertices: b.vertices,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/surfaces/{uuid}/duplicate", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn duplicate(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::DuplicateSurface { uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/surfaces/{uuid}/flip-horizontal", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn flip_horizontal(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::FlipSurfaceHorizontal { uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/surfaces/{uuid}/flip-vertical", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn flip_vertical(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::FlipSurfaceVertical { uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Missing Parity Routes ─────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct InsertVertexBody {
    /// Index of the existing vertex after which to insert.
    pub after_vert_idx: usize,
    /// Position of the new vertex as [x, y].
    pub position: [f32; 2],
}
#[derive(Deserialize, ToSchema)]
pub struct RadiusBody {
    /// New radius for the circle surface.
    pub radius: f32,
}
#[derive(Deserialize, ToSchema)]
pub struct SidesBody {
    /// Number of polygon sides for the circle approximation.
    pub sides: u32,
}
#[derive(Deserialize, ToSchema)]
pub struct CombineBody {
    /// UUIDs of the surfaces to combine.
    pub uuids: Vec<String>,
}
#[derive(Deserialize, ToSchema)]
pub struct MoveBody {
    /// Horizontal offset in normalised coordinates.
    pub dx: f32,
    /// Vertical offset in normalised coordinates.
    pub dy: f32,
}
#[derive(Deserialize, ToSchema)]
pub struct ContourVerticesBody {
    /// Contour index within the surface.
    pub contour: usize,
    /// Updated vertex positions as [x, y] pairs.
    pub vertices: Vec<[f32; 2]>,
}

#[utoipa::path(post, path = "/api/surfaces/{uuid}/vertices/insert", params(("uuid" = String, Path, description = "Surface UUID")), request_body = InsertVertexBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn insert_vertex(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<InsertVertexBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::InsertSurfaceVertex {
            uuid,
            after_vert_idx: b.after_vert_idx,
            position: b.position,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/surfaces/{uuid}/circle/radius", params(("uuid" = String, Path, description = "Surface UUID")), request_body = RadiusBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_circle_radius(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<RadiusBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetCircleRadius {
            uuid,
            radius: b.radius,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/surfaces/{uuid}/circle/sides", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SidesBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_circle_sides(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SidesBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetCircleSides {
            uuid,
            sides: b.sides,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/surfaces/{uuid}/convert-to-polygon", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn convert_to_polygon(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ConvertSurfaceToPolygon { uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/surfaces/combine", request_body = CombineBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn combine(
    State(s): State<SharedState>,
    Json(b): Json<CombineBody>,
) -> impl IntoResponse {
    const MAX_UUIDS: usize = 256;
    if b.uuids.len() > MAX_UUIDS {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!("Too many UUIDs: {} (max {MAX_UUIDS})", b.uuids.len()),
        )
            .into_response();
    }
    match s
        .send_command(EngineCommand::CombineSurfaces { uuids: b.uuids })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/surfaces/{uuid}/move", params(("uuid" = String, Path, description = "Surface UUID")), request_body = MoveBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn move_surface(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<MoveBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MoveSurface {
            uuid,
            dx: b.dx,
            dy: b.dy,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/surfaces/{uuid}/contour-vertices", params(("uuid" = String, Path, description = "Surface UUID")), request_body = ContourVerticesBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn update_contour_vertices(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<ContourVerticesBody>,
) -> impl IntoResponse {
    const MAX_VERTICES: usize = 10_000;
    if b.vertices.len() > MAX_VERTICES {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!(
                "Too many vertices: {} (max {MAX_VERTICES})",
                b.vertices.len()
            ),
        )
            .into_response();
    }
    match s
        .send_command(EngineCommand::UpdateSurfaceContourVertices {
            uuid,
            contour: b.contour,
            vertices: b.vertices,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
