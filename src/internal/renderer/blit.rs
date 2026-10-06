use super::edge_blend::SurfaceOverlapZones;
/// Blit pipelines: texture copy, shader compositing, and polygon surfaces.
use crate::surface::mask::{DEFAULT_MASK_RES, bake_hole_mask};
use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

/// Initial and minimum per-draw parameter slots in a ring buffer.
const MAX_DRAW_SLOTS: u64 = 16;

/// Uniform buffer for blit and final presentation parameters.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    opacity: f32,
    rotation: u32,
    /// UV scale (1.0, 1.0 = none).
    uv_scale: [f32; 2],
    /// UV offset (0.0, 0.0 = none).
    uv_offset: [f32; 2],
    /// 1 = premultiplied source (opacity scales rgb and a); 0 = straight
    /// (opacity scales alpha only).
    premultiplied: u32,
    /// 1 = apply the sRGB transfer on output. See `BlitPipeline::set_srgb_encode`.
    srgb_encode: u32,
    /// Number of integer code intervals at the destination (255 or 1023).
    quantization_levels: f32,
    /// 1 enables deterministic destination-aware RGB dithering.
    dither_enabled: u32,
    /// Transfer: 0 = SDR (sRGB), 1 = HDR10 (PQ), 2 = HLG, 3 = EDR linear.
    /// Matches `blit.wgsl`.
    transfer: u32,
    /// Peak luminance in cd/m² for PQ. Unused otherwise.
    peak_nits: f32,
    /// Pads to a 16-byte multiple for WGSL.
    _padding: [u32; 2],
}

pub struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
    /// Per-draw params, grown by [`Self::ensure_ring_slots`].
    ring_buffer: RefCell<wgpu::Buffer>,
    ring_slots: Cell<usize>,
    /// Byte stride between slots, aligned to the device minimum.
    ring_stride: u64,
}

fn presentation_encoding(
    presentation: &crate::engine::value::render::ResolvedPresentation,
) -> (f32, bool, bool) {
    use crate::engine::value::render::{PresentationDepth, PresentationPixelFormat};

    let adapter_encodes_after_the_blit =
        matches!(presentation.pixel_format, PresentationPixelFormat::P216);
    let quantization_levels = match presentation.resolved {
        PresentationDepth::Sdr8 => 255.0,
        PresentationDepth::Sdr10 => 1023.0,
    };
    // HDR surfaces are never `*Srgb`, so the shader always encodes them.
    let explicit_srgb_encode = !presentation.transfer.is_hdr()
        && presentation.resolved == PresentationDepth::Sdr10
        && !adapter_encodes_after_the_blit;
    // EDR is a float surface with no quantization to dither.
    let dither = presentation.dither
        && presentation.transfer.encodes_a_transfer()
        && !adapter_encodes_after_the_blit;
    (quantization_levels, explicit_srgb_encode, dither)
}

impl BlitPipeline {
    /// Create a blit pipeline with REPLACE blend, for final output.
    ///
    /// # Errors
    ///
    /// Propagates any error from [`Self::with_blend`].
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        Self::with_blend(device, target_format, wgpu::BlendState::REPLACE)
    }

    /// Create a blit pipeline with a specific blend state.
    ///
    /// # Errors
    ///
    /// Never returns `Err`; validation failures surface on the device's error
    /// scope. The `Result` matches the other pipeline constructors.
    pub fn with_blend(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        blend_state: wgpu::BlendState,
    ) -> Result<Self> {
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
                // Params uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<BlitParams>() as u64),
                    },
                    count: None,
                },
            ],
        });

        // Default opacity 1.0.
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blit Params Buffer"),
            contents: bytemuck::cast_slice(&[BlitParams {
                opacity: 1.0,
                rotation: 0,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                premultiplied: 0,
                srgb_encode: 0,
                quantization_levels: 0.0,
                dither_enabled: 0,
                transfer: 0,
                peak_nits: 0.0,
                _padding: [0; 2],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blit Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blit Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(blend_state),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
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
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blit Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Pre-allocate ring buffer for batched per-draw params.
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let param_size = std::mem::size_of::<BlitParams>() as u64;
        let ring_stride = param_size.div_ceil(align) * align;
        let ring_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blit Params Ring Buffer"),
            size: MAX_DRAW_SLOTS * ring_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            params_buffer,
            ring_buffer: RefCell::new(ring_buffer),
            ring_slots: Cell::new(MAX_DRAW_SLOTS as usize),
            ring_stride,
        })
    }

    /// Create a bind group for a texture view using the static `params_buffer`.
    ///
    /// # Panics
    ///
    /// Panics if `size_of::<BlitParams>()` is zero, which cannot happen.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let param_size = std::mem::size_of::<BlitParams>() as u64;
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
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.params_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(param_size).unwrap()),
                    }),
                },
            ],
        })
    }

    /// Set the opacity. Call before render.
    pub fn set_opacity(&self, queue: &wgpu::Queue, opacity: f32) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[BlitParams {
                opacity,
                rotation: 0,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                premultiplied: 0,
                srgb_encode: 0,
                quantization_levels: 0.0,
                dither_enabled: 0,
                transfer: 0,
                peak_nits: 0.0,
                _padding: [0; 2],
            }]),
        );
    }

    /// Set the UV transform for scaling modes.
    pub fn set_uv_transform(
        &self,
        queue: &wgpu::Queue,
        opacity: f32,
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
    ) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[BlitParams {
                opacity,
                rotation: 0,
                uv_scale,
                uv_offset,
                premultiplied: 0,
                srgb_encode: 0,
                quantization_levels: 0.0,
                dither_enabled: 0,
                transfer: 0,
                peak_nits: 0.0,
                _padding: [0; 2],
            }]),
        );
    }

    /// Encode linear to sRGB on output.
    ///
    /// For egui previews, which expect gamma-encoded textures; linear ones show
    /// too dark. The target must be plain `Rgba8Unorm`, not `*UnormSrgb`, or
    /// the hardware decode cancels the encode.
    pub fn set_srgb_encode(&self, queue: &wgpu::Queue, encode: bool) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[BlitParams {
                opacity: 1.0,
                rotation: 0,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                premultiplied: 0,
                srgb_encode: u32::from(encode),
                quantization_levels: 0.0,
                dither_enabled: 0,
                transfer: 0,
                peak_nits: 0.0,
                _padding: [0; 2],
            }]),
        );
    }

    /// Set rotation for the final blit pass (0=0°, 1=90°, 2=180°, 3=270°).
    pub fn set_rotation(&self, queue: &wgpu::Queue, rotation: u32) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[BlitParams {
                opacity: 1.0,
                rotation,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                premultiplied: 0,
                srgb_encode: 0,
                quantization_levels: 0.0,
                dither_enabled: 0,
                transfer: 0,
                peak_nits: 0.0,
                _padding: [0; 2],
            }]),
        );
    }

    /// Configure the final SDR transfer, quantization depth, dither, and rotation.
    pub fn set_presentation(
        &self,
        queue: &wgpu::Queue,
        rotation: u32,
        presentation: &crate::engine::value::render::ResolvedPresentation,
    ) {
        let (quantization_levels, explicit_srgb_encode, dither_enabled) =
            presentation_encoding(presentation);
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[BlitParams {
                opacity: 1.0,
                rotation,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                premultiplied: 0,
                srgb_encode: u32::from(explicit_srgb_encode),
                quantization_levels,
                // NDI P216 dithers Y, U and V in its own conversion pass.
                dither_enabled: u32::from(dither_enabled),
                transfer: match presentation.transfer {
                    crate::engine::value::render::PresentationTransfer::Sdr => 0,
                    crate::engine::value::render::PresentationTransfer::Hdr10Pq => 1,
                    crate::engine::value::render::PresentationTransfer::Hlg => 2,
                    // EDR passes linear values through unencoded.
                    crate::engine::value::render::PresentationTransfer::EdrLinear => 3,
                },
                peak_nits: presentation.peak_nits.map_or(0.0, f32::from),
                _padding: [0; 2],
            }]),
        );
    }

    /// Render a texture to a render pass.
    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Set the opacity and render.
    pub fn render_with_opacity<'a>(
        &'a self,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
        opacity: f32,
    ) {
        self.set_opacity(queue, opacity);
        self.render(render_pass, bind_group);
    }

    /// Grow the params ring buffer to at least `needed` slots. Call once before
    /// a batch: growing mid-batch invalidates earlier bind groups.
    pub fn ensure_ring_slots(&self, device: &wgpu::Device, needed: usize) {
        if needed <= self.ring_slots.get() {
            return;
        }
        let new_slots = needed.next_power_of_two().max(MAX_DRAW_SLOTS as usize);
        *self.ring_buffer.borrow_mut() = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blit Params Ring Buffer"),
            size: new_slots as u64 * self.ring_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.ring_slots.set(new_slots);
    }

    /// Write blit params into a slot of the pre-allocated ring buffer.
    /// Call once per draw before `create_bind_group_for_slot`.
    pub fn write_params_slot(
        &self,
        queue: &wgpu::Queue,
        slot: usize,
        opacity: f32,
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
        premultiplied: bool,
    ) {
        queue.write_buffer(
            &self.ring_buffer.borrow(),
            slot as u64 * self.ring_stride,
            bytemuck::cast_slice(&[BlitParams {
                opacity,
                rotation: 0,
                uv_scale,
                uv_offset,
                premultiplied: u32::from(premultiplied),
                srgb_encode: 0,
                quantization_levels: 0.0,
                dither_enabled: 0,
                transfer: 0,
                peak_nits: 0.0,
                _padding: [0; 2],
            }]),
        );
    }

    /// Create a bind group for a specific ring buffer slot.
    /// The slot offset is baked in; no dynamic offset.
    ///
    /// # Panics
    ///
    /// Panics if `size_of::<BlitParams>()` is zero, which cannot happen.
    pub fn create_ring_bind_group(
        &self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
        slot: usize,
    ) -> wgpu::BindGroup {
        let param_size = std::mem::size_of::<BlitParams>() as u64;
        let offset = slot as u64 * self.ring_stride;
        let ring = self.ring_buffer.borrow();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blit Bind Group (ring)"),
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
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &ring,
                        offset,
                        size: Some(NonZeroU64::new(param_size).unwrap()),
                    }),
                },
            ],
        })
    }

    /// Render using a ring buffer slot's bind group.
    /// The bind group must have been created with `create_ring_bind_group`.
    pub fn render_at_slot<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

