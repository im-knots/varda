/// Edge blend post-process pipeline — applies smoothstep alpha ramps
/// on output edges for seamless multi-projector overlap blending.
use anyhow::Result;
use wgpu::util::DeviceExt;

/// Controls whether edge blend config is user-set or auto-computed from surface topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum EdgeBlendMode {
    /// User sets each edge manually (default — preserves existing behavior).
    Manual,
    /// Blend config is auto-derived from overlapping surfaces across outputs.
    Auto,
}

impl Default for EdgeBlendMode {
    fn default() -> Self {
        Self::Manual
    }
}

/// Per-edge blend configuration.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, utoipa::ToSchema)]
pub struct EdgeBlendEdge {
    pub enabled: bool,
    /// Blend zone width as fraction of output dimension (0.0–0.5).
    pub width: f32,
    /// Gamma curve exponent for the blend ramp (typically 1.0–3.0).
    pub gamma: f32,
}

impl Default for EdgeBlendEdge {
    fn default() -> Self {
        Self { enabled: false, width: 0.1, gamma: 2.2 }
    }
}

/// Edge blending configuration for an output — four independent edges.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, utoipa::ToSchema)]
pub struct EdgeBlendConfig {
    pub left: EdgeBlendEdge,
    pub right: EdgeBlendEdge,
    pub top: EdgeBlendEdge,
    pub bottom: EdgeBlendEdge,
}

impl Default for EdgeBlendConfig {
    fn default() -> Self {
        Self {
            left: EdgeBlendEdge::default(),
            right: EdgeBlendEdge::default(),
            top: EdgeBlendEdge::default(),
            bottom: EdgeBlendEdge::default(),
        }
    }
}

impl EdgeBlendConfig {
    /// Returns true if any edge has blending enabled.
    pub fn any_enabled(&self) -> bool {
        self.left.enabled || self.right.enabled || self.top.enabled || self.bottom.enabled
    }
}

// ── Per-surface overlap zone blending (Auto mode) ────────────────────

/// Maximum number of overlap zones per surface.
/// Each zone adds 8 floats (32 bytes) to the GPU uniform.
pub const MAX_OVERLAP_ZONES: usize = 4;

/// A single overlap zone in surface-local UV space [0..1].
/// Defines a rectangle where this surface overlaps with a surface on another output.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct OverlapZone {
    /// Overlap rectangle in surface UV: [u_min, v_min, u_max, v_max].
    pub uv_rect: [f32; 4],
    /// Smoothstep gamma exponent for the blend ramp.
    pub gamma: f32,
    /// Horizontal ramp direction: +1.0 = fade toward u_max, -1.0 = fade toward u_min, 0.0 = none.
    pub ramp_x: f32,
    /// Vertical ramp direction: +1.0 = fade toward v_max, -1.0 = fade toward v_min, 0.0 = none.
    pub ramp_y: f32,
}

/// Per-surface overlap zones for Auto mode blending.
/// Up to `MAX_OVERLAP_ZONES` zones per surface, sorted by area descending.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SurfaceOverlapZones {
    pub zones: Vec<OverlapZone>,
}

impl SurfaceOverlapZones {
    /// Returns true if any overlap zones are present.
    pub fn any_enabled(&self) -> bool {
        !self.zones.is_empty()
    }

    /// Add a zone, keeping only the largest `MAX_OVERLAP_ZONES` by area.
    pub fn add_zone(&mut self, zone: OverlapZone) {
        self.zones.push(zone);
        if self.zones.len() > MAX_OVERLAP_ZONES {
            self.zones.sort_by(|a, b| {
                let area_a = (a.uv_rect[2] - a.uv_rect[0]) * (a.uv_rect[3] - a.uv_rect[1]);
                let area_b = (b.uv_rect[2] - b.uv_rect[0]) * (b.uv_rect[3] - b.uv_rect[1]);
                area_b.partial_cmp(&area_a).unwrap_or(std::cmp::Ordering::Equal)
            });
            self.zones.truncate(MAX_OVERLAP_ZONES);
        }
    }
}

