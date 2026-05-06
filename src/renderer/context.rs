use anyhow::{Context, Result};
use wgpu::util::DeviceExt;
use winit::window::Window;

/// GPU rendering context — device, queue, and adapter.
///
/// Owns the GPU resources needed for rendering (mixer, deck, channel, effects).
/// Does NOT own any window surface — that's a presentation concern owned by
/// the UI consumer (WindowSurface) or output windows (OutputWindow).
///
/// Can be created with a window hint (for adapter compatibility) or headless.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub texture_format: wgpu::TextureFormat,
}

/// Window surface for presentation — surface, swapchain config, and size.
///
/// Owned by the UI consumer. Handles surface acquisition, resize, and present.
/// The engine never touches this directly.
pub struct WindowSurface {
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
}

impl GpuContext {
    /// Create a GPU context + window surface from a window.
    ///
    /// The adapter is selected for compatibility with the window's surface.
    /// Returns both the GPU context (for the engine) and the window surface (for the UI).
    pub async fn new_for_window(window: &'static Window) -> Result<(Self, WindowSurface)> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window)
            .context("Failed to create surface")?;

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

        let mut required_features = wgpu::Features::empty();
        if adapter.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            required_features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
            log::info!("GPU supports BC texture compression (HAP video enabled)");
        } else {
            log::warn!("GPU does not support BC texture compression — HAP video will fall back to ffmpeg CPU decode");
        }

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Varda Device"),
                    required_features,
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                    experimental_features: Default::default(),
                    trace: Default::default(),
                },
            )
            .await
            .context("Failed to create device")?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        log::info!("Present mode: {:?} (available: {:?})", present_mode, surface_caps.present_modes);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 3,
        };

        surface.configure(&device, &surface_config);

        let gpu = GpuContext { instance, adapter, device, queue, texture_format: surface_format };
        let win_surface = WindowSurface { surface, surface_config, size };

        Ok((gpu, win_surface))
    }

    /// Create a headless GPU context (no window surface) for testing.
    ///
    /// Uses the default backend and requests a device without any surface
    /// compatibility requirements. Falls back to software adapter if no
    /// hardware GPU is available.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn new_headless() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .context("Failed to find GPU adapter for headless context")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Varda Headless Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
                experimental_features: Default::default(),
                trace: Default::default(),
            },
        ))
        .context("Failed to create headless device")?;

        Ok(GpuContext {
            instance,
            adapter,
            device,
            queue,
            texture_format: wgpu::TextureFormat::Rgba8UnormSrgb,
        })
    }

    /// Create a texture for rendering
    pub fn create_render_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Render Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.texture_format,
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

impl WindowSurface {
    /// Resize the window surface
    pub fn resize(&mut self, device: &wgpu::Device, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(device, &self.surface_config);
        }
    }
}

/// Content source that an output window can display
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum OutputSource {
    /// The master mix (final composited output)
    Master,
    /// A specific channel's composited output (by index)
    Channel(usize),
    /// A subset of channels composited together (sub-mix).
    /// Each channel contributes with its own opacity and blend mode.
    /// Master effects are NOT applied to sub-mixes.
    Channels(Vec<usize>),
    /// A specific deck's raw output (channel index, deck index)
    Deck(usize, usize),
}

impl OutputSource {
    /// Returns the channel indices involved in this source, if any.
    pub fn channel_indices(&self) -> Option<Vec<usize>> {
        match self {
            OutputSource::Channel(idx) => Some(vec![*idx]),
            OutputSource::Channels(indices) => Some(indices.clone()),
            _ => None,
        }
    }
}