// === Composite blend pipeline ===

/// Composite blend parameters, 32 bytes. Matches `CompositeParams` in composite.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeParams {
    opacity: f32,
    blend_mode: u32,
    uv_scale: [f32; 2],
    uv_offset: [f32; 2],
    /// 1 = premultiplied source, un-premultiplied before the blend math; 0 = straight.
    premultiplied: u32,
    _pad: f32,
}

/// Shader compositing: reads source and destination and blends per pixel. The
/// blend mode is a uniform integer.
pub struct CompositeBlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
    /// Per-draw params, grown by [`Self::ensure_ring_slots`].
    ring_buffer: RefCell<wgpu::Buffer>,
    ring_slots: Cell<usize>,
    /// Byte stride between slots, aligned to the device minimum.
    ring_stride: u64,
}

impl CompositeBlitPipeline {
    /// Create a composite blend pipeline.
    ///
    /// # Errors
    ///
    /// Never returns `Err`; validation failures surface on the device's error
    /// scope. The `Result` matches the other pipeline constructors.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Composite Bind Group Layout"),
            entries: &[
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Source texture (layer being composited)
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
                // Destination texture (composite-so-far snapshot)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Params uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<CompositeParams>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Composite Params Buffer"),
            contents: bytemuck::cast_slice(&[CompositeParams {
                opacity: 1.0,
                blend_mode: 0,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                premultiplied: 0,
                _pad: 0.0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Composite Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Composite Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Composite Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Composite Blend Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
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
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Composite Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Pre-allocate ring buffer for batched per-draw params.
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let param_size = std::mem::size_of::<CompositeParams>() as u64;
        let ring_stride = param_size.div_ceil(align) * align;
        let ring_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Composite Params Ring Buffer"),
            size: MAX_DRAW_SLOTS * ring_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            params_buffer,
            ring_buffer: RefCell::new(ring_buffer),
            ring_slots: Cell::new(MAX_DRAW_SLOTS as usize),
            ring_stride,
        })
    }

    /// Update blend parameters.
    pub fn set_params(
        &self,
        queue: &wgpu::Queue,
        opacity: f32,
        blend_mode: u32,
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
    ) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[CompositeParams {
                opacity,
                blend_mode,
                uv_scale,
                uv_offset,
                premultiplied: 0,
                _pad: 0.0,
            }]),
        );
    }

    /// Create a bind group for compositing source onto destination.
    ///
    /// # Panics
    ///
    /// Panics if `size_of::<CompositeParams>()` is zero, which cannot happen.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        source_view: &wgpu::TextureView,
        dest_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let param_size = std::mem::size_of::<CompositeParams>() as u64;
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Composite Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(dest_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.params_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(param_size).unwrap()),
                    }),
                },
            ],
        })
    }

    /// Draw a fullscreen quad with the composite shader.
    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Grow the params ring buffer to at least `needed` slots. Call once before
    /// a batch: growing mid-batch invalidates earlier bind groups.
    pub fn ensure_ring_slots(&self, device: &wgpu::Device, needed: usize) {
        if needed <= self.ring_slots.get() {
            return;
        }
        let new_slots = needed.next_power_of_two().max(MAX_DRAW_SLOTS as usize);
        *self.ring_buffer.borrow_mut() = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Composite Params Ring Buffer"),
            size: new_slots as u64 * self.ring_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.ring_slots.set(new_slots);
    }

    /// Write composite params into a slot of the pre-allocated ring buffer.
    /// Call once per draw before `create_bind_group_for_slot`.
    #[allow(clippy::too_many_arguments)]
    pub fn write_params_slot(
        &self,
        queue: &wgpu::Queue,
        slot: usize,
        opacity: f32,
        blend_mode: u32,
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
        premultiplied: bool,
    ) {
        queue.write_buffer(
            &self.ring_buffer.borrow(),
            slot as u64 * self.ring_stride,
            bytemuck::cast_slice(&[CompositeParams {
                opacity,
                blend_mode,
                uv_scale,
                uv_offset,
                premultiplied: u32::from(premultiplied),
                _pad: 0.0,
            }]),
        );
    }

    /// Create a bind group for a specific ring buffer slot.
    /// The slot offset is baked in; no dynamic offset.
    ///
    /// # Panics
    ///
    /// Panics if `size_of::<CompositeParams>()` is zero, which cannot happen.
    pub fn create_ring_bind_group(
        &self,
        device: &wgpu::Device,
        source_view: &wgpu::TextureView,
        dest_view: &wgpu::TextureView,
        slot: usize,
    ) -> wgpu::BindGroup {
        let param_size = std::mem::size_of::<CompositeParams>() as u64;
        let offset = slot as u64 * self.ring_stride;
        let ring = self.ring_buffer.borrow();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Composite Bind Group (ring)"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(dest_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &ring,
                        offset,
                        size: Some(NonZeroU64::new(param_size).unwrap()),
                    }),
                },
            ],
        })
    }

    /// Render using a ring buffer slot's bind group.
    /// The bind group must have been created with `create_ring_bind_group`.
    pub fn render_at_slot<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