/// Result of auto overlap-zone detection for a single surface.
#[derive(Debug, Clone)]
pub struct AutoBlendResult {
    /// Index of the output this surface belongs to.
    pub output_idx: usize,
    /// UUID of the surface.
    pub surface_uuid: String,
    /// Computed overlap zones for this surface.
    pub overlap_zones: SurfaceOverlapZones,
}

// ── Auto edge-blend detection ────────────────────────────────────────

/// A mapped region on the canvas belonging to a specific output.
/// Used as input to `compute_auto_edge_blend`.
#[derive(Debug, Clone)]
pub struct MappedRegion {
    /// Stringified OutputSource key (e.g. "Master", "Channel(0)").
    pub source_key: String,
    /// Axis-aligned bounding box [x, y, width, height] in normalized canvas coords.
    pub bbox: [f32; 4],
    /// UUID of the surface this region belongs to.
    pub surface_uuid: String,
    /// Primary contour vertices in canvas coords (for precise polygon intersection).
    pub vertices: Vec<[f32; 2]>,
    /// Additional contours for combined surfaces.
    pub extra_contours: Vec<Vec<[f32; 2]>>,
}

/// Describes one output's surface topology for auto edge-blend computation.
#[derive(Debug, Clone)]
pub struct OutputSurfaceInfo {
    /// Index into the outputs array.
    pub output_idx: usize,
    /// Current edge blend mode for this output.
    pub edge_blend_mode: EdgeBlendMode,
    /// Default gamma to apply when auto-computing blend edges.
    pub default_gamma: f32,
    /// All Mapped regions assigned to this output.
    pub regions: Vec<MappedRegion>,
}

/// Compute precise polygon intersection and return its AABB.
/// Falls back to AABB intersection when polygon data is unavailable or degenerate.
fn polygon_intersect_aabb(a: &MappedRegion, b: &MappedRegion) -> Option<[f32; 4]> {
    use crate::surface::verts_to_geo;
    use geo::BooleanOps;

    // Build geo polygons including extra contours.
    let build_multi = |r: &MappedRegion| -> Option<geo::MultiPolygon<f64>> {
        let mut polys = Vec::new();
        if let Some(p) = verts_to_geo(&r.vertices) {
            polys.push(p);
        }
        for contour in &r.extra_contours {
            if let Some(p) = verts_to_geo(contour) {
                polys.push(p);
            }
        }
        if polys.is_empty() { None } else { Some(geo::MultiPolygon::new(polys)) }
    };

    if let (Some(ma), Some(mb)) = (build_multi(a), build_multi(b)) {
        let inter = ma.intersection(&mb);
        if inter.0.is_empty() {
            return None;
        }
        use geo::BoundingRect;
        if let Some(rect) = inter.bounding_rect() {
            use geo::Coord;
            let min: Coord<f64> = rect.min();
            let max: Coord<f64> = rect.max();
            let x = min.x as f32;
            let y = min.y as f32;
            let w = (max.x - min.x) as f32;
            let h = (max.y - min.y) as f32;
            if w > 1e-6 && h > 1e-6 {
                return Some([x, y, w, h]);
            }
        }
        return None;
    }

    // Fallback: AABB intersection when vertices are missing.
    aabb_intersect(a.bbox, b.bbox)
}

/// Compute AABB intersection. Returns `Some([x, y, w, h])` if boxes overlap, `None` otherwise.
fn aabb_intersect(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let ax2 = a[0] + a[2];
    let ay2 = a[1] + a[3];
    let bx2 = b[0] + b[2];
    let by2 = b[1] + b[3];
    let ix = a[0].max(b[0]);
    let iy = a[1].max(b[1]);
    let ix2 = ax2.min(bx2);
    let iy2 = ay2.min(by2);
    let iw = ix2 - ix;
    let ih = iy2 - iy;
    if iw > 1e-6 && ih > 1e-6 {
        Some([ix, iy, iw, ih])
    } else {
        None
    }
}

