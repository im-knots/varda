/// Simple blit pipeline for copying textures to the screen
use anyhow::Result;
use wgpu::util::DeviceExt;

/// Uniform buffer for blit parameters - 32 bytes (8 x f32)
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    opacity: f32,
    _pad: f32,
    /// UV scale factor (default 1.0, 1.0 = no scaling)
    uv_scale: [f32; 2],
    /// UV offset (default 0.0, 0.0 = no offset)
    uv_offset: [f32; 2],
    _pad2: [f32; 2],
}

pub struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
}

impl BlitPipeline {
    /// Create a new blit pipeline with REPLACE blend (for final screen output)
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        Self::with_blend(device, target_format, wgpu::BlendState::REPLACE)
    }

    /// Create a blit pipeline with a specific blend state
    pub fn with_blend(device: &wgpu::Device, target_format: wgpu::TextureFormat, blend_state: wgpu::BlendState) -> Result<Self> {
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blit Bind Group Layout"),
            entries: &[
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Texture
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
                // Params uniform buffer (opacity)
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

        // Create params buffer with default opacity of 1.0
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blit Params Buffer"),
            contents: bytemuck::cast_slice(&[BlitParams {
                opacity: 1.0,
                _pad: 0.0,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                _pad2: [0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blit Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Load shaders
        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blit Pipeline"),
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
                    blend: Some(blend_state),
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
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blit Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            params_buffer,
        })
    }

    /// Create a bind group for a texture view
    pub fn create_bind_group(&self, device: &wgpu::Device, texture_view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blit Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Update the opacity value (call before render)
    pub fn set_opacity(&self, queue: &wgpu::Queue, opacity: f32) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[BlitParams {
            opacity,
            _pad: 0.0,
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            _pad2: [0.0, 0.0],
        }]));
    }

    /// Set UV transform parameters for scaling modes
    pub fn set_uv_transform(&self, queue: &wgpu::Queue, opacity: f32, uv_scale: [f32; 2], uv_offset: [f32; 2]) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[BlitParams {
            opacity,
            _pad: 0.0,
            uv_scale,
            uv_offset,
            _pad2: [0.0, 0.0],
        }]));
    }

    /// Render a texture to a render pass
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, bind_group: &'a wgpu::BindGroup) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Render with specific opacity (updates buffer and renders)
    pub fn render_with_opacity<'a>(&'a self, queue: &wgpu::Queue, render_pass: &mut wgpu::RenderPass<'a>, bind_group: &'a wgpu::BindGroup, opacity: f32) {
        self.set_opacity(queue, opacity);
        self.render(render_pass, bind_group);
    }

    /// Create a bind group with its own params buffer baked in.
    /// Use this when you need multiple surfaces with different UV transforms in one render pass.
    pub fn create_bind_group_with_params(
        &self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
        opacity: f32,
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
    ) -> wgpu::BindGroup {
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blit Params Buffer (per-surface)"),
            contents: bytemuck::cast_slice(&[BlitParams {
                opacity,
                _pad: 0.0,
                uv_scale,
                uv_offset,
                _pad2: [0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blit Bind Group (per-surface)"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        })
    }
}

// === Polygon rendering pipeline ===

/// Extended params for polygon pipeline — includes homography matrix for warp.
/// Must match the PolygonParams struct in polygon.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PolygonParams {
    opacity: f32,
    _pad: f32,
    uv_scale: [f32; 2],
    uv_offset: [f32; 2],
    _pad2: [f32; 2],
    // 3x3 homography matrix stored as 3 × vec4 (xyz used, w = 0 padding)
    h_row0: [f32; 4],
    h_row1: [f32; 4],
    h_row2: [f32; 4],
}

impl PolygonParams {
    /// Identity homography (no warp)
    fn identity_homography() -> [[f32; 4]; 3] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]
    }
}

/// Vertex for polygon rendering — position + UV
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PolygonVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}

impl PolygonVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PolygonVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
        ],
    };
}

