//! Outputs and the sinks they deliver through.
//!
//! An [`Output`] owns everything about the picture: the surfaces assigned to
//! it, warp, edge blend, calibration, rotation, its tonemap override and its
//! presentation. Where the finished picture goes is its sink, supplied by a
//! provider: a window, a recording, a stream, an NDI sender, a Syphon server.
//! Nothing outside a sink's own module names a sink type.

pub mod compose;
pub mod ffmpeg;
pub mod registry;
pub mod share;
pub mod unavailable;
pub mod window;

pub use compose::{Content, Output, RenderedFrame};
pub use registry::SinkRegistry;
pub use unavailable::UnavailableSink;

pub use crate::engine::value::provider::{
    ControlKind, ControlSpec, ControlStatus, ControlValue, LibraryCreate, LibraryEntry,
    LibraryNotice, LibrarySection, ProviderConfig, ProviderTypeSnapshot, WidgetHint,
};

use crate::engine::value::render::{
    ModeAvailability, PresentationDepth, PresentationPixelFormat, PresentationRequest,
    ResolvedPresentation, Unassigned,
};
use crate::renderer::context::GpuContext;
use crate::renderer::{ReadbackFormat, ReadbackFrame};
use crate::source::{ControlError, Services};
use anyhow::Result;

/// An output sink as it is saved and sent: a type id plus that type's fields.
pub type SinkConfig = ProviderConfig;

/// How a sink takes the finished picture. Declared so the output loop can batch
/// readbacks and never reads back a frame a sink publishes from the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    /// The sink hands the output a surface texture to draw into and presents
    /// it: an OS window.
    Present,
    /// The sink takes the finished texture on the GPU: Syphon, Spout.
    Gpu,
    /// The sink converts the finished texture on the GPU and reads its own
    /// result back: NDI.
    Converted,
    /// The output reads the finished texture back and hands the sink each
    /// frame: recordings and streams.
    Readback,
}

/// What a frame handed to a sink came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    Ok,
    /// The sink cannot take more; the output stops with this message.
    Failed(String),
    /// The sink ended its session and must be started again (a stream listener
    /// whose client disconnected).
    Restart,
}

/// What writing a sink setting did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamEffect {
    /// Applied in place.
    Applied,
    /// The sink must be rebuilt from its new config.
    Rebuild,
}

/// A presentation a sink resolved a request to, and every mode with the
/// reason it cannot deliver it.
#[derive(Debug, Clone)]
pub struct Presentation {
    pub resolved: ResolvedPresentation,
    pub modes: Vec<ModeAvailability>,
}

/// What a sink needs to start delivering.
pub struct SinkStart {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub request: PresentationRequest,
    pub readback_format: ReadbackFormat,
    /// Audio to carry, when the sink asked for a device and it opened.
    pub audio: Option<crate::delivery::AudioInput>,
}

/// The finished frame, as a GPU sink sees it.
pub struct SinkFrame<'a> {
    pub gpu: &'a GpuContext,
    pub services: &'a mut Services,
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub request: PresentationRequest,
    pub resolved: &'a ResolvedPresentation,
    pub fps: u32,
}

/// What a provider reads to build or describe sinks.
pub struct SinkQuery<'a> {
    pub services: &'a Services,
}

/// What a provider changes to build a sink.
pub struct SinkEnv<'a> {
    pub gpu: &'a GpuContext,
    pub services: &'a mut Services,
    /// The render resolution, which every sink but a window delivers at.
    pub width: u32,
    pub height: u32,
}

impl SinkEnv<'_> {
    /// The read-only view of this environment.
    pub fn query(&self) -> SinkQuery<'_> {
        SinkQuery {
            services: self.services,
        }
    }
}

/// A kind of output sink. Registered once for the process.
pub trait OutputSinkProvider: 'static {
    /// Stable id: the `type` tag `stage.json` saves. Never renamed.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// A short glyph shown next to the label.
    fn icon(&self) -> &'static str;

    /// The settings every sink of this type has.
    fn params(&self) -> &'static [ControlSpec] {
        &[]
    }

    /// `Err(reason)` when this build or host cannot create this type.
    ///
    /// # Errors
    ///
    /// Returns the reason the type cannot be created.
    fn availability(&self, _query: &SinkQuery) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Whether the output panel offers the type.
    fn listed(&self, query: &SinkQuery) -> bool {
        self.availability(query).is_ok()
    }

    /// What can be targeted: monitors for a display, nothing for a stream.
    fn library(&self, _query: &SinkQuery) -> LibrarySection {
        LibrarySection::default()
    }

    /// Run a library action, such as `rescan`.
    ///
    /// # Errors
    ///
    /// Fails for an action this type does not offer.
    fn library_action(&mut self, action: &str, _env: &mut SinkEnv) -> Result<()> {
        anyhow::bail!("'{}' outputs have no library action '{action}'", self.id())
    }

    /// The config a new output of this type starts from.
    fn default_config(&self) -> SinkConfig {
        SinkConfig::new(self.id())
    }

    /// Build a sink from its config.
    ///
    /// # Errors
    ///
    /// Fails when the config is malformed.
    fn create(
        &mut self,
        config: &SinkConfig,
        env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>>;

    /// Once per frame, before any output renders.
    fn tick(&mut self, _env: &mut SinkEnv) {}
}

