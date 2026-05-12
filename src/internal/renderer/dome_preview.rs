//! 3D Dome Preview — renders a hemisphere with domemaster texture for the Stage Editor.
//!
//! Uses a UV sphere mesh with equidistant azimuthal UVs, an orbit camera,
//! and renders to an offscreen texture that egui can display.

use anyhow::Result;
use wgpu::util::DeviceExt;

/// Default resolution of the preview render target.
const DEFAULT_PREVIEW_SIZE: u32 = 512;

/// Hemisphere mesh vertex density (segments × rings).
const SPHERE_SEGMENTS: u32 = 32;
const SPHERE_RINGS: u32 = 16;

/// Vertex for the hemisphere mesh.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DomeVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

impl DomeVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
        ],
    };
}

/// Orbit camera for 3D dome preview.
#[derive(Debug, Clone)]
pub struct OrbitCamera {
    /// Azimuth angle in radians (horizontal rotation).
    pub azimuth: f32,
    /// Elevation angle in radians (vertical rotation, clamped to avoid gimbal lock).
    pub elevation: f32,
    /// Distance from center.
    pub distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            azimuth: 0.0,
            elevation: 0.7, // ~40° above horizon
            distance: 3.0,
        }
    }
}

impl OrbitCamera {
    /// Apply mouse drag to rotate the camera.
    pub fn rotate(&mut self, delta_x: f32, delta_y: f32) {
        self.azimuth += delta_x * 0.01;
        self.elevation = (self.elevation - delta_y * 0.01)
            .clamp(0.05, std::f32::consts::FRAC_PI_2 - 0.05);
    }

    /// Apply scroll to zoom.
    pub fn zoom(&mut self, delta: f32) {
        self.distance = (self.distance - delta * 0.1).clamp(1.5, 10.0);
    }

    /// Reset to default view.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Compute the view matrix (camera position looking at origin).
    fn view_matrix(&self) -> [[f32; 4]; 4] {
        let (sa, ca) = self.azimuth.sin_cos();
        let (se, ce) = self.elevation.sin_cos();
        let eye = [
            self.distance * ce * sa,
            self.distance * se,
            self.distance * ce * ca,
        ];
        look_at(eye, [0.0, 0.2, 0.0], [0.0, 1.0, 0.0])
    }
}

/// Dome preview uniform buffer: MVP + content rotation.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DomeUniforms {
    mvp: [[f32; 4]; 4],
    /// Content rotation: [azimuth_rad, elevation_rad, roll_rad, 0]
    content_rotation: [f32; 4],
}

/// Vertex for the slice overlay mesh (position + color).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OverlayVertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl OverlayVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
        ],
    };
}

/// 3D dome preview renderer.
pub struct DomePreviewRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Output render target.
    pub output_texture: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    /// Depth buffer — kept alive so `depth_view` remains valid.
    #[allow(dead_code)]
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    /// Camera state.
    pub camera: OrbitCamera,
    /// Current render target dimensions.
    width: u32,
    height: u32,
    /// Texture format (needed for resize).
    format: wgpu::TextureFormat,
    // ── Slice overlay ──
    overlay_pipeline: wgpu::RenderPipeline,
    overlay_bgl: wgpu::BindGroupLayout,
    overlay_vertex_buffer: wgpu::Buffer,
    overlay_num_vertices: u32,
}