/// Pipeline for rendering textured polygon surfaces using vertex buffers.
pub struct PolygonBlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl PolygonBlitPipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Polygon Blit Bind Group Layout"),
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Polygon Blit Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Combined vertex + fragment shader with homography support
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Polygon Warp Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/polygon.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Polygon Blit Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &[PolygonVertex::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
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
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Polygon Blit Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self { pipeline, bind_group_layout, sampler })
    }

    /// Create a bind group for a texture with UV transform and homography warp.
    /// `homography` is a 3×3 matrix packed as 12 floats (3 rows × 4, with w padding).
    /// Pass `None` for identity (no warp).
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
        homography: Option<&[f32; 12]>,
    ) -> wgpu::BindGroup {
        let h = homography.copied().unwrap_or_else(|| {
            let id = PolygonParams::identity_homography();
            [id[0][0], id[0][1], id[0][2], id[0][3],
             id[1][0], id[1][1], id[1][2], id[1][3],
             id[2][0], id[2][1], id[2][2], id[2][3]]
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Polygon Params Buffer"),
            contents: bytemuck::cast_slice(&[PolygonParams {
                opacity: 1.0,
                _pad: 0.0,
                uv_scale,
                uv_offset,
                _pad2: [0.0, 0.0],
                h_row0: [h[0], h[1], h[2], h[3]],
                h_row1: [h[4], h[5], h[6], h[7]],
                h_row2: [h[8], h[9], h[10], h[11]],
            }]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Polygon Blit Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture_view) },
                wgpu::BindGroupEntry { binding: 2, resource: params_buffer.as_entire_binding() },
            ],
        })
    }

    /// Fan-triangulate polygon vertices and render.
    /// `vertices` are in normalized canvas coords [0..1], UVs computed from bounding box.
    pub fn render_polygon<'a>(
        &'a self,
        _device: &wgpu::Device,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
        vertex_buffer: &'a wgpu::Buffer,
        num_triangles: u32,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.draw(0..num_triangles * 3, 0..1);
    }

    /// Build an ear-clipping triangulated vertex buffer from polygon vertices.
    /// Handles concave polygons correctly (fan triangulation only works for convex).
    /// Returns (buffer, num_triangles).
    /// UVs are set so that the bounding box maps to [0..1] (for Fill mode,
    /// the blit shader's uv_scale/uv_offset handle the rest).
    pub fn triangulate(
        device: &wgpu::Device,
        canvas_verts: &[[f32; 2]],
        bb_x: f32, bb_y: f32, bb_w: f32, bb_h: f32,
    ) -> (wgpu::Buffer, u32) {
        let n = canvas_verts.len();
        if n < 3 {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Polygon Vertex Buffer (empty)"),
                contents: &[],
                usage: wgpu::BufferUsages::VERTEX,
            });
            return (buffer, 0);
        }

        let indices = ear_clip_triangulate(canvas_verts);
        let num_tris = (indices.len() / 3) as u32;

        let to_vert = |v: &[f32; 2]| -> PolygonVertex {
            let u = if bb_w > 0.0 { (v[0] - bb_x) / bb_w } else { 0.0 };
            let t = if bb_h > 0.0 { (v[1] - bb_y) / bb_h } else { 0.0 };
            PolygonVertex { position: *v, uv: [u, t] }
        };

        let mut gpu_verts: Vec<PolygonVertex> = Vec::with_capacity(indices.len());
        for &idx in &indices {
            gpu_verts.push(to_vert(&canvas_verts[idx as usize]));
        }

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Polygon Vertex Buffer"),
            contents: bytemuck::cast_slice(&gpu_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        (buffer, num_tris)
    }
}


// === Ear-clipping triangulation for concave polygons ===

/// Ear-clipping triangulation for a simple (non-self-intersecting) polygon.
/// Returns triangle indices into the vertex array.
fn ear_clip_triangulate(verts: &[[f32; 2]]) -> Vec<u32> {
    let n = verts.len();
    if n < 3 { return Vec::new(); }

    let mut idx: Vec<usize> = (0..n).collect();
    let mut result = Vec::with_capacity((n - 2) * 3);

    // Determine winding via signed area (y-down coords: negative = CCW)
    let signed_area: f32 = (0..n).map(|i| {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        (b[0] - a[0]) * (b[1] + a[1])
    }).sum();
    let ccw = signed_area < 0.0;

    let mut remaining = n;
    let mut fail_count = 0;
    let mut i = 0;

    while remaining > 2 && fail_count < remaining {
        let pi = idx[(i + remaining - 1) % remaining];
        let ci = idx[i % remaining];
        let ni = idx[(i + 1) % remaining];

        if ear_clip_is_ear(verts, &idx, pi, ci, ni, ccw) {
            result.push(pi as u32);
            result.push(ci as u32);
            result.push(ni as u32);
            idx.remove(i % remaining);
            remaining -= 1;
            fail_count = 0;
            if i >= remaining && remaining > 0 { i = 0; }
        } else {
            i = (i + 1) % remaining;
            fail_count += 1;
        }
    }

    result
}

fn ear_clip_cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn ear_clip_is_ear(verts: &[[f32; 2]], idx: &[usize], prev: usize, curr: usize, next: usize, ccw: bool) -> bool {
    let cross = ear_clip_cross(verts[prev], verts[curr], verts[next]);
    if ccw { if cross <= 0.0 { return false; } } else { if cross >= 0.0 { return false; } }

    for &vi in idx {
        if vi == prev || vi == curr || vi == next { continue; }
        if ear_clip_point_in_tri(verts[vi], verts[prev], verts[curr], verts[next]) {
            return false;
        }
    }
    true
}

fn ear_clip_point_in_tri(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d0 = ear_clip_cross(a, b, p);
    let d1 = ear_clip_cross(b, c, p);
    let d2 = ear_clip_cross(c, a, p);
    let has_neg = (d0 < 0.0) || (d1 < 0.0) || (d2 < 0.0);
    let has_pos = (d0 > 0.0) || (d1 > 0.0) || (d2 > 0.0);
    !(has_neg && has_pos)
}