/// Compute ramp direction for surface A's overlap zone toward surface B.
/// Returns (ramp_x, ramp_y) based on relative center positions.
fn compute_ramp_direction(bbox_a: &[f32; 4], bbox_b: &[f32; 4]) -> (f32, f32) {
    let center_a = [bbox_a[0] + bbox_a[2] * 0.5, bbox_a[1] + bbox_a[3] * 0.5];
    let center_b = [bbox_b[0] + bbox_b[2] * 0.5, bbox_b[1] + bbox_b[3] * 0.5];
    let dx = center_b[0] - center_a[0];
    let dy = center_b[1] - center_a[1];
    // Ramp toward the other surface: +1 if B is to the right/below, -1 if left/above.
    // Use a threshold to avoid tiny ramps from nearly-aligned centers.
    let ramp_x = if dx.abs() > 1e-4 { dx.signum() } else { 0.0 };
    let ramp_y = if dy.abs() > 1e-4 { dy.signum() } else { 0.0 };
    (ramp_x, ramp_y)
}

/// Derive per-surface overlap zones for each Auto-mode output from surface topology.
///
/// Algorithm:
/// 1. For each Auto-mode output, iterate its regions (surfaces).
/// 2. For each region, compare against regions on every other output with the same `source_key`.
/// 3. Compute AABB intersection in stage space → convert to surface-local UV rect.
/// 4. Compute ramp direction from relative surface positions.
/// 5. Collect zones, keep top `MAX_OVERLAP_ZONES` by area.
///
/// Returns `Vec<AutoBlendResult>` — one entry per surface on Auto-mode outputs.
pub fn compute_auto_edge_blend(infos: &[OutputSurfaceInfo]) -> Vec<AutoBlendResult> {
    let mut results: Vec<AutoBlendResult> = Vec::new();

    for info in infos {
        log::debug!(
            "[edge-blend] output {} mode={:?} regions={}",
            info.output_idx, info.edge_blend_mode, info.regions.len()
        );

        if info.edge_blend_mode != EdgeBlendMode::Auto {
            continue;
        }
        let gamma = info.default_gamma;

        for region_a in &info.regions {
            let mut zones = SurfaceOverlapZones::default();

            for other in infos {
                if other.output_idx == info.output_idx {
                    continue;
                }
                for region_b in &other.regions {
                    if region_a.source_key != region_b.source_key {
                        continue;
                    }
                    if let Some(overlap) = polygon_intersect_aabb(region_a, region_b) {
                        let uv_rect = stage_to_surface_uv(&region_a.bbox, &overlap);
                        let (ramp_x, ramp_y) = compute_ramp_direction(&region_a.bbox, &region_b.bbox);
                        zones.add_zone(OverlapZone { uv_rect, gamma, ramp_x, ramp_y });
                    }
                }
            }

            results.push(AutoBlendResult {
                output_idx: info.output_idx,
                surface_uuid: region_a.surface_uuid.clone(),
                overlap_zones: zones,
            });
        }
    }

    results
}

/// Convert a stage-space AABB overlap into surface-local UV coordinates [0..1].
fn stage_to_surface_uv(region: &[f32; 4], overlap: &[f32; 4]) -> [f32; 4] {
    let rw = region[2].max(1e-6);
    let rh = region[3].max(1e-6);
    let u_min = ((overlap[0] - region[0]) / rw).clamp(0.0, 1.0);
    let v_min = ((overlap[1] - region[1]) / rh).clamp(0.0, 1.0);
    let u_max = ((overlap[0] + overlap[2] - region[0]) / rw).clamp(0.0, 1.0);
    let v_max = ((overlap[1] + overlap[3] - region[1]) / rh).clamp(0.0, 1.0);
    [u_min, v_min, u_max, v_max]
}

/// GPU-side uniform for the edge blend shader. 16 floats = 64 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EdgeBlendParams {
    left_enabled: f32, left_width: f32, left_gamma: f32, _pad0: f32,
    right_enabled: f32, right_width: f32, right_gamma: f32, _pad1: f32,
    top_enabled: f32, top_width: f32, top_gamma: f32, _pad2: f32,
    bottom_enabled: f32, bottom_width: f32, bottom_gamma: f32, _pad3: f32,
}