// === Polygon rendering pipeline ===

/// Polygon pipeline params: UV transform, warp homography, and overlap zones.
/// Matches `PolygonParams` in polygon.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PolygonParams {
    opacity: f32,
    _pad: f32,
    uv_scale: [f32; 2],
    uv_offset: [f32; 2],
    _pad2: [f32; 2],
    // 3x3 homography as 3 × vec4 (w = 0 padding).
    h_row0: [f32; 4],
    h_row1: [f32; 4],
    h_row2: [f32; 4],
    // Overlap zone count (f32 for alignment) + padding.
    zone_count: f32,
    _zone_pad: [f32; 3],
    // Up to 4 zones: [u_min, v_min, u_max, v_max] + [gamma, _pad, _pad, _pad].
    zone0_rect: [f32; 4],
    zone0_cfg: [f32; 4],
    zone1_rect: [f32; 4],
    zone1_cfg: [f32; 4],
    zone2_rect: [f32; 4],
    zone2_cfg: [f32; 4],
    zone3_rect: [f32; 4],
    zone3_cfg: [f32; 4],
}

impl PolygonParams {
    fn identity_homography() -> [[f32; 4]; 3] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]
    }

    /// Build params from a surface's UV transform, optional homography warp,
    /// and overlap zones. `homography` is a 3×3 matrix packed as 12 floats
    /// (3 rows × 4, w padding); `None` uses identity.
    fn build(
        uv_scale: [f32; 2],
        uv_offset: [f32; 2],
        homography: Option<&[f32; 12]>,
        overlap_zones: &SurfaceOverlapZones,
    ) -> Self {
        let h = homography.copied().unwrap_or_else(|| {
            let id = Self::identity_homography();
            [
                id[0][0], id[0][1], id[0][2], id[0][3], id[1][0], id[1][1], id[1][2], id[1][3],
                id[2][0], id[2][1], id[2][2], id[2][3],
            ]
        });

        let z = |i: usize| -> ([f32; 4], [f32; 4]) {
            if let Some(zone) = overlap_zones.zones.get(i) {
                (zone.uv_rect, [zone.gamma, zone.ramp_x, zone.ramp_y, 0.0])
            } else {
                ([0.0; 4], [0.0; 4])
            }
        };
        let (z0r, z0c) = z(0);
        let (z1r, z1c) = z(1);
        let (z2r, z2c) = z(2);
        let (z3r, z3c) = z(3);

        Self {
            opacity: 1.0,
            _pad: 0.0,
            uv_scale,
            uv_offset,
            _pad2: [0.0, 0.0],
            h_row0: [h[0], h[1], h[2], h[3]],
            h_row1: [h[4], h[5], h[6], h[7]],
            h_row2: [h[8], h[9], h[10], h[11]],
            zone_count: overlap_zones.zones.len().min(4) as f32,
            _zone_pad: [0.0; 3],
            zone0_rect: z0r,
            zone0_cfg: z0c,
            zone1_rect: z1r,
            zone1_cfg: z1c,
            zone2_rect: z2r,
            zone2_cfg: z2c,
            zone3_rect: z3r,
            zone3_cfg: z3c,
        }
    }
}

/// Polygon vertex: position and UV.
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

/// Initial vertex-pool capacity in bytes (≈1024 polygon vertices).
const POLYGON_INITIAL_VERTEX_BYTES: u64 = 1024 * std::mem::size_of::<PolygonVertex>() as u64;

/// Frames the persistent pools rotate through, so the CPU writes the next frame
/// while the GPU still reads the previous one (no write-after-read stall).
const POLYGON_FRAMES_IN_FLIGHT: usize = 3;

/// A surface ready to draw from the shared pools. Produced by
/// [`PolygonBlitPipeline::prepare`], consumed by [`PolygonBlitPipeline::draw`].
pub struct PreparedPolygon {
    bind_group: wgpu::BindGroup,
    /// Byte offset of this surface's vertices within the shared vertex pool.
    vertex_offset: u64,
    /// Byte length of this surface's vertices.
    vertex_bytes: u64,
    num_triangles: u32,
}

/// Per-surface input to [`PolygonBlitPipeline::prepare`]. `vertices` come from
/// [`PolygonBlitPipeline::triangulate_verts`] or [`PolygonBlitPipeline::mesh_verts`].
pub struct PolygonDrawDesc<'a> {
    pub content_view: &'a wgpu::TextureView,
    pub uv_scale: [f32; 2],
    pub uv_offset: [f32; 2],
    pub homography: Option<[f32; 12]>,
    pub overlap_zones: &'a SurfaceOverlapZones,
    pub vertices: Vec<PolygonVertex>,
    /// Surface uuid; cache key for its baked hole mask.
    pub mask_uuid: &'a str,
    /// Hole contours in surface UV space (`[0..1]²`). Empty binds the 1×1
    /// white default mask.
    pub mask_uv_contours: Vec<Vec<[f32; 2]>>,
}

/// A baked hole coverage mask. `hash` fingerprints the surface's UV-space hole
/// contours; the mask rebakes only when it changes. `_texture` keeps `view` alive.
struct CachedMask {
    hash: u64,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Fingerprint uv-space hole contours so a mask rebakes only when they change.
fn hash_uv_contours(contours: &[Vec<[f32; 2]>]) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in contours {
        h.write_usize(c.len());
        for p in c {
            h.write_u32(p[0].to_bits());
            h.write_u32(p[1].to_bits());
        }
    }
    h.finish()
}

/// Upload an `R8Unorm` coverage mask, returning the texture and its view.
fn upload_mask_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    res: u32,
    bytes: &[u8],
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: res,
        height: res,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Surface Hole Mask"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(res),
            rows_per_image: Some(res),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Renders textured polygon surfaces.
///
/// Per-surface params and vertices go into persistent, growable pools instead
/// of per-frame buffers, which exhausted memory on low-VRAM Metal devices. The
/// pools rotate over [`POLYGON_FRAMES_IN_FLIGHT`] frames.
pub struct PolygonBlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Params ring buffers, one per frame in flight, each grown to fit.
    ring_buffers: [RefCell<wgpu::Buffer>; POLYGON_FRAMES_IN_FLIGHT],
    /// Byte stride between ring slots, aligned to the device minimum.
    ring_stride: u64,
    /// Slots currently allocated in each `ring_buffers` entry.
    ring_slots: [Cell<usize>; POLYGON_FRAMES_IN_FLIGHT],
    /// Vertex pools, one per frame in flight, each grown to fit.
    vertex_buffers: [RefCell<wgpu::Buffer>; POLYGON_FRAMES_IN_FLIGHT],
    /// Capacity in bytes of each `vertex_buffers` entry.
    vertex_capacity: [Cell<u64>; POLYGON_FRAMES_IN_FLIGHT],
    /// Index of the pool set the next `prepare` will write, advanced each frame.
    frame_cursor: Cell<usize>,
    /// Reused staging for a frame's params, uploaded with one `write_buffer`.
    scratch_params: RefCell<Vec<u8>>,
    /// Reused staging for a frame's vertices, uploaded with one `write_buffer`.
    scratch_verts: RefCell<Vec<PolygonVertex>>,
    /// Baked hole masks keyed by surface uuid.
    mask_cache: RefCell<HashMap<String, CachedMask>>,
    /// Lazily built 1×1 white mask for surfaces without holes.
    default_mask: RefCell<Option<(wgpu::Texture, wgpu::TextureView)>>,
}