impl DomePreviewRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Result<Self> {
        Self::new_with_size(device, format, DEFAULT_PREVIEW_SIZE, DEFAULT_PREVIEW_SIZE)
    }

    pub fn new_with_size(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Result<Self> {
        let (vertices, indices) = generate_hemisphere(SPHERE_SEGMENTS, SPHERE_RINGS);
        let num_indices = indices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dome Preview VB"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dome Preview IB"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dome Preview Uniforms"),
            contents: bytemuck::cast_slice(&[DomeUniforms { mvp: identity_matrix(), content_rotation: [0.0; 4] }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dome Preview Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dome Preview BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dome Preview Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dome_preview.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dome Preview Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dome Preview Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader, entry_point: Some("vs_main"),
                buffers: &[DomeVertex::LAYOUT], compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format, blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None, cache: None,
        });

        // ── Overlay pipeline (colored triangles with alpha blending) ──
        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dome Slice Overlay Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dome_slice_overlay.wgsl").into()),
        });

        // Overlay shares the same uniform buffer (MVP), so reuse bind group layout entry 0 only
        let overlay_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dome Overlay BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let overlay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dome Overlay Pipeline Layout"),
            bind_group_layouts: &[&overlay_bgl],
            push_constant_ranges: &[],
        });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dome Slice Overlay Pipeline"),
            layout: Some(&overlay_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader, entry_point: Some("vs_main"),
                buffers: &[OverlayVertex::LAYOUT], compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader, entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // render both sides
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false, // don't write depth (overlay sits on top)
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: -2, // pull overlay slightly toward camera
                    slope_scale: -1.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None, cache: None,
        });

        // Empty initial overlay vertex buffer
        let overlay_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dome Overlay VB"),
            contents: &[0u8; 4], // minimum size
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let (output_texture, output_view, depth_texture, depth_view) =
            Self::create_textures(device, format, width, height);

        Ok(Self {
            pipeline, bind_group_layout, vertex_buffer, index_buffer, num_indices,
            uniform_buffer, sampler, output_texture, output_view, depth_texture, depth_view,
            camera: OrbitCamera::default(),
            width, height, format,
            overlay_pipeline, overlay_bgl, overlay_vertex_buffer, overlay_num_vertices: 0,
        })
    }

    /// Create output and depth textures at the given dimensions.
    fn create_textures(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::Texture, wgpu::TextureView) {
        let create_tex = |label, fmt, usage| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: fmt, usage, view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };
        let (output_texture, output_view) = create_tex(
            "Dome Preview Output", format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth_texture, depth_view) = create_tex(
            "Dome Preview Depth", wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        (output_texture, output_view, depth_texture, depth_view)
    }

    /// Resize the render target. Returns true if the size changed.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let w = width.max(1);
        let h = height.max(1);
        if w == self.width && h == self.height {
            return false;
        }
        let (ot, ov, dt, dv) = Self::create_textures(device, self.format, w, h);
        self.output_texture = ot;
        self.output_view = ov;
        self.depth_texture = dt;
        self.depth_view = dv;
        self.width = w;
        self.height = h;
        true
    }

    /// Current width in pixels.
    pub fn width(&self) -> u32 { self.width }
    /// Current height in pixels.
    pub fn height(&self) -> u32 { self.height }

    /// Render the dome preview with the given domemaster texture.
    /// `content_az`, `content_el`, `content_roll` are in radians.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        domemaster_view: &wgpu::TextureView,
        content_az: f32,
        content_el: f32,
        content_roll: f32,
    ) {
        // Update uniforms
        let view = self.camera.view_matrix();
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let proj = perspective(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
        let mvp = mat4_mul(&proj, &view);
        let uniforms = DomeUniforms {
            mvp,
            content_rotation: [content_az, content_el, content_roll, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dome Preview Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(domemaster_view) },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Dome Preview Encoder"),
        });

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Dome Preview Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.15, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            rp.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            rp.draw_indexed(0..self.num_indices, 0, 0..1);

            // Draw slice overlays on top of the dome
            if self.overlay_num_vertices > 0 {
                let overlay_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Dome Overlay Bind Group"),
                    layout: &self.overlay_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                    ],
                });
                rp.set_pipeline(&self.overlay_pipeline);
                rp.set_bind_group(0, &overlay_bg, &[]);
                rp.set_vertex_buffer(0, self.overlay_vertex_buffer.slice(..));
                rp.draw(0..self.overlay_num_vertices, 0..1);
            }
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Preview size in pixels (returns width, kept for compatibility).
    pub fn size(&self) -> u32 { self.width }

    /// Build overlay geometry from a dome setup's projector configs.
    /// Each projector gets a colored semi-transparent wedge on the hemisphere.
    pub fn set_slice_overlays(
        &mut self,
        device: &wgpu::Device,
        setup: &crate::renderer::slicer::DomeSetup,
    ) {
        const COLORS: [[f32; 3]; 8] = [
            [230.0/255.0, 57.0/255.0, 70.0/255.0],   // Red
            [42.0/255.0, 157.0/255.0, 143.0/255.0],   // Teal
            [69.0/255.0, 123.0/255.0, 157.0/255.0],   // Blue
            [241.0/255.0, 196.0/255.0, 15.0/255.0],   // Yellow
            [230.0/255.0, 126.0/255.0, 34.0/255.0],   // Orange
            [155.0/255.0, 89.0/255.0, 182.0/255.0],   // Purple
            [26.0/255.0, 188.0/255.0, 156.0/255.0],   // Cyan
            [232.0/255.0, 67.0/255.0, 147.0/255.0],   // Pink
        ];
        const ALPHA: f32 = 0.3;
        const GRID: u32 = 9; // overlay grid density

        let mut verts: Vec<OverlayVertex> = Vec::new();

        for (pi, proj) in setup.projectors.iter().enumerate() {
            let [cr, cg, cb] = COLORS[pi % COLORS.len()];
            let color = [cr, cg, cb, ALPHA];

            // Compute a grid of 3D dome positions for this projector's coverage
            let positions = projector_dome_positions(proj, &setup.geometry, GRID);

            // Triangulate the grid into triangle strips
            for row in 0..(GRID - 1) {
                for col in 0..(GRID - 1) {
                    let tl = (row * GRID + col) as usize;
                    let tr = tl + 1;
                    let bl = ((row + 1) * GRID + col) as usize;
                    let br = bl + 1;
                    // Two triangles per quad
                    verts.push(OverlayVertex { position: positions[tl], color });
                    verts.push(OverlayVertex { position: positions[bl], color });
                    verts.push(OverlayVertex { position: positions[tr], color });
                    verts.push(OverlayVertex { position: positions[tr], color });
                    verts.push(OverlayVertex { position: positions[bl], color });
                    verts.push(OverlayVertex { position: positions[br], color });
                }
            }
        }

        self.overlay_num_vertices = verts.len() as u32;
        if !verts.is_empty() {
            self.overlay_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Dome Overlay VB"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        }
    }

    /// Clear all slice overlays.
    pub fn clear_slice_overlays(&mut self) {
        self.overlay_num_vertices = 0;
    }
}