/// One output's sink.
pub trait OutputSinkInstance: 'static {
    /// The id of the provider that built it.
    fn sink_type(&self) -> &str;

    /// What the output panel says the sink is sending to.
    fn label(&self) -> String;

    /// The config that rebuilds this sink as it is now.
    fn config(&self) -> SinkConfig;

    /// Take `config` in place when it names the same kind of sink, as a window
    /// moving to another monitor does. `false` means rebuild instead.
    fn patch(&mut self, _config: &SinkConfig, _services: &Services) -> bool {
        false
    }

    fn frame_path(&self) -> FramePath;

    /// What the output shows with no surfaces assigned, unless the user chose.
    fn default_unassigned(&self) -> Unassigned {
        Unassigned::Program
    }

    /// The size the output renders at, given the render resolution.
    fn size(&self, render: (u32, u32)) -> (u32, u32) {
        render
    }

    /// Whether the sink can take a frame now. A window waiting for the event
    /// loop to create it cannot.
    fn is_ready(&self) -> bool {
        true
    }

    /// Resolve `request` against what this sink can carry, and reconfigure for
    /// the result.
    ///
    /// # Errors
    ///
    /// Fails when nothing can be delivered at all (a window surface with no
    /// SDR format).
    fn configure(
        &mut self,
        gpu: &GpuContext,
        query: &SinkQuery,
        request: PresentationRequest,
    ) -> Result<Presentation>;

    /// The format the output's final pass writes: the delivered texture, or a
    /// window's surface.
    fn target_format(&self, resolved: &ResolvedPresentation) -> wgpu::TextureFormat {
        storage_formats(resolved).0
    }

    /// The format surfaces are composed in before the final pass.
    fn intermediate_format(&self, resolved: &ResolvedPresentation) -> wgpu::TextureFormat {
        if resolved.resolved == PresentationDepth::Sdr10 || resolved.transfer.is_hdr() {
            crate::renderer::context::COLOR_PATH_FORMAT
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        }
    }

    /// Whether the sink is started and stopped. A window always shows.
    fn startable(&self) -> bool {
        true
    }

    /// Settle the presentation a start will deliver before anything is
    /// allocated for it (a recording probes its encoder).
    ///
    /// # Errors
    ///
    /// Fails when the sink cannot start with this request.
    fn prepare_start(
        &mut self,
        _gpu: &GpuContext,
        _request: PresentationRequest,
    ) -> Result<Option<ResolvedPresentation>> {
        Ok(None)
    }

    /// Begin delivering. May answer with the presentation it settled on.
    ///
    /// # Errors
    ///
    /// Fails when the sink cannot start (ffmpeg missing, a port taken).
    fn start(&mut self, _start: SinkStart) -> Result<Option<ResolvedPresentation>> {
        Ok(None)
    }

    /// Stop delivering and release what the session held.
    fn stop(&mut self) {}

    /// The audio device this sink carries, if any.
    fn audio_device(&self) -> Option<&str> {
        None
    }

    /// Whether a change of render resolution stops the sink. An encoder opened
    /// at a fixed size must stop; a sender that carries its size per frame
    /// need not.
    fn restart_on_resize(&self) -> bool {
        false
    }

    /// A surface texture to draw into, for [`FramePath::Present`].
    fn acquire(&mut self, _gpu: &GpuContext) -> Option<wgpu::SurfaceTexture> {
        None
    }

    /// Show a frame drawn into [`Self::acquire`]'s texture.
    fn present(&mut self, _gpu: &GpuContext, frame: wgpu::SurfaceTexture) {
        drop(frame);
    }

    /// Record GPU work on the finished texture before submission, for
    /// [`FramePath::Converted`].
    ///
    /// # Errors
    ///
    /// Fails when the conversion cannot run; the output stops.
    fn encode(
        &mut self,
        _frame: &mut SinkFrame,
        _encoder: &mut wgpu::CommandEncoder,
    ) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Take the submitted frame, for [`FramePath::Gpu`] and
    /// [`FramePath::Converted`].
    ///
    /// # Errors
    ///
    /// Fails when the frame cannot be sent; the output stops.
    fn publish(&mut self, _frame: &mut SinkFrame) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Take a read-back frame, for [`FramePath::Readback`].
    fn deliver(&mut self, _frame: ReadbackFrame) -> Delivered {
        Delivered::Ok
    }

    /// Encoder health while an encoder runs.
    fn encoder_health(&self) -> Option<crate::delivery::EncoderHealth> {
        None
    }

    /// Audio passthrough health while audio is carried.
    fn audio_health(&self) -> Option<crate::delivery::AudioHealth> {
        None
    }

    /// How long the running session has been sending, by the sink's own clock.
    fn duration(&self) -> Option<std::time::Duration> {
        None
    }

    /// The settings this sink has. The same for every sink of a type.
    fn schema(&self) -> &'static [ControlSpec] {
        &[]
    }

    /// Current value of a setting.
    fn param(&self, _name: &str) -> Option<ControlValue> {
        None
    }

    /// Write a setting.
    ///
    /// # Errors
    ///
    /// Fails for an unknown setting or a value of the wrong shape.
    fn set_param(
        &mut self,
        name: &str,
        _value: &ControlValue,
    ) -> std::result::Result<ParamEffect, ControlError> {
        Err(ControlError::Unknown(name.to_string()))
    }

    /// Fire an action.
    ///
    /// # Errors
    ///
    /// Fails for an unknown action.
    fn trigger(&mut self, name: &str) -> std::result::Result<(), ControlError> {
        Err(ControlError::Unknown(name.to_string()))
    }

    /// The sink's settings and anything live, for snapshots.
    fn status(&self) -> ControlStatus {
        let mut status = ControlStatus::default();
        for spec in self.schema() {
            if let Some(value) = self.param(&spec.name) {
                status.params.insert(spec.name.clone(), value);
            }
        }
        status
    }

    /// The OS window this sink still needs, if it draws into one that the
    /// event loop has not created yet. The window host creates it and hands
    /// it to [`Self::attach_window`].
    fn window_request(&self) -> Option<winit::window::WindowAttributes> {
        None
    }

    /// Take the window [`Self::window_request`] asked for.
    ///
    /// # Errors
    ///
    /// Fails when no surface can be created for it.
    fn attach_window(
        &mut self,
        _gpu: &GpuContext,
        _services: &Services,
        _window: winit::window::Window,
    ) -> Result<()> {
        anyhow::bail!("'{}' outputs draw into no window", self.sink_type())
    }

    /// Something the performer should be told about the sink, once: a display
    /// that opened floating because its monitor was gone.
    fn take_notice(&mut self) -> Option<String> {
        None
    }

    /// The output was renamed; a window shows it in its title.
    fn set_title(&mut self, _title: &str) {}

    /// The OS window this sink shows in, for routing window events.
    fn window_id(&self) -> Option<winit::window::WindowId> {
        None
    }

    /// The OS window was resized.
    fn window_resized(&mut self, _gpu: &GpuContext, _width: u32, _height: u32) {}

    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// The texture and readback formats a resolved presentation delivers in, for
/// every sink that is not a window.
pub fn storage_formats(
    presentation: &ResolvedPresentation,
) -> (wgpu::TextureFormat, ReadbackFormat) {
    match presentation.pixel_format {
        PresentationPixelFormat::Rgba16 => (
            wgpu::TextureFormat::Rgba16Unorm,
            ReadbackFormat::Rgba16Unorm,
        ),
        PresentationPixelFormat::P216 => (wgpu::TextureFormat::Rgba16Float, ReadbackFormat::P216),
        _ if presentation.resolved == PresentationDepth::Sdr10 => {
            (wgpu::TextureFormat::Rgb10a2Unorm, ReadbackFormat::Rgb10A2)
        }
        _ => (wgpu::TextureFormat::Rgba8UnormSrgb, ReadbackFormat::Rgba8),
    }
}

/// Downcast a sink to its concrete type.
pub fn downcast_mut<T: 'static>(sink: &mut dyn OutputSinkInstance) -> Option<&mut T> {
    sink.as_any_mut().downcast_mut::<T>()
}

/// Downcast a sink to its concrete type.
pub fn downcast_ref<T: 'static>(sink: &dyn OutputSinkInstance) -> Option<&T> {
    sink.as_any().downcast_ref::<T>()
}
