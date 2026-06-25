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
    /// Linear-light compositing format (Rgba16Float) for channel/mixer composites.
    /// Distinct from `texture_format` which is the surface/presentation format.
    pub compositing_format: wgpu::TextureFormat,
    pub timestamp_supported: bool,
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
        let (instance, surface, size) = Self::create_surface_for_window(window)?;
        Self::new_with_surface(instance, surface, size).await
    }

    /// Create the wgpu instance and surface on the current (main) thread.
    /// On macOS, `create_surface` accesses `NSView`/`CAMetalLayer` which must
    /// happen on the main thread.  The returned objects are `Send` and can be
    /// passed to a background thread for adapter/device creation.
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
            display: Default::default(),
            memory_budget_thresholds: Default::default(),
        });
        let surface = instance
            .create_surface(window)
            .context("Failed to create surface")?;
        Ok((instance, surface, size))
    }

    /// Complete GPU initialization given a pre-created instance and surface.
    /// Safe to call from a background thread — all Metal dispatch work is
    /// resolved through the pre-created surface.
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
                memory_hints: Default::default(),
                experimental_features: Default::default(),
                trace: Default::default(),
            })
            .await
            .context("Failed to create device")?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
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

        let gpu = GpuContext {
            instance,
            adapter,
            device,
            queue,
            texture_format: surface_format,
            compositing_format: wgpu::TextureFormat::Rgba16Float,
            timestamp_supported,
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
            if adapter
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            {
                required_features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
                timestamp_supported = true;
                log::info!("GPU supports timestamp queries inside encoders (GPU timing enabled)");
            } else {
                log::warn!("GPU supports TIMESTAMP_QUERY but not TIMESTAMP_QUERY_INSIDE_ENCODERS — GPU timing disabled");
            }
        }

        (required_features, timestamp_supported)
    }

    /// Create a headless GPU context (no window surface).
    ///
    /// Requests the same optional features as the windowed path (notably
    /// `TEXTURE_COMPRESSION_BC`) so HAP video uses the GPU-native BCn path in
    /// headless installations. Falls back to software adapter if no hardware
    /// GPU is available. Used for headless mode and tests.
    pub fn new_headless() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: Default::default(),
            memory_budget_thresholds: Default::default(),
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
            memory_hints: Default::default(),
            experimental_features: Default::default(),
            trace: Default::default(),
        }))
        .context("Failed to create headless device")?;

        Ok(GpuContext {
            instance,
            adapter,
            device,
            queue,
            texture_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            compositing_format: wgpu::TextureFormat::Rgba16Float,
            timestamp_supported,
        })
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

    /// Create a texture for compositing in linear-light space (Rgba16Float).
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

/// Per-output rotation applied at the final blit stage.
/// For 90°/270°, intermediate textures are created at swapped dimensions
/// (portrait content for landscape projectors and vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OutputRotation {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl OutputRotation {
    /// All rotation variants for UI dropdowns.
    pub const ALL: [OutputRotation; 4] = [
        OutputRotation::Deg0,
        OutputRotation::Deg90,
        OutputRotation::Deg180,
        OutputRotation::Deg270,
    ];

    /// GPU-side index (0–3) for the shader uniform.
    pub fn index(&self) -> u32 {
        match self {
            OutputRotation::Deg0 => 0,
            OutputRotation::Deg90 => 1,
            OutputRotation::Deg180 => 2,
            OutputRotation::Deg270 => 3,
        }
    }

    /// Whether this rotation swaps width and height.
    pub fn swaps_dimensions(&self) -> bool {
        matches!(self, OutputRotation::Deg90 | OutputRotation::Deg270)
    }

    /// Effective texture dimensions after rotation.
    /// For 0°/180° returns (w, h); for 90°/270° returns (h, w).
    pub fn effective_dimensions(&self, w: u32, h: u32) -> (u32, u32) {
        if self.swaps_dimensions() {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Human-readable label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            OutputRotation::Deg0 => "0°",
            OutputRotation::Deg90 => "90°",
            OutputRotation::Deg180 => "180°",
            OutputRotation::Deg270 => "270°",
        }
    }
}

/// Content source that an output window can display
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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
    /// The domemaster fisheye output (equidistant azimuthal projection)
    Domemaster,
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
            OutputSource::Domemaster => write!(f, "Domemaster"),
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
    /// Warp mode: CornerPin or Mesh. None = no warp (render at polygon's native position).
    pub warp_mode: Option<super::warp::WarpMode>,
    /// Per-surface overlap zones (Auto mode). Default = no zones.
    pub overlap_zones: super::edge_blend::SurfaceOverlapZones,
}

/// Assignment of a surface to an output, with per-surface warp calibration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SurfaceAssignment {
    /// UUID of the assigned surface
    pub surface_uuid: String,
    /// Warp mode: corner-pin (4-point homography) or arbitrary mesh warp.
    pub warp_mode: super::warp::WarpMode,
    /// Whether this assignment is enabled
    pub enabled: bool,
    /// Per-surface overlap zones (set by Auto mode detection).
    #[serde(default)]
    pub overlap_zones: super::edge_blend::SurfaceOverlapZones,
}