// ── Projector footprint → 3D dome positions ────────────────────────────

/// Compute a grid of 3D positions on the unit hemisphere for a projector's coverage.
/// Uses the same ray-tracing approach as the slicer, but outputs 3D positions
/// instead of domemaster UVs.
fn projector_dome_positions(
    proj: &crate::renderer::slicer::ProjectorConfig,
    _geometry: &crate::renderer::slicer::DomeGeometry,
    grid: u32,
) -> Vec<[f32; 3]> {
    let half_fov_h = (proj.fov_degrees * 0.5).to_radians();
    let half_fov_v = ((proj.fov_degrees / proj.aspect_ratio) * 0.5).to_radians();
    let az = proj.azimuth_degrees.to_radians();
    let el = proj.elevation_degrees.to_radians();

    let mut positions = Vec::with_capacity((grid * grid) as usize);

    for row in 0..grid {
        let v = row as f32 / (grid - 1) as f32;
        let angle_v = half_fov_v * (1.0 - 2.0 * v);

        for col in 0..grid {
            let u = col as f32 / (grid - 1) as f32;
            let angle_h = half_fov_h * (2.0 * u - 1.0);

            // Ray direction in projector-local space
            let local_len = (angle_h.tan().powi(2) + angle_v.tan().powi(2) + 1.0).sqrt();
            let local_dir = [angle_h.tan() / local_len, angle_v.tan() / local_len, 1.0 / local_len];

            // Rotate by elevation (X axis)
            let (se, ce) = el.sin_cos();
            let after_el = [
                local_dir[0],
                local_dir[1] * ce - local_dir[2] * se,
                local_dir[1] * se + local_dir[2] * ce,
            ];

            // Rotate by azimuth (Y axis)
            let (sa, ca) = az.sin_cos();
            let world_dir = [
                after_el[0] * ca + after_el[2] * sa,
                after_el[1],
                -after_el[0] * sa + after_el[2] * ca,
            ];

            // Normalize to unit sphere — this IS the dome surface position
            let len = (world_dir[0].powi(2) + world_dir[1].powi(2) + world_dir[2].powi(2)).sqrt();
            let pos = if len > 1e-6 {
                [world_dir[0] / len, world_dir[1] / len, world_dir[2] / len]
            } else {
                [0.0, 1.0, 0.0] // fallback: zenith
            };

            // Clamp to upper hemisphere (y >= 0)
            let pos = if pos[1] < 0.0 {
                // Project onto equator
                let xz_len = (pos[0].powi(2) + pos[2].powi(2)).sqrt().max(1e-6);
                [pos[0] / xz_len, 0.0, pos[2] / xz_len]
            } else {
                pos
            };

            // Slight offset outward to avoid z-fighting with dome mesh
            positions.push([pos[0] * 1.002, pos[1] * 1.002, pos[2] * 1.002]);
        }
    }

    positions
}

