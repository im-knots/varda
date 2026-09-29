//! Stage editor state kept in egui memory across frames (tool, selection,
//! drags), plus the hit-test result aliases.

use super::gizmo::{RotateDrag, ScaleDrag};
use crate::surface::CubicHandle;

/// Edge drag: (`surface_uuid`, `contour_idx`, `edge_start_idx`, `original_v0`,
/// `original_v1`, `grab_point_on_edge`).
pub(super) type DraggingEdge = (String, usize, usize, [f32; 2], [f32; 2], [f32; 2]);

/// Hit-test result for a vertex: (`surface_uuid`, `contour_idx`, `vertex_idx`)
pub(super) type HitVertex = (String, usize, usize);
/// Hit-test result for an edge: (`surface_uuid`, `contour_idx`, `edge_start_idx`, `projected_point`)
pub(super) type HitEdge = (String, usize, usize, [f32; 2]);
/// Hit-test result for a surface body: (`surface_uuid`, nx, ny)
pub(super) type HitSurface = (String, f32, f32);
/// Combined hit-test result: (vertex, edge, surface)
pub(super) type HitTestResult = (Option<HitVertex>, Option<HitEdge>, Option<HitSurface>);

/// Stage editor mode: 2D polygon editing or 3D dome mode.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) enum StageEditorMode {
    #[default]
    Polygon2D,
    Dome3D,
}

/// Stage editor drawing tool.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) enum DrawingTool {
    #[default]
    Select,
    Rectangle,
    Polygon,
    Circle,
    Bezier,
}

/// In-progress drawing and drag state.
#[derive(Debug, Clone, Default)]
pub(super) struct StageEditorState {
    pub(super) tool: DrawingTool,
    /// Rectangle tool: drag start.
    pub(super) rect_start: Option<[f32; 2]>,
    /// Polygon tool: vertices placed so far.
    pub(super) polygon_verts: Vec<[f32; 2]>,
    /// Circle tool: center.
    pub(super) circle_center: Option<[f32; 2]>,
    /// Sides of the circle's N-gon approximation.
    pub(super) circle_sides: u32,
    /// Selected surface UUIDs.
    pub(super) selected_surfaces: std::collections::BTreeSet<String>,
    /// Vertex drag in select mode.
    pub(super) dragging_vertex: Option<(String, usize, usize)>, // (surface_uuid, contour_idx, vertex_idx)
    /// Whole-surface move in select mode.
    pub(super) moving_surface: Option<(String, f32, f32)>, // (surface_uuid, last_x, last_y)
    /// Marquee selection start, in normalized coordinates.
    pub(super) selection_rect_start: Option<[f32; 2]>,
    /// Radius handle drag on a circle surface.
    pub(super) dragging_radius: Option<String>, // surface_uuid
    /// Edge drag.
    pub(super) dragging_edge: Option<DraggingEdge>,
    /// Drag state for the transform gizmo's scale handles.
    pub(super) dragging_scale: Option<ScaleDrag>,
    /// Drag state for the transform gizmo's rotation knob.
    pub(super) dragging_rotate: Option<RotateDrag>,
    /// Drag state for a curve anchor: (`surface_uuid`, `anchor_idx`)
    pub(super) dragging_anchor: Option<(String, usize)>,
    /// Drag state for a cubic control handle: (`surface_uuid`, `segment_idx`, handle)
    pub(super) dragging_handle: Option<(String, usize, CubicHandle)>,
}
