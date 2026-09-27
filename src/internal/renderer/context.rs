use anyhow::{Context, Result};
use wgpu::util::DeviceExt;
use winit::window::Window;

// Plain, framework-free output value types live in `config` so the engine
// contract layer can name them without importing this wgpu/winit file. Kept
// re-exported here so existing `crate::renderer::context::…` paths still work.
pub use super::config::{
    AlphaMode, CalibrationMode, ModeAvailability, OutputRotation, OutputSource,
    PresentationCapabilities, PresentationColorProfile, PresentationDepth, PresentationFormat,
    PresentationMode, PresentationPixelFormat, PresentationRequest, PresentationTransfer,
    RecordingCodec, ResolvedPresentation, RtmpCodecContract, SrtCodec, StreamingCodec, TonemapMode,
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

fn surface_pixel_format(format: wgpu::TextureFormat) -> PresentationPixelFormat {
    match format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            PresentationPixelFormat::Bgra8
        }
        _ => PresentationPixelFormat::Rgba8,
    }
}

pub(crate) struct SurfacePresentationSelection {
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) color_space: wgpu::SurfaceColorSpace,
    pub(crate) resolved: ResolvedPresentation,
    /// Every mode with the reason this surface cannot deliver it. Computed here
    /// because the capabilities are already in hand; re-deriving it later would
    /// mean re-querying the surface every frame.
    /// See /spec/presentation-mode-offering.md.
    pub(crate) mode_availability: Vec<ModeAvailability>,
}

pub(crate) fn select_surface_presentation(
    request: PresentationRequest,
    capabilities: &wgpu::SurfaceCapabilities,
) -> Result<SurfacePresentationSelection> {
    let sdr8_format = capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| capabilities.formats.first().copied())
        .context("output surface exposes no SDR presentation format")?;
    // EDR is a float surface, unlike every other contract here. Requested
    // explicitly rather than relying on `Auto`, which resolves to
    // `ExtendedSrgbLinear` for `Rgba16Float` and would make an HDR contract an
    // accident of format choice. See /spec/hdr-edr-display.md.
    let supports_edr = capabilities
        .color_spaces(wgpu::TextureFormat::Rgba16Float)
        .contains(wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR);
    let rgb10_color_spaces = capabilities.color_spaces(wgpu::TextureFormat::Rgb10a2Unorm);
    let supports_rgb10_srgb = rgb10_color_spaces.contains(wgpu::SurfaceColorSpaces::SRGB);
    // HDR10 on a display needs the PQ colour space on the same format. Selecting
    // RGB10A2 alone proves nothing about the transfer the compositor will apply.
    let supports_rgb10_pq = rgb10_color_spaces.contains(wgpu::SurfaceColorSpaces::BT2100_PQ);

    // Adapter preference order: the resolver reads this top down when it has to
    // degrade, so HDR first, then the widest SDR result.
    let mut formats = Vec::with_capacity(4);
    if supports_edr {
        formats.push(PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::EdrLinear,
            pixel_format: PresentationPixelFormat::Rgba16,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
        });
    }
    if supports_rgb10_pq {
        formats.push(PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::Hdr10Pq,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Full,
            alpha_mode: AlphaMode::Opaque,
        });
    }
    if supports_rgb10_srgb {
        formats.push(PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::Sdr,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
        });
    }
    formats.push(PresentationFormat {
        depth: PresentationDepth::Sdr8,
        transfer: PresentationTransfer::Sdr,
        pixel_format: surface_pixel_format(sdr8_format),
        color_profile: PresentationColorProfile::SrgbFull,
        alpha_mode: AlphaMode::Opaque,
    });
    let reason = (!supports_edr && request.transfer == PresentationTransfer::EdrLinear)
        .then(|| {
            "this display surface does not expose an extended-range float color space".to_string()
        })
        .or_else(|| {
            (!supports_rgb10_pq && request.transfer.is_hdr()).then(|| {
                if supports_rgb10_srgb {
                    "this display surface offers RGB10A2 but not the BT.2100 PQ color space"
                        .to_string()
                } else {
                    "this display surface does not expose RGB10A2 with an HDR color space"
                        .to_string()
                }
            })
        })
        .or_else(|| {
            (!supports_rgb10_srgb).then(|| {
                if capabilities
                    .format_capabilities
                    .iter()
                    .any(|candidate| candidate.format == wgpu::TextureFormat::Rgb10a2Unorm)
                {
                    "RGB10A2 is available, but not with an sRGB color-space contract".to_string()
                } else {
                    "the output surface does not expose RGB10A2".to_string()
                }
            })
        });
    let capabilities = PresentationCapabilities::new(formats, reason);
    let mode_availability = capabilities.mode_availability();
    let resolved = capabilities.resolve(request)?;
    let (format, color_space) = if resolved.transfer == PresentationTransfer::EdrLinear {
        (
            wgpu::TextureFormat::Rgba16Float,
            wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
        )
    } else if resolved.transfer.is_hdr() {
        (
            wgpu::TextureFormat::Rgb10a2Unorm,
            wgpu::SurfaceColorSpace::Bt2100Pq,
        )
    } else if resolved.resolved == PresentationDepth::Sdr10 {
        (
            wgpu::TextureFormat::Rgb10a2Unorm,
            wgpu::SurfaceColorSpace::Srgb,
        )
    } else {
        (sdr8_format, wgpu::SurfaceColorSpace::Auto)
    };

    Ok(SurfacePresentationSelection {
        format,
        color_space,
        resolved,
        mode_availability,
    })
}

