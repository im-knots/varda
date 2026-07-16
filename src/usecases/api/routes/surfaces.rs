//! Surface management write routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::internal::renderer::context::OutputSource;
use crate::internal::surface::{
    ContentMapping, CubicHandle, SurfaceOutputType, SurfacePath, SurfaceReorderOp,
};
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
pub struct RotateBody {
    /// Rotation in radians (clockwise in canvas space, y-down).
    pub angle: f32,
    /// Pivot point [x, y] in normalised coordinates.
    pub pivot: [f32; 2],
}
#[derive(Deserialize, ToSchema)]
pub struct ScaleBody {
    /// Horizontal scale factor.
    pub sx: f32,
    /// Vertical scale factor.
    pub sy: f32,
    /// Pivot point [x, y] in normalised coordinates.
    pub pivot: [f32; 2],
}
#[derive(Deserialize, ToSchema)]
pub struct ContourVerticesBody {
    /// Contour index within the surface.
    pub contour: usize,
    /// Updated vertex positions as [x, y] pairs.
    pub vertices: Vec<[f32; 2]>,
}
#[derive(Deserialize, ToSchema)]
pub struct ConvertEdgeBody {
    /// Index of the curve-path edge to convert.
    pub edge_idx: usize,
    /// `true` converts the edge to a cubic bezier; `false` back to a line.
    pub to_cubic: bool,
}
#[derive(Deserialize, ToSchema)]
pub struct MovePathAnchorBody {
    /// Index of the curve-path anchor to move.
    pub anchor_idx: usize,
    /// New anchor position [x, y] in normalised coordinates.
    pub pos: [f32; 2],
}
#[derive(Deserialize, ToSchema)]
pub struct MovePathHandleBody {
    /// Index of the cubic segment whose control handle is being moved.
    pub segment_idx: usize,
    /// Which control handle of the cubic segment (C1 or C2).
    pub handle: CubicHandle,
    /// New handle position [x, y] in normalised coordinates.
    pub pos: [f32; 2],
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
#[utoipa::path(put, path = "/api/surfaces/{uuid}/rotate", params(("uuid" = String, Path, description = "Surface UUID")), request_body = RotateBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn rotate_surface(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<RotateBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RotateSurface {
            uuid,
            angle: b.angle,
            pivot: b.pivot,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/surfaces/{uuid}/scale", params(("uuid" = String, Path, description = "Surface UUID")), request_body = ScaleBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn scale_surface(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<ScaleBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ScaleSurface {
            uuid,
            sx: b.sx,
            sy: b.sy,
            pivot: b.pivot,
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

#[utoipa::path(put, path = "/api/surfaces/{uuid}/edge/convert", params(("uuid" = String, Path, description = "Surface UUID")), request_body = ConvertEdgeBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn convert_edge(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<ConvertEdgeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ConvertSurfaceEdge {
            uuid,
            edge_idx: b.edge_idx,
            to_cubic: b.to_cubic,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/path/anchor", params(("uuid" = String, Path, description = "Surface UUID")), request_body = MovePathAnchorBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn move_path_anchor(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<MovePathAnchorBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MovePathAnchor {
            uuid,
            anchor_idx: b.anchor_idx,
            pos: b.pos,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/path/handle", params(("uuid" = String, Path, description = "Surface UUID")), request_body = MovePathHandleBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn move_path_handle(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<MovePathHandleBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MovePathHandle {
            uuid,
            segment_idx: b.segment_idx,
            handle: b.handle,
            pos: b.pos,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Per-surface warp (8i.5) ───────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SetWarpCornerBody {
    /// Corner index (0–3, TL/TR/BR/BL).
    pub corner_idx: usize,
    /// New [x, y] position for the corner in normalised coordinates.
    pub position: [f32; 2],
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/warp/corner", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetWarpCornerBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_warp_corner(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetWarpCornerBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetWarpCorner {
            surface_uuid: uuid,
            corner_idx: b.corner_idx,
            position: b.position,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/surfaces/{uuid}/warp/reset", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn reset_warp(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ResetWarp { surface_uuid: uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SetWarpSubdivisionsBody {
    /// Number of grid columns (clamped to [2, 64]).
    pub cols: u32,
    /// Number of grid rows (clamped to [2, 64]).
    pub rows: u32,
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/warp/subdivisions", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetWarpSubdivisionsBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_warp_subdivisions(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetWarpSubdivisionsBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetWarpSubdivisions {
            surface_uuid: uuid,
            cols: b.cols,
            rows: b.rows,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SetWarpMeshPointBody {
    /// Grid row of the point (0-based).
    pub row: usize,
    /// Grid column of the point (0-based).
    pub col: usize,
    /// New [x, y] position in normalised coordinates.
    pub position: [f32; 2],
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/warp/mesh-point", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetWarpMeshPointBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_warp_mesh_point(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetWarpMeshPointBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetWarpMeshPoint {
            surface_uuid: uuid,
            row: b.row,
            col: b.col,
            position: b.position,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SetWarpBoundBody {
    /// `true` = auto-conform the warp to the surface shape; `false` = unbind for
    /// manual fine-tuning (materialises the conforming warp).
    pub bound: bool,
}

#[utoipa::path(post, path = "/api/surfaces/{uuid}/warp/bind", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetWarpBoundBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_warp_bound(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetWarpBoundBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetWarpBound {
            surface_uuid: uuid,
            bound: b.bound,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/surfaces/{uuid}/warp/bezier", params(("uuid" = String, Path, description = "Surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn convert_warp_to_bezier(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ConvertWarpToBezier { surface_uuid: uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct MoveWarpAnchorBody {
    /// Anchor grid row (0-based).
    pub row: usize,
    /// Anchor grid column (0-based).
    pub col: usize,
    /// New [x, y] position in normalised coordinates.
    pub position: [f32; 2],
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/warp/anchor", params(("uuid" = String, Path, description = "Surface UUID")), request_body = MoveWarpAnchorBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn move_warp_anchor(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<MoveWarpAnchorBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MoveWarpAnchor {
            surface_uuid: uuid,
            row: b.row,
            col: b.col,
            position: b.position,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct MoveWarpHandleBody {
    /// `true` = a horizontal edge ((r,c)→(r,c+1)); `false` = a vertical edge
    /// ((r,c)→(r+1,c)).
    pub horizontal: bool,
    /// Edge start-anchor grid row (0-based).
    pub row: usize,
    /// Edge start-anchor grid column (0-based).
    pub col: usize,
    /// Which handle of the edge: 0 (near start anchor) or 1 (near end anchor).
    pub which: usize,
    /// New [x, y] position in normalised coordinates.
    pub position: [f32; 2],
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/warp/handle", params(("uuid" = String, Path, description = "Surface UUID")), request_body = MoveWarpHandleBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn move_warp_handle(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<MoveWarpHandleBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::MoveWarpHandle {
            surface_uuid: uuid,
            horizontal: b.horizontal,
            row: b.row,
            col: b.col,
            which: b.which,
            position: b.position,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SetBezierCageBody {
    /// Number of anchor columns (clamped to [2, 64]).
    pub cols: u32,
    /// Number of anchor rows (clamped to [2, 64]).
    pub rows: u32,
}

#[utoipa::path(put, path = "/api/surfaces/{uuid}/warp/cage", params(("uuid" = String, Path, description = "Surface UUID")), request_body = SetBezierCageBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn set_bezier_cage_subdivisions(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<SetBezierCageBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetBezierCageSubdivisions {
            surface_uuid: uuid,
            cols: b.cols,
            rows: b.rows,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Subtractive holes (8i.7) ──────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AddHoleBody {
    /// Closed curve path defining the cut-out, in normalised canvas coordinates.
    pub hole: SurfacePath,
}

#[utoipa::path(post, path = "/api/surfaces/{uuid}/holes", params(("uuid" = String, Path, description = "Surface UUID")), request_body = AddHoleBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn add_hole(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<AddHoleBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddSurfaceHole { uuid, hole: b.hole })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/surfaces/{uuid}/holes/{index}", params(("uuid" = String, Path, description = "Surface UUID"), ("index" = usize, Path, description = "Hole index")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn remove_hole(
    State(s): State<SharedState>,
    Path((uuid, index)): Path<(String, usize)>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RemoveSurfaceHole {
            uuid,
            hole_index: index,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

/// "Make Hole" (8i.7): convert the surface identified by `uuid` into a cut-out
/// hole in the topmost other surface under its centroid, consuming the source.
#[utoipa::path(post, path = "/api/surfaces/{uuid}/punch", params(("uuid" = String, Path, description = "Source surface UUID")), responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn punch_hole(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::PunchSurfaceHole { source_uuid: uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderBody {
    /// The stacking-order move to apply.
    pub op: SurfaceReorderOp,
}

/// Change a surface's global stacking order (8i.12): move it front/back/up/down
/// within the authoritative surface order (index 0 = bottom, last = top).
#[utoipa::path(post, path = "/api/surfaces/{uuid}/reorder", params(("uuid" = String, Path, description = "Surface UUID")), request_body = ReorderBody, responses((status = 200, body = CommandResult)), tag = "Surfaces")]
pub async fn reorder(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<ReorderBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ReorderSurface { uuid, op: b.op })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