impl PolygonBlitPipeline {
    /// Create the polygon warp/blit pipeline.
    ///
    /// # Errors
    ///
    /// Never returns `Err`; validation failures surface on the device's error
    /// scope. The `Result` matches the other pipeline constructors.
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
                // Hole coverage mask.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Polygon Blit Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Vertex + fragment shader with homography.
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
                buffers: &[Some(PolygonVertex::LAYOUT)],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
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
            multiview_mask: None,
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

        // One ring and vertex pool per frame in flight, grown on demand.
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let param_size = std::mem::size_of::<PolygonParams>() as u64;
        let ring_stride = param_size.div_ceil(align) * align;
        let initial_slots = MAX_DRAW_SLOTS as usize;
        let ring_buffers = std::array::from_fn(|i| {
            RefCell::new(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Polygon Params Ring Buffer {i}")),
                size: initial_slots as u64 * ring_stride,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }))
        });
        let vertex_buffers = std::array::from_fn(|i| {
            RefCell::new(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Polygon Vertex Pool {i}")),
                size: POLYGON_INITIAL_VERTEX_BYTES,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }))
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            ring_buffers,
            ring_stride,
            ring_slots: std::array::from_fn(|_| Cell::new(initial_slots)),
            vertex_buffers,
            vertex_capacity: std::array::from_fn(|_| Cell::new(POLYGON_INITIAL_VERTEX_BYTES)),
            frame_cursor: Cell::new(0),
            scratch_params: RefCell::new(Vec::new()),
            scratch_verts: RefCell::new(Vec::new()),
            mask_cache: RefCell::new(HashMap::new()),
            default_mask: RefCell::new(None),
        })
    }

    /// Bake or refresh the hole coverage mask for each drawn surface that has
    /// holes. Cached by uuid; rebakes only when the uv-contour hash changes.
    fn ensure_masks(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draws: &[PolygonDrawDesc<'_>],
    ) {
        let mut cache = self.mask_cache.borrow_mut();
        for d in draws {
            if d.mask_uv_contours.is_empty() {
                continue;
            }
            let hash = hash_uv_contours(&d.mask_uv_contours);
            let fresh = cache.get(d.mask_uuid).is_some_and(|m| m.hash == hash);
            if fresh {
                continue;
            }
            let res = DEFAULT_MASK_RES;
            let bytes = bake_hole_mask(&d.mask_uv_contours, res);
            let (texture, view) = upload_mask_texture(device, queue, res, &bytes);
            cache.insert(
                d.mask_uuid.to_string(),
                CachedMask {
                    hash,
                    _texture: texture,
                    view,
                },
            );
        }
    }

    /// Lazily build the 1×1 white mask for surfaces without holes.
    fn ensure_default_mask(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let mut slot = self.default_mask.borrow_mut();
        if slot.is_none() {
            *slot = Some(upload_mask_texture(device, queue, 1, &[255u8]));
        }
    }

    /// Grow frame `idx`'s params ring buffer so it holds at least `needed` slots.
    fn ensure_ring_slots(&self, device: &wgpu::Device, idx: usize, needed: usize) {
        if needed <= self.ring_slots[idx].get() {
            return;
        }
        let new_slots = needed.next_power_of_two().max(MAX_DRAW_SLOTS as usize);
        *self.ring_buffers[idx].borrow_mut() = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("Polygon Params Ring Buffer {idx}")),
            size: new_slots as u64 * self.ring_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.ring_slots[idx].set(new_slots);
    }

    /// Grow frame `idx`'s vertex pool so it holds at least `needed_bytes`.
    fn ensure_vertex_capacity(&self, device: &wgpu::Device, idx: usize, needed_bytes: u64) {
        if needed_bytes <= self.vertex_capacity[idx].get() {
            return;
        }
        let new_cap = needed_bytes
            .next_power_of_two()
            .max(POLYGON_INITIAL_VERTEX_BYTES);
        *self.vertex_buffers[idx].borrow_mut() = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("Polygon Vertex Pool {idx}")),
            size: new_cap,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.vertex_capacity[idx].set(new_cap);
    }

    /// Prepare a batch of surfaces for drawing this frame.
    ///
    /// Writes params and vertices into this frame's pools and builds a bind
    /// group per surface. Returns the prepared surfaces and the vertex pool
    /// handle, which the caller holds across the render pass and passes to
    /// [`Self::draw`].
    ///
    /// # Panics
    ///
    /// Panics if the 1×1 default coverage mask is missing after
    /// `ensure_default_mask`, or if `size_of::<PolygonParams>()` is zero.
    /// Neither is reachable.
    pub fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draws: &[PolygonDrawDesc<'_>],
    ) -> (Vec<PreparedPolygon>, wgpu::Buffer) {
        let n = draws.len();
        // Rotate to the next pool set.
        let idx = self.frame_cursor.get();
        self.frame_cursor.set((idx + 1) % POLYGON_FRAMES_IN_FLIGHT);
        self.ensure_ring_slots(device, idx, n);
        let vertex_size = std::mem::size_of::<PolygonVertex>();
        let total_verts: usize = draws.iter().map(|d| d.vertices.len()).sum();
        self.ensure_vertex_capacity(device, idx, (total_verts * vertex_size) as u64);

        let param_size = std::mem::size_of::<PolygonParams>();
        let stride = self.ring_stride as usize;

        // Stage all params and all vertices so the frame uploads with one
        // write_buffer each, not two per surface.
        let mut params_blob = self.scratch_params.borrow_mut();
        let mut verts_blob = self.scratch_verts.borrow_mut();
        params_blob.clear();
        params_blob.resize(n * stride, 0);
        verts_blob.clear();
        verts_blob.reserve(total_verts);

        let mut meta: Vec<(u64, u64, u32)> = Vec::with_capacity(n);
        let mut vertex_offset = 0u64;
        for (slot, d) in draws.iter().enumerate() {
            let params = PolygonParams::build(
                d.uv_scale,
                d.uv_offset,
                d.homography.as_ref(),
                d.overlap_zones,
            );
            let off = slot * stride;
            params_blob[off..off + param_size].copy_from_slice(bytemuck::bytes_of(&params));

            let vertex_bytes = (d.vertices.len() * vertex_size) as u64;
            verts_blob.extend_from_slice(&d.vertices);
            meta.push((vertex_offset, vertex_bytes, (d.vertices.len() / 3) as u32));
            vertex_offset += vertex_bytes;
        }

        let ring = self.ring_buffers[idx].borrow();
        let vpool = self.vertex_buffers[idx].borrow();
        if n > 0 {
            queue.write_buffer(&ring, 0, &params_blob);
        }
        if !verts_blob.is_empty() {
            queue.write_buffer(&vpool, 0, bytemuck::cast_slice(&verts_blob));
        }

        // Rebakes only on contour change.
        self.ensure_masks(device, queue, draws);
        self.ensure_default_mask(device, queue);
        let mask_cache = self.mask_cache.borrow();
        let default_mask = self.default_mask.borrow();
        let default_view = &default_mask.as_ref().expect("default mask initialized").1;

        let param_size = param_size as u64;
        let mut prepared = Vec::with_capacity(n);
        for (slot, (d, &(v_off, v_bytes, num_tris))) in draws.iter().zip(meta.iter()).enumerate() {
            let mask_view = if d.mask_uv_contours.is_empty() {
                default_view
            } else {
                mask_cache
                    .get(d.mask_uuid)
                    .map_or(default_view, |m| &m.view)
            };
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Polygon Blit Bind Group (ring)"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(d.content_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &ring,
                            offset: slot as u64 * self.ring_stride,
                            size: Some(NonZeroU64::new(param_size).unwrap()),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(mask_view),
                    },
                ],
            });

            prepared.push(PreparedPolygon {
                bind_group,
                vertex_offset: v_off,
                vertex_bytes: v_bytes,
                num_triangles: num_tris,
            });
        }

        let vbuf = vpool.clone();
        (prepared, vbuf)
    }

    /// Draw a prepared surface batch into `render_pass`.
    ///
    /// `vertex_buffer` must be the pool handle returned by [`Self::prepare`];
    /// hold it across the render pass so its slices stay valid.
    pub fn draw<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        prepared: &'a [PreparedPolygon],
        vertex_buffer: &'a wgpu::Buffer,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        for p in prepared {
            if p.num_triangles == 0 {
                continue;
            }
            render_pass.set_bind_group(0, &p.bind_group, &[]);
            render_pass.set_vertex_buffer(
                0,
                vertex_buffer.slice(p.vertex_offset..p.vertex_offset + p.vertex_bytes),
            );
            render_pass.draw(0..p.num_triangles * 3, 0..1);
        }
    }

    /// Ear-clip triangulate a polygon (concave allowed) into vertices.
    ///
    /// UVs map the bounding box to [0..1]; the shader's `uv_scale/uv_offset`
    /// do the rest. Returns an empty vec for fewer than 3 vertices.
    pub fn triangulate_verts(
        canvas_verts: &[[f32; 2]],
        bb_x: f32,
        bb_y: f32,
        bb_w: f32,
        bb_h: f32,
    ) -> Vec<PolygonVertex> {
        if canvas_verts.len() < 3 {
            return Vec::new();
        }

        let indices = ear_clip_triangulate(canvas_verts);

        let to_vert = |v: &[f32; 2]| -> PolygonVertex {
            let u = if bb_w > 0.0 {
                (v[0] - bb_x) / bb_w
            } else {
                0.0
            };
            let t = if bb_h > 0.0 {
                (v[1] - bb_y) / bb_h
            } else {
                0.0
            };
            PolygonVertex {
                position: *v,
                uv: [u, t],
            }
        };

        let mut verts: Vec<PolygonVertex> = Vec::with_capacity(indices.len());
        for &idx in &indices {
            verts.push(to_vert(&canvas_verts[idx as usize]));
        }
        verts
    }

    /// Triangulate a combined (multi-contour) surface against its shared
    /// bounding box into one triangle list, so one draw renders every piece.
    /// Used instead of the warp path, which can't represent disjoint contours.
    pub fn triangulate_multi(
        primary: &[[f32; 2]],
        extras: &[Vec<[f32; 2]>],
        bb_x: f32,
        bb_y: f32,
        bb_w: f32,
        bb_h: f32,
    ) -> Vec<PolygonVertex> {
        let mut verts = Self::triangulate_verts(primary, bb_x, bb_y, bb_w, bb_h);
        for contour in extras {
            verts.extend(Self::triangulate_verts(contour, bb_x, bb_y, bb_w, bb_h));
        }
        verts
    }

    /// Build CPU-side vertices from a UV warp mesh grid.
    ///
    /// Each grid cell becomes 2 triangles; positions are output space, UVs
    /// source space. Use an identity homography with mesh warp. Returns an
    /// empty vec for an invalid mesh.
    pub fn mesh_verts(mesh: &crate::surface::warp::WarpMesh) -> Vec<PolygonVertex> {
        let cols = mesh.cols as usize;
        let rows = mesh.rows as usize;
        if cols < 2 || rows < 2 || mesh.points.len() != cols * rows {
            log::warn!(
                "Invalid mesh: cols={cols}, rows={rows}, points={} (expected {}). Returning empty mesh.",
                mesh.points.len(),
                cols * rows
            );
            return Vec::new();
        }

        let num_cells = (cols - 1) * (rows - 1);
        let mut verts: Vec<PolygonVertex> = Vec::with_capacity(num_cells * 6);

        for r in 0..(rows - 1) {
            for c in 0..(cols - 1) {
                let tl = &mesh.points[r * cols + c];
                let tr = &mesh.points[r * cols + c + 1];
                let bl = &mesh.points[(r + 1) * cols + c];
                let br = &mesh.points[(r + 1) * cols + c + 1];

                // Positions stay in output space [0..1]; the vertex shader
                // converts to NDC, as for `triangulate_verts`.
                let to_vert = |p: &crate::surface::warp::MeshPoint| -> PolygonVertex {
                    PolygonVertex {
                        position: p.position,
                        uv: p.uv,
                    }
                };

                // Triangle 1: TL, TR, BL
                verts.push(to_vert(tl));
                verts.push(to_vert(tr));
                verts.push(to_vert(bl));

                // Triangle 2: TR, BR, BL
                verts.push(to_vert(tr));
                verts.push(to_vert(br));
                verts.push(to_vert(bl));
            }
        }
        verts
    }
}