/// Where an output sends its content — unified across windowed and headless outputs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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
    /// Record frames to a video file via ffmpeg subprocess
    Recording {
        path: String,
        codec: RecordingCodec,
        /// Audio passthrough device NAME (None = silent). See spec/audio-passthrough.md.
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames via SRT (Secure Reliable Transport) through ffmpeg
    SrtStream {
        url: String,
        codec: SrtCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames as HLS segments via ffmpeg
    HlsStream {
        name: String,
        codec: StreamingCodec,
        low_latency: bool,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames as DASH segments via ffmpeg
    DashStream {
        name: String,
        codec: StreamingCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Push frames to an RTMP/RTMPS ingest endpoint via ffmpeg
    RtmpStream {
        url: String,
        codec: StreamingCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Send frames over NDI network protocol
    NdiSend { sender_name: String },
    /// Publish frames via Syphon (macOS inter-app sharing)
    SyphonServer { server_name: String },
}

impl OutputTarget {
    /// Whether this target requires an OS window.
    pub fn is_windowed(&self) -> bool {
        matches!(self, OutputTarget::Windowed | OutputTarget::Display { .. })
    }

    /// Whether this target is headless (no OS window).
    pub fn is_headless(&self) -> bool {
        !self.is_windowed()
    }

    /// The selected audio passthrough device name, if this is an ffmpeg target
    /// configured with audio. `None` for video-only or non-ffmpeg targets.
    pub fn audio_device(&self) -> Option<&str> {
        match self {
            OutputTarget::Recording { audio_device, .. }
            | OutputTarget::SrtStream { audio_device, .. }
            | OutputTarget::HlsStream { audio_device, .. }
            | OutputTarget::DashStream { audio_device, .. }
            | OutputTarget::RtmpStream { audio_device, .. } => audio_device.as_deref(),
            _ => None,
        }
    }

    /// Return a clone of this target with the audio passthrough device replaced.
    /// No-op for non-ffmpeg targets. Lets the GUI flip the device without
    /// re-specifying every variant field.
    pub fn with_audio_device(&self, device: Option<String>) -> OutputTarget {
        let mut target = self.clone();
        match &mut target {
            OutputTarget::Recording { audio_device, .. }
            | OutputTarget::SrtStream { audio_device, .. }
            | OutputTarget::HlsStream { audio_device, .. }
            | OutputTarget::DashStream { audio_device, .. }
            | OutputTarget::RtmpStream { audio_device, .. } => *audio_device = device,
            _ => {}
        }
        target
    }
}

impl std::fmt::Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::Windowed => write!(f, "Windowed"),
            OutputTarget::Display { name, .. } => write!(f, "{}", name),
            OutputTarget::Recording { path, codec, .. } => write!(f, "Rec [{}]: {}", codec, path),
            OutputTarget::SrtStream { url, codec, .. } => write!(f, "SRT [{}]: {}", codec, url),
            OutputTarget::HlsStream {
                name,
                codec,
                low_latency,
                ..
            } => {
                if *low_latency {
                    write!(f, "LL-HLS [{}]: {}", codec, name)
                } else {
                    write!(f, "HLS [{}]: {}", codec, name)
                }
            }
            OutputTarget::DashStream { name, codec, .. } => write!(f, "DASH [{}]: {}", codec, name),
            OutputTarget::RtmpStream { url, codec, .. } => write!(f, "RTMP [{}]: {}", codec, url),
            OutputTarget::NdiSend { sender_name } => write!(f, "NDI: {}", sender_name),
            OutputTarget::SyphonServer { server_name } => write!(f, "Syphon: {}", server_name),
        }
    }
}

/// An output window that displays content on a separate display/projector.
///
/// Each output window has its own OS window and wgpu surface, but shares
/// the device and queue from the GpuContext.
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
    /// Whether calibration mode is active (shows warp handles)
    pub calibration_mode: bool,
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
    /// When edge blend is on, edge blend shader writes here from surface_texture.
    pub preview_texture: wgpu::Texture,
    pub preview_texture_view: wgpu::TextureView,
    /// Per-output rotation applied at the final blit stage.
    pub rotation: OutputRotation,
}

impl OutputWindow {
    /// Create a new output window with its own surface, sharing the given device/queue.
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
            .find(|f| f.is_srgb())
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
            calibration_mode: false,
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

    /// Render the routed content to this output window's surface (simple single-source blit)
    pub fn render(&self, context: &GpuContext, content_view: &wgpu::TextureView) {
        let fullscreen_quad: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        self.render_surfaces(
            context,
            &[SurfaceRenderInfo {
                content_view,
                vertices: &fullscreen_quad,
                bounding_box: [0.0, 0.0, 1.0, 1.0],
                uv_scale: [1.0, 1.0],
                uv_offset: [0.0, 0.0],
                warp_mode: None,
                overlap_zones: Default::default(),
            }],
        );
    }

