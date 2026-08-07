use anyhow::{Context, Result};
use wgpu::util::DeviceExt;
use winit::window::Window;

// Plain, framework-free output value types live in `config` so the engine
// contract layer can name them without importing this wgpu/winit file. Kept
// re-exported here so existing `crate::renderer::context::…` paths still work;
// window-lifecycle inherent impls (e.g. `OutputWindow::set_target`) stay below.
pub use super::config::{
    CalibrationMode, OutputRotation, OutputSource, OutputTarget, RecordingCodec, SrtCodec,
    StreamingCodec,
};

/// Linear-light format used by the entire color path: deck render targets, all
/// three effect tiers, ISF pass buffers, compute output, channel/mixer
/// composites, and the dome/edge-blend intermediates.
///
/// Single source of truth — see spec/unified-color-pipeline.md. 8-bit appears
/// only at the two boundaries where it is correct: sRGB-tagged source ingest
/// (hardware EOTF on sample) and sRGB output encode (hardware OETF on write).
/// Non-color data textures (analyzer, audio, calibration, MSDF atlases) keep
/// their own formats and are deliberately excluded.
pub const COLOR_PATH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// GPU rendering context — device, queue, and adapter.
///
/// Owns the GPU resources needed for rendering (mixer, deck, channel, effects).
/// Does NOT own any window surface — that's a presentation concern owned by
/// the UI consumer (`WindowSurface`) or output windows (`OutputWindow`).
///
/// Can be created with a window hint (for adapter compatibility) or headless.
///
/// `Clone` is cheap — wgpu types are internally `Arc`-wrapped.
/// Cloning produces a handle to the same GPU resources, useful for
/// background thread deck creation.
#[derive(Clone)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub texture_format: wgpu::TextureFormat,
    /// Linear-light color-path format (`COLOR_PATH_FORMAT`). Distinct from
    /// `texture_format`, which is the surface/presentation format.
    pub compositing_format: wgpu::TextureFormat,
    pub timestamp_supported: bool,
    /// Catches GPU errors that would otherwise abort the process, and attributes
    /// them to whatever was being drawn. See spec/error-handling.md.
    pub errors: super::gpu_guard::GpuErrorGuard,
    /// Counts command buffer commits. Read once per frame by the frame loop.
    pub submits: super::submit_stats::SubmitCounter,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the surface cannot be created for the window, if no
    /// suitable adapter is found, or if device creation fails.
    pub async fn new_for_window(window: &'static Window) -> Result<(Self, WindowSurface)> {
        let (instance, surface, size) = Self::create_surface_for_window(window)?;
        Self::new_with_surface(instance, surface, size).await
    }

    /// Create the wgpu instance and surface on the current (main) thread.
    /// On macOS, `create_surface` accesses `NSView`/`CAMetalLayer` which must
    /// happen on the main thread.  The returned objects are `Send` and can be
    /// passed to a background thread for adapter/device creation.
    ///
    /// # Errors
    ///
    /// Returns an error if `wgpu` cannot create a surface for the window
    /// (unsupported window handle or no compatible backend).
    pub fn create_surface_for_window(
        window: &'static Window,
    ) -> Result<(
        wgpu::Instance,
        wgpu::Surface<'static>,
        winit::dpi::PhysicalSize<u32>,
    )> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });
        let surface = instance
            .create_surface(window)
            .context("Failed to create surface")?;
        Ok((instance, surface, size))
    }

    /// Complete GPU initialization given a pre-created instance and surface.
    /// Safe to call from a background thread — all Metal dispatch work is
    /// resolved through the pre-created surface.
    ///
    /// # Errors
    ///
    /// Returns an error if no adapter compatible with the surface is found, or
    /// if the device request fails (e.g. the requested limits are unsupported).
    pub async fn new_with_surface(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        size: winit::dpi::PhysicalSize<u32>,
    ) -> Result<(Self, WindowSurface)> {
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

        let (required_features, timestamp_supported) = Self::select_optional_features(&adapter);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Varda Device"),
                required_features,
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: 16384,
                    ..wgpu::Limits::default()
                },
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            })
            .await
            .context("Failed to create device")?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);

        // Prefer Immediate to avoid macOS ProMotion throttling the render loop.
        // The UI event loop drives frame pacing via request_redraw().
        // Fallback: Mailbox (non-blocking vsync) > Fifo (blocking vsync, last resort).
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
        {
            wgpu::PresentMode::Immediate
        } else if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        log::info!(
            "Present mode: {:?} (available: {:?})",
            present_mode,
            surface_caps.present_modes
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_config);

        // Installed before anything renders: wgpu's default handler panics, and
        // a panic on the render thread ends the performance.
        let errors = super::gpu_guard::GpuErrorGuard::new();
        errors.install(&device);

        let gpu = GpuContext {
            instance,
            adapter,
            device,
            queue,
            texture_format: surface_format,
            compositing_format: COLOR_PATH_FORMAT,
            timestamp_supported,
            errors,
            submits: super::submit_stats::SubmitCounter::new(),
        };
        let win_surface = WindowSurface {
            surface,
            surface_config,
            size,
        };

        Ok((gpu, win_surface))
    }

    /// Select the optional device features to request from an adapter.
    ///
    /// Shared by the windowed and headless paths so HAP (BC texture
    /// compression) and GPU timing behave identically regardless of whether a
    /// window surface exists. Returns the feature set to request and whether
    /// timestamp queries are usable for GPU timing.
    fn select_optional_features(adapter: &wgpu::Adapter) -> (wgpu::Features, bool) {
        let mut required_features = wgpu::Features::empty();
        if adapter
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            required_features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
            log::info!("GPU supports BC texture compression (HAP video enabled)");
        } else {
            log::warn!("GPU does not support BC texture compression — HAP video will fall back to ffmpeg CPU decode");
        }

        let mut timestamp_supported = false;
        if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
            if !adapter
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            {
                log::warn!("GPU supports TIMESTAMP_QUERY but not TIMESTAMP_QUERY_INSIDE_ENCODERS — GPU timing disabled");
            } else if encoder_timestamps_are_trustworthy(adapter) {
                required_features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
                timestamp_supported = true;
                log::info!("GPU supports timestamp queries inside encoders (GPU timing enabled)");
            } else {
                log::warn!("Apple GPU: encoder timestamps are not implementable on this hardware — GPU timing disabled");
            }
        }

        (required_features, timestamp_supported)
    }

    /// Create a headless GPU context (no window surface).
    ///
    /// Requests the same optional features as the windowed path (notably
    /// `TEXTURE_COMPRESSION_BC`) so HAP video uses the GPU-native `BCn` path in
    /// headless installations. Falls back to software adapter if no hardware
    /// GPU is available. Used for headless mode and tests.
    ///
    /// # Errors
    ///
    /// Returns an error if no GPU adapter is available at all, or if the
    /// headless device request fails.
    pub fn new_headless() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .context("Failed to find GPU adapter for headless context")?;

        log::info!("Using GPU: {}", adapter.get_info().name);
        log::info!("Backend: {:?}", adapter.get_info().backend);

        let (required_features, timestamp_supported) = Self::select_optional_features(&adapter);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Varda Headless Device"),
            required_features,
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: 16384,
                ..wgpu::Limits::default()
            },
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::default(),
        }))
        .context("Failed to create headless device")?;

        let errors = super::gpu_guard::GpuErrorGuard::new();
        errors.install(&device);

        Ok(GpuContext {
            instance,
            adapter,
            device,
            queue,
            texture_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            compositing_format: COLOR_PATH_FORMAT,
            timestamp_supported,
            errors,
            submits: super::submit_stats::SubmitCounter::new(),
        })
    }

    /// Submit command buffers, counting the commit.
    ///
    /// Prefer this over `context.queue.submit()` anywhere on the per-frame path
    /// so the submit tally stays accurate. See `submit_stats`.
    pub fn submit<I>(&self, command_buffers: I) -> wgpu::SubmissionIndex
    where
        I: IntoIterator<Item = wgpu::CommandBuffer>,
    {
        self.submits.record();
        self.queue.submit(command_buffers)
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
            format: self.texture_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Create a texture for compositing in linear-light space (`Rgba16Float`).
    /// Used for channel composites, mixer composites, effect ping-pong, and sub-mixes.
    pub fn create_compositing_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Compositing Texture (Rgba16Float)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Create a uniform buffer
    pub fn create_uniform_buffer<T: bytemuck::Pod>(&self, data: &T) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Uniform Buffer"),
                contents: bytemuck::cast_slice(&[*data]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Update a uniform buffer
    pub fn update_uniform_buffer<T: bytemuck::Pod>(&self, buffer: &wgpu::Buffer, data: &T) {
        self.queue
            .write_buffer(buffer, 0, bytemuck::cast_slice(&[*data]));
    }
}

/// Whether `CommandEncoder::write_timestamp` produces usable results here.
///
/// Apple GPUs are tile-based deferred renderers: Metal only permits counter
/// sampling at stage boundaries (via a pass descriptor's
/// `sampleBufferAttachments`), so there is no `sampleCountersInBuffer` for an
/// encoder-level timestamp to lower to. wgpu 29 still advertises
/// `TIMESTAMP_QUERY_INSIDE_ENCODERS` on this hardware and emulates it with a
/// dummy blit pass, but that emulation drops the render work submitted
/// alongside it — decks composite from never-written textures and the frame
/// fills with NaN. Upstream has since stopped advertising the feature here
/// (gfx-rs/wgpu ef9974f); until we take that release, refuse it ourselves.
fn encoder_timestamps_are_trustworthy(adapter: &wgpu::Adapter) -> bool {
    /// Apple's PCI vendor ID, as reported for Metal adapters.
    const APPLE_VENDOR: u32 = 0x106B;

    let info = adapter.get_info();
    !(info.backend == wgpu::Backend::Metal
        && (info.vendor == APPLE_VENDOR || info.name.starts_with("Apple")))
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

/// Info for rendering one surface into an output window.
pub struct SurfaceRenderInfo<'a> {
    /// Surface uuid — cache key for its baked hole mask.
    pub uuid: &'a str,
    /// The content texture to sample from
    pub content_view: &'a wgpu::TextureView,
    /// Polygon vertices in normalized canvas coords [0..1] (primary contour)
    pub vertices: &'a [[f32; 2]],
    /// Additional disjoint contours for combined surfaces. Empty for simple
    /// surfaces. When non-empty, the surface renders every contour (no warp).
    pub extra_contours: &'a [Vec<[f32; 2]>],
    /// Bounding box: [x, y, width, height] in [0..1]
    pub bounding_box: [f32; 4],
    /// UV scale for content sampling (Fill=[1,1], Mapped=[`bb_w`, `bb_h`])
    pub uv_scale: [f32; 2],
    /// UV offset for content sampling (Fill=[0,0], Mapped=[`bb_x`, `bb_y`])
    pub uv_offset: [f32; 2],
    /// Warp mode: `CornerPin` or Mesh. None = no warp (render at polygon's native position).
    pub warp_mode: Option<super::warp::WarpMode>,
    /// Per-surface overlap zones (Auto mode). Default = no zones.
    pub overlap_zones: super::edge_blend::SurfaceOverlapZones,
    /// Flattened subtractive hole contours in surface uv space (8i.7). Empty =
    /// no holes.
    pub hole_uv_contours: Vec<Vec<[f32; 2]>>,
}

/// Membership of a surface in an output. Warp now lives on the `Surface`
/// itself (`Surface.warp`); an assignment only records inclusion and the
/// per-output overlap zones used for edge blending.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SurfaceAssignment {
    /// UUID of the assigned surface
    pub surface_uuid: String,
    /// Whether this assignment is enabled
    pub enabled: bool,
    /// Per-surface overlap zones (set by Auto mode detection).
    #[serde(default)]
    pub overlap_zones: super::edge_blend::SurfaceOverlapZones,
}

