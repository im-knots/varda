use anyhow::{Context, Result};
use wgpu::util::DeviceExt;
use winit::window::Window;

/// GPU rendering context — shared device/queue plus the main window's surface.
///
/// All rendering code (mixer, deck, channel, effects) accesses `device`, `queue`,
/// and `surface_config.format` through this struct. The `surface` field is only
/// used by the main window's present path.
///
/// For multi-output, additional output windows create their own surfaces via
/// `OutputWindow`, but share the same device/queue from this context.
pub struct RenderContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
}

impl RenderContext {
    /// Create a new render context from a window
    pub async fn new(window: &'static Window) -> Result<Self> {
        let size = window.inner_size();

        // Create wgpu instance
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Create surface
        let surface = instance.create_surface(window)
            .context("Failed to create surface")?;

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("Failed to find suitable GPU adapter")?;

        log::info!("Using GPU: {}", adapter.get_info().name);
        log::info!("Backend: {:?}", adapter.get_info().backend);

        // Request device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Varda Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                    experimental_features: Default::default(),
                    trace: Default::default(),
                },
            )
            .await
            .context("Failed to create device")?;

        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_config);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            size,
        })
    }

    /// Resize the main window surface
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    /// Create a texture for rendering
    pub fn create_render_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Render Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.surface_config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                 | wgpu::TextureUsages::TEXTURE_BINDING
                 | wgpu::TextureUsages::COPY_SRC
                 | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Create a uniform buffer
    pub fn create_uniform_buffer<T: bytemuck::Pod>(&self, data: &T) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[*data]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// Update a uniform buffer
    pub fn update_uniform_buffer<T: bytemuck::Pod>(&self, buffer: &wgpu::Buffer, data: &T) {
        self.queue.write_buffer(buffer, 0, bytemuck::cast_slice(&[*data]));
    }
}

/// Content source that an output window can display
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum OutputSource {
    /// The master mix (final composited output)
    Master,
    /// A specific channel's composited output (by index)
    Channel(usize),
    /// A specific deck's raw output (channel index, deck index)
    Deck(usize, usize),
}

impl std::fmt::Display for OutputSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputSource::Master => write!(f, "Master"),
            OutputSource::Channel(idx) => write!(f, "Channel {}", idx),
            OutputSource::Deck(ch, dk) => write!(f, "Ch{} Deck {}", ch, dk),
        }
    }
}

/// Info for rendering one surface into an output window.
pub struct SurfaceRenderInfo<'a> {
    /// The content texture to sample from
    pub content_view: &'a wgpu::TextureView,
    /// Polygon vertices in normalized canvas coords [0..1]
    pub vertices: &'a [[f32; 2]],
    /// Bounding box: [x, y, width, height] in [0..1]
    pub bounding_box: [f32; 4],
    /// UV scale for content sampling (Fill=[1,1], Mapped=[bb_w, bb_h])
    pub uv_scale: [f32; 2],
    /// UV offset for content sampling (Fill=[0,0], Mapped=[bb_x, bb_y])
    pub uv_offset: [f32; 2],
    /// Per-surface warp corners in output space [0..1] — TL, TR, BR, BL.
    /// None = no warp (render at polygon's native position).
    pub warp_corners: Option<[[f32; 2]; 4]>,
}

/// Assignment of a surface to an output, with per-surface warp calibration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SurfaceAssignment {
    /// Index into SurfaceManager.surfaces
    pub surface_idx: usize,
    /// Warp corners in output-normalized coords [0..1] — TL, TR, BR, BL.
    /// These define where the surface's bounding box corners map to in the output frame.
    /// Default = the surface's bounding box corners (identity/no warp).
    pub warp_corners: [[f32; 2]; 4],
    /// Whether this assignment is enabled
    pub enabled: bool,
}

/// Where an output window is displayed.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputTarget {
    /// Floating window (default)
    Windowed,
    /// Fullscreen/borderless on a specific monitor (identified by name + index)
    Display {
        /// Monitor name (e.g. "Built-in Retina Display", "HDMI-1")
        name: String,
        /// Index into the available monitors list (for lookup)
        monitor_index: usize,
    },
}

impl std::fmt::Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::Windowed => write!(f, "Windowed"),
            OutputTarget::Display { name, .. } => write!(f, "{}", name),
        }
    }
}