    /// Render multiple surfaces composited at their canvas positions.
    /// Each surface is rendered as a textured polygon using fan triangulation.
    /// Warp is applied per the WarpMode: CornerPin uses homography in the vertex shader,
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

                // Dispatch warp mode: CornerPin → homography, Mesh → vertex-baked, None → identity
                let (homography, vertices) = match &surf.warp_mode {
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
                };

                super::blit::PolygonDrawDesc {
                    content_view: surf.content_view,
                    uv_scale: surf.uv_scale,
                    uv_offset: surf.uv_offset,
                    homography,
                    overlap_zones: &surf.overlap_zones,
                    vertices,
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

        context.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    /// Set the display target for this output window.
    /// `monitor` should be the MonitorHandle for Display targets.
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

// ── Headless Output ─────────────────────────────────────────────────

/// Recording codec for ffmpeg subprocess.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum RecordingCodec {
    /// H.264 ultrafast preset (-c:v libx264 -preset ultrafast -crf 18)
    H264,
    /// H.265 / HEVC (-c:v libx265 -preset ultrafast -crf 20)
    H265,
    /// AV1 via SVT-AV1 (-c:v libsvtav1 -preset 10 -crf 28)
    AV1,
    /// ProRes 422 (-c:v prores_ks -profile:v 2)
    ProRes,
    /// HAP (-c:v hap -format hap)
    Hap,
    /// HAP Alpha (-c:v hap -format hap_alpha)
    HapAlpha,
    /// HAP Q (-c:v hap -format hap_q)
    HapQ,
}

impl std::fmt::Display for RecordingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingCodec::H264 => write!(f, "H.264"),
            RecordingCodec::H265 => write!(f, "H.265 (HEVC)"),
            RecordingCodec::AV1 => write!(f, "AV1"),
            RecordingCodec::ProRes => write!(f, "ProRes 422"),
            RecordingCodec::Hap => write!(f, "HAP"),
            RecordingCodec::HapAlpha => write!(f, "HAP Alpha"),
            RecordingCodec::HapQ => write!(f, "HAP Q"),
        }
    }
}

/// Streaming codec for SRT output.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum SrtCodec {
    /// H.264 ultrafast + zerolatency
    #[default]
    H264,
    /// H.265 / HEVC ultrafast + zerolatency
    H265,
}

impl std::fmt::Display for SrtCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SrtCodec::H264 => write!(f, "H.264"),
            SrtCodec::H265 => write!(f, "H.265 (HEVC)"),
        }
    }
}

/// Streaming codec for HLS/DASH output.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum StreamingCodec {
    /// H.264 ultrafast preset
    #[default]
    H264,
    /// H.265 / HEVC ultrafast preset
    H265,
    /// AV1 via SVT-AV1
    AV1,
}

impl std::fmt::Display for StreamingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamingCodec::H264 => write!(f, "H.264"),
            StreamingCodec::H265 => write!(f, "H.265 (HEVC)"),
            StreamingCodec::AV1 => write!(f, "AV1"),
        }
    }
}

// HeadlessOutputTarget has been merged into the unified OutputTarget enum above.

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
/// Unlike OutputWindow, this has no OS window or surface — it renders
/// offscreen via ReadbackBuffer.
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
    /// Offscreen render texture (COPY_SRC for readback)
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
    /// Active ffmpeg subprocess (for Recording/SRT targets)
    pub subprocess: Option<super::FfmpegSubprocess>,
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
        #[cfg(target_os = "macos")] syphon_manager: &mut crate::syphon::SyphonManager,
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
            #[cfg(target_os = "macos")]
            OutputTarget::SyphonServer { .. } => {
                syphon_manager.publish_frame(frame_data, self.width, self.height);
                DeliveryResult::Ok
            }
            #[cfg(not(target_os = "macos"))]
            OutputTarget::SyphonServer { .. } => {
                log::warn!("Syphon output not supported on this platform");
                DeliveryResult::Ok
            }
            _ => DeliveryResult::Ok,
        }
    }

    /// Create a new headless output with the given resolution and target.
    pub fn new(
        device: &wgpu::Device,
        name: String,
        source: OutputSource,
        target: OutputTarget,
        width: u32,
        height: u32,
    ) -> Self {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Headless Output Texture"),
            size: wgpu::Extent3d {
                width,
                height,
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
        let readback = super::ReadbackBuffer::new(device, width, height);
        let blit_pipeline =
            BlitPipeline::new(device, format).expect("Failed to create headless blit pipeline");
        let polygon_pipeline = PolygonBlitPipeline::new(device, format)
            .expect("Failed to create headless polygon pipeline");
        let edge_blend_pipeline = super::edge_blend::EdgeBlendPipeline::new(device, format)
            .expect("Failed to create headless edge blend pipeline");
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
    use super::OutputRotation;

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