// ── Hemisphere mesh generation ──────────────────────────────────────────

/// Generate a hemisphere mesh with equidistant azimuthal UV mapping.
/// Returns (vertices, indices) for indexed triangle rendering.
pub fn generate_hemisphere(segments: u32, rings: u32) -> (Vec<DomeVertex>, Vec<u16>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let polar = std::f32::consts::FRAC_PI_2 * (ring as f32 / rings as f32);
        let y = polar.cos();
        let xz_radius = polar.sin();

        for seg in 0..=segments {
            let azimuth = 2.0 * std::f32::consts::PI * (seg as f32 / segments as f32);
            let x = xz_radius * azimuth.cos();
            let z = xz_radius * azimuth.sin();

            // Equidistant azimuthal UV: center = zenith, edge = horizon
            let r = polar / std::f32::consts::FRAC_PI_2; // 0 at zenith, 1 at horizon
            let uv_x = 0.5 + r * 0.5 * azimuth.cos();
            let uv_y = 0.5 + r * 0.5 * azimuth.sin();

            vertices.push(DomeVertex {
                position: [x, y, z],
                uv: [uv_x, uv_y],
            });
        }
    }

    // Generate triangle indices
    let verts_per_ring = segments + 1;
    for ring in 0..rings {
        for seg in 0..segments {
            let tl = (ring * verts_per_ring + seg) as u16;
            let tr = tl + 1;
            let bl = ((ring + 1) * verts_per_ring + seg) as u16;
            let br = bl + 1;
            // Two triangles per quad (CCW winding when viewed from outside)
            indices.extend_from_slice(&[tl, tr, bl, tr, br, bl]);
        }
    }

    (vertices, indices)
}

// ── Matrix math ─────────────────────────────────────────────────────────

fn identity_matrix() -> [[f32; 4]; 4] {
    [[1.0, 0.0, 0.0, 0.0],
     [0.0, 1.0, 0.0, 0.0],
     [0.0, 0.0, 1.0, 0.0],
     [0.0, 0.0, 0.0, 1.0]]
}

/// View matrix in column-major order: m[col][row] for WGSL mat4x4.
fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = vec3_normalize([target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]]);
    let s = vec3_normalize(vec3_cross(f, up));
    let u = vec3_cross(s, f);

    // Column-major: m[col][row]
    [[s[0],              s[1],              s[2],              0.0],  // col 0
     [u[0],              u[1],              u[2],              0.0],  // col 1
     [-f[0],             -f[1],             -f[2],             0.0],  // col 2
     [-vec3_dot(s, eye), -vec3_dot(u, eye), vec3_dot(f, eye),  1.0]] // col 3
}

/// Perspective projection for wgpu (z maps to [0, 1], column-major: m[col][row]).
fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y * 0.5).tan();
    let nf = 1.0 / (near - far);
    // Column-major: m[col][row], wgpu clip space z ∈ [0, 1]
    [[f / aspect, 0.0,  0.0,             0.0],  // col 0
     [0.0,        f,    0.0,             0.0],   // col 1
     [0.0,        0.0,  far * nf,       -1.0],   // col 2
     [0.0,        0.0,  far * near * nf, 0.0]]   // col 3
}

/// Column-major matrix multiply: result = a * b, where m[col][row].
fn mat4_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut r = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            r[col][row] = a[0][row] * b[col][0]
                        + a[1][row] * b[col][1]
                        + a[2][row] * b[col][2]
                        + a[3][row] * b[col][3];
        }
    }
    r
}