impl From<&EdgeBlendConfig> for EdgeBlendParams {
    fn from(cfg: &EdgeBlendConfig) -> Self {
        Self {
            left_enabled: if cfg.left.enabled { 1.0 } else { 0.0 },
            left_width: cfg.left.width.max(0.001),
            left_gamma: cfg.left.gamma,
            _pad0: 0.0,
            right_enabled: if cfg.right.enabled { 1.0 } else { 0.0 },
            right_width: cfg.right.width.max(0.001),
            right_gamma: cfg.right.gamma,
            _pad1: 0.0,
            top_enabled: if cfg.top.enabled { 1.0 } else { 0.0 },
            top_width: cfg.top.width.max(0.001),
            top_gamma: cfg.top.gamma,
            _pad2: 0.0,
            bottom_enabled: if cfg.bottom.enabled { 1.0 } else { 0.0 },
            bottom_width: cfg.bottom.width.max(0.001),
            bottom_gamma: cfg.bottom.gamma,
            _pad3: 0.0,
        }
    }
}

/// Full-screen post-process pipeline that applies edge blending.
pub struct EdgeBlendPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
}

impl EdgeBlendPipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Edge Blend BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Edge Blend Params"),
            contents: bytemuck::cast_slice(&[EdgeBlendParams::from(&EdgeBlendConfig::default())]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Edge Blend Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Edge Blend VS"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });
        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Edge Blend FS"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/edge_blend.wgsl").into()),
        });


        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Edge Blend Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Edge Blend Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self { pipeline, bind_group_layout, sampler, params_buffer })
    }

    /// Run the edge blend pass: reads from `source_view`, writes to `target_view`.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        config: &EdgeBlendConfig,
    ) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[EdgeBlendParams::from(config)]),
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Edge Blend BG"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(source_view) },
                wgpu::BindGroupEntry { binding: 2, resource: self.params_buffer.as_entire_binding() },
            ],
        });

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Edge Blend Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.draw(0..3, 0..1);
        }
    }
}