/// GPU rendering context — device, queue, and adapter.
///
/// Owns the GPU resources needed for rendering (mixer, deck, channel, effects).
/// Does NOT own any window surface — that's a presentation concern owned by
/// the UI consumer (`WindowSurface`) or output windows (`crate::output::window`).
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

/// Which backends to ask wgpu for, in the order that decides ties.
///
/// **Windows gets DX12 alone, on purpose.** With `Backends::all()`, wgpu
/// registers Vulkan before DX12 and then sorts adapters by device type with a
/// *stable* sort. A discrete GPU reports `DiscreteGpu` on both backends, so the
/// keys tie and enumeration order wins: Vulkan, on essentially every machine
/// with normal drivers.
///
/// That silently disables Spout. The bridge needs a real `ID3D12Device` through
/// `as_hal::<Dx12>`, which returns `None` on a Vulkan device, so
/// `SpoutManager` marks itself unavailable and the feature is dead for the users
/// it was built for. `WGPU_BACKEND` cannot rescue it either, since that is only
/// read by `InstanceDescriptor::from_env_or_default`, which this does not call.
///
/// Requiring D3D is what every Spout implementation does. `KlakSpout`, the Unity
/// plugin, states it outright: "currently supports only Direct3D 11 and 12;
/// other graphics APIs such as OpenGL or Vulkan aren't available", and makes
/// users change Unity's graphics API by hand. Notch says the same. The Rust
/// `spout2-rs` crate exposes `dx`, `dx12` and `gl` and no Vulkan. Spout itself
/// has never had a Vulkan path: the request has been open since 2021. Choosing
/// the backend here means a Varda user never has to know any of that.
///
/// Everywhere else keeps `all()`, so macOS still gets Metal and Linux still gets
/// Vulkan. See [`fallback_backends`] for what happens if DX12 is absent.
fn preferred_backends() -> wgpu::Backends {
    if cfg!(target_os = "windows") {
        wgpu::Backends::DX12
    } else {
        wgpu::Backends::all()
    }
}