fn vec3_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vec3_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn vec3_normalize(v: [f32; 3]) -> [f32; 3] {
    let len = vec3_dot(v, v).sqrt();
    if len < 1e-10 { return [0.0, 0.0, 1.0]; }
    [v[0] / len, v[1] / len, v[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hemisphere_vertex_count() {
        let (verts, _) = generate_hemisphere(4, 2);
        // (segments+1) * (rings+1) = 5 * 3 = 15
        assert_eq!(verts.len(), 15);
    }

    #[test]
    fn hemisphere_index_count() {
        let (_, indices) = generate_hemisphere(4, 2);
        // segments * rings * 6 = 4 * 2 * 6 = 48
        assert_eq!(indices.len(), 48);
    }

    #[test]
    fn hemisphere_zenith_at_top() {
        let (verts, _) = generate_hemisphere(8, 4);
        // First ring (ring=0) should be at y ≈ 1.0 (top)
        let top = &verts[0];
        assert!((top.position[1] - 1.0).abs() < 1e-5, "Zenith y should be 1.0, got {}", top.position[1]);
    }

    #[test]
    fn hemisphere_equator_at_y_zero() {
        let (verts, _) = generate_hemisphere(8, 4);
        // Last ring (ring=4) should be at y ≈ 0.0 (equator)
        let segments = 8;
        let last_ring_start = 4 * (segments + 1);
        let equator = &verts[last_ring_start as usize];
        assert!((equator.position[1]).abs() < 1e-5, "Equator y should be 0.0, got {}", equator.position[1]);
    }

    #[test]
    fn hemisphere_uvs_in_range() {
        let (verts, _) = generate_hemisphere(16, 8);
        for v in &verts {
            assert!(v.uv[0] >= 0.0 && v.uv[0] <= 1.0, "UV x out of range: {}", v.uv[0]);
            assert!(v.uv[1] >= 0.0 && v.uv[1] <= 1.0, "UV y out of range: {}", v.uv[1]);
        }
    }

    #[test]
    fn hemisphere_zenith_uv_at_center() {
        let (verts, _) = generate_hemisphere(8, 4);
        // Zenith (ring=0) should have UV at center (0.5, 0.5)
        let top = &verts[0];
        assert!((top.uv[0] - 0.5).abs() < 1e-4, "Zenith UV x should be 0.5, got {}", top.uv[0]);
        assert!((top.uv[1] - 0.5).abs() < 1e-4, "Zenith UV y should be 0.5, got {}", top.uv[1]);
    }

    #[test]
    fn orbit_camera_default() {
        let cam = OrbitCamera::default();
        assert!(cam.distance > 0.0);
        assert!(cam.elevation > 0.0);
    }

    #[test]
    fn orbit_camera_rotate() {
        let mut cam = OrbitCamera::default();
        let orig_az = cam.azimuth;
        cam.rotate(10.0, 0.0);
        assert!((cam.azimuth - orig_az).abs() > 0.01);
    }

    #[test]
    fn orbit_camera_zoom_clamped() {
        let mut cam = OrbitCamera::default();
        cam.zoom(1000.0); // zoom in a lot
        assert!(cam.distance >= 1.5);
        cam.zoom(-1000.0); // zoom out a lot
        assert!(cam.distance <= 10.0);
    }

    #[test]
    fn orbit_camera_elevation_clamped() {
        let mut cam = OrbitCamera::default();
        cam.rotate(0.0, -1000.0); // try to go past 90°
        assert!(cam.elevation < std::f32::consts::FRAC_PI_2);
        cam.rotate(0.0, 1000.0); // try to go below 0°
        assert!(cam.elevation > 0.0);
    }

    #[test]
    fn identity_matrix_is_identity() {
        let m = identity_matrix();
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((m[i][j] - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn mat4_mul_identity() {
        let a = identity_matrix();
        let b = [[2.0, 0.0, 0.0, 0.0],
                  [0.0, 3.0, 0.0, 0.0],
                  [0.0, 0.0, 4.0, 0.0],
                  [0.0, 0.0, 0.0, 5.0]];
        let r = mat4_mul(&a, &b);
        assert!((r[0][0] - 2.0).abs() < 1e-6);
        assert!((r[1][1] - 3.0).abs() < 1e-6);
        assert!((r[2][2] - 4.0).abs() < 1e-6);
        assert!((r[3][3] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn dome_vertex_size() {
        assert_eq!(std::mem::size_of::<DomeVertex>(), 20); // 3*4 + 2*4
    }
}