impl std::fmt::Display for OutputSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputSource::Master => write!(f, "Master"),
            OutputSource::Channel(idx) => write!(f, "Ch {}", idx),
            OutputSource::Channels(indices) => {
                let names: Vec<String> = indices.iter().map(|i| format!("Ch {}", i)).collect();
                write!(f, "{}", names.join("+"))
            }
            OutputSource::Deck(ch, dk) => write!(f, "Ch {} Deck {}", ch + 1, dk + 1),
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
/// the device and queue from the GpuContext.
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
        context: &GpuContext,
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

        // Output windows use Immediate mode for lowest latency to projectors/displays.
        // This avoids output windows throttling the main render loop via vsync contention.
        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 3,
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
    pub fn render(&self, context: &GpuContext, content_view: &wgpu::TextureView) {
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
    pub fn render_surfaces(&self, context: &GpuContext, surfaces: &[SurfaceRenderInfo<'_>]) {
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

    /// Destroy this output window, closing the OS window and reclaiming leaked memory.
    /// Must be called instead of just dropping the struct if you want the window to close.
    pub fn destroy(self) {
        // Reclaim the leaked Box<Window> so it gets dropped, which closes the OS window.
        // Safety: the pointer was created by Box::leak in create_pending_outputs,
        // and we are the sole owner (no other references exist after removal from the vec).
        let window_ptr = self.window as *const Window as *mut Window;
        // Drop surface first (it references the window)
        drop(self.surface);
        // Now reclaim and drop the window
        unsafe {
            let _ = Box::from_raw(window_ptr);
        }
    }
}

use super::blit::{BlitPipeline, PolygonBlitPipeline};

/// Calibration card colors for distinct surface identification.
/// Each surface gets a different accent color for its test card.
const CALIBRATION_COLORS: [[u8; 3]; 8] = [
    [255, 80, 80],   // Red
    [80, 200, 120],  // Green
    [80, 140, 255],  // Blue
    [255, 200, 60],  // Yellow
    [200, 80, 255],  // Purple
    [80, 220, 220],  // Cyan
    [255, 140, 60],  // Orange
    [255, 100, 180], // Pink
];

/// Generate a calibration test card as RGBA pixel data.
///
/// Everything lives inside the border/corner brackets:
/// - **Grid + crosshair + circle** (upper ~70% of interior)
/// - **Gradient bars** (lower ~30% of interior): grayscale, R, G, B, stepped gray
///
/// Each surface gets a distinct accent color border for identification.
pub fn generate_calibration_card(width: u32, height: u32, color_index: usize) -> Vec<u8> {
    let [cr, cg, cb] = CALIBRATION_COLORS[color_index % CALIBRATION_COLORS.len()];
    let mut pixels = vec![0u8; (width * height * 4) as usize];

    let bg = [20u8, 20, 30, 255];
    let border_color = [cr, cg, cb, 255];
    let grid_color = [cr / 3, cg / 3, cb / 3, 255];
    let grid_bright = [cr / 2, cg / 2, cb / 2, 255];
    let center_color = [255u8, 255, 255, 200];
    let corner_color = [255u8, 255, 255, 255];

    let border_w = (width.min(height) / 40).max(2);
    let _corner_size = (width.min(height) / 8).max(8);

    // Interior content region (inside border)
    let inset = border_w + 1;
    let inner_w = width.saturating_sub(inset * 2);
    let inner_h = height.saturating_sub(inset * 2);

    // Split interior: top 70% = grid zone, bottom 30% = gradient bars
    let grid_h = (inner_h as f32 * 0.70) as u32;
    let grad_h = inner_h - grid_h;
    let bar_h = grad_h / 5; // 5 bars
    let grid_zone_bottom = inset + grid_h;

    // Crosshair centered on FULL card (not just grid zone)
    let cx = width / 2;
    let cy = height / 2;
    let cross_len = height.min(inner_w) / 4;
    let cross_thick = (width.min(height) / 200).max(1);

    // Corner brackets sit at the very edge of the output (pixel 0)
    let bracket_len = (width.min(height) / 6).max(10);
    let bracket_thick = (width.min(height) / 80).max(2);

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let mut color = bg;

            let inside = x >= inset && x < width - inset && y >= inset && y < height - inset;

            if inside {
                // === Gradient bars (bottom 30% of interior) ===
                if y >= grid_zone_bottom {
                    let bar_idx = (y - grid_zone_bottom) / bar_h.max(1);
                    let t = (x - inset) as f32 / inner_w.max(1) as f32;
                    let v = (t * 255.0) as u8;

                    color = match bar_idx {
                        0 => [v, v, v, 255],       // Grayscale
                        1 => [v, 0, 0, 255],       // Red
                        2 => [0, v, 0, 255],       // Green
                        3 => [0, 0, v, 255],       // Blue
                        _ => {                     // 16-step gray
                            let step = (t * 16.0).floor().min(15.0) as u8;
                            let sv = step * 17;
                            [sv, sv, sv, 255]
                        }
                    };
                }
                // === Grid zone (top 70% of interior) ===
                else {
                    let gx_norm = (x - inset) as f32 / inner_w.max(1) as f32;
                    let gy_norm = (y - inset) as f32 / grid_h.max(1) as f32;

                    // 8×8 grid
                    let gx_frac = (gx_norm * 8.0).fract();
                    let gy_frac = (gy_norm * 8.0).fract();
                    if gx_frac < 0.02 || gx_frac > 0.98 || gy_frac < 0.02 || gy_frac > 0.98 {
                        color = grid_color;
                    }

                    // Sub-grid
                    if (gx_frac - 0.5).abs() < 0.01 || (gy_frac - 0.5).abs() < 0.01 {
                        color = [grid_color[0] / 2, grid_color[1] / 2, grid_color[2] / 2, 180];
                    }
                }
            }

            // Center crosshair — spans full card, drawn on top of everything except corners
            if (x.abs_diff(cx) <= cross_thick && y.abs_diff(cy) <= cross_len)
                || (y.abs_diff(cy) <= cross_thick && x.abs_diff(cx) <= cross_len)
            {
                color = center_color;
            }

            // Center circle
            let dx = x as f32 - cx as f32;
            let dy = y as f32 - cy as f32;
            let dist = (dx * dx + dy * dy).sqrt();
            if (dist - cross_len as f32 * 0.6).abs() < 1.5 {
                color = border_color;
            }

            // Edge midpoint markers (on the border itself)
            let edge_pts = [
                (cx, 0u32),              // top center
                (cx, height - 1),        // bottom center
                (0u32, cy),              // left center
                (width - 1, cy),         // right center
            ];
            for (ex, ey) in edge_pts {
                if (x.abs_diff(ex) <= cross_thick * 3 && y.abs_diff(ey) <= border_w + 4)
                    || (y.abs_diff(ey) <= cross_thick * 3 && x.abs_diff(ex) <= border_w + 4)
                {
                    color = grid_bright;
                }
            }

            // Border
            if x < border_w || x >= width - border_w || y < border_w || y >= height - border_w {
                color = border_color;
            }

            // Corner brackets at the very edge (pixel 0) — drawn LAST, on top of border
            let at_tl = x < bracket_len && y < bracket_len;
            let at_tr = x >= width - bracket_len && y < bracket_len;
            let at_br = x >= width - bracket_len && y >= height - bracket_len;
            let at_bl = x < bracket_len && y >= height - bracket_len;
            if at_tl || at_tr || at_br || at_bl {
                let on_h = y < bracket_thick || y >= height - bracket_thick;
                let on_v = x < bracket_thick || x >= width - bracket_thick;
                if on_h || on_v {
                    color = corner_color;
                }
            }

            pixels[idx..idx + 4].copy_from_slice(&color);
        }
    }
    pixels
}

/// Create calibration card textures for N colors, returning (texture, view) pairs.
pub fn create_calibration_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    count: usize,
) -> Vec<(wgpu::Texture, wgpu::TextureView)> {
    let card_w = 512u32;
    let card_h = 512u32;

    (0..count)
        .map(|i| {
            let pixels = generate_calibration_card(card_w, card_h, i);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Calibration Card {}", i)),
                size: wgpu::Extent3d {
                    width: card_w,
                    height: card_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
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
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * card_w),
                    rows_per_image: Some(card_h),
                },
                wgpu::Extent3d {
                    width: card_w,
                    height: card_h,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        })
        .collect()
}
