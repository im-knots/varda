//! Domemaster renderer — generates equidistant azimuthal fisheye projection
//! from mixer output via cubemap capture.
//!
//! The renderer captures 5 cubemap faces (front, right, back, left, top) from
//! the mixer composite, then projects them into a circular domemaster image
//! using the equidistant azimuthal projection.

use anyhow::Result;
use wgpu::util::DeviceExt;

/// Domemaster resolution presets.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DomemasterResolution {
    /// 1024×1024
    R1K,
    /// 2048×2048
    R2K,
    /// 4096×4096
    R4K,
}

impl DomemasterResolution {
    pub fn pixels(self) -> u32 {
        match self {
            Self::R1K => 1024,
            Self::R2K => 2048,
            Self::R4K => 4096,
        }
    }
}

impl Default for DomemasterResolution {
    fn default() -> Self { Self::R2K }
}

impl std::fmt::Display for DomemasterResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::R1K => write!(f, "1K"),
            Self::R2K => write!(f, "2K"),
            Self::R4K => write!(f, "4K"),
        }
    }
}

/// Configuration for the domemaster renderer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DomemasterConfig {
    /// Output resolution (square)
    pub resolution: DomemasterResolution,
    /// Field of view in degrees (180 = full hemisphere)
    pub fov_degrees: f32,
    /// Content tilt in degrees (0 = zenith centered)
    pub tilt_degrees: f32,
}

impl Default for DomemasterConfig {
    fn default() -> Self {
        Self {
            resolution: DomemasterResolution::R2K,
            fov_degrees: 180.0,
            tilt_degrees: 0.0,
        }
    }
}

/// GPU uniform for the domemaster shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DomemasterParams {
    fov: f32,
    tilt: f32,
    content_az: f32,
    content_el: f32,
    content_roll: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// Cubemap face index constants.
const FACE_FRONT: usize = 0;
const FACE_RIGHT: usize = 1;
const FACE_BACK: usize = 2;
const FACE_LEFT: usize = 3;
const FACE_TOP: usize = 4;
const NUM_FACES: usize = 5;

/// Domemaster renderer: captures cubemap faces from the mixer output and
/// projects them into a fisheye domemaster image.
pub struct DomemasterRenderer {
    /// Per-face render textures — kept alive so `face_views` remain valid.
    #[allow(dead_code)]
    face_textures: Vec<wgpu::Texture>,
    face_views: Vec<wgpu::TextureView>,
    /// Output domemaster texture (square)
    pub output_texture: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    /// Blit pipeline for rendering source into each face
    face_blit: super::blit::BlitPipeline,
    /// Projection pipeline (cubemap → fisheye)
    projection_pipeline: wgpu::RenderPipeline,
    projection_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
    /// Current configuration
    pub config: DomemasterConfig,
    /// Size of each cubemap face
    face_size: u32,
    /// Whether the renderer is enabled
    pub enabled: bool,
    /// Content rotation (radians), updated each frame from UI
    pub content_rotation: [f32; 3],
}


impl DomemasterRenderer {
    /// Create a new domemaster renderer with the given configuration.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, config: DomemasterConfig) -> Result<Self> {
        let output_size = config.resolution.pixels();
        // Cubemap faces are half the output resolution for performance
        let face_size = output_size / 2;