/// What to try when [`preferred_backends`] finds no adapter at all.
///
/// A Windows machine with no D3D12 is unusual but not impossible: a very old
/// GPU, a stripped container, a remote session. Losing Spout there is much
/// better than refusing to start, so the second attempt asks for everything.
fn fallback_backends() -> wgpu::Backends {
    wgpu::Backends::all()
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

        let build = |backends: wgpu::Backends| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                flags: wgpu::InstanceFlags::default(),
                backend_options: wgpu::BackendOptions::default(),
                display: None,
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            });
            let surface = instance.create_surface(window)?;
            Ok::<_, wgpu::CreateSurfaceError>((instance, surface))
        };

        let (instance, surface) =
            build(preferred_backends()).context("Failed to create surface")?;

        // Probe for an adapter that can actually drive this surface before
        // committing. A backend can exist and still present nothing usable, and
        // finding that out here means one honest fallback rather than a failure
        // later that looks like a driver problem. See `fallback_backends`.
        if preferred_backends() != fallback_backends() {
            let usable =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                }));
            if usable.is_err() {
                log::warn!(
                    "No surface-compatible adapter on {:?}; retrying on every backend. \
                     Spout needs Dx12 and will report unavailable.",
                    preferred_backends()
                );
                drop(surface);
                let (instance, surface) =
                    build(fallback_backends()).context("Failed to create surface")?;
                return Ok((instance, surface, size));
            }
        }

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
                apply_limit_buckets: false,
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
            color_space: wgpu::SurfaceColorSpace::Auto,
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
            log::warn!(
                "GPU does not support BC texture compression — HAP video will fall back to ffmpeg CPU decode"
            );
        }

        let rgba16_features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        let rgba16_renderable = adapter.features().contains(rgba16_features)
            && adapter
                .get_texture_format_features(wgpu::TextureFormat::Rgba16Unorm)
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
        if rgba16_renderable {
            required_features |= rgba16_features;
            log::info!("GPU supports normalized RGBA16 recording targets");
        } else {
            log::warn!(
                "GPU does not support normalized RGBA16 targets — 10-bit ProRes 4444 will \
                 fall back to eight-bit SDR"
            );
        }

        let mut timestamp_supported = false;
        if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
            if !adapter
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            {
                log::warn!(
                    "GPU supports TIMESTAMP_QUERY but not TIMESTAMP_QUERY_INSIDE_ENCODERS — GPU timing disabled"
                );
            } else if encoder_timestamps_are_trustworthy(adapter) {
                required_features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
                timestamp_supported = true;
                log::info!("GPU supports timestamp queries inside encoders (GPU timing enabled)");
            } else {
                log::warn!(
                    "Apple GPU: encoder timestamps are not implementable on this hardware — GPU timing disabled"
                );
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
            backends: preferred_backends(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let headless_adapter = |instance: &wgpu::Instance| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))
        };

        // Second attempt on the widest set, so a machine without the preferred
        // backend still starts. See `fallback_backends`.
        let (instance, adapter) = match headless_adapter(&instance) {
            Ok(adapter) => (instance, adapter),
            Err(_) if preferred_backends() != fallback_backends() => {
                log::warn!(
                    "No adapter on {:?}; retrying on every backend. Spout needs Dx12 \
                     and will report unavailable.",
                    preferred_backends()
                );
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: fallback_backends(),
                    flags: wgpu::InstanceFlags::default(),
                    backend_options: wgpu::BackendOptions::default(),
                    display: None,
                    memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                });
                let adapter = headless_adapter(&instance)
                    .context("Failed to find GPU adapter for headless context")?;
                (instance, adapter)
            }
            Err(e) => {
                return Err(anyhow::Error::new(e))
                    .context("Failed to find GPU adapter for headless context");
            }
        };

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
    pub warp_mode: Option<crate::surface::warp::WarpMode>,
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

#[cfg(test)]
mod tests {
    use super::{OutputRotation, aspect_fit_rect, select_surface_presentation};

    use crate::engine::value::render::{
        PresentationDepth, PresentationRequest, PresentationTransfer,
    };

    // ── offered modes per target (/spec/presentation-mode-offering.md) ──