// === Ear-clipping triangulation for concave polygons ===

/// Triangle indices for a simple (non-self-intersecting) polygon.
fn ear_clip_triangulate(verts: &[[f32; 2]]) -> Vec<u32> {
    let n = verts.len();
    if n < 3 {
        return Vec::new();
    }

    let mut idx: Vec<usize> = (0..n).collect();
    let mut result = Vec::with_capacity((n - 2) * 3);

    // Winding from signed area (y-down: negative = CCW).
    let signed_area: f32 = (0..n)
        .map(|i| {
            let a = verts[i];
            let b = verts[(i + 1) % n];
            (b[0] - a[0]) * (b[1] + a[1])
        })
        .sum();
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
            if i >= remaining && remaining > 0 {
                i = 0;
            }
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

fn ear_clip_is_ear(
    verts: &[[f32; 2]],
    idx: &[usize],
    prev: usize,
    curr: usize,
    next: usize,
    ccw: bool,
) -> bool {
    let cross = ear_clip_cross(verts[prev], verts[curr], verts[next]);
    if ccw {
        if cross <= 0.0 {
            return false;
        }
    } else if cross >= 0.0 {
        return false;
    }

    for &vi in idx {
        if vi == prev || vi == curr || vi == next {
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::context::GpuContext;

    const QUAD: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    /// Every emitted index must reference a real vertex, in whole triangles.
    fn assert_valid_indices(indices: &[u32], n: usize) {
        assert_eq!(indices.len() % 3, 0, "indices must form whole triangles");
        for &i in indices {
            assert!((i as usize) < n, "index {i} out of range for {n} verts");
        }
    }

    #[test]
    fn presentation_blit_shader_builds_for_eight_and_ten_bit_targets() {
        let Some(ctx) = crate::testing::headless_gpu() else {
            return;
        };
        for format in [
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgb10a2Unorm,
        ] {
            BlitPipeline::new(&ctx.device, format).expect("presentation blit pipeline");
        }
    }

    #[test]
    fn encoder_native_rgb10_gets_srgb_encoding_and_dither() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            pixel_format: PresentationPixelFormat::EncoderNative("yuv420p10le".into()),
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
            ..ResolvedPresentation::default()
        };

        assert_eq!(presentation_encoding(&presentation), (1023.0, true, true));
    }

    #[test]
    fn eight_bit_presentation_quantizes_to_255_and_can_dither() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr8,
            resolved: PresentationDepth::Sdr8,
            pixel_format: PresentationPixelFormat::Bgra8,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Premultiplied,
            dither: true,
            fallback_reason: None,
            ..ResolvedPresentation::default()
        };
        assert_eq!(presentation_encoding(&presentation), (255.0, false, true));
    }

    #[test]
    fn rgb10_presentation_encodes_and_dithers_in_the_blit() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
            ..ResolvedPresentation::default()
        };
        assert_eq!(presentation_encoding(&presentation), (1023.0, true, true));
    }

    #[test]
    fn p216_defers_transfer_and_dither_to_its_conversion_pass() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            pixel_format: PresentationPixelFormat::P216,
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
            ..ResolvedPresentation::default()
        };

        assert_eq!(presentation_encoding(&presentation), (1023.0, false, false));
    }

    #[test]
    fn rgba16_recording_encodes_rgb_but_never_dithers_alpha() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            pixel_format: PresentationPixelFormat::Rgba16,
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode: AlphaMode::Straight,
            dither: true,
            fallback_reason: None,
            ..ResolvedPresentation::default()
        };

        assert_eq!(presentation_encoding(&presentation), (1023.0, true, true));
    }

    #[test]
    fn rgba16_recording_target_preserves_known_rgb_codes_and_alpha_ramp() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };

        const WIDTH: u32 = 4;
        const ROW_BYTES: u32 = 256;
        let Some(ctx) = crate::testing::headless_gpu() else {
            return;
        };
        let required_features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        if !ctx.device.features().contains(required_features) {
            return;
        }
        let pipeline = BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgba16Unorm).unwrap();
        let source = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGBA16 Code Source"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut source_words = Vec::with_capacity(WIDTH as usize * 4);
        for x in 0..WIDTH {
            source_words.extend_from_slice(&[
                half::f16::ZERO.to_bits(),
                half::f16::from_f32(0.214_041_14).to_bits(),
                half::f16::ONE.to_bits(),
                half::f16::from_f32(x as f32 / (WIDTH - 1) as f32).to_bits(),
            ]);
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&source_words),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGBA16 Code Target"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = pipeline.create_bind_group(&ctx.device, &source_view);
        pipeline.set_presentation(
            &ctx.queue,
            0,
            &ResolvedPresentation {
                requested: PresentationDepth::Sdr10,
                resolved: PresentationDepth::Sdr10,
                pixel_format: PresentationPixelFormat::Rgba16,
                color_profile: PresentationColorProfile::Rec709Limited,
                alpha_mode: AlphaMode::Straight,
                dither: false,
                fallback_reason: None,
                ..ResolvedPresentation::default()
            },
        );
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RGBA16 Code Readback"),
            size: u64::from(ROW_BYTES),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("RGBA16 Code Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RGBA16 Code Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
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
                multiview_mask: None,
            });
            pipeline.render(&mut pass, &bind_group);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let words = bytemuck::cast_slice::<u8, u16>(&mapped[..(WIDTH * 8) as usize]);

        for (x, pixel) in words.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(pixel[0], 0);
            assert!(pixel[1].abs_diff(32_768) < 64, "green code {}", pixel[1]);
            assert_eq!(pixel[2], u16::MAX);
            let expected_alpha =
                ((x as f32 / (WIDTH - 1) as f32) * f32::from(u16::MAX)).round() as u16;
            assert!(pixel[3].abs_diff(expected_alpha) < 64);
        }
    }

    #[test]
    fn rgb10_presentation_preserves_more_than_eight_bits_of_a_gradient() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };

        const WIDTH: u32 = 1024;
        let Some(ctx) = crate::testing::headless_gpu() else {
            return;
        };
        let Ok(pipeline) = BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgb10a2Unorm) else {
            return;
        };
        let source = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGB10 Gradient Source"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut source_words = Vec::with_capacity(WIDTH as usize * 4);
        for x in 0..WIDTH {
            let value = half::f16::from_f32(x as f32 / (WIDTH - 1) as f32).to_bits();
            source_words.extend_from_slice(&[value, value, value, half::f16::ONE.to_bits()]);
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&source_words),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGB10 Gradient Target"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = pipeline.create_bind_group(&ctx.device, &source_view);
        pipeline.set_presentation(
            &ctx.queue,
            0,
            &ResolvedPresentation {
                requested: PresentationDepth::Sdr10,
                resolved: PresentationDepth::Sdr10,
                pixel_format: PresentationPixelFormat::Rgb10A2,
                color_profile: PresentationColorProfile::SrgbFull,
                alpha_mode: AlphaMode::Opaque,
                dither: false,
                fallback_reason: None,
                ..ResolvedPresentation::default()
            },
        );
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RGB10 Gradient Readback"),
            size: u64::from(WIDTH * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("RGB10 Gradient Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RGB10 Gradient Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
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
                multiview_mask: None,
            });
            pipeline.render(&mut pass, &bind_group);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH * 4),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let unique_red_codes = bytemuck::cast_slice::<u8, u32>(&mapped)
            .iter()
            .map(|packed| packed & 0x3ff)
            .collect::<std::collections::HashSet<_>>();

        assert!(
            unique_red_codes.len() > 256,
            "RGB10 output retained only {} distinct red codes",
            unique_red_codes.len()
        );
    }

    fn blit_rgb10_constant(
        ctx: &GpuContext,
        pipeline: &BlitPipeline,
        width: u32,
        linear: f32,
        dither: bool,
    ) -> Vec<u32> {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let row_bytes = width
            .saturating_mul(4)
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .saturating_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let source = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGB10 Dither Source"),
            size: wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut source_words = Vec::with_capacity(width as usize * 4);
        let value = half::f16::from_f32(linear).to_bits();
        for _ in 0..width {
            source_words.extend_from_slice(&[value, value, value, half::f16::ONE.to_bits()]);
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&source_words),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGB10 Dither Target"),
            size: wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = pipeline.create_bind_group(&ctx.device, &source_view);
        pipeline.set_presentation(
            &ctx.queue,
            0,
            &ResolvedPresentation {
                requested: PresentationDepth::Sdr10,
                resolved: PresentationDepth::Sdr10,
                pixel_format: PresentationPixelFormat::Rgb10A2,
                color_profile: PresentationColorProfile::SrgbFull,
                alpha_mode: AlphaMode::Opaque,
                dither,
                fallback_reason: None,
                ..ResolvedPresentation::default()
            },
        );
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RGB10 Dither Readback"),
            size: u64::from(row_bytes),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("RGB10 Dither Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RGB10 Dither Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
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
                multiview_mask: None,
            });
            pipeline.render(&mut pass, &bind_group);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        bytemuck::cast_slice::<u8, u32>(&mapped[..(width * 4) as usize]).to_vec()
    }

    #[test]
    fn rgb10_dither_is_stable_and_changes_codes_versus_off() {
        const WIDTH: u32 = 32;
        let Some(ctx) = crate::testing::headless_gpu() else {
            return;
        };
        let Ok(pipeline) = BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgb10a2Unorm) else {
            return;
        };
        let undithered = blit_rgb10_constant(&ctx, &pipeline, WIDTH, 0.42, false);
        let first = blit_rgb10_constant(&ctx, &pipeline, WIDTH, 0.42, true);
        let second = blit_rgb10_constant(&ctx, &pipeline, WIDTH, 0.42, true);
        assert_eq!(first, second, "destination-aware dither must be stable");
        assert_ne!(
            undithered, first,
            "dither must change at least one destination code"
        );
        let undithered_red = undithered[0] & 0x3ff;
        for packed in &first {
            let red = packed & 0x3ff;
            assert!(
                red.abs_diff(undithered_red) <= 1,
                "dither amplitude exceeded one 10-bit LSB: {red} vs {undithered_red}"
            );
        }
    }

    #[test]
    fn ear_clip_triangulate_degenerate_is_empty() {
        assert_eq!(ear_clip_triangulate(&[]).len(), 0);
        assert_eq!(ear_clip_triangulate(&[[0.0, 0.0]]).len(), 0);
        assert_eq!(ear_clip_triangulate(&[[0.0, 0.0], [1.0, 0.0]]).len(), 0);
    }

    #[test]
    fn ear_clip_triangulate_convex_quad_is_two_triangles() {
        let indices = ear_clip_triangulate(&QUAD);
        assert_eq!(indices.len(), 6, "a quad is two triangles");
        assert_valid_indices(&indices, QUAD.len());
    }

    #[test]
    fn ear_clip_triangulate_concave_polygon_is_fully_triangulated() {
        // Concave L-shape: still n-2 valid triangles.
        let l_shape = [
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ];
        let indices = ear_clip_triangulate(&l_shape);
        assert_eq!(indices.len(), (l_shape.len() - 2) * 3);
        assert_valid_indices(&indices, l_shape.len());
    }

    #[test]
    fn ear_clip_cross_sign_tracks_turn_direction() {
        let o = [0.0, 0.0];
        let a = [1.0, 0.0];
        assert!(ear_clip_cross(o, a, [1.0, 1.0]) > 0.0);
        assert!(ear_clip_cross(o, a, [1.0, -1.0]) < 0.0);
        assert_eq!(ear_clip_cross(o, a, [2.0, 0.0]), 0.0);
    }

    #[test]
    fn ear_clip_point_in_tri_detects_inside_outside_edge() {
        let a = [0.0, 0.0];
        let b = [4.0, 0.0];
        let c = [0.0, 4.0];
        assert!(ear_clip_point_in_tri([1.0, 1.0], a, b, c), "interior");
        assert!(!ear_clip_point_in_tri([3.0, 3.0], a, b, c), "exterior");
        assert!(ear_clip_point_in_tri([2.0, 0.0], a, b, c), "on edge");
    }

    #[test]
    fn ear_clip_is_ear_accepts_convex_and_rejects_enclosing_corner() {
        // QUAD winds CCW under the signed-area convention (y-down) -> ccw = true.
        let idx = [0usize, 1, 2, 3];
        assert!(ear_clip_is_ear(&QUAD, &idx, 0, 1, 2, true));

        // Concave quad: the corner's triangle swallows the reflex vertex 2, so
        // it is not a valid ear.
        let concave = [[0.0, 0.0], [2.0, 1.0], [1.0, 1.0], [0.0, 2.0]];
        let cidx = [0usize, 1, 2, 3];
        assert!(!ear_clip_is_ear(&concave, &cidx, 3, 0, 1, true));
    }

    /// UVs are the vertex position normalized within the bounding box.
    #[test]
    fn triangulate_verts_maps_uv_within_bounding_box() {
        let verts = PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 2.0, 2.0);
        assert_eq!(verts.len(), 6);
        for v in &verts {
            assert!(
                (0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]),
                "uv must fall within the bounding box, got {:?}",
                v.uv
            );
        }
        // The far corner [1,1] under a 2x2 BB rooted at origin maps to [0.5,0.5].
        assert!(verts.iter().any(|v| v.position == [1.0, 1.0]
            && (v.uv[0] - 0.5).abs() < 1e-6
            && (v.uv[1] - 0.5).abs() < 1e-6));
    }

    #[test]
    fn triangulate_verts_quad_yields_two_triangles() {
        let verts = PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 1.0, 1.0);
        // A quad triangulates to 2 triangles = 6 vertices.
        assert_eq!(verts.len(), 6);
    }

    #[test]
    fn triangulate_verts_degenerate_is_empty() {
        let verts =
            PolygonBlitPipeline::triangulate_verts(&[[0.0, 0.0], [1.0, 0.0]], 0.0, 0.0, 1.0, 1.0);
        assert!(verts.is_empty());
    }

    #[test]
    fn triangulate_multi_covers_all_contours() {
        // Primary quad + two extra quads: 6 verts each, 18 total.
        let primary = QUAD;
        let extras = vec![QUAD.to_vec(), QUAD.to_vec()];
        let verts = PolygonBlitPipeline::triangulate_multi(&primary, &extras, 0.0, 0.0, 1.0, 1.0);
        assert_eq!(verts.len(), 18);
        // With only the primary and no extras, it matches triangulate_verts.
        let just_primary =
            PolygonBlitPipeline::triangulate_multi(&primary, &[], 0.0, 0.0, 1.0, 1.0);
        assert_eq!(just_primary.len(), 6);
    }

    #[test]
    fn mesh_verts_invalid_is_empty() {
        let mesh = crate::surface::warp::WarpMesh {
            cols: 1,
            rows: 1,
            points: vec![],
        };
        assert!(PolygonBlitPipeline::mesh_verts(&mesh).is_empty());
    }

    /// Mesh positions are in output space [0..1], not NDC; the shader converts once.
    #[test]
    fn mesh_verts_positions_stay_in_output_space() {
        let mesh = crate::surface::warp::WarpMesh::identity(2, 2);
        let verts = PolygonBlitPipeline::mesh_verts(&mesh);
        assert_eq!(verts.len(), 6);
        for v in &verts {
            assert!(
                (0.0..=1.0).contains(&v.position[0]) && (0.0..=1.0).contains(&v.position[1]),
                "identity mesh vertex must be in [0..1] output space, got {:?}",
                v.position
            );
        }
        // The grid must span the full unit square (TL=[0,0], BR=[1,1]).
        assert!(verts.iter().any(|v| v.position == [0.0, 0.0]));
        assert!(verts.iter().any(|v| v.position == [1.0, 1.0]));
    }

    /// `prepare` grows the pools past the initial capacity and packs offsets
    /// contiguously without overlap.
    #[test]
    fn prepare_grows_pools_and_packs_vertices() {
        let Some(ctx) = crate::testing::headless_gpu() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let content = ctx.create_render_texture(64, 64);
        let view = content.create_view(&wgpu::TextureViewDescriptor::default());
        let pipeline = PolygonBlitPipeline::new(&ctx.device, ctx.texture_format).expect("pipeline");
        let zones = SurfaceOverlapZones::default();

        let make_draws = |n: usize| -> Vec<PolygonDrawDesc<'_>> {
            (0..n)
                .map(|_| PolygonDrawDesc {
                    content_view: &view,
                    uv_scale: [1.0, 1.0],
                    uv_offset: [0.0, 0.0],
                    homography: None,
                    overlap_zones: &zones,
                    vertices: PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 1.0, 1.0),
                    mask_uuid: "",
                    mask_uv_contours: Vec::new(),
                })
                .collect()
        };

        // Small batch fits the initial ring.
        let (prepared, _pool) = pipeline.prepare(&ctx.device, &ctx.queue, &make_draws(2));
        assert_eq!(prepared.len(), 2);
        let stride = std::mem::size_of::<PolygonVertex>() as u64 * 6;
        assert_eq!(prepared[0].vertex_offset, 0);
        assert_eq!(prepared[1].vertex_offset, stride);
        assert_eq!(prepared[0].num_triangles, 2);

        // Large batch (> MAX_DRAW_SLOTS) must grow both pools and stay packed.
        let big = MAX_DRAW_SLOTS as usize * 4;
        let (prepared, _pool) = pipeline.prepare(&ctx.device, &ctx.queue, &make_draws(big));
        assert_eq!(prepared.len(), big);
        for (i, p) in prepared.iter().enumerate() {
            assert_eq!(p.vertex_offset, i as u64 * stride);
            assert_eq!(p.num_triangles, 2);
        }
        assert!(pipeline.ring_slots.iter().any(|s| s.get() >= big));
    }

    /// Each `prepare` advances to the next pool set, wrapping after
    /// `POLYGON_FRAMES_IN_FLIGHT` frames.
    #[test]
    fn prepare_rotates_frame_pools() {
        let Some(ctx) = crate::testing::headless_gpu() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let content = ctx.create_render_texture(64, 64);
        let view = content.create_view(&wgpu::TextureViewDescriptor::default());
        let pipeline = PolygonBlitPipeline::new(&ctx.device, ctx.texture_format).expect("pipeline");
        let zones = SurfaceOverlapZones::default();
        let draw = || PolygonDrawDesc {
            content_view: &view,
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            homography: None,
            overlap_zones: &zones,
            vertices: PolygonBlitPipeline::triangulate_verts(&QUAD, 0.0, 0.0, 1.0, 1.0),
            mask_uuid: "",
            mask_uv_contours: Vec::new(),
        };

        assert_eq!(pipeline.frame_cursor.get(), 0);
        for expected in 1..=POLYGON_FRAMES_IN_FLIGHT {
            let _ = pipeline.prepare(&ctx.device, &ctx.queue, &[draw()]);
            assert_eq!(
                pipeline.frame_cursor.get(),
                expected % POLYGON_FRAMES_IN_FLIGHT
            );
        }
        // Cursor wrapped back to the first pool set after a full rotation.
        assert_eq!(pipeline.frame_cursor.get(), 0);
    }

    #[test]
    fn hdr10_presentation_always_encodes_in_the_shader() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            PresentationTransfer, ResolvedPresentation,
        };
        // HDR surfaces are never `*Srgb`, so the shader encodes.
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Hdr10Pq,
            peak_nits: Some(1000),
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
        };
        // `explicit_srgb_encode` must be false: the PQ branch runs instead.
        assert_eq!(presentation_encoding(&presentation), (1023.0, false, true));
    }

    #[test]
    fn hdr10_blit_matches_the_cpu_pq_reference_at_known_luminances() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            PresentationTransfer, ResolvedPresentation,
        };
        use crate::renderer::hdr;

        const PEAK: u16 = 1000;
        // Row pitch must be a multiple of `COPY_BYTES_PER_ROW_ALIGNMENT`: 64
        // texels at 4 bytes. Probes fill the first few texels.
        const WIDTH: u32 = 64;
        // Neutral grays stay neutral through the BT.2020 matrix, so the shader
        // must match the scalar reference exactly.
        let probes: [f32; 6] = [
            0.0,
            0.5,
            1.0,
            2.0,
            hdr::linear_headroom(f32::from(PEAK)),
            50.0,
        ];
        let width = WIDTH;
        assert!(probes.len() as u32 <= WIDTH);

        let Some(ctx) = crate::testing::headless_gpu() else {
            return;
        };
        let Ok(pipeline) = BlitPipeline::new(&ctx.device, wgpu::TextureFormat::Rgb10a2Unorm) else {
            return;
        };
        let source = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HDR10 Probe Source"),
            size: wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut words = Vec::with_capacity(WIDTH as usize * 4);
        for index in 0..WIDTH as usize {
            let probe = probes.get(index).copied().unwrap_or(0.0);
            let v = half::f16::from_f32(probe).to_bits();
            words.extend_from_slice(&[v, v, v, half::f16::ONE.to_bits()]);
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&words),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HDR10 Probe Target"),
            size: wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = pipeline.create_bind_group(&ctx.device, &source_view);
        pipeline.set_presentation(
            &ctx.queue,
            0,
            &ResolvedPresentation {
                requested: PresentationDepth::Sdr10,
                resolved: PresentationDepth::Sdr10,
                requested_transfer: PresentationTransfer::Hdr10Pq,
                transfer: PresentationTransfer::Hdr10Pq,
                peak_nits: Some(PEAK),
                hdr_metadata: None,
                pixel_format: PresentationPixelFormat::Rgb10A2,
                color_profile: PresentationColorProfile::Pq2020Limited,
                alpha_mode: AlphaMode::Opaque,
                dither: false,
                fallback_reason: None,
            },
        );
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HDR10 Probe Readback"),
            size: u64::from(width * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("HDR10 Probe Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HDR10 Probe Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
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
                multiview_mask: None,
            });
            pipeline.render(&mut pass, &bind_group);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let codes: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&mapped)
            .iter()
            .map(|packed| packed & 0x3ff)
            .collect();

        for (probe, code) in probes.iter().zip(&codes) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let expected = (hdr::pq_from_linear(*probe, f32::from(PEAK)) * 1023.0).round() as u32;
            assert!(
                code.abs_diff(expected) <= 1,
                "linear {probe} encoded to {code}, reference says {expected}"
            );
        }

        // Linear 1.0 is BT.2408 reference white: 203 cd/m², PQ 10-bit code 594.
        assert!(
            codes[2].abs_diff(594) <= 1,
            "reference white encoded to {} instead of 594",
            codes[2]
        );
        // Everything at or above the headroom clamps to the configured peak.
        assert_eq!(
            codes[4], codes[5],
            "values past the peak must clamp, not keep climbing"
        );
        drop(mapped);
        readback.unmap();
    }

    #[test]
    fn edr_encodes_nothing_and_dithers_nothing() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            PresentationTransfer, ResolvedPresentation,
        };
        // EDR is a float surface: no transfer, no quantization to dither.
        let presentation = ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::EdrLinear,
            transfer: PresentationTransfer::EdrLinear,
            peak_nits: Some(1000),
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgba16,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
            // Requested, but must be refused.
            dither: true,
            fallback_reason: None,
        };
        let (_, srgb_encode, dither) = presentation_encoding(&presentation);
        assert!(!srgb_encode, "EDR must not apply a transfer");
        assert!(!dither, "EDR has no integer quantization to dither into");
    }

    #[test]
    fn only_edr_skips_the_transfer_encode() {
        use crate::engine::value::render::PresentationTransfer;
        for transfer in PresentationTransfer::ALL {
            assert_eq!(
                transfer.encodes_a_transfer(),
                transfer != PresentationTransfer::EdrLinear,
                "{transfer:?}"
            );
        }
    }
}