        let create_texture = |label: &str, size: u32| -> (wgpu::Texture, wgpu::TextureView) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                     | wgpu::TextureUsages::TEXTURE_BINDING
                     | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };

        let face_labels = ["Dome Face Front", "Dome Face Right", "Dome Face Back",
                           "Dome Face Left", "Dome Face Top"];
        let mut face_textures = Vec::with_capacity(NUM_FACES);
        let mut face_views = Vec::with_capacity(NUM_FACES);
        for label in &face_labels {
            let (tex, view) = create_texture(label, face_size);
            face_textures.push(tex);
            face_views.push(view);
        }

        let (output_texture, output_view) = create_texture("Domemaster Output", output_size);
        let face_blit = super::blit::BlitPipeline::new(device, format)?;

        // Projection pipeline: fullscreen pass reads 5 face textures → fisheye output
        let tex_entry = |binding: u32| -> wgpu::BindGroupLayoutEntry {
            wgpu::BindGroupLayoutEntry {
                binding, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
                }, count: None,
            }
        };

        let projection_bind_group_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("Domemaster Projection BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    tex_entry(1), tex_entry(2), tex_entry(3), tex_entry(4), tex_entry(5),
                    wgpu::BindGroupLayoutEntry {
                        binding: 6, visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false, min_binding_size: None,
                        }, count: None,
                    },
                ],
            },
        );

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Domemaster Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });
        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Domemaster Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/domemaster.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Domemaster Projection Pipeline Layout"),
            bind_group_layouts: &[&projection_bind_group_layout],
            push_constant_ranges: &[],
        });

        let projection_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Domemaster Projection Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader, entry_point: Some("vs_main"),
                buffers: &[], compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader, entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format, blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None, cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Domemaster Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Domemaster Params Buffer"),
            contents: bytemuck::cast_slice(&[DomemasterParams {
                fov: config.fov_degrees.to_radians(),
                tilt: config.tilt_degrees.to_radians(),
                content_az: 0.0, content_el: 0.0, content_roll: 0.0,
                _pad0: 0.0, _pad1: 0.0, _pad2: 0.0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Ok(Self {
            face_textures, face_views, output_texture, output_view,
            face_blit, projection_pipeline, projection_bind_group_layout,
            sampler, params_buffer, config, face_size, enabled: false,
            content_rotation: [0.0; 3],
        })
    }

    /// Update the shader params from the current config + content rotation.
    pub fn update_params(&self, queue: &wgpu::Queue) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[DomemasterParams {
            fov: self.config.fov_degrees.to_radians(),
            tilt: self.config.tilt_degrees.to_radians(),
            content_az: self.content_rotation[0],
            content_el: self.content_rotation[1],
            content_roll: self.content_rotation[2],
            _pad0: 0.0, _pad1: 0.0, _pad2: 0.0,
        }]));
    }

    /// Set content rotation (azimuth, elevation, roll) in radians.
    pub fn set_content_rotation(&mut self, az: f32, el: f32, roll: f32) {
        self.content_rotation = [az, el, roll];
    }

    /// Render the domemaster from the mixer composite.
    ///
    /// The mixer composite texture is treated as a flat content plane positioned
    /// in front of the dome center. Each cubemap face captures a 90° FOV view
    /// of this content. The final fisheye projection merges all faces.
    ///
    /// For the initial implementation, we blit the mixer output directly into
    /// the front face and leave other faces black. This gives a simple forward-
    /// facing dome projection that can be enhanced later with full cubemap
    /// rendering when 3D content positioning is added.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source_view: &wgpu::TextureView,
    ) {
        if !self.enabled { return; }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Domemaster Encoder"),
        });

        // Step 1: Render source into each cubemap face.
        // Front face gets the mixer composite content; other faces get cleared to black.
        // UV transforms per face map the source content onto the dome.
        let face_uv_configs: [(f32, [f32; 2], [f32; 2]); NUM_FACES] = [
            (1.0, [1.0, 1.0], [0.0, 0.0]), // Front: full source
            (0.0, [1.0, 1.0], [0.0, 0.0]), // Right: black (no content)
            (0.0, [1.0, 1.0], [0.0, 0.0]), // Back: black
            (0.0, [1.0, 1.0], [0.0, 0.0]), // Left: black
            (0.0, [1.0, 1.0], [0.0, 0.0]), // Top: black
        ];

        for (i, (opacity, uv_scale, uv_offset)) in face_uv_configs.iter().enumerate() {
            let bind_group = self.face_blit.create_bind_group_with_params(
                device, source_view, *opacity, *uv_scale, *uv_offset,
            );
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Dome Face Render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.face_views[i],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if *opacity > 0.0 {
                self.face_blit.render(&mut rp, &bind_group);
            }
        }

        // Step 2: Projection pass — cubemap faces → fisheye domemaster
        let projection_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Domemaster Projection Bind Group"),
            layout: &self.projection_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.face_views[FACE_FRONT]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.face_views[FACE_RIGHT]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.face_views[FACE_BACK]) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.face_views[FACE_LEFT]) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.face_views[FACE_TOP]) },
                wgpu::BindGroupEntry { binding: 6, resource: self.params_buffer.as_entire_binding() },
            ],
        });

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Domemaster Projection Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.projection_pipeline);
            rp.set_bind_group(0, &projection_bind_group, &[]);
            rp.draw(0..3, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Get the output domemaster texture view for downstream sampling.
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.output_view
    }

    /// Current output resolution in pixels (square).
    pub fn output_size(&self) -> u32 {
        self.config.resolution.pixels()
    }

    /// Size of each cubemap face in pixels.
    pub fn face_size(&self) -> u32 {
        self.face_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_pixels() {
        assert_eq!(DomemasterResolution::R1K.pixels(), 1024);
        assert_eq!(DomemasterResolution::R2K.pixels(), 2048);
        assert_eq!(DomemasterResolution::R4K.pixels(), 4096);
    }

    #[test]
    fn resolution_default_is_2k() {
        assert_eq!(DomemasterResolution::default(), DomemasterResolution::R2K);
    }

    #[test]
    fn resolution_display() {
        assert_eq!(format!("{}", DomemasterResolution::R1K), "1K");
        assert_eq!(format!("{}", DomemasterResolution::R2K), "2K");
        assert_eq!(format!("{}", DomemasterResolution::R4K), "4K");
    }

    #[test]
    fn config_default_values() {
        let cfg = DomemasterConfig::default();
        assert_eq!(cfg.resolution, DomemasterResolution::R2K);
        assert!((cfg.fov_degrees - 180.0).abs() < f32::EPSILON);
        assert!((cfg.tilt_degrees - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn config_serialization_roundtrip() {
        let cfg = DomemasterConfig {
            resolution: DomemasterResolution::R4K,
            fov_degrees: 160.0,
            tilt_degrees: 30.0,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: DomemasterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.resolution, DomemasterResolution::R4K);
        assert!((deserialized.fov_degrees - 160.0).abs() < f32::EPSILON);
        assert!((deserialized.tilt_degrees - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn params_alignment() {
        // DomemasterParams must be 32 bytes (2 × vec4) for GPU uniform alignment
        assert_eq!(std::mem::size_of::<DomemasterParams>(), 32);
    }

    #[test]
    fn output_source_domemaster_display() {
        use crate::renderer::context::OutputSource;
        let source = OutputSource::Domemaster;
        assert_eq!(format!("{}", source), "Domemaster");
    }

    #[test]
    fn output_source_domemaster_channel_indices_none() {
        use crate::renderer::context::OutputSource;
        assert!(OutputSource::Domemaster.channel_indices().is_none());
    }
}