/// The largest centred `[x, y, w, h]` of `content_aspect` (width ÷ height) that
/// fits a `canvas_width` × `canvas_height` canvas, in normalised coordinates.
///
/// Content narrower than the canvas gets bars on the left and right, wider gets
/// them above and below, and an exact match returns the full unit square so the
/// usual case costs nothing. Degenerate inputs — a zero-sized canvas, or an
/// aspect that is zero, negative, infinite or NaN — also return the full square,
/// which stretches rather than producing a NaN-sized quad.
pub fn aspect_fit_rect(content_aspect: f32, canvas_width: u32, canvas_height: u32) -> [f32; 4] {
    if canvas_width == 0
        || canvas_height == 0
        || !content_aspect.is_finite()
        || content_aspect <= 0.0
    {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let canvas_aspect = canvas_width as f32 / canvas_height as f32;
    let (w, h) = if content_aspect > canvas_aspect {
        (1.0, canvas_aspect / content_aspect)
    } else {
        (content_aspect / canvas_aspect, 1.0)
    };
    [(1.0 - w) * 0.5, (1.0 - h) * 0.5, w, h]
}

/// An output window that displays content on a separate display/projector.
///
/// Each output window has its own OS window and wgpu surface, but shares
/// the device and queue from the `GpuContext`.
pub struct OutputWindow {
    pub uuid: String,
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
    /// Calibration display mode (Off / Projector test card / per-Surface cards).
    pub calibration_mode: CalibrationMode,
    /// Whether edge blend is auto-computed or manually configured.
    pub edge_blend_mode: super::edge_blend::EdgeBlendMode,
    /// Edge blending configuration for multi-projector overlap zones.
    pub edge_blend: super::edge_blend::EdgeBlendConfig,
    /// GPU pipeline for applying edge blend post-process.
    pub edge_blend_pipeline: super::edge_blend::EdgeBlendPipeline,
    /// Pre-blend intermediate: surfaces render here when edge blending is active.
    pub surface_texture: wgpu::Texture,
    pub surface_texture_view: wgpu::TextureView,
    /// Post-blend result texture. UI preview reads from this.
    /// When edge blend is off, surfaces render directly here.
    /// When edge blend is on, edge blend shader writes here from `surface_texture`.
    pub preview_texture: wgpu::Texture,
    pub preview_texture_view: wgpu::TextureView,
    /// Per-output rotation applied at the final blit stage.
    pub rotation: OutputRotation,
}

impl OutputWindow {
    /// Create a new output window with its own surface, sharing the given device/queue.
    ///
    /// # Errors
    ///
    /// Returns an error if the output surface cannot be created for the window,
    /// or if any of the blit/polygon/edge-blend pipelines fail to build.
    pub fn new(context: &GpuContext, window: &'static Window, name: String) -> Result<Self> {
        let size = window.inner_size();

        let surface = context
            .instance
            .create_surface(window)
            .context("Failed to create output surface")?;

        let surface_caps = surface.get_capabilities(&context.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);

        // Output windows use Immediate mode for lowest latency to projectors/displays.
        // This avoids output windows throttling the main render loop via vsync contention.
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
        {
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
        let edge_blend_pipeline =
            super::edge_blend::EdgeBlendPipeline::new(&context.device, surface_config.format)?;
        let (surface_texture, surface_texture_view) = Self::create_intermediate_texture(
            &context.device,
            size.width,
            size.height,
            surface_config.format,
            "Surface Intermediate",
        );
        let (preview_texture, preview_texture_view) = Self::create_intermediate_texture(
            &context.device,
            size.width,
            size.height,
            surface_config.format,
            "Preview",
        );

        Ok(Self {
            uuid: crate::deck::generate_short_uuid(),
            name,
            window,
            surface,
            surface_config,
            size,
            blit_pipeline,
            polygon_pipeline,
            target: OutputTarget::Windowed,
            surface_assignments: Vec::new(),
            calibration_mode: CalibrationMode::Off,
            edge_blend_mode: super::edge_blend::EdgeBlendMode::default(),
            edge_blend: super::edge_blend::EdgeBlendConfig::default(),
            edge_blend_pipeline,
            surface_texture,
            surface_texture_view,
            preview_texture,
            preview_texture_view,
            rotation: OutputRotation::default(),
        })
    }

    /// Create an intermediate GPU texture for the render pipeline.
    fn create_intermediate_texture(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    /// Resize this output window's surface
    pub fn resize(&mut self, device: &wgpu::Device, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(device, &self.surface_config);
            let fmt = self.surface_config.format;
            let (ew, eh) = self
                .rotation
                .effective_dimensions(new_size.width, new_size.height);
            let (tex, view) =
                Self::create_intermediate_texture(device, ew, eh, fmt, "Surface Intermediate");
            self.surface_texture = tex;
            self.surface_texture_view = view;
            let (tex, view) = Self::create_intermediate_texture(device, ew, eh, fmt, "Preview");
            self.preview_texture = tex;
            self.preview_texture_view = view;
        }
    }

    /// Set output rotation and rebuild intermediate textures at effective dimensions.
    pub fn set_rotation(&mut self, device: &wgpu::Device, rotation: OutputRotation) {
        self.rotation = rotation;
        let fmt = self.surface_config.format;
        let (ew, eh) = rotation.effective_dimensions(self.size.width, self.size.height);
        let (tex, view) =
            Self::create_intermediate_texture(device, ew, eh, fmt, "Surface Intermediate");
        self.surface_texture = tex;
        self.surface_texture_view = view;
        let (tex, view) = Self::create_intermediate_texture(device, ew, eh, fmt, "Preview");
        self.preview_texture = tex;
        self.preview_texture_view = view;
    }

    /// Render the routed content stretched over the whole window.
    ///
    /// Used for the projector calibration card, which has to reach the physical
    /// edges of the output for alignment against it to mean anything. Content
    /// should go through [`render_fit`](Self::render_fit) instead.
    pub fn render(&self, context: &GpuContext, content_view: &wgpu::TextureView) {
        self.render_quad(context, content_view, [0.0, 0.0, 1.0, 1.0]);
    }

    /// Render the routed content letterboxed to `content_aspect` (width ÷ height).
    ///
    /// With no surfaces defined the window is the whole canvas, so content used
    /// to be stretched to fill it: a 1080×1920 composite arrived correctly
    /// proportioned and was squashed into a 16:9 window. The pass clears to
    /// black, so insetting the quad puts bars around the content rather than
    /// distorting it. A window that already matches the content aspect insets by
    /// nothing and renders exactly as before.
    pub fn render_fit(
        &self,
        context: &GpuContext,
        content_view: &wgpu::TextureView,
        content_aspect: f32,
    ) {
        // Measured against the rotated dimensions: the surface pass draws into
        // an intermediate at those and rotation is applied on the way to the
        // swap chain, so a 90° output turns a landscape window into a portrait
        // canvas.
        let (ew, eh) = self
            .rotation
            .effective_dimensions(self.size.width, self.size.height);
        self.render_quad(
            context,
            content_view,
            aspect_fit_rect(content_aspect, ew, eh),
        );
    }

    /// Blit `content_view` into one normalised `[x, y, w, h]` rectangle of the
    /// window, with the content's full extent mapped across it.
    fn render_quad(&self, context: &GpuContext, content_view: &wgpu::TextureView, rect: [f32; 4]) {
        let [x, y, w, h] = rect;
        let quad: [[f32; 2]; 4] = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        self.render_surfaces(
            context,
            &[SurfaceRenderInfo {
                uuid: "",
                content_view,
                vertices: &quad,
                extra_contours: &[],
                bounding_box: rect,
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                warp_mode: None,
                overlap_zones: super::edge_blend::SurfaceOverlapZones::default(),
                hole_uv_contours: Vec::new(),
            }],
        );
    }

    /// Render multiple surfaces composited at their canvas positions.
    /// Each surface is rendered as a textured polygon using fan triangulation.
    /// Warp is applied per the `WarpMode`: `CornerPin` uses homography in the vertex shader,
    /// Mesh mode bakes warp into triangle vertices directly.
    pub fn render_surfaces(&self, context: &GpuContext, surfaces: &[SurfaceRenderInfo<'_>]) {
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) => output,
            wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                log::warn!(
                    "Output '{}': surface suboptimal, will reconfigure",
                    self.name
                );
                output
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                log::warn!("Output '{}': surface outdated, reconfiguring", self.name);
                self.surface
                    .configure(&context.device, &self.surface_config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(output)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
                    other => {
                        log::error!(
                            "Output '{}': failed to get surface texture after reconfigure: {:?}",
                            self.name,
                            other
                        );
                        return;
                    }
                }
            }
            other => {
                log::debug!("Output '{}': surface unavailable: {:?}", self.name, other);
                return;
            }
        };
        let final_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // Post-process edge blend only applies in Manual mode.
        // Auto mode uses per-surface blend in the polygon shader.
        let use_edge_blend = self.edge_blend_mode == super::edge_blend::EdgeBlendMode::Manual
            && self.edge_blend.any_enabled();

        // Pipeline:
        //   No edge blend:  surfaces → preview_texture → swap chain  (2 passes)
        //   Edge blend:     surfaces → surface_texture → edge blend → preview_texture → swap chain  (3 passes)
        // The UI preview always reads preview_texture_view.
        let surface_render_target = if use_edge_blend {
            &self.surface_texture_view
        } else {
            &self.preview_texture_view
        };

        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(&format!("Output '{}' Encoder", self.name)),
            });

        // Pass 1: Render surfaces into the surface render target.
        // Triangulate on the CPU, then prepare draws from the pipeline's
        // persistent param/vertex pools (no per-frame GPU buffer allocation).
        let draws: Vec<super::blit::PolygonDrawDesc<'_>> = surfaces
            .iter()
            .map(|surf| {
                let bb = surf.bounding_box;

                // Combined (multi-contour) surface: a single warp mesh can't
                // represent disjoint contours, so render every contour as a
                // bounding-box UV fill. Matches the stage editor and fixes only
                // the primary contour rendering (see combine_surfaces).
                let (homography, vertices) = if surf.extra_contours.is_empty() {
                    // Dispatch warp mode: CornerPin → homography, Mesh → vertex-baked, None → identity
                    match &surf.warp_mode {
                        Some(super::warp::WarpMode::CornerPin { corners }) => {
                            let src_corners = [
                                [bb[0], bb[1]],
                                [bb[0] + bb[2], bb[1]],
                                [bb[0] + bb[2], bb[1] + bb[3]],
                                [bb[0], bb[1] + bb[3]],
                            ];
                            let h = super::warp::compute_forward_homography(&src_corners, corners);
                            let verts = PolygonBlitPipeline::triangulate_verts(
                                surf.vertices,
                                bb[0],
                                bb[1],
                                bb[2],
                                bb[3],
                            );
                            (Some(h), verts)
                        }
                        Some(super::warp::WarpMode::Mesh(mesh)) => {
                            // Mesh mode: warp baked into vertices, identity homography
                            (None, PolygonBlitPipeline::mesh_verts(mesh))
                        }
                        Some(super::warp::WarpMode::Bezier(b)) => {
                            // Bezier: tessellate the control cage into a mesh, then bake.
                            (None, PolygonBlitPipeline::mesh_verts(&b.tessellate()))
                        }
                        None => {
                            let verts = PolygonBlitPipeline::triangulate_verts(
                                surf.vertices,
                                bb[0],
                                bb[1],
                                bb[2],
                                bb[3],
                            );
                            (None, verts)
                        }
                    }
                } else {
                    (
                        None,
                        PolygonBlitPipeline::triangulate_multi(
                            surf.vertices,
                            surf.extra_contours,
                            bb[0],
                            bb[1],
                            bb[2],
                            bb[3],
                        ),
                    )
                };

                super::blit::PolygonDrawDesc {
                    content_view: surf.content_view,
                    uv_scale: surf.uv_scale,
                    uv_offset: surf.uv_offset,
                    homography,
                    overlap_zones: &surf.overlap_zones,
                    vertices,
                    mask_uuid: surf.uuid,
                    mask_uv_contours: surf.hole_uv_contours.clone(),
                }
            })
            .collect();

        let (prepared, vertex_pool) =
            self.polygon_pipeline
                .prepare(&context.device, &context.queue, &draws);

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("Output '{}' Surface Pass", self.name)),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_render_target,
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

            self.polygon_pipeline
                .draw(&mut render_pass, &prepared, &vertex_pool);
        }

        // Pass 2 (edge blend only): surface_texture → edge blend → preview_texture
        if use_edge_blend {
            self.edge_blend_pipeline.render(
                &context.device,
                &context.queue,
                &mut encoder,
                &self.surface_texture_view,
                &self.preview_texture_view,
                &self.edge_blend,
            );
        }

        // Final pass: blit preview_texture → swap chain (with rotation)
        {
            self.blit_pipeline
                .set_rotation(&context.queue, self.rotation.index());
            let blit_bg = self
                .blit_pipeline
                .create_bind_group(&context.device, &self.preview_texture_view);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("Output '{}' Swap Blit", self.name)),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &final_view,
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
            self.blit_pipeline.render(&mut pass, &blit_bg);
        }

        context.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    /// Set the display target for this output window.
    /// `monitor` should be the `MonitorHandle` for Display targets.
    pub fn set_target(
        &mut self,
        target: OutputTarget,
        monitor: Option<winit::monitor::MonitorHandle>,
    ) {
        use winit::window::Fullscreen;
        match &target {
            OutputTarget::Windowed => {
                self.window.set_fullscreen(None);
            }
            OutputTarget::Display { .. } => {
                self.window
                    .set_fullscreen(Some(Fullscreen::Borderless(monitor)));
            }
            _ => {
                log::warn!("Cannot set headless target on a windowed output");
                return;
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
        let window_ptr = std::ptr::from_ref::<Window>(self.window).cast_mut();
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
// gx/gy, grid_h/grad_h and at_tl/at_tr/at_bl/at_br are the clearest names for this 2D geometry.
#[allow(clippy::similar_names)]
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
                        0 => [v, v, v, 255], // Grayscale
                        1 => [v, 0, 0, 255], // Red
                        2 => [0, v, 0, 255], // Green
                        3 => [0, 0, v, 255], // Blue
                        _ => {
                            // 16-step gray
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
                    if !(0.02..=0.98).contains(&gx_frac) || !(0.02..=0.98).contains(&gy_frac) {
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
                (cx, 0u32),       // top center
                (cx, height - 1), // bottom center
                (0u32, cy),       // left center
                (width - 1, cy),  // right center
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
                label: Some(&format!("Calibration Card {i}")),
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

// ── Headless Output ─────────────────────────────────────────────────
// Codec value types (`RecordingCodec`, `SrtCodec`, `StreamingCodec`) and the
// unified `OutputTarget` are defined in the framework-free `config` module and
// re-exported at the top of this file.

/// A live audio passthrough subscription held by an active output, used to
/// unsubscribe on stop and to report passthrough health (dropped chunks).
/// See spec/audio-passthrough.md.
pub struct AudioPassthrough {
    /// The audio source this output is tee'd from.
    pub source_id: crate::audio::AudioSourceId,
    /// Subscription token, for unsubscribe on stop.
    pub token: crate::audio::PcmToken,
    /// PCM chunks dropped on backpressure (producer side health stat).
    pub dropped: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// A headless output renders content to a GPU texture, reads it back to CPU,
/// and sends it to an external target (NDI, Syphon, recording, SRT).
///
/// Unlike `OutputWindow`, this has no OS window or surface — it renders
/// offscreen via `ReadbackBuffer`.
pub struct HeadlessOutput {
    /// Stable UUID (8-char hex)
    pub uuid: String,
    /// Human-readable name for this output
    pub name: String,
    /// What content to render (Master, Channel, Deck, etc.)
    pub source: OutputSource,
    /// GPU readback infrastructure (double-buffered staging)
    pub readback: super::ReadbackBuffer,
    /// Where to send the readback frames (unified target)
    pub target: OutputTarget,
    /// Offscreen render texture (`COPY_SRC` for readback)
    pub texture: wgpu::Texture,
    /// View into the offscreen render texture
    pub texture_view: wgpu::TextureView,
    /// Blit pipeline for copying source content into the offscreen texture
    pub blit_pipeline: BlitPipeline,
    /// Polygon pipeline for rendering assigned surfaces with warp
    pub polygon_pipeline: PolygonBlitPipeline,
    /// Width of the output
    pub width: u32,
    /// Height of the output
    pub height: u32,
    /// Active ffmpeg subprocess (for Recording/SRT targets). Boxed for the same
    /// reason as `audio_pcm`, and because its inline size is platform-dependent:
    /// unboxed it made `HeadlessOutput` 224 bytes larger than `OutputWindow` on
    /// Windows, tripping `clippy::large_enum_variant` on that platform only.
    pub subprocess: Option<Box<super::FfmpegSubprocess>>,
    /// Active audio passthrough subscription (None = video-only). Boxed to keep
    /// the rarely-set field off the hot `UnifiedOutput` enum's size.
    pub audio_pcm: Option<Box<AudioPassthrough>>,
    /// Whether this output is actively streaming/recording
    pub active: bool,
    /// When this output was started (for duration tracking on non-subprocess outputs)
    pub started_at: Option<std::time::Instant>,
    /// Surface assignments — which surfaces this output renders, with per-surface warp.
    /// Empty = render source directly (fallback behavior).
    pub surface_assignments: Vec<SurfaceAssignment>,
    /// Whether edge blend is auto-computed or manually configured.
    pub edge_blend_mode: super::edge_blend::EdgeBlendMode,
    /// Edge blending configuration for multi-projector overlap zones.
    pub edge_blend: super::edge_blend::EdgeBlendConfig,
    /// GPU pipeline for applying edge blend post-process.
    pub edge_blend_pipeline: super::edge_blend::EdgeBlendPipeline,
    /// Intermediate texture used when edge blending is active.
    pub edge_blend_texture: wgpu::Texture,
    /// View into the intermediate edge blend texture.
    pub edge_blend_texture_view: wgpu::TextureView,
    /// Per-output rotation applied at the final blit stage.
    pub rotation: OutputRotation,
}

/// Result of delivering a frame to an output target.
pub enum DeliveryResult {
    /// Frame delivered successfully (or no-op for unhandled targets).
    Ok,
    /// Subprocess write failed — output should be deactivated.
    Failed(String),
    /// SRT client disconnected: the old subprocess has been stopped and the
    /// caller must respawn the listener. The caller owns the respawn (rather
    /// than this method) so it can re-subscribe audio passthrough — `deliver_frame`
    /// has no `AudioManager` handle.
    SrtNeedsRestart,
}

impl HeadlessOutput {
    /// Deliver readback frame data to the configured output target.
    ///
    /// For subprocess targets (Recording, SRT, HLS, DASH, RTMP), feeds the frame to ffmpeg.
    /// For NDI/Syphon, publishes directly through the respective manager.
    /// Returns a `DeliveryResult` indicating what happened.
    pub fn deliver_frame(
        &mut self,
        frame_data: &[u8],
        ndi_manager: &mut crate::ndi::NdiManager,
    ) -> DeliveryResult {
        match &mut self.target {
            OutputTarget::Recording { .. }
            | OutputTarget::HlsStream { .. }
            | OutputTarget::DashStream { .. }
            | OutputTarget::RtmpStream { .. } => {
                if let Some(sub) = &mut self.subprocess {
                    if !sub.feed_frame(frame_data) {
                        if let Some(mut sub) = self.subprocess.take() {
                            sub.stop();
                        }
                        return DeliveryResult::Failed(format!(
                            "Subprocess write failed for '{}'",
                            self.name
                        ));
                    }
                }
                DeliveryResult::Ok
            }
            OutputTarget::SrtStream { .. } => {
                if let Some(sub) = &mut self.subprocess {
                    if !sub.feed_frame(frame_data) {
                        // Client disconnected. Tear down the dead listener and
                        // hand the respawn back to the caller, which re-subscribes
                        // audio passthrough (this method has no AudioManager).
                        if let Some(mut sub) = self.subprocess.take() {
                            sub.stop();
                        }
                        return DeliveryResult::SrtNeedsRestart;
                    }
                }
                DeliveryResult::Ok
            }
            OutputTarget::NdiSend { ref sender_name } => {
                ndi_manager.send_frame(sender_name, frame_data, self.width, self.height);
                DeliveryResult::Ok
            }
            // Syphon output is published GPU-side (zero-copy) in the headless
            // render loop before this point, so on macOS it never reaches here.
            #[cfg(not(target_os = "macos"))]
            OutputTarget::SyphonServer { .. } => {
                log::warn!("Syphon output not supported on this platform");
                DeliveryResult::Ok
            }
            _ => DeliveryResult::Ok,
        }
    }

    /// Create a new headless output with the given resolution and target.
    ///
    /// # Panics
    ///
    /// Panics if the blit, polygon or edge-blend pipeline cannot be built for
    /// the headless `Rgba8UnormSrgb` format — a device-level failure that
    /// leaves the output unusable.
    pub fn new(
        device: &wgpu::Device,
        name: String,
        source: OutputSource,
        target: OutputTarget,
        width: u32,
        height: u32,
    ) -> Self {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let (texture, texture_view, eb_tex, eb_view) = Self::create_textures(device, width, height);
        let readback = super::ReadbackBuffer::new(device, width, height);
        let blit_pipeline =
            BlitPipeline::new(device, format).expect("Failed to create headless blit pipeline");
        let polygon_pipeline = PolygonBlitPipeline::new(device, format)
            .expect("Failed to create headless polygon pipeline");
        let edge_blend_pipeline = super::edge_blend::EdgeBlendPipeline::new(device, format)
            .expect("Failed to create headless edge blend pipeline");

        Self {
            uuid: crate::deck::generate_short_uuid(),
            name,
            source,
            readback,
            target,
            texture,
            texture_view,
            blit_pipeline,
            polygon_pipeline,
            width,
            height,
            subprocess: None,
            audio_pcm: None,
            active: false,
            started_at: None,
            surface_assignments: Vec::new(),
            edge_blend_mode: super::edge_blend::EdgeBlendMode::default(),
            edge_blend: super::edge_blend::EdgeBlendConfig::default(),
            edge_blend_pipeline,
            edge_blend_texture: eb_tex,
            edge_blend_texture_view: eb_view,
            rotation: OutputRotation::default(),
        }
    }

    /// The render target and the edge-blend intermediate, which are the only
    /// two resources whose size depends on the output dimensions. The pipelines
    /// are size-independent, so [`resize`](Self::resize) rebuilds just these.
    fn create_textures(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
    ) {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Headless Output Texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let eb_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Headless Edge Blend Intermediate"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let eb_view = eb_tex.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, texture_view, eb_tex, eb_view)
    }

    /// Rebuild this output's GPU resources at a new size.
    ///
    /// A headless output used to take the render resolution once, when it was
    /// created, and never look at it again. A recording made before the project
    /// was switched to a portrait resolution therefore kept a landscape buffer,
    /// the portrait composite was stretch-blitted into it, and the saved file
    /// came out the old shape with squashed content. The same held for NDI and
    /// Syphon, which published the stale dimensions.
    ///
    /// Callers must stop any subprocess-backed target before resizing: ffmpeg is
    /// spawned with a fixed `-s WxH` and raw frames of a different size desync
    /// the stream rather than being rejected. NDI and Syphon take the dimensions
    /// per frame and need no restart.
    ///
    /// A resize to the current size is a no-op, so this is safe to call
    /// unconditionally.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return;
        }
        let (texture, texture_view, eb_tex, eb_view) = Self::create_textures(device, width, height);
        self.texture = texture;
        self.texture_view = texture_view;
        self.edge_blend_texture = eb_tex;
        self.edge_blend_texture_view = eb_view;
        self.readback = super::ReadbackBuffer::new(device, width, height);
        self.width = width;
        self.height = height;
    }

    /// Set output rotation. Headless outputs don't have intermediate textures to rebuild,
    /// but the rotation is stored for the blit shader.
    pub fn set_rotation(&mut self, rotation: OutputRotation) {
        self.rotation = rotation;
    }
}

/// Unified output — wraps either a windowed or headless output.
/// Provides shared accessors for name, target, and source.
pub enum UnifiedOutput {
    Window(OutputWindow),
    Headless(HeadlessOutput),
}

impl UnifiedOutput {
    /// Stable UUID of this output.
    pub fn uuid(&self) -> &str {
        match self {
            UnifiedOutput::Window(w) => &w.uuid,
            UnifiedOutput::Headless(h) => &h.uuid,
        }
    }

    /// Human-readable name of this output.
    pub fn name(&self) -> &str {
        match self {
            UnifiedOutput::Window(w) => &w.name,
            UnifiedOutput::Headless(h) => &h.name,
        }
    }

    /// The output target for this output.
    pub fn target(&self) -> &OutputTarget {
        match self {
            UnifiedOutput::Window(w) => &w.target,
            UnifiedOutput::Headless(h) => &h.target,
        }
    }

    /// Whether this output is windowed.
    pub fn is_windowed(&self) -> bool {
        matches!(self, UnifiedOutput::Window(_))
    }

    /// Whether this output is headless.
    pub fn is_headless(&self) -> bool {
        matches!(self, UnifiedOutput::Headless(_))
    }

    /// Whether this headless output is actively streaming/recording.
    pub fn is_active(&self) -> bool {
        match self {
            UnifiedOutput::Window(_) => true, // windowed outputs are always "active"
            UnifiedOutput::Headless(h) => h.active,
        }
    }

    /// Mutable access to surface assignments for either variant.
    pub fn surface_assignments_mut(&mut self) -> &mut Vec<SurfaceAssignment> {
        match self {
            UnifiedOutput::Window(w) => &mut w.surface_assignments,
            UnifiedOutput::Headless(h) => &mut h.surface_assignments,
        }
    }

    /// Immutable access to surface assignments for either variant.
    pub fn surface_assignments(&self) -> &[SurfaceAssignment] {
        match self {
            UnifiedOutput::Window(w) => &w.surface_assignments,
            UnifiedOutput::Headless(h) => &h.surface_assignments,
        }
    }

    /// Current edge blend mode.
    pub fn edge_blend_mode(&self) -> super::edge_blend::EdgeBlendMode {
        match self {
            UnifiedOutput::Window(w) => w.edge_blend_mode,
            UnifiedOutput::Headless(h) => h.edge_blend_mode,
        }
    }

    /// Current edge blend config.
    pub fn edge_blend(&self) -> super::edge_blend::EdgeBlendConfig {
        match self {
            UnifiedOutput::Window(w) => w.edge_blend,
            UnifiedOutput::Headless(h) => h.edge_blend,
        }
    }

    /// Current output rotation.
    pub fn rotation(&self) -> OutputRotation {
        match self {
            UnifiedOutput::Window(w) => w.rotation,
            UnifiedOutput::Headless(h) => h.rotation,
        }
    }

    /// Active duration for headless outputs (subprocess or NDI/Syphon).
    pub fn active_duration(&self) -> std::time::Duration {
        match self {
            UnifiedOutput::Window(_) => std::time::Duration::ZERO,
            UnifiedOutput::Headless(h) => {
                // Subprocess-based outputs (Recording/SRT) track their own duration
                if let Some(sub) = &h.subprocess {
                    return sub.duration();
                }
                // Non-subprocess outputs (NDI/Syphon) use started_at timestamp
                h.started_at.map(|t| t.elapsed()).unwrap_or_default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{aspect_fit_rect, HeadlessOutput, OutputRotation, OutputWindow};

    /// `UnifiedOutput` holds both variants inline, and `clippy::large_enum_variant`
    /// fails the build when one exceeds the other by more than 200 bytes. The two
    /// stay close only because `HeadlessOutput`'s rarely-set `subprocess` and
    /// `audio_pcm` fields are boxed. With `subprocess` unboxed the gap was 184
    /// bytes on macOS but 224 on Windows — under the threshold on the platform
    /// most of us build on, over it on the one we don't. This catches that in the
    /// test suite, which runs everywhere, rather than in a per-platform lint job.
    #[test]
    fn output_variants_stay_under_the_large_enum_variant_threshold() {
        let window = std::mem::size_of::<OutputWindow>();
        let headless = std::mem::size_of::<HeadlessOutput>();
        let gap = window.abs_diff(headless);
        assert!(
            gap <= 200,
            "UnifiedOutput variants differ by {gap} bytes (OutputWindow {window}, \
             HeadlessOutput {headless}) — box a rarely-set field on the larger variant"
        );
    }

    /// Content and canvas agree, so nothing is inset — the case every 16:9
    /// project has always been in, and the one that must not change.
    #[test]
    fn matching_aspect_fills_the_canvas() {
        assert_eq!(
            aspect_fit_rect(16.0 / 9.0, 1920, 1080),
            [0.0, 0.0, 1.0, 1.0]
        );
        assert_eq!(aspect_fit_rect(1.0, 800, 800), [0.0, 0.0, 1.0, 1.0]);
    }

    /// A portrait project in a landscape window: full height, pillarboxed.
    #[test]
    fn portrait_content_in_a_landscape_canvas_is_pillarboxed() {
        let [x, y, w, h] = aspect_fit_rect(1080.0 / 1920.0, 1920, 1080);
        // 9:16 inside 16:9 leaves a column (9/16)/(16/9) = 0.3164 of the width.
        assert!((w - 0.316_406).abs() < 1e-4, "width {w}");
        assert_eq!(h, 1.0);
        assert!((x - (1.0 - w) / 2.0).abs() < 1e-6, "not centred: {x}");
        assert_eq!(y, 0.0);
    }

    /// The inverse, which is what a 16:9 project sent to a phone-shaped output
    /// gets: full width, bars above and below.
    #[test]
    fn landscape_content_in_a_portrait_canvas_is_letterboxed() {
        let [x, y, w, h] = aspect_fit_rect(16.0 / 9.0, 1080, 1920);
        assert_eq!(w, 1.0);
        assert!((h - 0.316_406).abs() < 1e-4, "height {h}");
        assert_eq!(x, 0.0);
        assert!((y - (1.0 - h) / 2.0).abs() < 1e-6, "not centred: {y}");
    }

    /// The fitted rectangle must stay inside the canvas whatever it is handed,
    /// or content spills off the edge of the projector.
    #[test]
    fn the_fitted_rect_never_leaves_the_unit_square() {
        for aspect in [0.1_f32, 0.5, 1.0, 1.777, 4.0, 32.0] {
            for (cw, ch) in [(1920u32, 1080u32), (1080, 1920), (1000, 1000), (3840, 800)] {
                let [x, y, w, h] = aspect_fit_rect(aspect, cw, ch);
                assert!(
                    x >= 0.0 && y >= 0.0 && x + w <= 1.000_01 && y + h <= 1.000_01,
                    "aspect {aspect} in {cw}×{ch} gave {:?}",
                    [x, y, w, h]
                );
            }
        }
    }

    /// A malformed scene or a window mid-minimise must not produce a NaN quad.
    #[test]
    fn degenerate_inputs_fall_back_to_filling() {
        assert_eq!(aspect_fit_rect(16.0 / 9.0, 0, 0), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(aspect_fit_rect(0.0, 1920, 1080), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(aspect_fit_rect(-2.0, 1920, 1080), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(aspect_fit_rect(f32::NAN, 1920, 1080), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(
            aspect_fit_rect(f32::INFINITY, 1920, 1080),
            [0.0, 0.0, 1.0, 1.0]
        );
    }

    #[test]
    fn output_rotation_default_is_deg0() {
        assert_eq!(OutputRotation::default(), OutputRotation::Deg0);
    }

    #[test]
    fn output_rotation_index_values() {
        assert_eq!(OutputRotation::Deg0.index(), 0);
        assert_eq!(OutputRotation::Deg90.index(), 1);
        assert_eq!(OutputRotation::Deg180.index(), 2);
        assert_eq!(OutputRotation::Deg270.index(), 3);
    }

    #[test]
    fn output_rotation_swaps_dimensions() {
        assert!(!OutputRotation::Deg0.swaps_dimensions());
        assert!(OutputRotation::Deg90.swaps_dimensions());
        assert!(!OutputRotation::Deg180.swaps_dimensions());
        assert!(OutputRotation::Deg270.swaps_dimensions());
    }

    #[test]
    fn output_rotation_effective_dimensions() {
        assert_eq!(
            OutputRotation::Deg0.effective_dimensions(1920, 1080),
            (1920, 1080)
        );
        assert_eq!(
            OutputRotation::Deg90.effective_dimensions(1920, 1080),
            (1080, 1920)
        );
        assert_eq!(
            OutputRotation::Deg180.effective_dimensions(1920, 1080),
            (1920, 1080)
        );
        assert_eq!(
            OutputRotation::Deg270.effective_dimensions(1920, 1080),
            (1080, 1920)
        );
    }

    #[test]
    fn output_rotation_labels() {
        assert_eq!(OutputRotation::Deg0.label(), "0°");
        assert_eq!(OutputRotation::Deg90.label(), "90°");
        assert_eq!(OutputRotation::Deg180.label(), "180°");
        assert_eq!(OutputRotation::Deg270.label(), "270°");
    }

    #[test]
    fn output_rotation_all_contains_all_variants() {
        assert_eq!(OutputRotation::ALL.len(), 4);
        assert_eq!(OutputRotation::ALL[0], OutputRotation::Deg0);
        assert_eq!(OutputRotation::ALL[1], OutputRotation::Deg90);
        assert_eq!(OutputRotation::ALL[2], OutputRotation::Deg180);
        assert_eq!(OutputRotation::ALL[3], OutputRotation::Deg270);
    }

    #[test]
    fn output_rotation_serde_roundtrip() {
        for rot in OutputRotation::ALL {
            let json = serde_json::to_string(&rot).unwrap();
            let deserialized: OutputRotation = serde_json::from_str(&json).unwrap();
            assert_eq!(rot, deserialized);
        }
    }

    #[test]
    fn output_rotation_deserialize_default() {
        // Missing field should deserialize as Deg0
        let config: OutputRotation = serde_json::from_str("\"Deg0\"").unwrap();
        assert_eq!(config, OutputRotation::Deg0);
    }

    #[test]
    fn headless_context_enables_bc_when_adapter_supports() {
        // Headless installations must take the HAP GPU path, so the headless
        // device has to request TEXTURE_COMPRESSION_BC whenever the adapter
        // exposes it. Skips gracefully when no GPU adapter is available.
        let Ok(gpu) = super::GpuContext::new_headless() else {
            return;
        };
        let adapter_bc = gpu
            .adapter
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let device_bc = gpu
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        assert_eq!(
            adapter_bc, device_bc,
            "headless device should request BC iff the adapter supports it"
        );
    }
}