/// An output window that displays content on a separate display/projector.
///
/// Each output window has its own OS window and wgpu surface, but shares
/// the device and queue from the main RenderContext.
pub struct OutputWindow {
    pub name: String,
    pub window: &'static Window,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    pub blit_pipeline: BlitPipeline,
    pub polygon_pipeline: PolygonBlitPipeline,
    /// Where this output is displayed (windowed or on a specific monitor)
    pub target: OutputTarget,
    /// Surface assignments — which surfaces this output renders, with per-surface warp.
    /// Empty = render all surfaces (fallback behavior).
    pub surface_assignments: Vec<SurfaceAssignment>,
    /// Whether calibration mode is active (shows warp handles)
    pub calibration_mode: bool,
}

impl OutputWindow {
    /// Create a new output window with its own surface, sharing the given device/queue.
    pub fn new(
        context: &RenderContext,
        window: &'static Window,
        name: String,
    ) -> Result<Self> {
        let size = window.inner_size();

        let surface = context.instance.create_surface(window)
            .context("Failed to create output surface")?;

        let surface_caps = surface.get_capabilities(&context.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&context.device, &surface_config);

        let blit_pipeline = BlitPipeline::new(&context.device, surface_config.format)?;
        let polygon_pipeline = PolygonBlitPipeline::new(&context.device, surface_config.format)?;

        Ok(Self {
            name,
            window,
            surface,
            surface_config,
            size,
            blit_pipeline,
            polygon_pipeline,
            target: OutputTarget::Windowed,
            surface_assignments: Vec::new(),
            calibration_mode: false,
        })
    }

    /// Resize this output window's surface
    pub fn resize(&mut self, device: &wgpu::Device, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(device, &self.surface_config);
        }
    }

    /// Render the routed content to this output window's surface (simple single-source blit)
    pub fn render(&self, context: &RenderContext, content_view: &wgpu::TextureView) {
        let fullscreen_quad: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        self.render_surfaces(context, &[SurfaceRenderInfo {
            content_view,
            vertices: &fullscreen_quad,
            bounding_box: [0.0, 0.0, 1.0, 1.0],
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            warp_corners: None,
        }]);
    }

    /// Render multiple surfaces composited at their canvas positions.
    /// Each surface is rendered as a textured polygon using fan triangulation.
    /// If a surface has warp_corners, a homography is computed and applied in the vertex shader
    /// for perspective-correct rendering.
    pub fn render_surfaces(&self, context: &RenderContext, surfaces: &[SurfaceRenderInfo<'_>]) {
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(e) => {
                log::error!("Output '{}': failed to get surface texture: {}", self.name, e);
                return;
            }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(&format!("Output '{}' Encoder", self.name)),
        });

        // Pre-create bind groups and vertex buffers for each surface
        let prepared: Vec<_> = surfaces.iter().map(|surf| {
            let bb = surf.bounding_box;

            // Compute forward homography: from bbox corners → warp corners
            let homography = surf.warp_corners.map(|warp_corners| {
                let src_corners = [
                    [bb[0], bb[1]],                     // TL
                    [bb[0] + bb[2], bb[1]],             // TR
                    [bb[0] + bb[2], bb[1] + bb[3]],     // BR
                    [bb[0], bb[1] + bb[3]],             // BL
                ];
                super::warp::compute_forward_homography(&src_corners, &warp_corners)
            });

            let bind_group = self.polygon_pipeline.create_bind_group(
                &context.device,
                surf.content_view,
                surf.uv_scale,
                surf.uv_offset,
                homography.as_ref(),
            );
            let (vb, num_tris) = PolygonBlitPipeline::triangulate(
                &context.device,
                surf.vertices,
                bb[0], bb[1], bb[2], bb[3],
            );
            (bind_group, vb, num_tris)
        }).collect();

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("Output '{}' Pass", self.name)),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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

            for (bind_group, vb, num_tris) in &prepared {
                if *num_tris > 0 {
                    self.polygon_pipeline.render_polygon(
                        &context.device,
                        &mut render_pass,
                        bind_group,
                        vb,
                        *num_tris,
                    );
                }
            }
        }

        context.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    /// Set the display target for this output window.
    /// `monitor` should be the MonitorHandle for Display targets.
    pub fn set_target(&mut self, target: OutputTarget, monitor: Option<winit::monitor::MonitorHandle>) {
        use winit::window::Fullscreen;
        match &target {
            OutputTarget::Windowed => {
                self.window.set_fullscreen(None);
            }
            OutputTarget::Display { .. } => {
                self.window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
            }
        }
        self.target = target;
    }
}

use super::blit::{BlitPipeline, PolygonBlitPipeline};