/// Compute the smoothstep blend alpha for a given normalized position.
/// Exported for unit testing.
pub fn blend_alpha(t_normalized: f32, gamma: f32) -> f32 {
    let t = t_normalized.clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    s.powf(gamma)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_blend_config_default_none_enabled() {
        let cfg = EdgeBlendConfig::default();
        assert!(!cfg.any_enabled());
    }

    #[test]
    fn edge_blend_config_any_enabled() {
        let mut cfg = EdgeBlendConfig::default();
        cfg.left.enabled = true;
        assert!(cfg.any_enabled());
    }

    #[test]
    fn blend_alpha_endpoints() {
        assert!((blend_alpha(0.0, 2.2) - 0.0).abs() < 1e-6);
        assert!((blend_alpha(1.0, 2.2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn blend_alpha_midpoint() {
        let a = blend_alpha(0.5, 1.0);
        assert!((a - 0.5).abs() < 1e-6);
    }

    #[test]
    fn blend_alpha_gamma_effect() {
        let a1 = blend_alpha(0.5, 1.0);
        let a2 = blend_alpha(0.5, 2.0);
        assert!(a2 < a1);
    }

    #[test]
    fn blend_alpha_clamps_input() {
        assert!((blend_alpha(-0.5, 1.0) - 0.0).abs() < 1e-6);
        assert!((blend_alpha(1.5, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn edge_blend_config_serialization_roundtrip() {
        let mut cfg = EdgeBlendConfig::default();
        cfg.left.enabled = true;
        cfg.left.width = 0.15;
        cfg.right.gamma = 1.8;
        cfg.bottom.enabled = true;
        let json = serde_json::to_string(&cfg).unwrap();
        let restored: EdgeBlendConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, restored);
    }

    #[test]
    fn edge_blend_params_from_config() {
        let mut cfg = EdgeBlendConfig::default();
        cfg.left.enabled = true;
        cfg.right.width = 0.0;
        let params = EdgeBlendParams::from(&cfg);
        assert_eq!(params.left_enabled, 1.0);
        assert_eq!(params.right_enabled, 0.0);
        assert!(params.right_width >= 0.001);
    }

    // ── EdgeBlendMode tests ──────────────────────────────────────

    #[test]
    fn edge_blend_mode_default_is_manual() {
        assert_eq!(EdgeBlendMode::default(), EdgeBlendMode::Manual);
    }

    #[test]
    fn edge_blend_mode_serialization_roundtrip() {
        for mode in [EdgeBlendMode::Manual, EdgeBlendMode::Auto] {
            let json = serde_json::to_string(&mode).unwrap();
            let restored: EdgeBlendMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, restored);
        }
    }

    // ── AABB intersection tests ──────────────────────────────────

    #[test]
    fn aabb_intersect_no_overlap() {
        assert!(aabb_intersect([0.0, 0.0, 0.3, 0.5], [0.5, 0.0, 0.3, 0.5]).is_none());
    }

    #[test]
    fn aabb_intersect_overlap() {
        let r = aabb_intersect([0.0, 0.0, 0.5, 1.0], [0.3, 0.0, 0.5, 1.0]).unwrap();
        assert!((r[0] - 0.3).abs() < 1e-6);
        assert!((r[1] - 0.0).abs() < 1e-6);
        assert!((r[2] - 0.2).abs() < 1e-6);
        assert!((r[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn aabb_intersect_touching_edges() {
        // Touching but not overlapping → None
        assert!(aabb_intersect([0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]).is_none());
    }

    // ── SurfaceOverlapZones tests ────────────────────────────────

    #[test]
    fn overlap_zones_default_none_enabled() {
        let zones = SurfaceOverlapZones::default();
        assert!(!zones.any_enabled());
    }

    #[test]
    fn overlap_zones_any_enabled() {
        let mut zones = SurfaceOverlapZones::default();
        zones.add_zone(OverlapZone { uv_rect: [0.8, 0.0, 1.0, 1.0], gamma: 2.2, ramp_x: 1.0, ramp_y: 0.0 });
        assert!(zones.any_enabled());
    }

    #[test]
    fn overlap_zones_max_capacity() {
        let mut zones = SurfaceOverlapZones::default();
        for i in 0..6 {
            let size = 0.1 * (i as f32 + 1.0);
            zones.add_zone(OverlapZone { uv_rect: [0.0, 0.0, size, size], gamma: 2.2, ramp_x: 1.0, ramp_y: 1.0 });
        }
        assert_eq!(zones.zones.len(), MAX_OVERLAP_ZONES);
        // Largest zones kept (sorted by area descending)
        let areas: Vec<f32> = zones.zones.iter().map(|z| {
            (z.uv_rect[2] - z.uv_rect[0]) * (z.uv_rect[3] - z.uv_rect[1])
        }).collect();
        for i in 0..areas.len() - 1 {
            assert!(areas[i] >= areas[i + 1]);
        }
    }

    #[test]
    fn overlap_zones_serialization_roundtrip() {
        let mut zones = SurfaceOverlapZones::default();
        zones.add_zone(OverlapZone { uv_rect: [0.7, 0.0, 1.0, 1.0], gamma: 2.2, ramp_x: 1.0, ramp_y: 0.0 });
        let json = serde_json::to_string(&zones).unwrap();
        let restored: SurfaceOverlapZones = serde_json::from_str(&json).unwrap();
        assert_eq!(zones, restored);
    }

    // ── stage_to_surface_uv tests ──────────────────────────────

    #[test]
    fn stage_to_uv_full_overlap() {
        let region = [0.0, 0.0, 1.0, 1.0];
        let overlap = [0.0, 0.0, 1.0, 1.0];
        let uv = stage_to_surface_uv(&region, &overlap);
        assert!((uv[0]).abs() < 1e-6); // u_min = 0
        assert!((uv[1]).abs() < 1e-6); // v_min = 0
        assert!((uv[2] - 1.0).abs() < 1e-6); // u_max = 1
        assert!((uv[3] - 1.0).abs() < 1e-6); // v_max = 1
    }

    #[test]
    fn stage_to_uv_right_strip() {
        // Surface [0,0,0.6,1.0], overlap [0.4,0,0.2,1.0]
        let region = [0.0, 0.0, 0.6, 1.0];
        let overlap = [0.4, 0.0, 0.2, 1.0];
        let uv = stage_to_surface_uv(&region, &overlap);
        assert!((uv[0] - 0.4 / 0.6).abs() < 1e-4); // u_min ≈ 0.667
        assert!((uv[1]).abs() < 1e-6);               // v_min = 0
        assert!((uv[2] - 1.0).abs() < 1e-4);         // u_max = 1.0
        assert!((uv[3] - 1.0).abs() < 1e-6);         // v_max = 1.0
    }

    #[test]
    fn stage_to_uv_left_strip() {
        // Surface [0.4,0,0.6,1.0], overlap [0.4,0,0.2,1.0]
        let region = [0.4, 0.0, 0.6, 1.0];
        let overlap = [0.4, 0.0, 0.2, 1.0];
        let uv = stage_to_surface_uv(&region, &overlap);
        assert!((uv[0]).abs() < 1e-6);               // u_min = 0
        assert!((uv[2] - 0.2 / 0.6).abs() < 1e-4);  // u_max ≈ 0.333
    }

    // ── compute_auto_edge_blend tests (overlap zones) ──────────────

    fn make_info(idx: usize, mode: EdgeBlendMode, regions: Vec<MappedRegion>) -> OutputSurfaceInfo {
        OutputSurfaceInfo { output_idx: idx, edge_blend_mode: mode, default_gamma: 2.2, regions }
    }

    fn make_region(source: &str, bbox: [f32; 4]) -> MappedRegion {
        let [x, y, w, h] = bbox;
        let vertices = vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        MappedRegion {
            source_key: source.to_string(),
            bbox,
            surface_uuid: format!("surf-{}-{}", source, bbox[0]),
            vertices,
            extra_contours: vec![],
        }
    }

    fn find_zones<'a>(results: &'a [AutoBlendResult], output_idx: usize) -> &'a SurfaceOverlapZones {
        &results.iter().find(|r| r.output_idx == output_idx).unwrap().overlap_zones
    }

    #[test]
    fn auto_blend_manual_outputs_skipped() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Manual, vec![make_region("Master", [0.0, 0.0, 0.5, 1.0])]),
            make_info(1, EdgeBlendMode::Manual, vec![make_region("Master", [0.3, 0.0, 0.5, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        assert!(results.is_empty());
    }

    #[test]
    fn auto_blend_no_overlap() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.4, 1.0])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.5, 0.0, 0.4, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        assert_eq!(results.len(), 2);
        assert!(!results[0].overlap_zones.any_enabled());
        assert!(!results[1].overlap_zones.any_enabled());
    }

    #[test]
    fn auto_blend_horizontal_overlap() {
        // Two surfaces overlapping horizontally by 0.2 in stage space
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.6, 1.0])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.4, 0.0, 0.6, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        let z0 = find_zones(&results, 0);
        let z1 = find_zones(&results, 1);

        // Each surface gets 1 overlap zone
        assert_eq!(z0.zones.len(), 1);
        assert_eq!(z1.zones.len(), 1);

        // Output 0: overlap zone on the right side (u_max ≈ 1.0), ramp toward right
        let rect0 = z0.zones[0].uv_rect;
        assert!((rect0[2] - 1.0).abs() < 1e-4); // u_max = 1.0
        assert!((rect0[0] - 0.4 / 0.6).abs() < 0.01); // u_min ≈ 0.667
        assert_eq!(z0.zones[0].ramp_x, 1.0);  // fade toward right (B is to the right)
        assert_eq!(z0.zones[0].ramp_y, 0.0);  // no vertical ramp

        // Output 1: overlap zone on the left side (u_min ≈ 0.0), ramp toward left
        let rect1 = z1.zones[0].uv_rect;
        assert!((rect1[0]).abs() < 1e-4); // u_min = 0.0
        assert!((rect1[2] - 0.2 / 0.6).abs() < 0.01); // u_max ≈ 0.333
        assert_eq!(z1.zones[0].ramp_x, -1.0); // fade toward left (A is to the left)
        assert_eq!(z1.zones[0].ramp_y, 0.0);  // no vertical ramp
    }

    #[test]
    fn auto_blend_different_sources_no_blend() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.6, 1.0])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Channel(0)", [0.4, 0.0, 0.6, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        assert!(!results[0].overlap_zones.any_enabled());
        assert!(!results[1].overlap_zones.any_enabled());
    }

    #[test]
    fn auto_blend_vertical_overlap() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 1.0, 0.6])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.4, 1.0, 0.6])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        let z0 = find_zones(&results, 0);
        let z1 = find_zones(&results, 1);

        assert_eq!(z0.zones.len(), 1);
        assert_eq!(z1.zones.len(), 1);

        // Output 0: overlap at bottom (v_max ≈ 1.0), ramp toward bottom
        assert!((z0.zones[0].uv_rect[3] - 1.0).abs() < 1e-4);
        assert_eq!(z0.zones[0].ramp_x, 0.0);  // no horizontal ramp (same x center)
        assert_eq!(z0.zones[0].ramp_y, 1.0);  // fade toward bottom
        // Output 1: overlap at top (v_min ≈ 0.0), ramp toward top
        assert!((z1.zones[0].uv_rect[1]).abs() < 1e-4);
        assert_eq!(z1.zones[0].ramp_x, 0.0);
        assert_eq!(z1.zones[0].ramp_y, -1.0); // fade toward top
    }

    #[test]
    fn auto_blend_mixed_modes() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.6, 1.0])]),
            make_info(1, EdgeBlendMode::Manual, vec![make_region("Master", [0.4, 0.0, 0.6, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].output_idx, 0);
        // Auto output still detects the overlap with the Manual output's surface
        assert!(results[0].overlap_zones.any_enabled());
    }

    #[test]
    fn auto_blend_uses_default_gamma() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.6, 1.0])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.4, 0.0, 0.6, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        let z0 = find_zones(&results, 0);
        assert!((z0.zones[0].gamma - 2.2).abs() < 1e-6);
    }

    #[test]
    fn auto_blend_empty_regions() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![]),
            make_info(1, EdgeBlendMode::Auto, vec![]),
        ];
        let results = compute_auto_edge_blend(&infos);
        assert!(results.is_empty());
    }

    #[test]
    fn auto_blend_three_outputs_chain() {
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.4, 1.0])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.3, 0.0, 0.4, 1.0])]),
            make_info(2, EdgeBlendMode::Auto, vec![make_region("Master", [0.6, 0.0, 0.4, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        let z0 = find_zones(&results, 0);
        let z1 = find_zones(&results, 1);
        let z2 = find_zones(&results, 2);

        // Output 0 overlaps with output 1 → 1 zone
        assert_eq!(z0.zones.len(), 1);
        // Output 1 overlaps with both 0 and 2 → 2 zones
        assert_eq!(z1.zones.len(), 2);
        // Output 2 overlaps with output 1 → 1 zone
        assert_eq!(z2.zones.len(), 1);
    }

    #[test]
    fn auto_blend_fully_overlapping() {
        // Two identical surfaces → each gets 1 zone covering [0,0,1,1]
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.5, 1.0])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.5, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        let z0 = find_zones(&results, 0);
        assert_eq!(z0.zones.len(), 1);
        let r = z0.zones[0].uv_rect;
        assert!((r[0]).abs() < 1e-4);
        assert!((r[1]).abs() < 1e-4);
        assert!((r[2] - 1.0).abs() < 1e-4);
        assert!((r[3] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn auto_blend_per_surface_isolation() {
        // Two surfaces on output 0, only one overlaps with output 1
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![
                make_region("Master", [0.0, 0.0, 0.3, 0.5]),  // no overlap
                make_region("Master", [0.4, 0.0, 0.6, 1.0]),  // overlaps with output 1
            ]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.8, 0.0, 0.2, 1.0])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        assert_eq!(results.len(), 3);
        let out0_results: Vec<_> = results.iter().filter(|r| r.output_idx == 0).collect();
        assert_eq!(out0_results.len(), 2);
        // First surface: no overlap
        assert!(!out0_results[0].overlap_zones.any_enabled());
        // Second surface: overlaps with output 1
        assert!(out0_results[1].overlap_zones.any_enabled());
    }

    #[test]
    fn auto_blend_corner_overlap_creates_zone() {
        // Corner overlap — previously would trigger spurious full-edge blends.
        // Now it creates a small overlap zone in the corner.
        let infos = vec![
            make_info(0, EdgeBlendMode::Auto, vec![make_region("Master", [0.0, 0.0, 0.6, 0.6])]),
            make_info(1, EdgeBlendMode::Auto, vec![make_region("Master", [0.5, 0.5, 0.5, 0.5])]),
        ];
        let results = compute_auto_edge_blend(&infos);
        let z0 = find_zones(&results, 0);
        assert_eq!(z0.zones.len(), 1);
        // Zone should be a small rectangle in the bottom-right corner of surface 0
        let r = z0.zones[0].uv_rect;
        assert!(r[0] > 0.5); // u_min well past midpoint
        assert!(r[1] > 0.5); // v_min well past midpoint
        assert!((r[2] - 1.0).abs() < 1e-4); // u_max = 1.0
        assert!((r[3] - 1.0).abs() < 1e-4); // v_max = 1.0
        // Ramp toward bottom-right (B is below and to the right of A)
        assert_eq!(z0.zones[0].ramp_x, 1.0);
        assert_eq!(z0.zones[0].ramp_y, 1.0);
    }

    // ── compute_ramp_direction tests ───────────────────────────────

    #[test]
    fn ramp_direction_b_right_of_a() {
        let (rx, ry) = compute_ramp_direction(&[0.0, 0.0, 0.5, 1.0], &[0.4, 0.0, 0.5, 1.0]);
        assert_eq!(rx, 1.0);
        assert_eq!(ry, 0.0);
    }

    #[test]
    fn ramp_direction_b_left_of_a() {
        let (rx, ry) = compute_ramp_direction(&[0.4, 0.0, 0.5, 1.0], &[0.0, 0.0, 0.5, 1.0]);
        assert_eq!(rx, -1.0);
        assert_eq!(ry, 0.0);
    }

    #[test]
    fn ramp_direction_b_below_a() {
        let (rx, ry) = compute_ramp_direction(&[0.0, 0.0, 1.0, 0.5], &[0.0, 0.3, 1.0, 0.5]);
        assert_eq!(rx, 0.0);
        assert_eq!(ry, 1.0);
    }

    #[test]
    fn ramp_direction_diagonal() {
        let (rx, ry) = compute_ramp_direction(&[0.0, 0.0, 0.5, 0.5], &[0.3, 0.3, 0.5, 0.5]);
        assert_eq!(rx, 1.0);
        assert_eq!(ry, 1.0);
    }

    // ── Polygon intersection tests ──────────────────────────────────

    #[test]
    fn auto_blend_circle_tighter_than_aabb() {
        // Two circles whose AABBs overlap significantly but whose actual geometry
        // overlaps in a much smaller region.
        let n = 32;
        let make_circle = |cx: f32, cy: f32, r: f32| -> Vec<[f32; 2]> {
            (0..n).map(|i| {
                let angle = 2.0 * std::f32::consts::PI * (i as f32) / (n as f32);
                [cx + r * angle.cos(), cy + r * angle.sin()]
            }).collect()
        };
        let verts_a = make_circle(0.3, 0.5, 0.25);
        let verts_b = make_circle(0.7, 0.5, 0.25);
        let bbox_a = [0.05, 0.25, 0.5, 0.5];
        let bbox_b = [0.45, 0.25, 0.5, 0.5];

        // AABB overlap would be [0.45, 0.25, 0.10, 0.50] → area = 0.05
        let aabb_overlap = aabb_intersect(bbox_a, bbox_b).unwrap();
        let aabb_area = aabb_overlap[2] * aabb_overlap[3];

        let region_a = MappedRegion {
            source_key: "Master".into(), bbox: bbox_a,
            surface_uuid: "a".into(), vertices: verts_a, extra_contours: vec![],
        };
        let region_b = MappedRegion {
            source_key: "Master".into(), bbox: bbox_b,
            surface_uuid: "b".into(), vertices: verts_b, extra_contours: vec![],
        };

        let poly_overlap = polygon_intersect_aabb(&region_a, &region_b);
        // Circles are far enough apart that polygon intersection should be None
        // or significantly smaller than the AABB overlap.
        match poly_overlap {
            None => {} // circles don't actually touch — correct
            Some(r) => {
                let poly_area = r[2] * r[3];
                assert!(poly_area < aabb_area, "polygon overlap area ({poly_area}) should be less than AABB overlap area ({aabb_area})");
            }
        }
    }
}