    fn surface_capabilities(
        format_capabilities: Vec<wgpu::SurfaceFormatCapabilities>,
    ) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: vec![
                wgpu::TextureFormat::Bgra8UnormSrgb,
                wgpu::TextureFormat::Rgb10a2Unorm,
            ],
            format_capabilities,
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        }
    }

    #[test]
    fn ten_bit_surface_requires_rgb10_and_srgb_as_a_pair() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: true,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.format, wgpu::TextureFormat::Rgb10a2Unorm);
        assert_eq!(selected.color_space, wgpu::SurfaceColorSpace::Srgb);
        assert_eq!(selected.resolved.resolved, PresentationDepth::Sdr10);
        assert!(selected.resolved.fallback_reason.is_none());
    }

    #[test]
    fn edr_selects_a_float_surface_and_the_extended_linear_color_space() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgba16Float,
            color_spaces: wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                transfer: PresentationTransfer::EdrLinear,
                peak_nits: 1000,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.format, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            selected.color_space,
            wgpu::SurfaceColorSpace::ExtendedSrgbLinear
        );
        assert_eq!(selected.resolved.transfer, PresentationTransfer::EdrLinear);
        // The peak names the deliverable being monitored, not the display.
        assert_eq!(selected.resolved.peak_nits, Some(1000));
        assert!(selected.resolved.fallback_reason.is_none());
    }

    #[test]
    fn edr_falls_back_and_says_so_without_an_extended_range_surface() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                transfer: PresentationTransfer::EdrLinear,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.resolved.transfer, PresentationTransfer::Sdr);
        let reason = selected
            .resolved
            .fallback_reason
            .expect("a degraded EDR request must explain itself");
        assert!(reason.contains("extended-range"), "reason was: {reason}");
    }

    #[test]
    fn an_sdr_request_is_never_answered_with_an_edr_surface() {
        // EDR is listed first in adapter preference order, so this guards the
        // same way the PQ test does: preference must not override the request.
        let capabilities = surface_capabilities(vec![
            wgpu::SurfaceFormatCapabilities {
                format: wgpu::TextureFormat::Rgba16Float,
                color_spaces: wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
            },
            wgpu::SurfaceFormatCapabilities {
                format: wgpu::TextureFormat::Rgb10a2Unorm,
                color_spaces: wgpu::SurfaceColorSpaces::SRGB,
            },
        ]);
        let selected = select_surface_presentation(
            PresentationRequest {
                depth: PresentationDepth::Sdr10,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();
        assert_eq!(selected.resolved.transfer, PresentationTransfer::Sdr);
    }

    #[test]
    fn hdr10_display_selects_rgb10_with_the_pq_color_space() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::BT2100_PQ,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                transfer: PresentationTransfer::Hdr10Pq,
                peak_nits: 1000,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.format, wgpu::TextureFormat::Rgb10a2Unorm);
        assert_eq!(selected.color_space, wgpu::SurfaceColorSpace::Bt2100Pq);
        assert_eq!(selected.resolved.transfer, PresentationTransfer::Hdr10Pq);
        assert_eq!(selected.resolved.peak_nits, Some(1000));
        assert!(selected.resolved.fallback_reason.is_none());
    }

    #[test]
    fn hdr10_falls_back_to_ten_bit_sdr_when_only_srgb_is_offered() {
        // RGB10A2 alone proves nothing about the transfer the compositor applies,
        // so the PQ colour space is required, not merely the format.
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                transfer: PresentationTransfer::Hdr10Pq,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.color_space, wgpu::SurfaceColorSpace::Srgb);
        assert_eq!(selected.resolved.transfer, PresentationTransfer::Sdr);
        assert_eq!(selected.resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(selected.resolved.peak_nits, None);
        let reason = selected
            .resolved
            .fallback_reason
            .expect("an HDR request that degrades must say why");
        assert!(reason.contains("PQ"), "reason was: {reason}");
    }

    #[test]
    fn an_sdr_request_is_never_answered_with_a_pq_surface() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::BT2100_PQ,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                depth: PresentationDepth::Sdr10,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.color_space, wgpu::SurfaceColorSpace::Srgb);
        assert_eq!(selected.resolved.transfer, PresentationTransfer::Sdr);
    }

    #[test]
    fn rgb10_without_srgb_falls_back_to_eight_bit() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::DISPLAY_P3,
        }]);
        let selected = select_surface_presentation(
            PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: true,
                ..PresentationRequest::default()
            },
            &capabilities,
        )
        .unwrap();

        assert_eq!(selected.format, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert_eq!(selected.color_space, wgpu::SurfaceColorSpace::Auto);
        assert_eq!(selected.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            selected
                .resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("not with an sRGB")
        );
    }

    #[test]
    fn eight_bit_request_keeps_the_srgb_surface() {
        let capabilities = surface_capabilities(vec![wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB,
        }]);
        let selected =
            select_surface_presentation(PresentationRequest::default(), &capabilities).unwrap();

        assert_eq!(selected.format, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert_eq!(selected.resolved.resolved, PresentationDepth::Sdr8);
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
        let Some(gpu) = crate::testing::headless_gpu() else {
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

    #[test]
    fn headless_context_enables_rgba16_unorm_when_adapter_supports_it() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        let renderable = gpu.adapter.features().contains(features)
            && gpu
                .adapter
                .get_texture_format_features(wgpu::TextureFormat::Rgba16Unorm)
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
        if renderable {
            assert!(
                gpu.device.features().contains(features),
                "the device must request RGBA16 normalized texture support"
            );
        }
    }
}
